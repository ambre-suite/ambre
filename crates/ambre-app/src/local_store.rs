//! The local store as the Main thread drives it (`ambre-spec-residency.md`
//! section 3).
//!
//! It performs the writes Logic sends, and runs the ledger that keeps the
//! store within its budget. It meets a write the browser refuses for its
//! quota: it frees what the relay holds and retries once. A change the relay
//! has not acknowledged that still does not fit pauses editing, and is
//! retried until it lands or the relay acknowledges it.
//!
//! Each snapshot and each delta reaches the store sealed by the Sync worker,
//! and opens through it on the way back (`RESID-025`). The writes keep the
//! order Logic made them in while their seals are on the way.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ambre_bus::{DocType, RowPlace, SealedRow};
use ambre_core::id::{Document, Id};
use ambre_core::{Count, NarrowCount};
use ambre_logic::app::LogicCommand;
use ambre_logic::persistence::{PersistenceStore, Stored};
use ambre_logic::seal_queue::SealQueue;
use ambre_logic::store_ledger::{QuotaAttempt, Refusal, RowChange, StoreLedger};
use ambre_logic::{PersistWrite, SealedWrite};

use crate::vault::Vault;

/// How many frames a write held for the quota waits before it is tried
/// again: about two seconds at 60 Hz.
const RETRY_FRAMES: Count<HeldFrames> = Count::new(120);

/// The frames drawn since the held writes were last tried.
enum HeldFrames {}

impl NarrowCount for HeldFrames {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("HeldFrames");
}

/// Which try of one write this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attempt {
    /// The write as Logic sent it.
    First,
    /// The one retry after the store freed what the relay holds
    /// (`RESID-017`).
    Retry,
    /// A write held while editing is paused (`RESID-018`).
    Held,
}

/// When the store reads a document's copy back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadBack {
    /// At boot: a store with no copy hands nothing.
    Boot,
    /// Logic asked for the copy, after the writes queued before it.
    Requested,
}

/// The store, its ledger and the writes the quota holds back.
#[derive(Clone)]
pub(crate) struct LocalStore {
    inner: Rc<Inner>,
}

struct Inner {
    store: PersistenceStore,
    ledger: RefCell<StoreLedger>,
    /// Writes of a change the relay has not acknowledged, which the browser
    /// still refuses. Each is retried until it lands.
    held: RefCell<Vec<(Id<Document>, SealedWrite)>>,
    /// Frames since the held writes were last tried.
    frames: Cell<Count<HeldFrames>>,
    /// The launcher's outbox, which holds what a full queue refuses
    /// (`BUS-011`).
    to_logic: crate::outbox::ShellOutbox,
    vault: Vault,
    /// The writes on their way to the store, behind their seals.
    queue: RefCell<SealQueue>,
    /// The document the page opened on, restored once a session holds the
    /// key.
    boot: Cell<Option<(DocType, Id<Document>)>>,
}

impl LocalStore {
    /// Wrap an open store: read back what the ledger knew before the
    /// reload, and ask the browser to keep the store (`RESID-020`).
    pub(crate) async fn open(
        store: PersistenceStore,
        to_logic: crate::outbox::ShellOutbox,
        vault: Vault,
        boot: Option<(DocType, Id<Document>)>,
    ) -> Self {
        PersistenceStore::request_persistent();
        let mut ledger = StoreLedger::default();
        for (id, meta) in store.load_meta().await.unwrap_or_default() {
            ledger.restore(id, meta);
        }
        Self {
            inner: Rc::new(Inner {
                store,
                ledger: RefCell::new(ledger),
                held: RefCell::new(Vec::new()),
                frames: Cell::new(Count::ZERO),
                to_logic,
                vault,
                queue: RefCell::new(SealQueue::default()),
                boot: Cell::new(boot),
            }),
        }
    }

    /// The store itself, for the reads and the trusted keys.
    pub(crate) fn store(&self) -> &PersistenceStore {
        &self.inner.store
    }

    /// A session holds the key: the page's document comes back from its
    /// copy, if the store holds one. Before it, the copy cannot open.
    pub(crate) fn signed_in(&self) {
        if let Some((doc_type, document_id)) = self.inner.boot.take() {
            self.read_back(document_id, doc_type, ReadBack::Boot);
        }
    }

