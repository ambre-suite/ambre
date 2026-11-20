//! IndexedDB-backed persistence for Loro CRDT snapshots and deltas.
//!
//! Keeps a full Loro snapshot per document, and the deltas written since it.
//! An edit then survives a refresh straight from local storage
//! (offline-first), independent of the network. A keystroke appends its delta; a snapshot
//! replaces the document's row and drops the deltas it covers.
//!
//! Threading: every IndexedDB operation is async and resolves through a JS
//! callback, which only fires when an event loop is idle. The Logic thread
//! parks on its doorbell (Atomics-backed) and therefore starves its own event
//! loop. The store is owned and driven by the **Main thread** instead (see
//! `ambre-app`). Logic exports snapshot bytes over an `ambre_bus::conduit`
//! queue, and the Main thread persists them here. Reads/opens return a
//! one-shot receiver the Main thread awaits (never blocking the page).

use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{IdbDatabase, IdbOpenDbRequest, IdbTransactionMode};

use std::cell::RefCell;
use std::rc::Rc;

use ambre_bus::DraftKey;
use ambre_core::id::{Document, Id, User};
use ambre_core::{
    FixedBytes, OpenRange, Payload, SealedRowBytes, SessionNonce, Version, VersionKind,
};
use futures_channel::oneshot;

use crate::store_ledger::Meta;

/// The sending half of one request's answer, shared by its success and error
/// callbacks: whichever fires first answers, and the other finds it taken.
/// Both callbacks run on the Main thread, so nothing here is shared across
/// threads.
struct Answer<T>(Rc<RefCell<Option<oneshot::Sender<T>>>>);

impl<T> Clone for Answer<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T> Answer<T> {
    /// Answer, unless another callback already has. A receiver dropped
    /// meanwhile only means nobody waits for it.
    fn send(&self, value: T) {
        if let Some(tx) = self.0.borrow_mut().take() {
            let _ = tx.send(value);
        }
    }
}

/// A one-shot answer: the half the callbacks share, and the half the Main
/// thread awaits.
fn answer<T>() -> (Answer<T>, oneshot::Receiver<T>) {
    let (tx, rx) = oneshot::channel();
    (Answer(Rc::new(RefCell::new(Some(tx)))), rx)
}

/// The name of a database the store opens: the production one, or the one a
/// test opens apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DbName<'a>(&'a str);

/// The database every session opens.
const DB_NAME: DbName<'static> = DbName("ambre-persistence");

/// The versions of the database's shape. The browser runs the upgrade when
/// the version rises, and the upgrade adds every store the version holds.
/// IndexedDB reads any `u32` as a version.
enum IdbShape {}

impl VersionKind for IdbShape {
    type Repr = u32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("DbVersion");
}

impl OpenRange for IdbShape {}

// The upgrade handler creates each store only if absent, so a later version
// adds its new store without touching the ones a browser already holds.
const DB_VERSION: Version<IdbShape> = Version::new(1);

/// The object stores of the database, one variant each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum IdbStore {
    /// Each document's latest snapshot, keyed by [`IdbKey::document`].
    Snapshots,
    /// The deltas written since each document's snapshot, keyed by
    /// [`IdbKey::fresh_delta`]. A Loro import does not depend on the order it
    /// is fed, so the suffix only has to be unique.
    Deltas,
    /// This user's earlier sessions on each document, keyed by
    /// [`IdbKey::session`]. The value is the PeerID's eight little-endian
    /// bytes and then the nonce, so what a session left unsent travels at the
    /// next open (`AUTHOR-028`).
    Sessions,
    /// The trust pins. A TOFU pinned exchange key is keyed by
    /// [`IdbKey::user`]: the 32 raw bytes of the X25519 key confirmed on
    /// first invitation (`ambre-spec-key-verification.md`). The inviter an
    /// open proved is keyed by [`IdbKey::inviter`]: the 32 bytes of its pin
    /// (`SYNC-238`). No eviction of a copy reaches either.
    TrustedKeys,
    /// What the store's ledger knows of each document it holds, keyed by
    /// [`IdbKey::document`]: one [`Meta`] row, read back at boot
    /// (`RESID-008`).
    Meta,
    /// The drafts the user has not sent, one text row per channel or thread.
    /// The composer builds the key from the user (the Murmure composer spec,
    /// section 3). No document row names them, so no eviction reaches them
    /// (`ambre-spec-murmure-composer.md`).
    Drafts,
}

impl IdbStore {
    /// Every store, in the order the upgrade creates them.
    const ALL: [Self; 6] = [
        Self::Snapshots,
        Self::TrustedKeys,
        Self::Deltas,
        Self::Sessions,
        Self::Meta,
        Self::Drafts,
    ];

    /// The store's name, for the call into IndexedDB.
    const fn name(self) -> &'static str {
        match self {
            Self::Snapshots => "snapshots",
            Self::Deltas => "deltas",
            Self::Sessions => "sessions",
            Self::TrustedKeys => "trusted_keys",
            Self::Meta => "meta",
            Self::Drafts => "drafts",
        }
    }
}

/// The key of one row, as IndexedDB holds it: text. Each constructor names
/// what the row belongs to, so a user's key never stands for a document's.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct IdbKey(String);

impl IdbKey {
    /// A document's row: its id as 32 lowercase hex digits.
    fn document(id: Id<Document>) -> Self {
        Self(id.to_simple())
    }

    /// A user's row: its id as 32 lowercase hex digits.
    fn user(id: Id<User>) -> Self {
        Self(id.to_simple())
    }

    /// The row of the inviter `user` kept for `document` (`SYNC-238`,
    /// `SYNC-244`). It starts with the prefix `inviter:`, which no user's 32
    /// hex digits start with. The user's key, a colon and the document's key
    /// follow.
    /// [`IdbKey::inviters`] and [`IdbKey::inviter_of`] read the same prefix.
    fn inviter(user: Id<User>, document: Id<Document>) -> Self {
        Self(format!(
            "inviter:{}:{}",
            Self::user(user),
            Self::document(document)
        ))
    }

    /// Every key [`IdbKey::inviter`] writes for `user`, and no other: from
    /// the prefix up to the character after its last colon. A row an older
    /// build keyed by the document alone falls outside every user's range.
    fn inviters(user: Id<User>) -> Result<web_sys::IdbKeyRange, JsValue> {
        let user = Self::user(user);
        web_sys::IdbKeyRange::bound(
            &JsValue::from_str(&format!("inviter:{user}:")),
            &JsValue::from_str(&format!("inviter:{user};")),
        )
    }

    /// The document whose kept inviter IndexedDB keyed `key` for `user`, as
    /// [`IdbKey::inviter`] wrote it. `None` for any other key, another
    /// user's included.
    fn inviter_of(user: Id<User>, key: &JsValue) -> Option<Id<Document>> {
        let text = key.as_string()?;
        let rest = text.strip_prefix("inviter:")?;
        let rest = rest.strip_prefix(Self::user(user).0.as_str())?;
        Self::document_of(rest.strip_prefix(':')?)
    }

    /// The row of the log `user` kept for `document` (`SYNC-290`). It
    /// starts with the prefix `log:`, which no user's 32 hex digits start
    /// with, as [`IdbKey::inviter`] does.
    fn kept_log(user: Id<User>, document: Id<Document>) -> Self {
        Self(format!(
            "log:{}:{}",
            Self::user(user),
            Self::document(document)
        ))
    }

    /// Every key [`IdbKey::kept_log`] writes for `user`, and no other.
    fn kept_logs(user: Id<User>) -> Result<web_sys::IdbKeyRange, JsValue> {
        let user = Self::user(user);
        web_sys::IdbKeyRange::bound(
            &JsValue::from_str(&format!("log:{user}:")),
            &JsValue::from_str(&format!("log:{user};")),
        )
    }

    /// The document whose kept log IndexedDB keyed `key` for `user`, as
    /// [`IdbKey::kept_log`] wrote it. `None` for any other key.
    fn kept_log_of(user: Id<User>, key: &JsValue) -> Option<Id<Document>> {
        let text = key.as_string()?;
        let rest = text.strip_prefix("log:")?;
        let rest = rest.strip_prefix(Self::user(user).0.as_str())?;
        Self::document_of(rest.strip_prefix(':')?)
    }

