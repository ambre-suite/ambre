//! The call worker's entry point, in the browser (wasm only).
//!
//! The UI thread spawns this worker for a call, then hands it the port the Sync worker
//! writes the sender keys on (`CALL-064`). Each `RTCRtpScriptTransform` the UI thread
//! attaches to a sender or a receiver raises an `rtctransform` event here, with the stream
//! of its encoded frames. The worker seals or opens each frame on the way through, and
//! writes on only what it sealed or opened (`CALL-001`, `CALL-022`).
//!
//! The worker tells the UI thread two things alone, as a [`CallWorkerNote`]: that it listens,
//! and that it erased the keys of the call. No key and no chunk crosses to the UI thread.

use std::cell::RefCell;
use std::rc::Rc;

use ambre_core::TransformSide;
use ambre_core::{CallWorkerNote, ClearChunk, Confidential, Payload, Plaintext, SealedChunk};
use ambre_crypto::call::KeyHandoff;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent, MessagePort};

use crate::{CallWorker, Handled};

/// The state the transforms and the port share. The worker has one thread, so a `RefCell`
/// is the whole guard.
type Shared = Rc<RefCell<CallWorker>>;

/// Posts `note` to the UI thread.
fn tell(scope: &DedicatedWorkerGlobalScope, note: CallWorkerNote) {
    let _ = scope.post_message(&JsValue::from(note.byte()));
}

/// Reads one handoff the Sync worker posted on the port. The bytes cross as a transferred
/// buffer, which the worker zeroes once it holds the copy it reads.
fn read_handoff(data: &JsValue) -> Option<KeyHandoff> {
    let buffer: js_sys::ArrayBuffer = data.clone().dyn_into().ok()?;
    let view = js_sys::Uint8Array::new(&buffer);
    let port = Confidential::<Plaintext>::new(view.to_vec());
    view.fill(0, 0, view.length());
    KeyHandoff::from_port(&port).ok()
}

/// Listens on the port of the Sync worker.
fn listen(port: &MessagePort, state: Shared, scope: DedicatedWorkerGlobalScope) {
    let on_handoff = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        let Some(handoff) = read_handoff(&e.data()) else {
            return;
        };
        if state.borrow_mut().hand(handoff) == Handled::Erased {
            tell(&scope, CallWorkerNote::Erased);
        }
    });
    port.set_onmessage(Some(on_handoff.as_ref().unchecked_ref()));
    // The port lives as long as the worker, and the closure with it.
    on_handoff.forget();
}

/// Seals or opens the frame `frame` in place, or answers `false` when the worker drops it.
fn transform(state: &Shared, side: TransformSide, frame: &JsValue) -> bool {
    let Ok(data) = js_sys::Reflect::get(frame, &"data".into()) else {
        return false;
    };
    let Ok(buffer) = data.dyn_into::<js_sys::ArrayBuffer>() else {
        return false;
    };
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    let out = match side {
        TransformSide::Send => state
            .borrow_mut()
            .seal(&Payload::<ClearChunk>::new(bytes))
            .map(Payload::into_bytes),
        TransformSide::Receive => state
            .borrow_mut()
            .open(Payload::<SealedChunk>::new(bytes))
            .map(Payload::into_bytes),
    };
    let Some(out) = out else {
        return false;
    };
    let array = js_sys::Uint8Array::from(out.as_slice());
    js_sys::Reflect::set(frame, &"data".into(), &array.buffer()).is_ok()
}

/// Pumps the frames of one transformer through the worker until its stream ends.
async fn pump(state: Shared, transformer: JsValue) {
    let get = |name: &str| js_sys::Reflect::get(&transformer, &name.into()).ok();
    let side = get("options")
        .and_then(|options| js_sys::Reflect::get(&options, &"side".into()).ok())
        .and_then(|side| side.as_string())
        .and_then(|side| TransformSide::from_word(&side));
    let readable = get("readable").and_then(|r| r.dyn_into::<web_sys::ReadableStream>().ok());
    let writable = get("writable").and_then(|w| w.dyn_into::<web_sys::WritableStream>().ok());
    let (Some(side), Some(readable), Some(writable)) = (side, readable, writable) else {
        return;
    };
    let Ok(reader) = readable
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()
    else {
        return;
    };
    let Ok(writer) = writable.get_writer() else {
        return;
    };
    loop {
        let Ok(read) = wasm_bindgen_futures::JsFuture::from(reader.read()).await else {
            return;
        };
        let done = js_sys::Reflect::get(&read, &"done".into())
            .ok()
            .and_then(|d| d.as_bool())
            .unwrap_or(true);
        if done {
            return;
        }
        let Ok(frame) = js_sys::Reflect::get(&read, &"value".into()) else {
            return;
        };
        if transform(&state, side, &frame)
            && wasm_bindgen_futures::JsFuture::from(writer.write_with_chunk(&frame))
                .await
                .is_err()
        {
            return;
        }
    }
}

#[expect(
    clippy::expect_used,
    reason = "a call worker outside a dedicated worker scope has no channel to report on"
)]
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    let scope: DedicatedWorkerGlobalScope = js_sys::global()
        .dyn_into()
        .expect("the call worker must run in a DedicatedWorkerGlobalScope");
    let state: Shared = Rc::new(RefCell::new(CallWorker::default()));

    // The UI thread posts the port of the Sync worker once, in the ports of a message.
    let on_port = {
        let state = Rc::clone(&state);
        let scope = scope.clone();
        Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            if let Ok(port) = e.ports().get(0).dyn_into::<MessagePort>() {
                listen(&port, Rc::clone(&state), scope.clone());
            }
        })
    };
    scope.set_onmessage(Some(on_port.as_ref().unchecked_ref()));
    on_port.forget();

    // Each transform the UI thread attaches raises one event, with its transformer.
    let on_transform = {
        let state = Rc::clone(&state);
        Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            if let Ok(transformer) = js_sys::Reflect::get(&e, &"transformer".into()) {
                wasm_bindgen_futures::spawn_local(pump(Rc::clone(&state), transformer));
            }
        })
    };
    let _ = scope
        .add_event_listener_with_callback("rtctransform", on_transform.as_ref().unchecked_ref());
    on_transform.forget();

    tell(&scope, CallWorkerNote::Ready);
}
