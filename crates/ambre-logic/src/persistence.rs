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
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{IdbDatabase, IdbOpenDbRequest, IdbTransactionMode};

const DB_NAME: &str = "ambre-persistence";
const DB_VERSION: u32 = 1;
const STORE_SNAPSHOTS: &str = "snapshots";

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
        let (tx, rx) = flume::bounded(1);

        let factory = match web_sys::window().and_then(|w| w.indexed_db().ok().flatten()) {
            Some(f) => f,
            None => {
                let _ = tx.send(Err("IndexedDB unavailable".into()));
                return rx;
            }
        };

        let open_request: IdbOpenDbRequest = match factory.open_with_u32(DB_NAME, DB_VERSION) {
            Ok(req) => req,
            Err(e) => {
                let _ = tx.send(Err(format!("open request failed: {e:?}")));
                return rx;
            }
        };

        // Create the object store during upgrade (first visit or version bump).
        // The store does not exist yet when this fires, so create unconditionally.
        let on_upgrade = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(db) = request_database(&event) {
                let _ = db.create_object_store(STORE_SNAPSHOTS);
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
        let key = document_id_to_hex(&document_id);
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
        let key = document_id_to_hex(&document_id);

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
        let on_success = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
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

    /// Remove a document's snapshot (called on document close / delete).
    pub fn delete_snapshot(&self, document_id: [u8; 16]) {
        let key = document_id_to_hex(&document_id);
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
fn document_id_to_hex(id: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for byte in id {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}
