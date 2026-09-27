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
//! channel, and the Main thread persists them here. Reads/opens return a
//! one-shot receiver the Main thread awaits (never blocking the page).

use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{IdbDatabase, IdbOpenDbRequest, IdbTransactionMode};

use std::cell::RefCell;
use std::rc::Rc;

use futures_channel::oneshot;

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

const DB_NAME: &str = "ambre-persistence";
// v2 adds the `trusted_keys` store (`ambre-spec-key-verification.md`).
// The upgrade handler creates each store only if absent, so bumping the version
// on an existing v1 database adds `trusted_keys` without touching `snapshots`.
// v3 adds the `deltas` store, created the same guarded way.
const DB_VERSION: u32 = 3;
const STORE_SNAPSHOTS: &str = "snapshots";
/// The deltas written since each document's snapshot, keyed
/// `"{document hex}:{unique suffix}"`. A Loro import does not depend on the
/// order it is fed, so the suffix only has to be unique.
const STORE_DELTAS: &str = "deltas";
/// TOFU pinned exchange keys: `UserId` (hex) -> the 32 raw bytes of the X25519
/// key confirmed on first invitation (`ambre-spec-key-verification.md`).
const STORE_TRUSTED_KEYS: &str = "trusted_keys";

/// An open IndexedDB handle for persisting document CRDT snapshots.
///
/// Created once per session via [`open`](PersistenceStore::open). The database
/// and its object store are created on first visit. All operations are async.
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
    fn open_named(db_name: &str) -> oneshot::Receiver<Result<Self, String>> {
        let (tx, rx) = answer();

        let factory = match web_sys::window().and_then(|w| w.indexed_db().ok().flatten()) {
            Some(f) => f,
            None => {
                tx.send(Err("IndexedDB unavailable".into()));
                return rx;
            }
        };

        let open_request: IdbOpenDbRequest = match factory.open_with_u32(db_name, DB_VERSION) {
            Ok(req) => req,
            Err(e) => {
                tx.send(Err(format!("open request failed: {e:?}")));
                return rx;
            }
        };

        // Create the object stores during upgrade (first visit or version bump).
        // This fires for a fresh database (v0->2) AND for an existing v1 database
        // (v1->2). Each store is therefore created only if it does not already
        // exist. Calling `create_object_store` for a store that exists throws
        // `ConstraintError` and aborts the whole upgrade transaction.
        let on_upgrade = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(db) = request_database(&event) {
                let existing = db.object_store_names();
                if !existing.contains(STORE_SNAPSHOTS) {
                    let _ = db.create_object_store(STORE_SNAPSHOTS);
                }
                if !existing.contains(STORE_TRUSTED_KEYS) {
                    let _ = db.create_object_store(STORE_TRUSTED_KEYS);
                }
                if !existing.contains(STORE_DELTAS) {
                    let _ = db.create_object_store(STORE_DELTAS);
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

    /// Save a document's CRDT snapshot (fire-and-forget).
    ///
    /// Stores `snapshot` under `document_id` in the "snapshots" store. The
    /// same transaction drops the document's deltas: the snapshot was exported
    /// after every one of them, so it holds them all. The transaction
    /// auto-commits when the Main thread returns to its event loop. Errors are
    /// logged, not propagated: a lost snapshot is non-fatal.
    pub fn save_snapshot(&self, document_id: [u8; 16], snapshot: Vec<u8>) {
        let _ = self.write_snapshot(document_id, &snapshot);
    }

    /// Append one delta to a document's log (fire-and-forget).
    ///
    /// The next [`save_snapshot`](Self::save_snapshot) folds it away. Errors
    /// are logged: the next snapshot covers a delta lost here.
    pub fn save_delta(&self, document_id: [u8; 16], delta: Vec<u8>) {
        let hex = id_to_hex(&document_id);
        let key = format!("{hex}:{}", unique_suffix());
        let txn = match self
            .db
            .transaction_with_str_and_mode(STORE_DELTAS, IdbTransactionMode::Readwrite)
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Persistence: delta txn failed for {hex}: {e:?}");
                return;
            }
        };
        let Ok(store) = txn.object_store(STORE_DELTAS) else {
            return;
        };
        if let Err(e) = store.put_with_key(bytes_to_js(&delta).as_ref(), &JsValue::from_str(&key)) {
            tracing::warn!("Persistence: delta put failed for {hex}: {e:?}");
        }
    }

    /// Put `snapshot` and drop the document's deltas, in one transaction.
    /// `None` when the transaction could not be opened or written to.
    fn write_snapshot(
        &self,
        document_id: [u8; 16],
        snapshot: &[u8],
    ) -> Option<web_sys::IdbTransaction> {
        let key = id_to_hex(&document_id);
        let stores = js_sys::Array::of2(
            &JsValue::from_str(STORE_SNAPSHOTS),
            &JsValue::from_str(STORE_DELTAS),
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
            txn.object_store(STORE_SNAPSHOTS),
            txn.object_store(STORE_DELTAS),
        ) else {
            tracing::warn!("Persistence: save object_store failed for {key}");
            return None;
        };
        if let Err(e) =
            snapshots.put_with_key(bytes_to_js(snapshot).as_ref(), &JsValue::from_str(&key))
        {
            tracing::warn!("Persistence: save put failed for {key}: {e:?}");
            return None;
        }
        match deltas_of(&key) {
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
    /// accepted into it. `false` on any failure, so a caller can tell the user
    /// the file did not land instead of navigating to an empty document.
    pub fn save_snapshot_committed(
        &self,
        document_id: [u8; 16],
        snapshot: Vec<u8>,
    ) -> oneshot::Receiver<bool> {
        let (tx, rx) = answer();
        let Some(txn) = self.write_snapshot(document_id, &snapshot) else {
            tx.send(false);
            return rx;
        };

        let tx_done = tx.clone();
        let on_complete = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
            tx_done.send(true);
        });
        txn.set_oncomplete(Some(on_complete.as_ref().unchecked_ref()));
        on_complete.forget();

        // `error` and `abort` are distinct events on a transaction and either one
        // means the bytes are not there; both answer the same way.
        let tx_err = tx.clone();
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
            tx_err.send(false);
        });
        txn.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();
        let on_abort = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
            tx.send(false);
        });
        txn.set_onabort(Some(on_abort.as_ref().unchecked_ref()));
        on_abort.forget();

        rx
    }

    /// Load a document's most recent snapshot.
    ///
    /// Returns a one-shot receiver resolving to `Some(bytes)` if a snapshot
    /// exists, or `None` (no snapshot / read error). Awaited on the Main thread.
    pub fn load_snapshot(&self, document_id: [u8; 16]) -> oneshot::Receiver<Option<Vec<u8>>> {
        let (tx, rx) = answer();
        let key = id_to_hex(&document_id);

        let txn = match self.db.transaction_with_str(STORE_SNAPSHOTS) {
            Ok(t) => t,
            Err(_) => {
                tx.send(None);
                return rx;
            }
        };
        let store = match txn.object_store(STORE_SNAPSHOTS) {
            Ok(s) => s,
            Err(_) => {
                tx.send(None);
                return rx;
            }
        };
        let request = match store.get(&JsValue::from_str(&key)) {
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
                    .map(|a| a.to_vec());
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
    pub fn load_deltas(&self, document_id: [u8; 16]) -> oneshot::Receiver<Vec<Vec<u8>>> {
        let (tx, rx) = answer();
        let key = id_to_hex(&document_id);
        let request = self
            .db
            .transaction_with_str(STORE_DELTAS)
            .and_then(|txn| txn.object_store(STORE_DELTAS))
            .ok()
            .zip(deltas_of(&key))
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
                            .map(|a| a.to_vec())
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

    /// Load the pinned TOFU exchange key for `user_id`, if any.
    ///
    /// Returns a one-shot receiver resolving to `Some(key)` when this user's key
    /// has been confirmed before, or `None` (first invitation / read error). The
    /// Main thread awaits it and compares against the freshly fetched key to
    /// classify the trust state (`ambre-spec-key-verification.md`).
    pub fn load_trusted_key(&self, user_id: [u8; 16]) -> oneshot::Receiver<Option<[u8; 32]>> {
        let (tx, rx) = answer();
        let key = id_to_hex(&user_id);

        let txn = match self.db.transaction_with_str(STORE_TRUSTED_KEYS) {
            Ok(t) => t,
            Err(_) => {
                tx.send(None);
                return rx;
            }
        };
        let store = match txn.object_store(STORE_TRUSTED_KEYS) {
            Ok(s) => s,
            Err(_) => {
                tx.send(None);
                return rx;
            }
        };
        let request = match store.get(&JsValue::from_str(&key)) {
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
                    .and_then(|v| <[u8; 32]>::try_from(v).ok());
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
    pub fn save_trusted_key(&self, user_id: [u8; 16], key: [u8; 32]) {
        let hex = id_to_hex(&user_id);
        let txn = match self
            .db
            .transaction_with_str_and_mode(STORE_TRUSTED_KEYS, IdbTransactionMode::Readwrite)
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Persistence: trusted_key save txn failed for {hex}: {e:?}");
                return;
            }
        };
        let store = match txn.object_store(STORE_TRUSTED_KEYS) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Persistence: trusted_key object_store failed for {hex}: {e:?}");
                return;
            }
        };

        let array = Uint8Array::new_with_length(key.len() as u32);
        array.copy_from(&key);
        if let Err(e) = store.put_with_key(array.as_ref(), &JsValue::from_str(&hex)) {
            tracing::warn!("Persistence: trusted_key put failed for {hex}: {e:?}");
        }
    }

    /// Remove a document's snapshot and its deltas (called on document close /
    /// delete).
    pub fn delete_snapshot(&self, document_id: [u8; 16]) {
        let key = id_to_hex(&document_id);
        let stores = js_sys::Array::of2(
            &JsValue::from_str(STORE_SNAPSHOTS),
            &JsValue::from_str(STORE_DELTAS),
        );
        let Ok(txn) = self
            .db
            .transaction_with_str_sequence_and_mode(&stores, IdbTransactionMode::Readwrite)
        else {
            return;
        };
        if let Ok(store) = txn.object_store(STORE_SNAPSHOTS)
            && let Err(e) = store.delete(&JsValue::from_str(&key))
        {
            tracing::warn!("Persistence: delete failed for {key}: {e:?}");
        }
        if let (Ok(store), Some(range)) = (txn.object_store(STORE_DELTAS), deltas_of(&key))
            && let Err(e) = store.delete(&range)
        {
            tracing::warn!("Persistence: delta delete failed for {key}: {e:?}");
        }
    }
}