    /// A key no other delta of `document` holds. It is the document's key,
    /// then the time and a counter for this page. A random draw last parts
    /// another tab writing in the same millisecond.
    fn fresh_delta(document: Id<Document>) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let count = NEXT.fetch_add(1, Ordering::Relaxed);
        let now = js_sys::Date::now() as u64;
        let draw = (js_sys::Math::random() * f64::from(u32::MAX)) as u32;
        let document = Self::document(document);
        Self(format!("{document}:{now:015}:{count:010}:{draw:08x}"))
    }

    /// The row of one session of `peer` on `document`.
    fn session(document: Id<Document>, peer: ambre_core::PeerId) -> Self {
        Self(format!("{}:{peer}", Self::document(document)))
    }

    /// The document whose row IndexedDB keyed `text`, as
    /// [`IdbKey::document`] wrote it. `None` for any other text.
    fn document_of(text: &str) -> Option<Id<Document>> {
        if text.len() != 32 {
            return None;
        }
        Id::<Document>::parse_str(text).ok()
    }

    /// The key, for the call into IndexedDB.
    fn to_js(&self) -> JsValue {
        JsValue::from_str(&self.0)
    }
}

impl std::fmt::Display for IdbKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How one write ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    /// The transaction committed.
    Done,
    /// The browser refused it for its quota (`RESID-017`).
    Quota,
    /// It failed for any other reason. The write is lost, and logged.
    Failed,
}

/// Whether a write meant to outlive the page committed.
///
/// It does not reuse [`Stored`]: it answers on the transaction's commit, not
/// on the request's success. A caller about to leave the page acts on landed
/// or lost alone, and a quota refusal is lost all the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commit {
    /// The write failed, or its transaction aborted: the bytes are not there.
    Lost,
    /// The transaction committed: the bytes are durable.
    Landed,
}

/// An open IndexedDB handle for persisting document CRDT snapshots.
///
/// Created once per session via [`open`](PersistenceStore::open). The database
/// and its object store are created on first visit. All operations are async.
#[derive(Clone)]
pub struct PersistenceStore {
    db: IdbDatabase,
}

impl PersistenceStore {
    /// Open the database, creating the object store on first visit.
    ///
    /// Returns a one-shot receiver resolving once to the store (or an error
    /// string). The caller awaits it on the Main thread.
    pub fn open() -> oneshot::Receiver<Result<Self, String>> {
        Self::open_named(DB_NAME)
    }

    /// Open a named database at [`DB_VERSION`], running the shared upgrade path.
    /// [`open`](Self::open) uses the production [`DB_NAME`]; tests pass a unique
    /// name so each runs against an isolated database.
    fn open_named(db_name: DbName<'_>) -> oneshot::Receiver<Result<Self, String>> {
        Self::open_at(db_name, DB_VERSION)
    }