    /// Take one write or signal from Logic. A snapshot or a delta waits for
    /// its seal, and every write behind it waits its turn.
    pub(crate) fn apply(&self, id: Id<Document>, write: PersistWrite) {
        let seal = self.inner.queue.borrow_mut().push(id, write);
        let Some((ticket, plaintext)) = seal else {
            self.flush();
            return;
        };
        let this = self.clone();
        self.inner
            .vault
            .seal(RowPlace::Document(id), plaintext, move |sealed| {
                if sealed.is_none() {
                    tracing::warn!("Persistence: no session seals the copy, the write is dropped");
                }
                this.inner.queue.borrow_mut().sealed(ticket, sealed);
                this.flush();
            });
    }

    /// Perform the writes whose turn came.
    fn flush(&self) {
        let ready = self.inner.queue.borrow_mut().ready();
        for (id, write) in ready {
            self.land(id, write);
        }
    }

    /// Perform one write or signal, its row sealed.
    fn land(&self, id: Id<Document>, write: SealedWrite) {
        let observed = self.inner.ledger.borrow_mut().observe(id, &write);
        for gone in &observed.evict {
            self.inner.store.delete_snapshot(*gone);
        }
        if observed.record == RowChange::Changed
            && let Some(meta) = self.inner.ledger.borrow().meta(id)
        {
            self.inner.store.save_meta(id, &meta);
        }
        match write {
            PersistWrite::Snapshot(_) | PersistWrite::Delta(_) => {
                self.write(id, write, Attempt::First);
            }
            PersistWrite::Session { peer, nonce } => self.inner.store.save_session(id, peer, nonce),
            PersistWrite::ForgetSession { peer } => self.inner.store.forget_session(id, peer),
            // The relay holds every change: what the quota held back is a
            // copy it can serve again.
            PersistWrite::Sent => self.inner.held.borrow_mut().retain(|(held, _)| *held != id),
            // Read after every write queued before it, so the copy holds what
            // the document had when it left (`RESID-010`).
            PersistWrite::ReadBack { doc_type } => {
                self.read_back(id, doc_type, ReadBack::Requested);
            }
            PersistWrite::Shown | PersistWrite::Hidden => {}
            // The copy leaves whole, with what the quota held back of it
            // (`SYNC-233`).
            PersistWrite::Erase => {
                self.inner.held.borrow_mut().retain(|(held, _)| *held != id);
                self.inner.store.delete_snapshot(id);
            }
        }
    }

