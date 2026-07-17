//! IndexedDB-backed persistence for Loro CRDT snapshots.
//!
//! Saves a full Loro snapshot per document so an edit survives a refresh
//! straight from local storage (offline-first), independent of the network.
//!
//! Threading: every IndexedDB operation is async and resolves through a JS
//! callback, which only fires when an event loop is idle. The Logic thread
//! blocks on `flume::Selector` (Atomics-backed) and therefore starves its own
//! event loop, so the store is owned and driven by the **Main thread** (see
//! `ambre-app`): Logic exports snapshot bytes over a `flume` channel, the Main
//! thread persists them here. Reads/opens return a `flume::Receiver` the Main
//! thread awaits via `recv_async` (never blocking the page).

use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{IdbDatabase, IdbOpenDbRequest, IdbTransactionMode};

const DB_NAME: &str = "ambre-persistence";
// v2 adds the `trusted_keys` store (TOFU, ambre-spec-chrome.md §2.H). The
// upgrade handler creates each store only if absent, so bumping the version on
// an existing v1 database adds `trusted_keys` without touching `snapshots`.
const DB_VERSION: u32 = 2;
const STORE_SNAPSHOTS: &str = "snapshots";
/// TOFU pinned exchange keys: `UserId` (hex) -> the 32 raw bytes of the X25519
/// key confirmed on first invitation (`ambre-spec-chrome.md §2.H`).
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
    /// Returns a `flume::Receiver` resolving once to the store (or an error
    /// string). The caller awaits it with `recv_async` on the Main thread.
    pub fn open() -> flume::Receiver<Result<Self, String>> {
        Self::open_named(DB_NAME)
    }

    /// Open a named database at [`DB_VERSION`], running the shared upgrade path.
    /// [`open`](Self::open) uses the production [`DB_NAME`]; tests pass a unique
    /// name so each runs against an isolated database.
    fn open_named(db_name: &str) -> flume::Receiver<Result<Self, String>> {
        let (tx, rx) = flume::bounded(1);

        let factory = match web_sys::window().and_then(|w| w.indexed_db().ok().flatten()) {
            Some(f) => f,
            None => {
                let _ = tx.send(Err("IndexedDB unavailable".into()));
                return rx;
            }
        };

        let open_request: IdbOpenDbRequest = match factory.open_with_u32(db_name, DB_VERSION) {
            Ok(req) => req,
            Err(e) => {
                let _ = tx.send(Err(format!("open request failed: {e:?}")));
                return rx;
            }
        };

        // Create the object stores during upgrade (first visit or version bump).
        // This fires for a fresh database (v0->2) AND for an existing v1 database
        // (v1->2), so each store is created only if it does not already exist --
        // calling `create_object_store` for a store that exists throws
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
            }
        });
        open_request.set_onupgradeneeded(Some(on_upgrade.as_ref().unchecked_ref()));
        on_upgrade.forget();

        // Resolve on success with the open database.
        let tx_ok = tx.clone();
        let on_success = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            match request_database(&event) {
                Some(db) => {
                    let _ = tx_ok.send(Ok(Self { db }));
                }
                None => {
                    let _ = tx_ok.send(Err("open succeeded but no database".into()));
                }
            }
        });
        open_request.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
        on_success.forget();

        // Resolve on error.
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            let _ = tx.send(Err("IndexedDB open error".into()));
        });
        open_request.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();

        rx
    }

    /// Save a document's CRDT snapshot (fire-and-forget).
    ///
    /// Stores `snapshot` under `document_id` in the "snapshots" store. The
    /// transaction auto-commits when the Main thread returns to its event loop.
    /// Errors are logged, not propagated: a lost snapshot is non-fatal.
    pub fn save_snapshot(&self, document_id: [u8; 16], snapshot: Vec<u8>) {
        let key = id_to_hex(&document_id);
        let txn = match self
            .db
            .transaction_with_str_and_mode(STORE_SNAPSHOTS, IdbTransactionMode::Readwrite)
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Persistence: save txn failed for {key}: {e:?}");
                return;
            }
        };
        let store = match txn.object_store(STORE_SNAPSHOTS) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Persistence: save object_store failed for {key}: {e:?}");
                return;
            }
        };

        let array = Uint8Array::new_with_length(snapshot.len() as u32);
        array.copy_from(&snapshot);
        if let Err(e) = store.put_with_key(array.as_ref(), &JsValue::from_str(&key)) {
            tracing::warn!("Persistence: save put failed for {key}: {e:?}");
        }
    }

    /// Load a document's most recent snapshot.
    ///
    /// Returns a `flume::Receiver` resolving to `Some(bytes)` if a snapshot
    /// exists, or `None` (no snapshot / read error). Awaited on the Main thread.
    pub fn load_snapshot(&self, document_id: [u8; 16]) -> flume::Receiver<Option<Vec<u8>>> {
        let (tx, rx) = flume::bounded(1);
        let key = id_to_hex(&document_id);

        let txn = match self.db.transaction_with_str(STORE_SNAPSHOTS) {
            Ok(t) => t,
            Err(_) => {
                let _ = tx.send(None);
                return rx;
            }
        };
        let store = match txn.object_store(STORE_SNAPSHOTS) {
            Ok(s) => s,
            Err(_) => {
                let _ = tx.send(None);
                return rx;
            }
        };
        let request = match store.get(&JsValue::from_str(&key)) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Persistence: load get failed for {key}: {e:?}");
                let _ = tx.send(None);
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
                let _ = tx_ok.send(data);
            });
        request.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
        on_success.forget();

        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            let _ = tx.send(None);
        });
        request.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();

        rx
    }

    /// Load the pinned TOFU exchange key for `user_id`, if any.
    ///
    /// Returns a `flume::Receiver` resolving to `Some(key)` when this user's key
    /// has been confirmed before, or `None` (first invitation / read error). The
    /// Main thread awaits it and compares against the freshly fetched key to
    /// classify the trust state (`ambre-spec-chrome.md §2.H`).
    pub fn load_trusted_key(&self, user_id: [u8; 16]) -> flume::Receiver<Option<[u8; 32]>> {
        let (tx, rx) = flume::bounded(1);
        let key = id_to_hex(&user_id);

        let txn = match self.db.transaction_with_str(STORE_TRUSTED_KEYS) {
            Ok(t) => t,
            Err(_) => {
                let _ = tx.send(None);
                return rx;
            }
        };
        let store = match txn.object_store(STORE_TRUSTED_KEYS) {
            Ok(s) => s,
            Err(_) => {
                let _ = tx.send(None);
                return rx;
            }
        };
        let request = match store.get(&JsValue::from_str(&key)) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Persistence: trusted_key get failed for {key}: {e:?}");
                let _ = tx.send(None);
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
                let _ = tx_ok.send(data);
            });
        request.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
        on_success.forget();

        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            let _ = tx.send(None);
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

    /// Remove a document's snapshot (called on document close / delete).
    pub fn delete_snapshot(&self, document_id: [u8; 16]) {
        let key = id_to_hex(&document_id);
        let txn = match self
            .db
            .transaction_with_str_and_mode(STORE_SNAPSHOTS, IdbTransactionMode::Readwrite)
        {
            Ok(t) => t,
            Err(_) => return,
        };
        let Ok(store) = txn.object_store(STORE_SNAPSHOTS) else {
            return;
        };
        if let Err(e) = store.delete(&JsValue::from_str(&key)) {
            tracing::warn!("Persistence: delete failed for {key}: {e:?}");
        }
    }
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
// TOFU store adds (ambre-spec-chrome.md 2.H) that a native test cannot reach:
// the trusted_keys roundtrip, and the DB_VERSION 1 -> 2 upgrade that must add
// trusted_keys WITHOUT a ConstraintError on the existing snapshots store.
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
        let (tx, rx) = flume::bounded::<()>(1);
        let req = factory().delete_database(name).expect("delete_database");
        let tx2 = tx.clone();
        let done = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            let _ = tx2.send(());
        });
        req.set_onsuccess(Some(done.as_ref().unchecked_ref()));
        req.set_onerror(Some(done.as_ref().unchecked_ref()));
        done.forget();
        let _ = rx.recv_async().await;
    }

    /// Recreate the pre-TOFU (v1) database shape: version 1 with ONLY the
    /// `snapshots` store, holding one snapshot row. This is the state every
    /// existing user's browser is in before this upgrade ships.
    async fn seed_v1_with_snapshot(name: &str, doc_id: [u8; 16], bytes: &[u8]) {
        let (tx, rx) = flume::bounded::<()>(1);
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
            let _ = tx2.send(());
        });
        req.set_onsuccess(Some(success.as_ref().unchecked_ref()));
        success.forget();

        let _ = rx.recv_async().await;
    }

    #[wasm_bindgen_test]
    async fn trusted_key_roundtrips_and_absent_is_none() {
        let name = "ambre-test-trusted-roundtrip";
        delete_db(name).await;
        let store = PersistenceStore::open_named(name)
            .recv_async()
            .await
            .expect("channel")
            .expect("store opens");

        let user = [7u8; 16];
        // Absent before any write.
        assert_eq!(
            store.load_trusted_key(user).recv_async().await.unwrap(),
            None
        );

        // Pin, then read it back byte-for-byte.
        let key = [0xABu8; 32];
        store.save_trusted_key(user, key);
        assert_eq!(
            store.load_trusted_key(user).recv_async().await.unwrap(),
            Some(key)
        );

        // A different user is still absent (keys are per-UserId).
        assert_eq!(
            store
                .load_trusted_key([9u8; 16])
                .recv_async()
                .await
                .unwrap(),
            None
        );
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
            .recv_async()
            .await
            .expect("channel")
            .expect("v1 database upgrades to v2 cleanly");

        // The pre-existing snapshot survived the upgrade untouched.
        assert_eq!(
            store.load_snapshot(doc).recv_async().await.unwrap(),
            Some(snapshot)
        );

        // The new store is usable after the upgrade.
        let user = [4u8; 16];
        let key = [0x5Au8; 32];
        store.save_trusted_key(user, key);
        assert_eq!(
            store.load_trusted_key(user).recv_async().await.unwrap(),
            Some(key)
        );
    }
}
