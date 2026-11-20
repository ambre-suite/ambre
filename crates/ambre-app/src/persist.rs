//! The link from the Logic thread to the local store.
//!
//! The Main thread opens the IndexedDB store, where the event loop is live.
//! On open, the store restores the BOOT document's snapshot (if any), so an
//! edit survives a refresh without a network round-trip. The frame loop
//! drains the writes Logic emits into it, and the TOFU pin reads and writes
//! the same store.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ambre_bus::DocType;
use ambre_bus::conduit::Receiver;
use ambre_core::id::{Document, Id, User};
use ambre_logic::PersistWrite;
use ambre_logic::app::LogicCommand;
use ambre_logic::persistence::PersistenceStore;

use crate::host::DraftKeeper;
use crate::kept::{KeptInviters, Load, Proof};
use crate::local_store::LocalStore;
use crate::outbox::ShellOutbox;
use crate::vault::Vault;

/// Whether the store may still open.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Opening {
    /// The store is open, or its open is still running.
    Possible,
    /// The store will never open.
    Never,
}

/// Whether a session holds the key yet.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Session {
    /// No session has held the key on this page.
    Absent,
    /// A session of this user holds the key.
    Seen(Id<User>),
}

/// The local store once open, and the writes that wait for it.
pub(crate) struct PersistLink {
    /// The store, `None` until it opens.
    store: Rc<RefCell<Option<LocalStore>>>,
    /// Set when the store will never open, so the writes waiting for it go.
    gone: Rc<Cell<Opening>>,
    /// Set once a session holds the key: the page's document comes back from
    /// its copy only then.
    session_seen: Rc<Cell<Session>>,
    /// The store as soon as it opens, ahead of [`LocalStore`], for the read
    /// of the kept pins.
    opened: Rc<RefCell<Option<PersistenceStore>>>,
    /// Logic -> Main, carrying document snapshots and deltas.
    writes: Receiver<(Id<Document>, PersistWrite)>,
    /// The writes the store has not taken yet.
    waiting: Vec<(Id<Document>, PersistWrite)>,
    to_logic: ShellOutbox,
    /// The inviters earlier opens proved, which the store reads back and an
    /// open's proof adds to (`SYNC-238`).
    kept: KeptInviters,
}

impl PersistLink {
    /// Start opening the store, and answer the link the frame loop drains.
    pub(crate) fn open(
        keeper: DraftKeeper,
        to_logic: ShellOutbox,
        kept: KeptInviters,
        vault: Vault,
        boot_doc: Option<(DocType, Id<Document>)>,
        writes: Receiver<(Id<Document>, PersistWrite)>,
    ) -> Self {
        let link = Self {
            store: Rc::new(RefCell::new(None)),
            gone: Rc::new(Cell::new(Opening::Possible)),
            session_seen: Rc::new(Cell::new(Session::Absent)),
            opened: Rc::new(RefCell::new(None)),
            writes,
            waiting: Vec::new(),
            to_logic,
            kept: kept.clone(),
        };
        let kept_read = kept;
        let store_slot = Rc::clone(&link.store);
        let gone = Rc::clone(&link.gone);
        let full_tx = link.to_logic.clone();
        let logs_tx = link.to_logic.clone();
        let session_seen = Rc::clone(&link.session_seen);
        let opened = Rc::clone(&link.opened);
        wasm_bindgen_futures::spawn_local(async move {
            let store = match PersistenceStore::open().await {
                Ok(Ok(store)) => store,
                Ok(Err(e)) => {
                    tracing::warn!("Persistence: store unavailable ({e})");
                    gone.set(Opening::Never);
                    if let Some(user) = kept_read.user() {
                        kept_read.read(user, Vec::new());
                    }
                    return;
                }
                Err(_) => {
                    tracing::warn!("Persistence: open channel dropped");
                    gone.set(Opening::Never);
                    if let Some(user) = kept_read.user() {
                        kept_read.read(user, Vec::new());
                    }
                    return;
                }
            };
            *opened.borrow_mut() = Some(store.clone());
            // The opens wait for the pins earlier sessions of the user
            // proved, so they are read before anything else (`SYNC-240`).
            // Before a session names the user, the sign-in reads them
            // (`SYNC-244`).
            if let Some(user) = kept_read.user() {
                read_kept(&store, user, &kept_read, &logs_tx).await;
            }
            keeper.store_opened(store.clone());
            let local = LocalStore::open(store, full_tx, vault, boot_doc).await;
            // The page's document comes back from its copy once a session
            // holds the key, whichever of the two came first.
            if let Session::Seen(user) = session_seen.get() {
                local.signed_in(user);
            }
            *store_slot.borrow_mut() = Some(local);
        });
        link
    }

