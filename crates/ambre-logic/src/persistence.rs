//! IndexedDB-backed persistence for Loro CRDT state.
//!
//! Exports from the Logic thread: saves periodic Loro snapshots so a document
//! can reload instantly from local storage (offline-first), then catch up with
//! the server through the Sync thread's `RequestMissing` delta-sync path.
//!
//! Architecture: all IndexedDB operations are async. Writes are fire-and-forget
//! (spawned via `spawn_local`); reads block the async Logic loop via a oneshot
//! `flume` oneshot bridged from the `onsuccess` callback.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    IdbDatabase, IdbFactory, IdbObjectStore, IdbOpenDbRequest, IdbRequest,
    IdbTransaction, IdbTransactionMode,
};

const DB_NAME: &str = "ambre-persistence";
const DB_VERSION: u32 = 1;
const STORE_SNAPSHOTS: &str = "snapshots";

/// An open IndexedDB handle for persisting document CRDT snapshots.
///
/// Created once per Logic thread session via [`open`]. The database and its
/// object store are created/upgraded on first visit. All operations are async.
pub struct PersistenceStore {
    db: IdbDatabase,
}

impl PersistenceStore {
    /// Open the IndexedDB database, creating the object store on first visit.
    ///
    /// Returns an `flume::Receiver` that resolves to the store once the database
    /// is ready (success or error). The Logic thread's async loop awaits this
    /// before processing commands.
    pub fn open() -> flume::Receiver<Result<Self, String>> {
        let (tx, rx) = flume::bounded(1);

        let window = match web_sys::window() {
            Some(w) => w,
            None => {
                let _ = tx.send(Err("no window".into()));
                return rx;
            }
        };

        let factory: IdbFactory = match window.indexed_db() {
            Ok(Some(f)) => f,
            Ok(None) | Err(_) => {
                let _ = tx.send(Err("IndexedDB not available".into()));
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
        {
            let tx_clone = tx.clone();
            let on_upgrade = Closure::wrap(Box::new(move |event: web_sys::Event| {
                let db: IdbDatabase = match event
                    .target()
                    .and_then(|t| t.dyn_into::<IdbOpenDbRequest>().ok())
                    .and_then(|req| req.result().ok())
                    .and_then(|r| r.dyn_into::<IdbDatabase>().ok())
                {
                    Some(db) => db,
                    None => return,
                };
                if db.object_store_names().contains(STORE_SNAPSHOTS) {
                    return;
                }
                let _ = db.create_object_store(STORE_SNAPSHOTS);
            }) as Box<dyn FnMut(web_sys::Event)>);
            open_request.set_onupgradeneeded(Some(on_upgrade.as_ref().unchecked_ref()));
            on_upgrade.forget();
        }

        // Resolve on success: the database is open and ready.
        {
            let on_success = Closure::wrap(Box::new(move |_event: web_sys::Event| {
                let db = match open_request
                    .result()
                    .ok()
                    .and_then(|r| r.dyn_into::<IdbDatabase>().ok())
                {
                    Some(db) => db,
                    None => {
                        let _ = tx_clone.send(Err("open succeeded but no database".into()));
                        return;
                    }
                };
                let _ = tx_clone.send(Ok(Self { db }));
            }) as Box<dyn FnMut(web_sys::Event)>);
            open_request.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
            on_success.forget();
        }

        // Resolve on error.
        {
            let on_error = Closure::wrap(Box::new(move |event: web_sys::Event| {
                let msg = event
                    .target()
                    .and_then(|t| t.dyn_into::<IdbOpenDbRequest>().ok())
                    .and_then(|req| req.error())
                    .map(|e| {
                        format!(
                            "{:?}",
                            e.as_string().unwrap_or_else(|| "unknown error".into())
                        )
                    })
                    .unwrap_or_else(|| "open failed".into());
                let _ = tx.send(Err(msg));
            }) as Box<dyn FnMut(web_sys::Event)>);
            open_request.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            on_error.forget();
        }

        rx
    }

    /// Save a document's CRDT snapshot to IndexedDB (fire-and-forget).
    ///
    /// Serializes `snapshot` bytes into a `Uint8Array` and stores it under
    /// `document_id` in the "snapshots" object store. Errors are logged but not
    /// propagated — snapshot loss is non-fatal (the server still has the history).
    pub fn save_snapshot(&self, document_id: [u8; 16], snapshot: Vec<u8>) {
        let key = document_id_to_hex(&document_id);
        let Some(txn) = self
            .transaction(&[STORE_SNAPSHOTS], IdbTransactionMode::Readwrite)
        else {
            return;
        };
        let Some(store) = txn.object_store(STORE_SNAPSHOTS) else {
            return;
        };

        let array = Uint8Array::new_with_length(snapshot.len() as u32);
        array.copy_from(&snapshot);
        let value: &JsValue = array.as_ref();

        match store.put_with_key(value, &JsValue::from_str(&key)) {
            Ok(request) => {
                let _on_error = Self::log_error(request, "save_snapshot");
            }
            Err(e) => {
                tracing::warn!("Persistence: save_snapshot put failed for {key}: {e:?}");
            }
        }
    }

    /// Load a document's most recent CRDT snapshot from IndexedDB.
    ///
    /// Returns `None` if no snapshot exists for `document_id` or if the read
    /// fails. The Logic thread awaits this before requesting the server snapshot.
    pub fn load_snapshot(
        &self,
        document_id: [u8; 16],
    ) -> flume::Receiver<Option<Vec<u8>>> {
        let key = document_id_to_hex(&document_id);
        let (tx, rx) = flume::bounded(1);

        let Some(txn) = self
            .transaction(&[STORE_SNAPSHOTS], IdbTransactionMode::Readonly)
        else {
            let _ = tx.send(None);
            return rx;
        };
        let Some(store) = txn.object_store(STORE_SNAPSHOTS) else {
            let _ = tx.send(None);
            return rx;
        };

        match store.get(&JsValue::from_str(&key)) {
            Ok(request) => {
                let on_success = Closure::wrap(Box::new(move |_event: web_sys::Event| {
                    let result = request.result().ok().filter(|v| !v.is_undefined());
                    let data = result.and_then(|v| v.dyn_into::<Uint8Array>().ok()).map(|a| a.to_vec());
                    let _ = tx.send(data);
                }) as Box<dyn FnMut(web_sys::Event)>);
                request.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
                on_success.forget();

                let on_error = {
                    let tx = tx.clone();
                    Closure::wrap(Box::new(move |_event: web_sys::Event| {
                        let _ = tx.send(None);
                    }) as Box<dyn FnMut(web_sys::Event)>)
                };
                request.set_onerror(Some(on_error.as_ref().unchecked_ref()));
                on_error.forget();
            }
            Err(e) => {
                tracing::warn!("Persistence: load_snapshot get failed for {key}: {e:?}");
                let _ = tx.send(None);
            }
        }

        rx
    }

    /// Remove a document's snapshot from IndexedDB (called on document close).
    pub fn delete_snapshot(&self, document_id: [u8; 16]) {
        let key = document_id_to_hex(&document_id);
        let Some(txn) = self
            .transaction(&[STORE_SNAPSHOTS], IdbTransactionMode::Readwrite)
        else {
            return;
        };
        let Some(store) = txn.object_store(STORE_SNAPSHOTS) else {
            return;
        };

        match store.delete(&JsValue::from_str(&key)) {
            Ok(request) => {
                let _on_error = Self::log_error(request, "delete_snapshot");
            }
            Err(e) => {
                tracing::warn!("Persistence: delete_snapshot failed for {key}: {e:?}");
            }
        }
    }

    // -- Internal helpers -------------------------------------------------------

    fn transaction(
        &self,
        store_names: &[&str],
        mode: IdbTransactionMode,
    ) -> Option<IdbTransaction> {
        let names = js_sys::Array::new();
        for name in store_names {
            names.push(&JsValue::from_str(name));
        }
        self.db
            .transaction_with_str_sequence_and_mode(&names, mode)
            .map_err(|e| {
                tracing::warn!("Persistence: transaction failed: {e:?}");
            })
            .ok()
    }

    /// Attach a tracing-log on-error handler to a request, keeping the closure
    /// alive (leaked). The error is non-fatal so we just trace it.
    fn log_error(request: IdbRequest, context: &'static str) -> Closure<dyn FnMut(web_sys::Event)> {
        let on_error = Closure::wrap(Box::new(move |event: web_sys::Event| {
            let msg = event
                .target()
                .and_then(|t| t.dyn_into::<IdbRequest>().ok())
                .and_then(|req| req.error())
                .and_then(|e| e.as_string())
                .unwrap_or_else(|| "unknown".into());
            tracing::warn!("Persistence: {context} error: {msg}");
        }) as Box<dyn FnMut(web_sys::Event)>);
        request.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error
    }
}

/// Store a `DocumentId` as a hex string key.
fn document_id_to_hex(id: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for byte in id {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

impl IdbObjectStore {
    /// Typed helper: returns `self` as `IdbObjectStore` via wasm cast.
    fn from_store(store: &IdbObjectStore) -> Option<&IdbObjectStore> {
        Some(store)
    }
}
