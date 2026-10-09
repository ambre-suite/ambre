//! The local store as the Main thread drives it (`ambre-spec-residency.md`
//! section 3).
//!
//! It performs the writes Logic sends, and runs the ledger that keeps the
//! store within its budget. It meets a write the browser refuses for its
//! quota: it frees what the relay holds and retries once. A change the relay
//! has not acknowledged that still does not fit pauses editing, and is
//! retried until it lands or the relay acknowledges it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ambre_bus::conduit::Sender;
use ambre_bus::{DocumentId, LogicCommand};
use ambre_logic::PersistWrite;
use ambre_logic::persistence::{PersistenceStore, Stored};
use ambre_logic::store_ledger::{Refusal, StoreLedger};

/// How many frames a write held for the quota waits before it is tried
/// again: about two seconds at 60 Hz.
const RETRY_FRAMES: u32 = 120;

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
    held: RefCell<Vec<(DocumentId, PersistWrite)>>,
    /// Frames since the held writes were last tried.
    frames: Cell<u32>,
    to_logic: Sender<LogicCommand>,
}

impl LocalStore {
    /// Wrap an open store: read back what the ledger knew before the
    /// reload, and ask the browser to keep the store (`RESID-020`).
    pub(crate) async fn open(store: PersistenceStore, to_logic: Sender<LogicCommand>) -> Self {
        PersistenceStore::request_persistent();
        let mut ledger = StoreLedger::default();
        for (id, meta) in store.load_meta().await.unwrap_or_default() {
            ledger.restore(DocumentId::from_bytes(id), meta);
        }
        Self {
            inner: Rc::new(Inner {
                store,
                ledger: RefCell::new(ledger),
                held: RefCell::new(Vec::new()),
                frames: Cell::new(0),
                to_logic,
            }),
        }
    }

    /// The store itself, for the reads and the trusted keys.
    pub(crate) fn store(&self) -> &PersistenceStore {
        &self.inner.store
    }

    /// Perform one write or signal from Logic. A read back is the caller's.
    pub(crate) fn apply(&self, id: DocumentId, write: PersistWrite) {
        let observed = self.inner.ledger.borrow_mut().observe(id, &write);
        for gone in &observed.evict {
            self.inner.store.delete_snapshot(*gone.as_bytes());
        }
        if observed.record
            && let Some(meta) = self.inner.ledger.borrow().meta(id)
        {
            self.inner.store.save_meta(*id.as_bytes(), &meta);
        }
        let key = *id.as_bytes();
        match write {
            PersistWrite::Snapshot(_) | PersistWrite::Delta(_) => {
                self.write(id, write, Attempt::First);
            }
            PersistWrite::Session { peer, nonce } => {
                self.inner.store.save_session(key, peer, nonce)
            }
            PersistWrite::ForgetSession { peer } => self.inner.store.forget_session(key, peer),
            // The relay holds every change: what the quota held back is a
            // copy it can serve again.
            PersistWrite::Sent => self.inner.held.borrow_mut().retain(|(held, _)| *held != id),
            PersistWrite::Shown | PersistWrite::Hidden | PersistWrite::ReadBack { .. } => {}
        }
    }

    /// Try the held writes again, every [`RETRY_FRAMES`] frames.
    pub(crate) fn tick(&self) {
        let frames = self.inner.frames.get().saturating_add(1);
        if frames < RETRY_FRAMES {
            self.inner.frames.set(frames);
            return;
        }
        self.inner.frames.set(0);
        let held = std::mem::take(&mut *self.inner.held.borrow_mut());
        for (id, write) in held {
            self.write(id, write, Attempt::Held);
        }
    }

    /// Write a snapshot or a delta, and meet its outcome once it lands.
    fn write(&self, id: DocumentId, write: PersistWrite, attempt: Attempt) {
        let key = *id.as_bytes();
        let landed = match &write {
            PersistWrite::Snapshot(bytes) => self.inner.store.save_snapshot(key, bytes),
            PersistWrite::Delta(bytes) => self.inner.store.save_delta(key, bytes),
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
    fn refused(&self, id: DocumentId, write: PersistWrite, attempt: Attempt) {
        let need = match &write {
            PersistWrite::Snapshot(bytes) | PersistWrite::Delta(bytes) => bytes.len(),
            _ => 0,
        };
        let need = u64::try_from(need).unwrap_or(u64::MAX);
        let decision = self
            .inner
            .ledger
            .borrow_mut()
            .refused(id, need, attempt != Attempt::First);
        match decision {
            Refusal::Retry { evict } => {
                for gone in &evict {
                    self.inner.store.delete_snapshot(*gone.as_bytes());
                }
                self.write(id, write, Attempt::Retry);
            }
            Refusal::Full => {
                self.inner.held.borrow_mut().push((id, write));
                if attempt != Attempt::Held {
                    self.tell_logic(id, true);
                }
            }
            Refusal::Drop => {
                tracing::warn!("Persistence: the quota dropped a copy the relay holds");
            }
        }
    }

    /// A held write landed: once none is left for its document, editing
    /// resumes (`RESID-018`).
    fn resumed(&self, id: DocumentId) {
        if !self.inner.held.borrow().iter().any(|(held, _)| *held == id) {
            self.tell_logic(id, false);
        }
    }

    /// Tell Logic whether the store refuses the document's unsent change.
    fn tell_logic(&self, document_id: DocumentId, full: bool) {
        if self
            .inner
            .to_logic
            .try_send(LogicCommand::LocalStoreFull { document_id, full })
            .is_err()
        {
            tracing::warn!("Persistence: Logic did not take the store state");
        }
    }
}