    /// The store slot, `None` until the store opens.
    pub(crate) fn store(&self) -> Rc<RefCell<Option<LocalStore>>> {
        Rc::clone(&self.store)
    }

    /// Persist Logic -> local store. Writes wait here until the store is
    /// open: a delta dropped would be missing from every restore until the
    /// next snapshot. Best-effort; the put() commits as the event loop idles.
    pub(crate) fn drain(&mut self) {
        // A pin an open proved is the kept one from now on, before the store
        // takes it (`SYNC-238`). A pin the store holds asks no write.
        let kept = &self.kept;
        self.waiting.extend(
            self.writes
                .try_iter()
                .filter(|(document_id, write)| match write {
                    PersistWrite::KeepInviter(pin) => kept.keep(*document_id, *pin) == Proof::Fresh,
                    PersistWrite::KeepLog(log) => {
                        kept.keep_log(*document_id, *log);
                        true
                    }
                    _ => true,
                }),
        );
        if self.gone.get() == Opening::Never {
            // A store that never opens holds no copy: each reopen waiting on
            // one is answered empty, and fetches from the relay (`RESID-012`).
            for (document_id, write) in self.waiting.drain(..) {
                if let PersistWrite::ReadBack { doc_type } = write {
                    self.to_logic.send(LogicCommand::RestoreLocalSnapshot {
                        document_id,
                        doc_type,
                        snapshot: None,
                        deltas: Vec::new(),
                        sessions: Vec::new(),
                    });
                }
            }
        }
        if let Some(local) = self.store.borrow().as_ref() {
            local.tick();
            // Each write is sealed, then passes the store's ledger, which
            // keeps it within its budget and meets the quota (`RESID-002`,
            // `RESID-025`).
            for (document_id, write) in self.waiting.drain(..) {
                local.apply(document_id, write);
            }
        }
    }

    /// A session of `user` holds the key: the page's document comes back
    /// from its copy, now or once the store opens. The store reads that
    /// user's kept pins, and no other user's (`SYNC-244`).
    pub(crate) fn signed_in(&self, user: Id<User>) {
        self.session_seen.set(Session::Seen(user));
        if self.kept.signed_in(user) == Load::Needed {
            if self.gone.get() == Opening::Never {
                self.kept.read(user, Vec::new());
            } else if let Some(store) = self.opened.borrow().clone() {
                let kept = self.kept.clone();
                let to_logic = self.to_logic.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    read_kept(&store, user, &kept, &to_logic).await;
                });
            }
        } else {
            // The same user signs in again on this page: the worker forgot
            // the session's logs, and memory holds them (`SYNC-290`).
            send_kept_logs(&self.kept, &self.to_logic);
        }
        if let Some(local) = self.store.borrow().as_ref() {
            local.signed_in(user);
        }
    }
}

/// Reads the kept logs, then the kept pins, of `user`. Each log goes to the
/// Sync worker before the pins release any open. Every open of the session
/// then walks from the head an earlier session kept (`SYNC-240`,
/// `SYNC-290`).
async fn read_kept(
    store: &PersistenceStore,
    user: Id<User>,
    kept: &KeptInviters,
    to_logic: &ShellOutbox,
) {
    kept.read_logs(user, store.load_kept_logs(user).await.unwrap_or_default());
    send_kept_logs(kept, to_logic);
    kept.read(user, store.load_inviters(user).await.unwrap_or_default());
}

/// Sends every kept log of the session's user to the Sync worker.
fn send_kept_logs(kept: &KeptInviters, to_logic: &ShellOutbox) {
    for (document_id, log) in kept.logs() {
        to_logic.send(LogicCommand::KeepLog {
            document_id,
            kept: log,
        });
    }
}