    /// Open a named database at a given version. Only a test passes a version
    /// other than [`DB_VERSION`], to run the upgrade a later version runs.
    fn open_at(
        db_name: DbName<'_>,
        version: Version<IdbShape>,
    ) -> oneshot::Receiver<Result<Self, String>> {
        let (tx, rx) = answer();

        let factory = match web_sys::window().and_then(|w| w.indexed_db().ok().flatten()) {
            Some(f) => f,
            None => {
                tx.send(Err("IndexedDB unavailable".into()));
                return rx;
            }
        };

        let open_request: IdbOpenDbRequest = match factory.open_with_u32(db_name.0, version.get()) {
            Ok(req) => req,
            Err(e) => {
                tx.send(Err(format!("open request failed: {e:?}")));
                return rx;
            }
        };

        // Create the object stores during upgrade (first visit or version bump).
        // A bump fires the upgrade on a database that already holds stores, so
        // each store is created only if it does not already exist. Calling
        // `create_object_store` for a store that exists throws
        // `ConstraintError` and aborts the whole upgrade transaction.
        let on_upgrade = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(db) = request_database(&event) {
                let existing = db.object_store_names();
                for store in IdbStore::ALL {
                    if !existing.contains(store.name()) {
                        let _ = db.create_object_store(store.name());
                    }
                }
            }
        });
        open_request.set_onupgradeneeded(Some(on_upgrade.as_ref().unchecked_ref()));
        on_upgrade.forget();

        // Resolve on success with the open database.
        let tx_ok = tx.clone();
        let on_success = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            match request_database(&event) {
                Some(db) => {
                    tx_ok.send(Ok(Self { db }));
                }
                None => {
                    tx_ok.send(Err("open succeeded but no database".into()));
                }
            }
        });
        open_request.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
        on_success.forget();

        // Resolve on error.
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            tx.send(Err("IndexedDB open error".into()));
        });
        open_request.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();

        rx
    }

    /// Save a document's CRDT snapshot.
    ///
    /// Stores `snapshot` under `document_id` in the "snapshots" store. The
    /// same transaction drops the document's deltas: the snapshot was exported
    /// after every one of them, so it holds them all. The transaction
    /// auto-commits when the Main thread returns to its event loop. The
    /// receiver says how it ended, so a quota refusal can be met (`RESID-017`).
    pub fn save_snapshot(
        &self,
        document_id: Id<Document>,
        snapshot: &Payload<SealedRowBytes>,
    ) -> oneshot::Receiver<Stored> {
        outcome(self.write_snapshot(document_id, snapshot))
    }

    /// Append one delta to a document's log.
    ///
    /// The next [`save_snapshot`](Self::save_snapshot) folds it away. The
    /// receiver says how the write ended.
    pub fn save_delta(
        &self,
        document_id: Id<Document>,
        delta: &Payload<SealedRowBytes>,
    ) -> oneshot::Receiver<Stored> {
        let key = IdbKey::fresh_delta(document_id);
        let txn = match self
            .db
            .transaction_with_str_and_mode(IdbStore::Deltas.name(), IdbTransactionMode::Readwrite)
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Persistence: delta txn failed for {key}: {e:?}");
                return outcome(None);
            }
        };
        let Ok(store) = txn.object_store(IdbStore::Deltas.name()) else {
            return outcome(None);
        };
        if let Err(e) =
            store.put_with_key(Uint8Array::from(delta.as_bytes()).as_ref(), &key.to_js())
        {
            tracing::warn!("Persistence: delta put failed for {key}: {e:?}");
            return outcome(None);
        }
        outcome(Some(txn))
    }

    /// Write the ledger's row for a document (fire-and-forget). A row lost
    /// here only counts the document less precisely after a reload.
    pub fn save_meta(&self, document_id: Id<Document>, meta: &Meta) {
        let key = IdbKey::document(document_id);
        if let Err(e) = self
            .db
            .transaction_with_str_and_mode(IdbStore::Meta.name(), IdbTransactionMode::Readwrite)
            .and_then(|txn| txn.object_store(IdbStore::Meta.name()))
            .and_then(|store| {
                store.put_with_key(
                    Uint8Array::from(meta.encode().as_slice()).as_ref(),
                    &key.to_js(),
                )
            })
        {
            tracing::warn!("Persistence: meta put failed for {key}: {e:?}");
        }
    }

    /// Load every ledger row, keyed by its document. Resolves to an empty
    /// list when the read failed; a row that does not decode is skipped.
    pub fn load_meta(&self) -> oneshot::Receiver<Vec<(Id<Document>, Meta)>> {
        let (tx, rx) = answer();
        let Ok(txn) = self.db.transaction_with_str(IdbStore::Meta.name()) else {
            tx.send(Vec::new());
            return rx;
        };
        let requests = txn
            .object_store(IdbStore::Meta.name())
            .and_then(|store| Ok((store.get_all_keys()?, store.get_all()?)));
        let Ok((keys, rows)) = requests else {
            tx.send(Vec::new());
            return rx;
        };
        // Both requests list the rows in key order, so the transaction's
        // `complete` pairs them.
        let tx_ok = tx.clone();
        let on_complete =
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
                let as_array = |r: &web_sys::IdbRequest| {
                    r.result()
                        .ok()
                        .and_then(|v| v.dyn_into::<js_sys::Array>().ok())
                };
                let (Some(keys), Some(rows)) = (as_array(&keys), as_array(&rows)) else {
                    tx_ok.send(Vec::new());
                    return;
                };
                let metas = keys
                    .iter()
                    .zip(rows.iter())
                    .filter_map(|(key, row)| {
                        let id = IdbKey::document_of(&key.as_string()?)?;
                        let row = row.dyn_into::<Uint8Array>().ok()?.to_vec();
                        Some((id, Meta::decode(&row)?))
                    })
                    .collect();
                tx_ok.send(metas);
            });
        txn.set_oncomplete(Some(on_complete.as_ref().unchecked_ref()));
        on_complete.forget();
        let on_abort = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            tx.send(Vec::new());
        });
        txn.set_onabort(Some(on_abort.as_ref().unchecked_ref()));
        on_abort.forget();
        rx
    }

    /// Keep `inviter` as the inviter an open of `user` proved for
    /// `document_id` (`SYNC-238`, `SYNC-244`, fire-and-forget). A row already
    /// there stays: the add fails on it, and no later pin replaces the kept
    /// one (`SYNC-239`).
    pub fn save_inviter(
        &self,
        user: Id<User>,
        document_id: Id<Document>,
        inviter: &FixedBytes<ambre_core::InviterPin>,
    ) {
        let key = IdbKey::inviter(user, document_id);
        if let Err(e) = self
            .db
            .transaction_with_str_and_mode(
                IdbStore::TrustedKeys.name(),
                IdbTransactionMode::Readwrite,
            )
            .and_then(|txn| txn.object_store(IdbStore::TrustedKeys.name()))
            .and_then(|store| {
                store.add_with_key(Uint8Array::from(inviter.as_slice()).as_ref(), &key.to_js())
            })
        {
            tracing::warn!("Persistence: inviter add failed for {key}: {e:?}");
        }
    }

    /// Keep `kept`, where the log of `document_id` stood for `user`
    /// (`SYNC-290`, fire-and-forget). A later head replaces the row.
    pub fn save_kept_log(
        &self,
        user: Id<User>,
        document_id: Id<Document>,
        kept: &ambre_bus::KeptLog,
    ) {
        let key = IdbKey::kept_log(user, document_id);
        let Ok(row) = kept.to_row() else {
            tracing::warn!("Persistence: kept log of {key} does not encode");
            return;
        };
        if let Err(e) = self
            .db
            .transaction_with_str_and_mode(
                IdbStore::TrustedKeys.name(),
                IdbTransactionMode::Readwrite,
            )
            .and_then(|txn| txn.object_store(IdbStore::TrustedKeys.name()))
            .and_then(|store| {
                store.put_with_key(Uint8Array::from(row.as_bytes()).as_ref(), &key.to_js())
            })
        {
            tracing::warn!("Persistence: kept log put failed for {key}: {e:?}");
        }
    }

    /// Load every log `user` kept, keyed by its document (`SYNC-290`).
    /// Another user's row is never read. Resolves to an empty list when the
    /// read failed; a row that is no kept log is skipped.
    pub fn load_kept_logs(
        &self,
        user: Id<User>,
    ) -> oneshot::Receiver<Vec<(Id<Document>, ambre_bus::KeptLog)>> {
        let (tx, rx) = answer();
        let Ok(txn) = self.db.transaction_with_str(IdbStore::TrustedKeys.name()) else {
            tx.send(Vec::new());
            return rx;
        };
        let requests = IdbKey::kept_logs(user).and_then(|range| {
            let store = txn.object_store(IdbStore::TrustedKeys.name())?;
            Ok((
                store.get_all_keys_with_key(&range)?,
                store.get_all_with_key(&range)?,
            ))
        });
        let Ok((keys, rows)) = requests else {
            tx.send(Vec::new());
            return rx;
        };
        let tx_ok = tx.clone();
        let on_complete =
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
                let as_array = |r: &web_sys::IdbRequest| {
                    r.result()
                        .ok()
                        .and_then(|v| v.dyn_into::<js_sys::Array>().ok())
                };
                let (Some(keys), Some(rows)) = (as_array(&keys), as_array(&rows)) else {
                    tx_ok.send(Vec::new());
                    return;
                };
                let kept = keys
                    .iter()
                    .zip(rows.iter())
                    .filter_map(|(key, row)| {
                        let id = IdbKey::kept_log_of(user, &key)?;
                        let row = row.dyn_into::<Uint8Array>().ok()?.to_vec();
                        let row = ambre_core::Payload::<ambre_bus::KeptLogRow>::new(row);
                        Some((id, ambre_bus::KeptLog::from_row(&row)?))
                    })
                    .collect();
                tx_ok.send(kept);
            });
        txn.set_oncomplete(Some(on_complete.as_ref().unchecked_ref()));
        on_complete.forget();
        let on_abort = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            tx.send(Vec::new());
        });
        txn.set_onabort(Some(on_abort.as_ref().unchecked_ref()));
        on_abort.forget();
        rx
    }

    /// Load every inviter `user` kept, keyed by its document (`SYNC-238`).
    /// Another user's row is never read (`SYNC-244`). Resolves to an empty
    /// list when the read failed; a row that is not a pin is skipped.
    pub fn load_inviters(
        &self,
        user: Id<User>,
    ) -> oneshot::Receiver<Vec<(Id<Document>, FixedBytes<ambre_core::InviterPin>)>> {
        let (tx, rx) = answer();
        let Ok(txn) = self.db.transaction_with_str(IdbStore::TrustedKeys.name()) else {
            tx.send(Vec::new());
            return rx;
        };
        let requests = IdbKey::inviters(user).and_then(|range| {
            let store = txn.object_store(IdbStore::TrustedKeys.name())?;
            Ok((
                store.get_all_keys_with_key(&range)?,
                store.get_all_with_key(&range)?,
            ))
        });
        let Ok((keys, rows)) = requests else {
            tx.send(Vec::new());
            return rx;
        };
        // Both requests list the rows in key order, so the transaction's
        // `complete` pairs them.
        let tx_ok = tx.clone();
        let on_complete =
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
                let as_array = |r: &web_sys::IdbRequest| {
                    r.result()
                        .ok()
                        .and_then(|v| v.dyn_into::<js_sys::Array>().ok())
                };
                let (Some(keys), Some(rows)) = (as_array(&keys), as_array(&rows)) else {
                    tx_ok.send(Vec::new());
                    return;
                };
                let inviters = keys
                    .iter()
                    .zip(rows.iter())
                    .filter_map(|(key, row)| {
                        let id = IdbKey::inviter_of(user, &key)?;
                        let row = row.dyn_into::<Uint8Array>().ok()?.to_vec();
                        Some((id, FixedBytes::from_slice(&row).ok()?))
                    })
                    .collect();
                tx_ok.send(inviters);
            });
        txn.set_oncomplete(Some(on_complete.as_ref().unchecked_ref()));
        on_complete.forget();
        let on_abort = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            tx.send(Vec::new());
        });
        txn.set_onabort(Some(on_abort.as_ref().unchecked_ref()));
        on_abort.forget();
        rx
    }

    /// Ask the browser to keep the store through storage pressure
    /// (`RESID-020`). The browser may refuse; the store works either way.
    pub fn request_persistent() {
        let Some(storage) = web_sys::window().map(|w| w.navigator().storage()) else {
            return;
        };
        match storage.persist() {
            Ok(promise) => {
                wasm_bindgen_futures::spawn_local(async move {
                    let granted = wasm_bindgen_futures::JsFuture::from(promise)
                        .await
                        .ok()
                        .and_then(|v| v.as_bool());
                    tracing::info!("Persistence: persistent storage granted: {granted:?}");
                });
            }
            Err(e) => tracing::warn!("Persistence: persist() failed: {e:?}"),
        }
    }

    /// Put `snapshot` and drop the document's deltas, in one transaction.
    /// `None` when the transaction could not be opened or written to.
    fn write_snapshot(
        &self,
        document_id: Id<Document>,
        snapshot: &Payload<SealedRowBytes>,
    ) -> Option<web_sys::IdbTransaction> {
        let key = IdbKey::document(document_id);
        let stores = js_sys::Array::of2(
            &JsValue::from_str(IdbStore::Snapshots.name()),
            &JsValue::from_str(IdbStore::Deltas.name()),
        );
        let txn = match self
            .db
            .transaction_with_str_sequence_and_mode(&stores, IdbTransactionMode::Readwrite)
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Persistence: save txn failed for {key}: {e:?}");
                return None;
            }
        };
        let (Ok(snapshots), Ok(deltas)) = (
            txn.object_store(IdbStore::Snapshots.name()),
            txn.object_store(IdbStore::Deltas.name()),
        ) else {
            tracing::warn!("Persistence: save object_store failed for {key}");
            return None;
        };
        if let Err(e) =
            snapshots.put_with_key(Uint8Array::from(snapshot.as_bytes()).as_ref(), &key.to_js())
        {
            tracing::warn!("Persistence: save put failed for {key}: {e:?}");
            return None;
        }
        match rows_of(document_id) {
            Some(range) => {
                if let Err(e) = deltas.delete(&range) {
                    tracing::warn!("Persistence: delta sweep failed for {key}: {e:?}");
                }
            }
            None => tracing::warn!("Persistence: no delta range for {key}"),
        }
        Some(txn)
    }

    /// Save a snapshot and report when the write is **durable**.
    ///
    /// [`save_snapshot`](Self::save_snapshot) is fire-and-forget, which is right
    /// for the per-edit persist. The page stays open and the transaction commits
    /// on the next turn of the event loop. It is wrong for a caller that is about
    /// to navigate away. Opening an `.ambre` file whose document is of another
    /// kind hard-reloads the page (`route::needs_reload`). A transaction still
    /// open when the document unloads is aborted, taking the file with it.
    ///
    /// So this one resolves on the transaction's `complete`, not on the request's
    /// `success`. The former is the commit; the latter only says the write was
    /// accepted into it. [`Commit::Lost`] on any failure, so a caller can tell
    /// the user the file did not land instead of navigating to an empty
    /// document.
    pub fn save_snapshot_committed(
        &self,
        document_id: Id<Document>,
        snapshot: Payload<SealedRowBytes>,
    ) -> oneshot::Receiver<Commit> {
        let (tx, rx) = answer();
        let Some(txn) = self.write_snapshot(document_id, &snapshot) else {
            tx.send(Commit::Lost);
            return rx;
        };

        let tx_done = tx.clone();
        let on_complete = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
            tx_done.send(Commit::Landed);
        });
        txn.set_oncomplete(Some(on_complete.as_ref().unchecked_ref()));
        on_complete.forget();

        // `error` and `abort` are distinct events on a transaction and either one
        // means the bytes are not there; both answer the same way.
        let tx_err = tx.clone();
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
            tx_err.send(Commit::Lost);
        });
        txn.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();
        let on_abort = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
            tx.send(Commit::Lost);
        });
        txn.set_onabort(Some(on_abort.as_ref().unchecked_ref()));
        on_abort.forget();

        rx
    }

    /// Load a document's most recent snapshot.
    ///
    /// Returns a one-shot receiver resolving to `Some(bytes)` if a snapshot
    /// exists, or `None` (no snapshot / read error). Awaited on the Main thread.
    pub fn load_snapshot(
        &self,
        document_id: Id<Document>,
    ) -> oneshot::Receiver<Option<Payload<SealedRowBytes>>> {
        let (tx, rx) = answer();
        let key = IdbKey::document(document_id);

        let txn = match self.db.transaction_with_str(IdbStore::Snapshots.name()) {
            Ok(t) => t,
            Err(_) => {
                tx.send(None);
                return rx;
            }
        };
        let store = match txn.object_store(IdbStore::Snapshots.name()) {
            Ok(s) => s,
            Err(_) => {
                tx.send(None);
                return rx;
            }
        };
        let request = match store.get(&key.to_js()) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Persistence: load get failed for {key}: {e:?}");
                tx.send(None);
                return rx;
            }
        };

        let tx_ok = tx.clone();
        let req = request.clone();
        let on_success =
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
                let data = req
                    .result()
                    .ok()
                    .filter(|v| !v.is_undefined() && !v.is_null())
                    .and_then(|v| v.dyn_into::<Uint8Array>().ok())
                    .map(|a| Payload::<SealedRowBytes>::new(a.to_vec()));
                tx_ok.send(data);
            });
        request.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
        on_success.forget();

        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            tx.send(None);
        });
        request.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();

        rx
    }

    /// Load every delta written since a document's snapshot, in no set order.
    ///
    /// Resolves to an empty list when there is none or the read failed.
    pub fn load_deltas(
        &self,
        document_id: Id<Document>,
    ) -> oneshot::Receiver<Vec<Payload<SealedRowBytes>>> {
        let (tx, rx) = answer();
        let request = self
            .db
            .transaction_with_str(IdbStore::Deltas.name())
            .and_then(|txn| txn.object_store(IdbStore::Deltas.name()))
            .ok()
            .zip(rows_of(document_id))
            .and_then(|(store, range)| store.get_all_with_key(&range).ok());
        let Some(request) = request else {
            tx.send(Vec::new());
            return rx;
        };

        let tx_ok = tx.clone();
        let req = request.clone();
        let on_success =
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
                let deltas = req
                    .result()
                    .ok()
                    .and_then(|v| v.dyn_into::<js_sys::Array>().ok())
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| row.dyn_into::<Uint8Array>().ok())
                            .map(|a| Payload::<SealedRowBytes>::new(a.to_vec()))
                            .collect()
                    })
                    .unwrap_or_default();
                tx_ok.send(deltas);
            });
        request.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
        on_success.forget();

        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            tx.send(Vec::new());
        });
        request.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();

        rx
    }

    /// Record one session of this user on a document (fire-and-forget).
    ///
    /// Written at the session identity, before any of its operations
    /// (`AUTHOR-028`). A lost row only loses that session's unsent tail.
    pub fn save_session(
        &self,
        document_id: Id<Document>,
        peer: ambre_core::PeerId,
        nonce: FixedBytes<SessionNonce>,
    ) {
        let key = IdbKey::session(document_id, peer);
        let Some(store) = self.sessions_store(&key) else {
            return;
        };
        let mut value = [0u8; 24];
        let (head, tail) = value.split_at_mut(8);
        head.copy_from_slice(&peer.get().to_le_bytes());
        tail.copy_from_slice(nonce.as_slice());
        if let Err(e) =
            store.put_with_key(Uint8Array::from(value.as_slice()).as_ref(), &key.to_js())
        {
            tracing::warn!("Persistence: session put failed for {key}: {e:?}");
        }
    }

    /// Drop one session the relay already holds whole (fire-and-forget).
    pub fn forget_session(&self, document_id: Id<Document>, peer: ambre_core::PeerId) {
        let key = IdbKey::session(document_id, peer);
        let Some(store) = self.sessions_store(&key) else {
            return;
        };
        if let Err(e) = store.delete(&key.to_js()) {
            tracing::warn!("Persistence: session delete failed for {key}: {e:?}");
        }
    }

    /// The `sessions` store in a fresh read-write transaction, to write the
    /// row `key`.
    fn sessions_store(&self, key: &IdbKey) -> Option<web_sys::IdbObjectStore> {
        match self
            .db
            .transaction_with_str_and_mode(IdbStore::Sessions.name(), IdbTransactionMode::Readwrite)
            .and_then(|txn| txn.object_store(IdbStore::Sessions.name()))
        {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::warn!("Persistence: session txn failed for {key}: {e:?}");
                None
            }
        }
    }

    /// Load every recorded session of this user on a document.
    ///
    /// Resolves to an empty list when there is none or the read failed. A row
    /// of the wrong length is skipped.
    pub fn load_sessions(
        &self,
        document_id: Id<Document>,
    ) -> oneshot::Receiver<Vec<(ambre_core::PeerId, FixedBytes<SessionNonce>)>> {
        let (tx, rx) = answer();
        let request = self
            .db
            .transaction_with_str(IdbStore::Sessions.name())
            .and_then(|txn| txn.object_store(IdbStore::Sessions.name()))
            .ok()
            .zip(rows_of(document_id))
            .and_then(|(store, range)| store.get_all_with_key(&range).ok());
        let Some(request) = request else {
            tx.send(Vec::new());
            return rx;
        };

        let tx_ok = tx.clone();
        let req = request.clone();
        let on_success =
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
                let sessions = req
                    .result()
                    .ok()
                    .and_then(|v| v.dyn_into::<js_sys::Array>().ok())
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| row.dyn_into::<Uint8Array>().ok())
                            .filter_map(|a| session_of(&a.to_vec()))
                            .collect()
                    })
                    .unwrap_or_default();
                tx_ok.send(sessions);
            });
        request.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
        on_success.forget();

        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            tx.send(Vec::new());
        });
        request.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();

        rx
    }

    /// Load the pinned TOFU exchange key for `user_id`, if any.
    ///
    /// Returns a one-shot receiver resolving to `Some(key)` when this user's key
    /// has been confirmed before, or `None` (first invitation / read error). The
    /// Main thread awaits it and compares against the freshly fetched key to
    /// classify the trust state (`ambre-spec-key-verification.md`).
    pub fn load_trusted_key(
        &self,
        user_id: Id<User>,
    ) -> oneshot::Receiver<Option<FixedBytes<ambre_core::ExchangePublicKey>>> {
        let (tx, rx) = answer();
        let key = IdbKey::user(user_id);

        let txn = match self.db.transaction_with_str(IdbStore::TrustedKeys.name()) {
            Ok(t) => t,
            Err(_) => {
                tx.send(None);
                return rx;
            }
        };
        let store = match txn.object_store(IdbStore::TrustedKeys.name()) {
            Ok(s) => s,
            Err(_) => {
                tx.send(None);
                return rx;
            }
        };
        let request = match store.get(&key.to_js()) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Persistence: trusted_key get failed for {key}: {e:?}");
                tx.send(None);
                return rx;
            }
        };

        let tx_ok = tx.clone();
        let req = request.clone();
        let on_success =
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
                // Only a full 32-byte value is a valid pin; anything else is
                // treated as absent so the caller re-confirms rather than trusting
                // a truncated row.
                let data = req
                    .result()
                    .ok()
                    .filter(|v| !v.is_undefined() && !v.is_null())
                    .and_then(|v| v.dyn_into::<Uint8Array>().ok())
                    .map(|a| a.to_vec())
                    .and_then(|v| FixedBytes::<ambre_core::ExchangePublicKey>::from_slice(&v).ok());
                tx_ok.send(data);
            });
        request.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
        on_success.forget();

        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            tx.send(None);
        });
        request.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();

        rx
    }

    /// Pin `key` as the trusted TOFU exchange key for `user_id` (fire-and-forget).
    ///
    /// Called on the user's confirmation in the share dialog, and again when they
    /// deliberately re-pin after a "key changed" alert. The transaction
    /// auto-commits when the Main thread returns to its event loop.
    pub fn save_trusted_key(
        &self,
        user_id: Id<User>,
        key: &FixedBytes<ambre_core::ExchangePublicKey>,
    ) {
        let row = IdbKey::user(user_id);
        let txn = match self.db.transaction_with_str_and_mode(
            IdbStore::TrustedKeys.name(),
            IdbTransactionMode::Readwrite,
        ) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Persistence: trusted_key save txn failed for {row}: {e:?}");
                return;
            }
        };
        let store = match txn.object_store(IdbStore::TrustedKeys.name()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Persistence: trusted_key object_store failed for {row}: {e:?}");
                return;
            }
        };

        let array = Uint8Array::new_with_length(key.as_slice().len() as u32);
        array.copy_from(key.as_slice());
        if let Err(e) = store.put_with_key(array.as_ref(), &row.to_js()) {
            tracing::warn!("Persistence: trusted_key put failed for {row}: {e:?}");
        }
    }

    /// Remove a document's snapshot, its deltas, its sessions and its ledger
    /// row: the copy leaves the store, and a reopen fetches it (`RESID-012`).
    pub fn delete_snapshot(&self, document_id: Id<Document>) {
        let key = IdbKey::document(document_id);
        let stores = js_sys::Array::of4(
            &JsValue::from_str(IdbStore::Snapshots.name()),
            &JsValue::from_str(IdbStore::Deltas.name()),
            &JsValue::from_str(IdbStore::Sessions.name()),
            &JsValue::from_str(IdbStore::Meta.name()),
        );
        let Ok(txn) = self
            .db
            .transaction_with_str_sequence_and_mode(&stores, IdbTransactionMode::Readwrite)
        else {
            return;
        };
        for name in [IdbStore::Snapshots, IdbStore::Meta].map(IdbStore::name) {
            if let Ok(store) = txn.object_store(name)
                && let Err(e) = store.delete(&key.to_js())
            {
                tracing::warn!("Persistence: {name} delete failed for {key}: {e:?}");
            }
        }
        for name in [IdbStore::Deltas, IdbStore::Sessions].map(IdbStore::name) {
            if let (Ok(store), Some(range)) = (txn.object_store(name), rows_of(document_id))
                && let Err(e) = store.delete(&range)
            {
                tracing::warn!("Persistence: {name} delete failed for {key}: {e:?}");
            }
        }
    }

    /// Keep a draft's sealed `row` under `key`, in place of the row there
    /// (fire-and-forget). A lost write leaves the draft as it was at the
    /// write before.
    pub fn save_draft(&self, key: &DraftKey, row: &Payload<SealedRowBytes>) {
        if let Some(store) = self.drafts_store()
            && let Err(e) = store.put_with_key(
                &Uint8Array::from(row.as_bytes()),
                &JsValue::from_str(key.as_str()),
            )
        {
            tracing::warn!("Persistence: draft put failed: {e:?}");
        }
    }

    /// Delete the draft under `key` (fire-and-forget).
    pub fn delete_draft(&self, key: &DraftKey) {
        if let Some(store) = self.drafts_store()
            && let Err(e) = store.delete(&JsValue::from_str(key.as_str()))
        {
            tracing::warn!("Persistence: draft delete failed: {e:?}");
        }
    }

    /// Delete every draft whose key falls in `lower..=upper`
    /// (fire-and-forget).
    pub fn delete_drafts(&self, lower: &DraftKey, upper: &DraftKey) {
        let range = web_sys::IdbKeyRange::bound(
            &JsValue::from_str(lower.as_str()),
            &JsValue::from_str(upper.as_str()),
        );
        if let (Some(store), Ok(range)) = (self.drafts_store(), range)
            && let Err(e) = store.delete(&range)
        {
            tracing::warn!("Persistence: drafts delete failed: {e:?}");
        }
    }

    /// Delete every draft on the device (fire-and-forget).
    pub fn clear_drafts(&self) {
        if let Some(store) = self.drafts_store()
            && let Err(e) = store.clear()
        {
            tracing::warn!("Persistence: drafts clear failed: {e:?}");
        }
    }

    /// The `drafts` store in a fresh read-write transaction.
    fn drafts_store(&self) -> Option<web_sys::IdbObjectStore> {
        match self
            .db
            .transaction_with_str_and_mode(IdbStore::Drafts.name(), IdbTransactionMode::Readwrite)
            .and_then(|txn| txn.object_store(IdbStore::Drafts.name()))
        {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::warn!("Persistence: drafts txn failed: {e:?}");
                None
            }
        }
    }

    /// Load every draft whose key falls in `lower..=upper`, as its key and
    /// its row. Resolves to an empty list when the read failed. A row kept as
    /// text before the store was sealed comes back as its bytes. It then fails
    /// to open like any row that is not sealed.
    pub fn load_drafts(
        &self,
        lower: &DraftKey,
        upper: &DraftKey,
    ) -> oneshot::Receiver<Vec<(DraftKey, Payload<SealedRowBytes>)>> {
        let (tx, rx) = answer();
        let range = web_sys::IdbKeyRange::bound(
            &JsValue::from_str(lower.as_str()),
            &JsValue::from_str(upper.as_str()),
        );
        let Ok(txn) = self.db.transaction_with_str(IdbStore::Drafts.name()) else {
            tx.send(Vec::new());
            return rx;
        };
        let requests = txn.object_store(IdbStore::Drafts.name()).and_then(|store| {
            let range = range?;
            Ok((
                store.get_all_keys_with_key(&range)?,
                store.get_all_with_key(&range)?,
            ))
        });
        let Ok((keys, rows)) = requests else {
            tx.send(Vec::new());
            return rx;
        };
        // Both requests list the rows in key order, so the transaction's
        // `complete` pairs them.
        let tx_ok = tx.clone();
        let on_complete =
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
                let as_array = |r: &web_sys::IdbRequest| {
                    r.result()
                        .ok()
                        .and_then(|v| v.dyn_into::<js_sys::Array>().ok())
                };
                let (Some(keys), Some(rows)) = (as_array(&keys), as_array(&rows)) else {
                    tx_ok.send(Vec::new());
                    return;
                };
                let drafts = keys
                    .iter()
                    .zip(rows.iter())
                    .filter_map(|(key, row)| {
                        let row = match row.as_string() {
                            Some(text) => text.into_bytes(),
                            None => row.dyn_into::<Uint8Array>().ok()?.to_vec(),
                        };
                        Some((
                            DraftKey::new(key.as_string()?),
                            Payload::<SealedRowBytes>::new(row),
                        ))
                    })
                    .collect();
                tx_ok.send(drafts);
            });
        txn.set_oncomplete(Some(on_complete.as_ref().unchecked_ref()));
        on_complete.forget();
        let on_abort = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            tx.send(Vec::new());
        });
        txn.set_onabort(Some(on_abort.as_ref().unchecked_ref()));
        on_abort.forget();
        rx
    }
}