/// The key range holding every delta of the document keyed `hex`: from
/// `"{hex}:"` up to `"{hex};"`, the character after the colon.
fn deltas_of(hex: &str) -> Option<web_sys::IdbKeyRange> {
    web_sys::IdbKeyRange::bound(
        &JsValue::from_str(&format!("{hex}:")),
        &JsValue::from_str(&format!("{hex};")),
    )
    .ok()
}

/// A key suffix no other delta holds. It is the time, a counter for this
/// page, and a random draw for another tab writing in the same millisecond.
fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let count = NEXT.fetch_add(1, Ordering::Relaxed);
    let now = js_sys::Date::now() as u64;
    let draw = (js_sys::Math::random() * f64::from(u32::MAX)) as u32;
    format!("{now:015}:{count:010}:{draw:08x}")
}

/// Copy `bytes` into a fresh JS array.
fn bytes_to_js(bytes: &[u8]) -> Uint8Array {
    let array = Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    array
}

/// Extract the `IdbDatabase` from an IndexedDB open-request event target.
fn request_database(event: &web_sys::Event) -> Option<IdbDatabase> {
    event
        .target()
        .and_then(|t| t.dyn_into::<IdbOpenDbRequest>().ok())
        .and_then(|req| req.result().ok())
        .and_then(|r| r.dyn_into::<IdbDatabase>().ok())
}