    /// Read the document's copy back, open it, and hand it to Logic. A copy
    /// with a row that does not open leaves the store, and Logic receives
    /// none (`RESID-027`). At boot, a store that holds no copy hands
    /// nothing: the open fetches the document anyway.
    fn read_back(&self, document_id: Id<Document>, doc_type: DocType, moment: ReadBack) {
        let this = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let store = &this.inner.store;
            let snapshot = store.load_snapshot(document_id).await.ok().flatten();
            let deltas = store.load_deltas(document_id).await.unwrap_or_default();
            let sessions = store.load_sessions(document_id).await.unwrap_or_default();
            let has_snapshot = snapshot.is_some();
            let rows: Vec<SealedRow> = snapshot
                .into_iter()
                .chain(deltas)
                .map(|sealed| SealedRow {
                    place: RowPlace::Document(document_id),
                    sealed,
                })
                .collect();
            if moment == ReadBack::Boot && rows.is_empty() {
                return;
            }
            let opened_by = this.clone();
            this.inner.vault.open(rows, move |opened| {
                // Each row moves out of its wrapper with no copy, once every
                // row opened. A row that did not open leaves the others under
                // their wrappers, which wipe them.
                let copy = opened.map(|rows| {
                    rows.into_iter().collect::<Option<Vec<_>>>().map(|rows| {
                        rows.into_iter()
                            .map(ambre_core::Confidential::<ambre_core::Plaintext>::into_plaintext)
                            .collect::<Vec<_>>()
                    })
                });
                let (snapshot, deltas, sessions) = match copy {
                    Some(Some(mut rows)) => {
                        let snapshot = has_snapshot.then(|| rows.remove(0));
                        (snapshot, rows, sessions)
                    }
                    Some(None) => {
                        tracing::warn!("Persistence: a copy did not open, it leaves the store");
                        opened_by.forget(document_id);
                        (None, Vec::new(), Vec::new())
                    }
                    // No session holds the key: the copy stays for the next.
                    None => (None, Vec::new(), Vec::new()),
                };
                if moment == ReadBack::Boot && snapshot.is_none() && deltas.is_empty() {
                    return;
                }
                opened_by
                    .inner
                    .to_logic
                    .send(LogicCommand::RestoreLocalSnapshot {
                        document_id,
                        doc_type,
                        snapshot: snapshot.map(Into::into),
                        deltas: deltas.into_iter().map(Into::into).collect(),
                        sessions,
                    });
            });
        });
    }

    /// Drop the document's copy from the store and from the ledger.
    fn forget(&self, id: Id<Document>) {
        self.inner.store.delete_snapshot(id);
        self.inner.ledger.borrow_mut().forget(id);
    }

    /// Try the held writes again, every [`RETRY_FRAMES`] frames.
    pub(crate) fn tick(&self) {
        let frames = self.inner.frames.get().incremented();
        if frames < RETRY_FRAMES {
            self.inner.frames.set(frames);
            return;
        }
        self.inner.frames.set(Count::ZERO);
        let held = std::mem::take(&mut *self.inner.held.borrow_mut());
        for (id, write) in held {
            self.write(id, write, Attempt::Held);
        }
    }

    /// Write a snapshot or a delta, and meet its outcome once it lands.
    fn write(&self, id: Id<Document>, write: SealedWrite, attempt: Attempt) {
        let landed = match &write {
            PersistWrite::Snapshot(bytes) => self.inner.store.save_snapshot(id, bytes),
            PersistWrite::Delta(bytes) => self.inner.store.save_delta(id, bytes),
            _ => return,
        };
        let this = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match landed.await {
                Ok(Stored::Quota) => this.refused(id, write, attempt),
                Ok(Stored::Done) if attempt == Attempt::Held => this.resumed(id),
                _ => {}
            }
        });
    }

    /// Meet a write the browser refused for its quota (`RESID-017`,
    /// `RESID-018`).
    fn refused(&self, id: Id<Document>, write: SealedWrite, attempt: Attempt) {
        let need = match &write {
            PersistWrite::Snapshot(bytes) | PersistWrite::Delta(bytes) => bytes.len(),
            _ => ambre_core::Count::<ambre_core::Octets>::new(0),
        };
        let decision = self.inner.ledger.borrow_mut().refused(
            id,
            need,
            if attempt == Attempt::First {
                QuotaAttempt::First
            } else {
                QuotaAttempt::Retried
            },
        );
        match decision {
            Refusal::Retry { evict } => {
                for gone in &evict {
                    self.inner.store.delete_snapshot(*gone);
                }
                self.write(id, write, Attempt::Retry);
            }
            Refusal::Full => {
                self.inner.held.borrow_mut().push((id, write));
                if attempt != Attempt::Held {
                    self.tell_logic(id, ambre_bus::StoreRoom::Full);
                }
            }
            Refusal::Drop => {
                tracing::warn!("Persistence: the quota dropped a copy the relay holds");
            }
        }
    }

    /// A held write landed: once none is left for its document, editing
    /// resumes (`RESID-018`).
    fn resumed(&self, id: Id<Document>) {
        if !self.inner.held.borrow().iter().any(|(held, _)| *held == id) {
            self.tell_logic(id, ambre_bus::StoreRoom::Free);
        }
    }

    /// Tell Logic whether the store refuses the document's unsent change.
    fn tell_logic(&self, document_id: Id<Document>, room: ambre_bus::StoreRoom) {
        let sent = self
            .inner
            .to_logic
            .send(LogicCommand::LocalStoreFull { document_id, room });
        if !sent.accepted() {
            tracing::warn!("Persistence: Logic did not take the store state");
        }
    }
}

#[cfg(test)]
mod tests {
    use ambre_bus::conduit::{self, Receiver};
    use ambre_bus::{Confidential, Plaintext, Request, RestCommand, RestEvent};
    use ambre_core::Seq;
    use ambre_core::{Payload, SealedRowBytes};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    async fn sleep(ms: i32) {
        let wait = js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(wait).await;
    }

    /// The next command on `rx`, once one comes.
    async fn next(rx: &Receiver<LogicCommand>) -> LogicCommand {
        for _ in 0..200 {
            if let Ok(cmd) = rx.try_recv() {
                return cmd;
            }
            sleep(10).await;
        }
        panic!("nothing came");
    }

    /// The next service request the vault sent.
    async fn request(rx: &Receiver<LogicCommand>) -> (Seq<Request>, RestCommand) {
        match next(rx).await {
            LogicCommand::Rest { id, cmd } => (id, cmd),
            other => panic!("expected a service request, got {other:?}"),
        }
    }