/// How a write transaction ends: `complete`, or an `abort` that names the
/// quota or anything else. `None` is a write that never reached the store.
///
/// An `error` on a request aborts its transaction, so `abort` alone answers.
fn outcome(txn: Option<web_sys::IdbTransaction>) -> oneshot::Receiver<Stored> {
    let (tx, rx) = answer();
    let Some(txn) = txn else {
        tx.send(Stored::Failed);
        return rx;
    };
    let tx_done = tx.clone();
    let on_complete = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
        tx_done.send(Stored::Done);
    });
    txn.set_oncomplete(Some(on_complete.as_ref().unchecked_ref()));
    on_complete.forget();
    let aborted = txn.clone();
    let on_abort = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
        let name = aborted.error().map(|e| e.name());
        if name.as_deref() == Some("QuotaExceededError") {
            tx.send(Stored::Quota);
        } else {
            tracing::warn!("Persistence: a write aborted ({name:?})");
            tx.send(Stored::Failed);
        }
    });
    txn.set_onabort(Some(on_abort.as_ref().unchecked_ref()));
    on_abort.forget();
    rx
}

/// Parse one `sessions` row: the PeerID's eight little-endian bytes, then
/// the bytes of the nonce. `None` for any other length.
fn session_of(row: &[u8]) -> Option<(ambre_core::PeerId, FixedBytes<SessionNonce>)> {
    let (peer, rest) = row.split_first_chunk::<{ size_of::<u64>() }>()?;
    let (nonce, rest) = FixedBytes::<SessionNonce>::split_from(rest)?;
    if !rest.is_empty() {
        return None;
    }
    Some((ambre_core::PeerId::new(u64::from_le_bytes(*peer)), nonce))
}

