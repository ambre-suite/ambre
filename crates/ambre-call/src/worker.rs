//! The call worker's entry point, in the browser (wasm only).
//!
//! The UI thread spawns this worker for a call, then hands it the port the Sync worker
//! writes the sender keys on (`CALL-064`). Each `RTCRtpScriptTransform` the UI thread
//! attaches to a sender or a receiver raises an `rtctransform` event here, with the stream
//! of its encoded frames. The worker seals or opens each frame on the way through, and
//! writes on only what it sealed or opened (`CALL-001`, `CALL-022`).
//!
//! The worker tells the UI thread four things as a [`CallWorkerNote`]. It listens, it erased the
//! keys of the call, it dropped a frame for its size, or the screen slot carries a new sharer. As a [`SlotSender`], it tells the sender
//! a slot carries from its first chunk on (`CALL-084`). No key and no
//! chunk crosses to the UI thread.

use std::cell::RefCell;
use std::rc::Rc;

use ambre_core::index::Pos;
use ambre_core::{
    CallWorkerNote, ClearChunk, Confidential, Payload, PictureKind, Plaintext, SealedChunk,
    SlotSender, Track, TransformSide,
};
use ambre_crypto::call::KeyHandoff;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent, MessagePort};

use crate::{CallWorker, Handled, ScreenWatch, SlotWatch, Unsealed};

/// The state the transforms and the port share. The worker has one thread, so a `RefCell`
/// is the whole guard.
type Shared = Rc<RefCell<CallWorker>>;

/// Posts `note` to the UI thread.
fn tell(scope: &DedicatedWorkerGlobalScope, note: CallWorkerNote) {
    let _ = scope.post_message(&JsValue::from(note.byte()));
}

/// Posts `note` to the UI thread, as its bytes in a typed array: a number is a [`CallWorkerNote`].
fn tell_sender(scope: &DedicatedWorkerGlobalScope, note: SlotSender) {
    let bytes = js_sys::Uint8Array::from(note.to_bytes().as_slice());
    let _ = scope.post_message(&bytes);
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

/// Seals or opens the chunk `frame` of the track `track` in place, or answers `None` when the
/// worker drops it. The page reads on `scope` of an audio chunk dropped for its size
/// (`CALL-067`). It also reads of the first chunk of a new sender in the slot `watch` names
/// (`CALL-084`). It reads of the first chunk of a new sharer on the screen slot too (`CALL-107`).
fn transform(
    state: &Shared,
    side: TransformSide,
    track: Track,
    frame: &JsValue,
    scope: &DedicatedWorkerGlobalScope,
    watch: Option<&mut SlotWatch>,
    screen: &mut ScreenWatch,
) -> Option<()> {
    let data = js_sys::Reflect::get(frame, &"data".into()).ok()?;
    let buffer = data.dyn_into::<js_sys::ArrayBuffer>().ok()?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    let out = match (side, track) {
        (TransformSide::Send, Track::Camera | Track::Screen) => {
            // The browser names the kind of each encoded video chunk in its `type`.
            let kind = js_sys::Reflect::get(frame, &"type".into())
                .ok()
                .and_then(|kind| kind.as_string())
                .and_then(|kind| PictureKind::from_word(&kind))?;
            state
                .borrow_mut()
                .seal_video(track, &Payload::<ClearChunk>::new(bytes), kind)
                .ok()
                .map(Payload::into_bytes)
        }
        (TransformSide::Send, _) => {
            let sealed = state.borrow_mut().seal(&Payload::<ClearChunk>::new(bytes));
            if sealed == Err(Unsealed::TooLarge) {
                tell(scope, CallWorkerNote::Oversize);
            }
            sealed.ok().map(Payload::into_bytes)
        }
        (TransformSide::Receive, _) => {
            let opened = state
                .borrow_mut()
                .open(track, Payload::<SealedChunk>::new(bytes))?;
            if let Some(note) = watch.and_then(|watch| watch.opened(opened.sender)) {
                tell_sender(scope, note);
            }
            if track == Track::Screen && screen.opened(opened.sender) {
                tell(scope, CallWorkerNote::Screen);
            }
            Some(opened.clear.into_bytes())
        }
    }?;
    let array = js_sys::Uint8Array::from(out.as_slice());
    js_sys::Reflect::set(frame, &"data".into(), &array.buffer())
        .ok()
        .filter(|set| *set)
        .map(|_| ())
}

/// Pumps the frames of one transformer through the worker until its stream ends.
async fn pump(state: Shared, transformer: JsValue, scope: DedicatedWorkerGlobalScope) {
    let get = |name: &str| js_sys::Reflect::get(&transformer, &name.into()).ok();
    let options = get("options");
    let side = options
        .as_ref()
        .and_then(|options| js_sys::Reflect::get(options, &"side".into()).ok())
        .and_then(|side| side.as_string())
        .and_then(|side| TransformSide::from_word(&side));
    // The page names the track of each transform; a transform that names none carries audio.
    let track = options
        .as_ref()
        .and_then(|options| js_sys::Reflect::get(options, &"track".into()).ok())
        .and_then(|track| track.as_f64())
        .filter(|track| track.fract() == 0.0 && (0.0..=f64::from(u8::MAX)).contains(track))
        .map_or(Some(Track::Audio), |track| Track::from_bits(track as u8));
    // The page numbers each receive transform of audio; a slot it does not name tells nothing.
    let mut watch = options
        .and_then(|options| js_sys::Reflect::get(&options, &"slot".into()).ok())
        .and_then(|slot| slot.as_f64())
        .filter(|slot| slot.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(slot))
        .map(|slot| SlotWatch::new(Pos::new(slot as u32)));
    let readable = get("readable").and_then(|r| r.dyn_into::<web_sys::ReadableStream>().ok());
    let writable = get("writable").and_then(|w| w.dyn_into::<web_sys::WritableStream>().ok());
    let (Some(side), Some(track), Some(readable), Some(writable)) =
        (side, track, readable, writable)
    else {
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
    let mut screen = ScreenWatch::default();
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
        if transform(
            &state,
            side,
            track,
            &frame,
            &scope,
            watch.as_mut(),
            &mut screen,
        )
        .is_some()
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
        let scope = scope.clone();
        Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            if let Ok(transformer) = js_sys::Reflect::get(&e, &"transformer".into()) {
                let pumped = pump(Rc::clone(&state), transformer, scope.clone());
                wasm_bindgen_futures::spawn_local(pumped);
            }
        })
    };
    let _ = scope
        .add_event_listener_with_callback("rtctransform", on_transform.as_ref().unchecked_ref());
    on_transform.forget();

    tell(&scope, CallWorkerNote::Ready);
}