    /// What Logic receives for one read back.
    async fn restored(rx: &Receiver<LogicCommand>) -> (Option<Vec<u8>>, Vec<Vec<u8>>) {
        match next(rx).await {
            LogicCommand::RestoreLocalSnapshot {
                snapshot, deltas, ..
            } => (
                snapshot.map(Payload::<ambre_core::SnapshotBytes>::into_bytes),
                deltas
                    .into_iter()
                    .map(Payload::<ambre_core::DeltaBytes>::into_bytes)
                    .collect(),
            ),
            other => panic!("expected a restore, got {other:?}"),
        }
    }

    /// A copy reaches the store sealed: the bytes the store holds are the
    /// worker's, and the plaintext comes back only through its open. A copy
    /// whose row does not open leaves the store, and Logic receives none
    /// (`RESID-025`, `RESID-027`).
    #[wasm_bindgen_test]
    async fn a_copy_lands_sealed_and_one_that_does_not_open_leaves() {
        let store = PersistenceStore::open().await.unwrap().unwrap();
        let (logic_tx, logic_rx) =
            conduit::local(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<8>());
        let (vault_tx, vault_rx) =
            conduit::local(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<8>());
        let local = LocalStore::open(
            store.clone(),
            crate::outbox::ShellOutbox::new(logic_tx),
            Vault::new(crate::outbox::ShellOutbox::new(vault_tx.clone())),
            None,
        )
        .await;
        // The answers reach the vault as the switchboard hands them.
        let vault = local.inner.vault.clone();
        let doc = Id::<Document>::mint();

        local.apply(
            doc,
            PersistWrite::Snapshot(Payload::<ambre_core::SnapshotBytes>::new(
                b"the document".to_vec(),
            )),
        );
        local.apply(
            doc,
            PersistWrite::Delta(Payload::<ambre_core::DeltaBytes>::new(b"an edit".to_vec())),
        );
        let (first, cmd) = request(&vault_rx).await;
        let RestCommand::SealLocalRow { place, plaintext } = cmd else {
            panic!("expected a seal");
        };
        assert_eq!(place, RowPlace::Document(doc));
        assert_eq!(plaintext.into_plaintext(), b"the document");
        let (second, _) = request(&vault_rx).await;
        // The delta's seal answers first: the store still waits for the
        // snapshot's, so the delta never lands under it.
        vault.answer(
            Some(second),
            RestEvent::LocalRowSealed {
                sealed: Payload::<SealedRowBytes>::new(b"sealed edit".to_vec()),
            },
        );
        vault.answer(
            Some(first),
            RestEvent::LocalRowSealed {
                sealed: Payload::<SealedRowBytes>::new(b"sealed document".to_vec()),
            },
        );
        assert_eq!(
            store.load_snapshot(doc).await.unwrap(),
            Some(Payload::<SealedRowBytes>::new(b"sealed document".to_vec()))
        );
        assert_eq!(
            store.load_deltas(doc).await.unwrap(),
            [Payload::<SealedRowBytes>::new(b"sealed edit".to_vec())]
        );

        // A read back asks the worker to open both rows, in order.
        local.apply(
            doc,
            PersistWrite::ReadBack {
                doc_type: DocType::Prose,
            },
        );
        let (id, cmd) = request(&vault_rx).await;
        let RestCommand::OpenLocalRows { rows } = cmd else {
            panic!("expected an open");
        };
        let sealed: Vec<&[u8]> = rows.iter().map(|row| row.sealed.as_bytes()).collect();
        assert_eq!(sealed, [b"sealed document".as_slice(), b"sealed edit"]);
        vault.answer(
            Some(id),
            RestEvent::LocalRowsOpened {
                rows: vec![
                    Some(Confidential::<Plaintext>::new(b"the document".to_vec())),
                    Some(Confidential::<Plaintext>::new(b"an edit".to_vec())),
                ],
            },
        );
        assert_eq!(
            restored(&logic_rx).await,
            (Some(b"the document".to_vec()), vec![b"an edit".to_vec()])
        );

        // One row that does not open: the copy leaves.
        local.apply(
            doc,
            PersistWrite::ReadBack {
                doc_type: DocType::Prose,
            },
        );
        let (id, _) = request(&vault_rx).await;
        vault.answer(
            Some(id),
            RestEvent::LocalRowsOpened {
                rows: vec![
                    Some(Confidential::<Plaintext>::new(b"the document".to_vec())),
                    None,
                ],
            },
        );
        assert_eq!(restored(&logic_rx).await, (None, Vec::new()));
        assert_eq!(store.load_snapshot(doc).await.unwrap(), None);
        assert!(store.load_deltas(doc).await.unwrap().is_empty());
        drop(vault_tx);
    }
}