/// The key range holding every row of `document`, in a store whose keys open
/// with [`IdbKey::document`] and a colon. It runs from `"{key}:"` up to
/// `"{key};"`, the character after the colon.
fn rows_of(document: Id<Document>) -> Option<web_sys::IdbKeyRange> {
    let key = IdbKey::document(document);
    web_sys::IdbKeyRange::bound(
        &JsValue::from_str(&format!("{key}:")),
        &JsValue::from_str(&format!("{key};")),
    )
    .ok()
}

/// Extract the `IdbDatabase` from an IndexedDB open-request event target.
fn request_database(event: &web_sys::Event) -> Option<IdbDatabase> {
    event
        .target()
        .and_then(|t| t.dyn_into::<IdbOpenDbRequest>().ok())
        .and_then(|req| req.result().ok())
        .and_then(|r| r.dyn_into::<IdbDatabase>().ok())
}

// ============================================================================
// wasm tests (headless Chrome, `just test-wasm`). Exercise the two paths the
// TOFU store adds (`ambre-spec-key-verification.md`, CHROME-053) that a native
// test cannot reach. One is the trusted_keys roundtrip. The other is the
// upgrade to the next version, which must add trusted_keys WITHOUT a
// ConstraintError on the existing snapshots store.
// ============================================================================
#[cfg(test)]
mod wasm_tests {
    use super::*;
    use ambre_core::PeerId;
    use wasm_bindgen_test::wasm_bindgen_test;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// A stored row: the store keeps the bytes it is handed, sealed or not.
    fn row(bytes: &[u8]) -> Payload<SealedRowBytes> {
        Payload::<SealedRowBytes>::new(bytes.to_vec())
    }