/// Render a `DocumentId`'s 16 bytes as a 32-char lowercase hex key.
fn id_to_hex(id: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for byte in id {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

// ============================================================================
// wasm tests (headless Chrome via `wasm-pack test`). Exercise the two paths the
// TOFU store adds (`ambre-spec-key-verification.md`, CHROME-053) that a native
// test cannot reach. One is the trusted_keys roundtrip. The other is the
// DB_VERSION 1 -> 2 upgrade that must add trusted_keys WITHOUT a
// ConstraintError on the existing snapshots store.
// ============================================================================
#[cfg(test)]
mod wasm_tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

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

    /// Recreate the pre-TOFU (v1) database shape: version 1 with ONLY the
    /// `snapshots` store, holding one snapshot row. This is the state every
    /// existing user's browser is in before this upgrade ships.
    async fn seed_v1_with_snapshot(name: &str, doc_id: [u8; 16], bytes: &[u8]) {
        let (tx, rx) = answer::<()>();
        let req = factory().open_with_u32(name, 1).expect("open v1");

        // v1 upgrade: the old code created `snapshots` unconditionally.
        let upgrade = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(db) = request_database(&event) {
                let _ = db.create_object_store(STORE_SNAPSHOTS);
            }
        });
        req.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
        upgrade.forget();

        let key = id_to_hex(&doc_id);
        let payload = bytes.to_vec();
        let tx2 = tx.clone();
        let success = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(db) = request_database(&event) {
                if let Ok(txn) =
                    db.transaction_with_str_and_mode(STORE_SNAPSHOTS, IdbTransactionMode::Readwrite)
                {
                    if let Ok(store) = txn.object_store(STORE_SNAPSHOTS) {
                        let arr = Uint8Array::new_with_length(payload.len() as u32);
                        arr.copy_from(&payload);
                        let _ = store.put_with_key(arr.as_ref(), &JsValue::from_str(&key));
                    }
                }
                db.close();
            }
            tx2.send(());
        });
        req.set_onsuccess(Some(success.as_ref().unchecked_ref()));
        success.forget();

        let _ = rx.await;
    }

    #[wasm_bindgen_test]
    async fn trusted_key_roundtrips_and_absent_is_none() {
        let name = "ambre-test-trusted-roundtrip";
        delete_db(name).await;
        let store = PersistenceStore::open_named(name)
            .await
            .expect("channel")
            .expect("store opens");

        let user = [7u8; 16];
        // Absent before any write.
        assert_eq!(store.load_trusted_key(user).await.unwrap(), None);

        // Pin, then read it back byte-for-byte.
        let key = [0xABu8; 32];
        store.save_trusted_key(user, key);
        assert_eq!(store.load_trusted_key(user).await.unwrap(), Some(key));

        // A different user is still absent (keys are per-UserId).
        assert_eq!(store.load_trusted_key([9u8; 16]).await.unwrap(), None);
    }

    #[wasm_bindgen_test]
    async fn upgrade_v1_to_v2_adds_trusted_keys_and_keeps_snapshots() {
        let name = "ambre-test-migration-v1-v2";
        delete_db(name).await;

        let doc = [3u8; 16];
        let snapshot = vec![1u8, 2, 3, 4, 5];
        seed_v1_with_snapshot(name, doc, &snapshot).await;

        // Open with the current (v2) code: the guarded onupgradeneeded must add
        // `trusted_keys` without throwing ConstraintError on `snapshots`.
        let store = PersistenceStore::open_named(name)
            .await
            .expect("channel")
            .expect("v1 database upgrades to v2 cleanly");

        // The pre-existing snapshot survived the upgrade untouched.
        assert_eq!(store.load_snapshot(doc).await.unwrap(), Some(snapshot));

        // The new store is usable after the upgrade.
        let user = [4u8; 16];
        let key = [0x5Au8; 32];
        store.save_trusted_key(user, key);
        assert_eq!(store.load_trusted_key(user).await.unwrap(), Some(key));
    }

    #[wasm_bindgen_test]
    async fn deltas_log_until_a_snapshot_folds_them() {
        let name = "ambre-test-deltas";
        delete_db(name).await;
        let store = PersistenceStore::open_named(name)
            .await
            .expect("channel")
            .expect("store opens");
        let doc = [5u8; 16];
        let other = [6u8; 16];
        assert!(store.load_deltas(doc).await.unwrap().is_empty());

        store.save_delta(doc, vec![1]);
        store.save_delta(doc, vec![2]);
        store.save_delta(other, vec![9]);
        let mut deltas = store.load_deltas(doc).await.unwrap();
        deltas.sort();
        assert_eq!(
            deltas,
            vec![vec![1], vec![2]],
            "each document reads its own"
        );

        // A snapshot holds every delta before it, so it drops them.
        store.save_snapshot(doc, vec![7, 7]);
        assert!(store.load_deltas(doc).await.unwrap().is_empty());
        assert_eq!(store.load_snapshot(doc).await.unwrap(), Some(vec![7, 7]));
        assert_eq!(store.load_deltas(other).await.unwrap(), vec![vec![9]]);

        // A delete takes both.
        store.save_delta(doc, vec![3]);
        store.delete_snapshot(doc);
        assert!(store.load_deltas(doc).await.unwrap().is_empty());
        assert_eq!(store.load_snapshot(doc).await.unwrap(), None);
    }

    #[wasm_bindgen_test]
    async fn upgrade_v1_to_current_adds_deltas() {
        let name = "ambre-test-migration-deltas";
        delete_db(name).await;
        let doc = [8u8; 16];
        seed_v1_with_snapshot(name, doc, &[1, 2]).await;
        let store = PersistenceStore::open_named(name)
            .await
            .expect("channel")
            .expect("the database upgrades cleanly");
        assert_eq!(store.load_snapshot(doc).await.unwrap(), Some(vec![1, 2]));
        store.save_delta(doc, vec![4]);
        assert_eq!(store.load_deltas(doc).await.unwrap(), vec![vec![4]]);
    }
}
