// ambre-sync/src/worker.rs
// ============================================================================
// Sync Web Worker entry point (wasm only).
//
// This crate compiles to a standalone `cdylib` that runs in its OWN wasm
// instance inside a dedicated Web Worker. That instance is separate from the
// Main instance (UI + Logic). It owns all crypto, the WebSocket and HTTP I/O,
// and talks to Main exclusively over `postMessage`:
//
//   Main  --(postcard SyncCommand bytes, Transferable ArrayBuffer)-->  here
//   here  --(postcard LogicEvent  bytes, Transferable ArrayBuffer)-->  Main
//
// Inbound bytes are decoded in `onmessage` and pushed into an in-worker
// `ambre_bus::conduit::unbounded` channel feeding [`crate::run_wasm`]. Outbound
// events go through a [`crate::LogicSink`] that encodes and posts them back. The
// loop awaits its channels, so the event loop stays free for WebSocket / fetch
// callbacks.
// ============================================================================

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

use ambre_bus::{LogicEvent, SyncCommand};

use crate::{LogicSink, run_wasm};

/// Copy a JS `ArrayBuffer` (the `postMessage` payload) into a `Vec<u8>`.
fn array_buffer_to_vec(data: &JsValue) -> Vec<u8> {
    js_sys::Uint8Array::new(data).to_vec()
}

/// Encode `bytes` into a fresh `Uint8Array` and `postMessage` its backing
/// `ArrayBuffer` to Main, transferring it (zero-copy move, no structured clone).
fn post_bytes(scope: &DedicatedWorkerGlobalScope, bytes: &[u8]) {
    let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    let buffer = array.buffer();
    let transfer = js_sys::Array::of1(&buffer);
    if let Err(e) = scope.post_message_with_transfer(&buffer, &transfer) {
        tracing::warn!("[Sync] post_message to Main failed: {e:?}");
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    // Own tracing subscriber: this is a distinct wasm instance from Main.
    tracing_wasm::set_as_global_default();

    let scope: DedicatedWorkerGlobalScope = js_sys::global()
        .dyn_into()
        .expect("Sync worker must run in a DedicatedWorkerGlobalScope");

    // Same-origin worker => its host is the app host (API + WS are proxied
    // same-origin; see the cross-origin isolation note in the spec).
    let api_base = scope.location().host();

    // Outbound sink: encode each LogicEvent and post it back to Main.
    let sink_scope = scope.clone();
    let logic_sink = LogicSink::new(
        move |event: LogicEvent| match postcard::to_allocvec(&event) {
            Ok(bytes) => post_bytes(&sink_scope, &bytes),
            Err(e) => tracing::error!("[Sync] failed to encode LogicEvent: {e}"),
        },
    );

    // Inbound: onmessage decodes SyncCommand bytes and feeds the select loop.
    let (cmd_tx, cmd_rx) = ambre_bus::conduit::unbounded::<SyncCommand>();
    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        let bytes = array_buffer_to_vec(&e.data());
        match postcard::from_bytes::<SyncCommand>(&bytes) {
            Ok(cmd) => {
                let _ = cmd_tx.send(cmd);
            }
            Err(err) => tracing::error!("[Sync] failed to decode SyncCommand: {err}"),
        }
    });
    scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    // Leak the closure: it lives for the whole worker lifetime.
    onmessage.forget();

    // Readiness handshake: the inbound `onmessage` is now installed, so every
    // SyncCommand Main posts from here on is guaranteed to be delivered. Main
    // buffers all commands until it sees this sentinel. That closes the startup
    // race where a command posted during the worker's async wasm bootstrap
    // (before `onmessage` existed) was silently dropped. The boot `DevLogin` was
    // the notable one: its loss left the session key unset and every
    // `OpenDocument` queued forever behind the loading overlay. An empty payload
    // is the sentinel: a postcard-encoded LogicEvent is never zero-length.
    post_bytes(&scope, &[]);

    wasm_bindgen_futures::spawn_local(run_wasm(cmd_rx, logic_sink, api_base));
}