    /// The `IdbFactory` for this origin (headless Chrome always has one).
    fn factory() -> web_sys::IdbFactory {
        web_sys::window()
            .and_then(|w| w.indexed_db().ok().flatten())
            .expect("IndexedDB available in the test browser")
    }

    /// Delete a database and resolve once the request settles, so each test
    /// starts from a clean slate regardless of what a prior run left behind.
    async fn delete_db(name: &str) {
        let (tx, rx) = answer::<()>();
        let req = factory().delete_database(name).expect("delete_database");
        let tx2 = tx.clone();
        let done = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            tx2.send(());
        });
        req.set_onsuccess(Some(done.as_ref().unchecked_ref()));
        req.set_onerror(Some(done.as_ref().unchecked_ref()));
        done.forget();
        let _ = rx.await;
    }

    /// A database at [`DB_VERSION`] with ONLY the `snapshots` store, holding
    /// one snapshot row. It stands for a browser that holds documents and
    /// lacks a store the next version adds.
    async fn seed_snapshots_only(name: &str, doc_id: Id<Document>, bytes: &[u8]) {
        let (tx, rx) = answer::<()>();
        let req = factory()
            .open_with_u32(name, DB_VERSION.get())
            .expect("open at the current version");

        // An upgrade that creates `snapshots` alone, unconditionally.
        let upgrade = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(db) = request_database(&event) {
                let _ = db.create_object_store(IdbStore::Snapshots.name());
            }
        });
        req.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
        upgrade.forget();

        let key = IdbKey::document(doc_id);
        let payload = bytes.to_vec();
        let tx2 = tx.clone();
        let success = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(db) = request_database(&event) {
                if let Ok(txn) = db.transaction_with_str_and_mode(
                    IdbStore::Snapshots.name(),
                    IdbTransactionMode::Readwrite,
                ) && let Ok(store) = txn.object_store(IdbStore::Snapshots.name())
                {
                    let arr = Uint8Array::new_with_length(payload.len() as u32);
                    arr.copy_from(&payload);
                    let _ = store.put_with_key(arr.as_ref(), &key.to_js());
                }
                db.close();
            }
            tx2.send(());
        });
        req.set_onsuccess(Some(success.as_ref().unchecked_ref()));
        success.forget();

        let _ = rx.await;
    }

    /// A document's key is its 32 hex digits, and reads back as that
    /// document. The hyphenated form is no key.
    #[wasm_bindgen_test]
    fn a_document_key_reads_back_as_its_document() {
        let doc = Id::<Document>::from_bytes([0xab; 16]);
        let key = IdbKey::document(doc);
        assert_eq!(key.to_string(), "ab".repeat(16));
        assert_eq!(IdbKey::document_of(&key.to_string()), Some(doc));
        assert_eq!(IdbKey::document_of(&doc.to_string()), None);
    }

    #[wasm_bindgen_test]
    async fn trusted_key_roundtrips_and_absent_is_none() {
        let name = "ambre-test-trusted-roundtrip";
        delete_db(name).await;
        let store = PersistenceStore::open_named(DbName(name))
            .await
            .expect("channel")
            .expect("store opens");

        let user = Id::<User>::from_bytes([7; 16]);
        // Absent before any write.
        assert_eq!(store.load_trusted_key(user).await.unwrap(), None);

        // Pin, then read it back byte-for-byte.
        let key = FixedBytes::<ambre_core::ExchangePublicKey>::from_array([0xAB; 32]);
        store.save_trusted_key(user, &key);
        assert_eq!(store.load_trusted_key(user).await.unwrap(), Some(key));

        // A different user is still absent (keys are per-Id<User>).
        assert_eq!(
            store
                .load_trusted_key(Id::<User>::from_bytes([9; 16]))
                .await
                .unwrap(),
            None
        );
    }

    /// Confirming a key again moves that recipient's pin and no other
    /// (`CHROME-078`).
    #[wasm_bindgen_test]
    async fn a_repin_leaves_every_other_pin_as_it_was() {
        let name = "ambre-test-trusted-repin";
        delete_db(name).await;
        let store = PersistenceStore::open_named(DbName(name))
            .await
            .expect("channel")
            .expect("store opens");

        let (alice, bob) = (
            Id::<User>::from_bytes([1; 16]),
            Id::<User>::from_bytes([2; 16]),
        );
        let pin = FixedBytes::<ambre_core::ExchangePublicKey>::from_array;
        store.save_trusted_key(alice, &pin([0xA1; 32]));
        store.save_trusted_key(bob, &pin([0xB1; 32]));
        store.save_trusted_key(alice, &pin([0xA2; 32]));
        assert_eq!(
            store.load_trusted_key(alice).await.unwrap(),
            Some(pin([0xA2; 32]))
        );
        assert_eq!(
            store.load_trusted_key(bob).await.unwrap(),
            Some(pin([0xB1; 32]))
        );
    }

    /// Holds `SYNC-238` and `SYNC-239` at rest. A kept inviter reads back
    /// on a later open of the store. A second pin for the same document
    /// leaves the first, and the copy leaving the store leaves the pin.
    #[wasm_bindgen_test]
    async fn a_kept_inviter_outlives_the_page_and_is_never_replaced() {
        let name = "ambre-test-inviters";
        delete_db(name).await;
        let open = || async {
            PersistenceStore::open_named(DbName(name))
                .await
                .expect("channel")
                .expect("store opens")
        };
        let store = open().await;
        let me = Id::<User>::from_bytes([0x44; 16]);
        assert_eq!(store.load_inviters(me).await.unwrap(), Vec::new());
        let (doc, other) = (
            Id::<Document>::from_bytes([0x11; 16]),
            Id::<Document>::from_bytes([0x22; 16]),
        );
        let pin = FixedBytes::<ambre_core::InviterPin>::from_array;
        store.save_inviter(me, doc, &pin([0xA1; 32]));
        store.save_inviter(me, other, &pin([0xB1; 32]));
        store.save_inviter(me, doc, &pin([0xA2; 32]));
        store.delete_snapshot(doc);
        // A user's pin beside them is no inviter.
        let user = Id::<User>::from_bytes([0x33; 16]);
        store.save_trusted_key(
            user,
            &FixedBytes::<ambre_core::ExchangePublicKey>::from_array([0xC1; 32]),
        );
        // A later page opens the store afresh.
        let later = open().await;
        let mut kept = later.load_inviters(me).await.unwrap();
        kept.sort_by_key(|(id, _)| *id.as_bytes());
        assert_eq!(kept, vec![(doc, pin([0xA1; 32])), (other, pin([0xB1; 32]))]);
    }

    /// Holds `SYNC-290` at rest. A kept log reads back for its user alone,
    /// and a later head replaces the row. No pin of the same user reads as a
    /// log.
    #[wasm_bindgen_test]
    async fn a_kept_log_reads_back_for_its_user_and_a_later_head_replaces_it() {
        let name = "ambre-test-kept-logs";
        delete_db(name).await;
        let store = PersistenceStore::open_named(DbName(name))
            .await
            .expect("channel")
            .expect("store opens");
        let (me, other) = (
            Id::<User>::from_bytes([0x01; 16]),
            Id::<User>::from_bytes([0x02; 16]),
        );
        let doc = Id::<Document>::from_bytes([0x11; 16]);
        let kept = |position: u64| ambre_bus::KeptLog {
            anchor: FixedBytes::from_array([0xA1; 32]),
            head: ambre_bus::LogHead {
                position: ambre_core::Seq::new(position),
                previous: Some(FixedBytes::from_array([0xA2; 32])),
            },
            key: FixedBytes::from_array([0xA3; 16]),
        };
        store.save_inviter(me, doc, &FixedBytes::from_array([0xB1; 32]));
        store.save_kept_log(me, doc, &kept(2));
        store.save_kept_log(me, doc, &kept(5));
        assert_eq!(
            store.load_kept_logs(me).await.unwrap(),
            vec![(doc, kept(5))]
        );
        assert_eq!(store.load_kept_logs(other).await.unwrap(), Vec::new());
        assert_eq!(store.load_inviters(me).await.unwrap().len(), 1);
    }

    /// Holds `SYNC-244` at rest. Two accounts of one browser share the
    /// store, and each reads back its own pins alone. The second account
    /// keeps its own pin for a document the first one pinned. A row keyed by
    /// the document alone reads back for neither.
    #[wasm_bindgen_test]
    async fn a_second_account_reads_none_of_the_first_accounts_inviters() {
        let name = "ambre-test-inviters-per-account";
        delete_db(name).await;
        let store = PersistenceStore::open_named(DbName(name))
            .await
            .expect("channel")
            .expect("store opens");
        let (first, second) = (
            Id::<User>::from_bytes([0x01; 16]),
            Id::<User>::from_bytes([0x02; 16]),
        );
        let (doc, own) = (
            Id::<Document>::from_bytes([0x11; 16]),
            Id::<Document>::from_bytes([0x22; 16]),
        );
        let pin = FixedBytes::<ambre_core::InviterPin>::from_array;
        store.save_inviter(first, doc, &pin([0xA1; 32]));
        assert_eq!(store.load_inviters(second).await.unwrap(), Vec::new());
        // A row an older build keyed by the document alone.
        let txn = store
            .db
            .transaction_with_str_and_mode(
                IdbStore::TrustedKeys.name(),
                IdbTransactionMode::Readwrite,
            )
            .expect("transaction");
        txn.object_store(IdbStore::TrustedKeys.name())
            .expect("object store")
            .put_with_key(
                Uint8Array::from(pin([0xD1; 32]).as_slice()).as_ref(),
                &JsValue::from_str(&format!("inviter:{}", IdbKey::document(own))),
            )
            .expect("put");
        store.save_inviter(second, doc, &pin([0xB1; 32]));
        assert_eq!(
            store.load_inviters(first).await.unwrap(),
            vec![(doc, pin([0xA1; 32]))]
        );
        assert_eq!(
            store.load_inviters(second).await.unwrap(),
            vec![(doc, pin([0xB1; 32]))]
        );
    }

    #[wasm_bindgen_test]
    async fn an_upgrade_adds_trusted_keys_and_keeps_snapshots() {
        let name = "ambre-test-migration-trusted-keys";
        delete_db(name).await;

        let doc = Id::<Document>::from_bytes([3; 16]);
        let snapshot = vec![1u8, 2, 3, 4, 5];
        seed_snapshots_only(name, doc, &snapshot).await;

        // Open at the next version: the guarded onupgradeneeded must add
        // `trusted_keys` without throwing ConstraintError on `snapshots`.
        let store = PersistenceStore::open_at(DbName(name), DB_VERSION.next())
            .await
            .expect("channel")
            .expect("the database upgrades to the next version cleanly");

        // The pre-existing snapshot survived the upgrade untouched.
        assert_eq!(
            store.load_snapshot(doc).await.unwrap(),
            Some(row(&snapshot))
        );

        // The new store is usable after the upgrade.
        let user = Id::<User>::from_bytes([4; 16]);
        let key = FixedBytes::<ambre_core::ExchangePublicKey>::from_array([0x5A; 32]);
        store.save_trusted_key(user, &key);
        assert_eq!(store.load_trusted_key(user).await.unwrap(), Some(key));
    }

    #[wasm_bindgen_test]
    async fn deltas_log_until_a_snapshot_folds_them() {
        let name = "ambre-test-deltas";
        delete_db(name).await;
        let store = PersistenceStore::open_named(DbName(name))
            .await
            .expect("channel")
            .expect("store opens");
        let doc = Id::<Document>::from_bytes([5; 16]);
        let other = Id::<Document>::from_bytes([6; 16]);
        assert!(store.load_deltas(doc).await.unwrap().is_empty());

        assert_eq!(store.save_delta(doc, &row(&[1])).await, Ok(Stored::Done));
        assert_eq!(store.save_delta(doc, &row(&[2])).await, Ok(Stored::Done));
        assert_eq!(store.save_delta(other, &row(&[9])).await, Ok(Stored::Done));
        let mut deltas = store.load_deltas(doc).await.unwrap();
        deltas.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(
            deltas,
            vec![row(&[1]), row(&[2])],
            "each document reads its own"
        );

        // A snapshot holds every delta before it, so it drops them.
        assert_eq!(
            store.save_snapshot(doc, &row(&[7, 7])).await,
            Ok(Stored::Done)
        );
        assert!(store.load_deltas(doc).await.unwrap().is_empty());
        assert_eq!(store.load_snapshot(doc).await.unwrap(), Some(row(&[7, 7])));
        assert_eq!(store.load_deltas(other).await.unwrap(), vec![row(&[9])]);

        // A delete takes both.
        assert_eq!(store.save_delta(doc, &row(&[3])).await, Ok(Stored::Done));
        store.delete_snapshot(doc);
        assert!(store.load_deltas(doc).await.unwrap().is_empty());
        assert_eq!(store.load_snapshot(doc).await.unwrap(), None);
    }

    /// The eviction of a document leaves every draft, even one
    /// whose key starts with the document's. A range takes its own rows
    /// alone, and a clear takes them all.
    #[wasm_bindgen_test]
    async fn drafts_outlive_an_eviction() {
        let name = "ambre-test-drafts";
        delete_db(name).await;
        let store = PersistenceStore::open_named(DbName(name))
            .await
            .expect("channel")
            .expect("store opens");
        let doc = Id::<Document>::from_bytes([8; 16]);
        let hex = IdbKey::document(doc);
        let (mine, theirs) = (
            DraftKey::new(format!("{hex}:a")),
            DraftKey::new("z/b".to_owned()),
        );
        assert_eq!(store.save_snapshot(doc, &row(&[1])).await, Ok(Stored::Done));
        assert_eq!(store.save_delta(doc, &row(&[2])).await, Ok(Stored::Done));
        store.save_draft(&mine, &row(b"one"));
        store.save_draft(&theirs, &row(b"two"));
        store.delete_snapshot(doc);
        let (first, last) = DraftKey::every();
        let every = || store.load_drafts(&first, &last);
        let rows = every().await.unwrap();
        assert_eq!(
            rows,
            [(mine.clone(), row(b"one")), (theirs.clone(), row(b"two"))]
        );

        store.save_draft(&mine, &row(b"one again"));
        store.delete_drafts(
            &DraftKey::new("z/".to_owned()),
            &DraftKey::new("z/\u{10FFFF}".to_owned()),
        );
        assert_eq!(every().await.unwrap(), [(mine.clone(), row(b"one again"))]);
        store.delete_draft(&mine);
        assert!(every().await.unwrap().is_empty());

        store.save_draft(&mine, &row(b"one"));
        store.save_draft(&theirs, &row(b"two"));
        store.clear_drafts();
        assert!(every().await.unwrap().is_empty());
    }

    /// `RESID-008`: the ledger rows come back at boot, each with its
    /// document, and leave with the copy.
    #[wasm_bindgen_test]
    async fn ledger_rows_come_back_until_the_copy_leaves() {
        use crate::store_ledger::{Meta, StoreLedger};
        let name = "ambre-test-meta";
        delete_db(name).await;
        let store = PersistenceStore::open_named(DbName(name))
            .await
            .expect("channel")
            .expect("store opens");
        let (doc, other) = (
            Id::<Document>::from_bytes([9; 16]),
            Id::<Document>::from_bytes([10; 16]),
        );
        let mut ledger = StoreLedger::default();
        ledger.observe(doc, &crate::SealedWrite::Shown);
        ledger.observe(
            doc,
            &crate::SealedWrite::Snapshot(ambre_core::Payload::<ambre_core::SealedRowBytes>::new(
                vec![1; 40],
            )),
        );
        let meta = ledger.meta(doc).expect("held");
        store.save_meta(doc, &meta);
        store.save_meta(other, &meta);
        let rows = store.load_meta().await.unwrap();
        assert_eq!(rows, vec![(doc, meta), (other, meta)]);
        store.delete_snapshot(doc);
        let rows: Vec<(Id<Document>, Meta)> = store.load_meta().await.unwrap();
        assert_eq!(rows, vec![(other, meta)]);
    }

    /// `AUTHOR-028`: a session saved is read back with its nonce, per
    /// document, until it is forgotten or the document is deleted.
    #[wasm_bindgen_test]
    async fn sessions_are_kept_until_forgotten() {
        let name = "ambre-test-sessions";
        delete_db(name).await;
        let store = PersistenceStore::open_named(DbName(name))
            .await
            .expect("channel")
            .expect("store opens");
        let doc = Id::<Document>::from_bytes([7; 16]);
        let other = Id::<Document>::from_bytes([8; 16]);
        let (one, two) = (
            FixedBytes::<SessionNonce>::from_array([1; 16]),
            FixedBytes::<SessionNonce>::from_array([2; 16]),
        );

        store.save_session(doc, PeerId::new(11), one);
        store.save_session(doc, PeerId::new(22), two);
        store.save_session(other, PeerId::new(33), one);
        let mut sessions = store.load_sessions(doc).await.unwrap();
        sessions.sort_by_key(|(peer, _)| *peer);
        assert_eq!(
            sessions,
            vec![(PeerId::new(11), one), (PeerId::new(22), two)]
        );

        store.forget_session(doc, PeerId::new(11));
        assert_eq!(
            store.load_sessions(doc).await.unwrap(),
            vec![(PeerId::new(22), two)]
        );

        store.delete_snapshot(doc);
        assert!(store.load_sessions(doc).await.unwrap().is_empty());
        assert_eq!(
            store.load_sessions(other).await.unwrap(),
            vec![(PeerId::new(33), one)]
        );
    }

    #[wasm_bindgen_test]
    async fn an_upgrade_adds_deltas_and_sessions() {
        let name = "ambre-test-migration-deltas";
        delete_db(name).await;
        let doc = Id::<Document>::from_bytes([8; 16]);
        seed_snapshots_only(name, doc, &[1, 2]).await;
        let store = PersistenceStore::open_at(DbName(name), DB_VERSION.next())
            .await
            .expect("channel")
            .expect("the database upgrades cleanly");
        assert_eq!(store.load_snapshot(doc).await.unwrap(), Some(row(&[1, 2])));
        assert_eq!(store.save_delta(doc, &row(&[4])).await, Ok(Stored::Done));
        assert_eq!(store.load_deltas(doc).await.unwrap(), vec![row(&[4])]);
        let nonce = FixedBytes::<SessionNonce>::from_array([3; 16]);
        store.save_session(doc, PeerId::new(5), nonce);
        assert_eq!(
            store.load_sessions(doc).await.unwrap(),
            vec![(PeerId::new(5), nonce)]
        );
    }
}
