// ambre-app/src/lib.rs
// ============================================================================
// UI Thread entry point - main JS thread (cdylib, #[wasm_bindgen(start)]).
//
// Responsibilities:
//   - Own HTMLCanvasElement, GPUDevice, Vello renderer, Masonry widget tree.
//   - Drive the requestAnimationFrame render loop (60 FPS).
//   - Capture all input (keyboard / pointer / touch / resize).
//   - Spawn the Logic Worker and Sync Worker; wire the MessageChannel between them.
//
// Out of scope:
//   - fetch(), WebSocket, crypto, compression - exclusively in the Sync Worker.
//   - CRDT mutations, formula evaluation - exclusively in the Logic Worker.
//
// IPC:
//   UiEnvelope  (postcard + ArrayBuffer transfer) -> Logic Worker
//   LogicEnvelope (postcard + ArrayBuffer transfer) <- Logic Worker
//
// Inbound queue:
//   The Logic Worker's `onmessage` callback fires outside the rAF loop.
//   To avoid mutating UI state mid-frame, incoming LogicEnvelopes are pushed
//   into `pending: Rc<RefCell<VecDeque<LogicEnvelope>>>` and drained at the
//   start of each rAF frame (step 1 of the pipeline).
//
//   VecDeque is used instead of a channel (e.g. flume) because the UI Thread
//   is single-threaded: no atomics, no Send/Sync requirement, O(1) push/drain.
// ============================================================================

use ambre_bus::{IPC_TEST_DOC_ID, LogicEnvelope, UiEnvelope};
use ambre_trame::{TrameCommand, TrameViewUpdate};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageChannel, MessageEvent, Worker, WorkerOptions, WorkerType};

// Only needed by the unified dispatcher.
#[cfg(feature = "unified")]
use web_sys::DedicatedWorkerGlobalScope;

// -----------------------------------------------------------------------------
// Bridge helpers
// -----------------------------------------------------------------------------

/// Serialize `msg` with postcard and transfer its `ArrayBuffer` to `worker`.
///
/// The `ArrayBuffer` is neutered after the call (zero-copy on the JS side).
#[allow(dead_code)]
fn send_to_worker<T: serde::Serialize>(worker: &Worker, msg: &T) {
    let bytes = postcard::to_allocvec(msg).expect("postcard serialize");
    let js_bytes = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    js_bytes.copy_from(&bytes);
    let ab = js_bytes.buffer();
    let transfer = js_sys::Array::of1(&ab);
    worker
        .post_message_with_transfer(&ab, &transfer)
        .expect("postMessage to worker");
}

/// Deserialize a `LogicEnvelope` from a transferred `ArrayBuffer` in a `MessageEvent`.
///
/// Returns `None` if the event data is not an `ArrayBuffer` (should never happen
/// in normal operation, but defensive).
fn extract_logic_envelope(event: &MessageEvent) -> Option<LogicEnvelope> {
    let ab = event.data().dyn_into::<js_sys::ArrayBuffer>().ok()?;
    let js_bytes = js_sys::Uint8Array::new(&ab);
    let mut bytes = vec![0u8; js_bytes.length() as usize];
    js_bytes.copy_to(&mut bytes);
    postcard::from_bytes(&bytes).ok()
}

// -----------------------------------------------------------------------------
// UiState
// -----------------------------------------------------------------------------

struct UiState {
    /// Handle used to send UiEnvelopes to the Logic Worker.
    logic_worker: Worker,

    /// Inbound queue: LogicEnvelopes pushed by the `onmessage` callback and
    /// drained at the start of each rAF frame.
    ///
    /// Shared (`Rc` clone) between `UiState` and the `onmessage` closure.
    pending: Rc<RefCell<VecDeque<LogicEnvelope>>>,
    // -- Real fields (wired in a future commit) ----------------------------
    // canvas:         web_sys::HtmlCanvasElement,
    // gpu_device:     wgpu::Device,
    // gpu_queue:      wgpu::Queue,
    // surface:        wgpu::Surface<'static>,
    // vello_renderer: vello::Renderer,
    // masonry_root:   masonry::RootWidget,
    // router:         Router,
}

#[allow(dead_code)]
impl UiState {
    // -- Step 1 - drain inbound queue -----------------------------------------

    /// Drain all `LogicEnvelope`s queued since the last frame and dispatch them.
    ///
    /// Called at the start of each rAF frame, before the Masonry event pass.
    /// Using `swap` avoids holding the borrow across the dispatch loop.
    fn drain_pending(&mut self) {
        let mut batch = VecDeque::new();
        std::mem::swap(&mut batch, &mut self.pending.borrow_mut());
        for envelope in batch {
            self.handle_logic_envelope(envelope);
        }
    }

    /// Dispatch a single [`LogicEnvelope`] received from the Logic Worker.
    fn handle_logic_envelope(&mut self, envelope: LogicEnvelope) {
        match envelope {
            LogicEnvelope::ViewportUpdate { document_id: _, payload } => {
                // IPC smoke-test: log key count and round-trip latency.
                // Remove once the Vello/wgpu render pipeline is wired.
                if let Ok(update) = postcard::from_bytes::<TrameViewUpdate>(&payload) {
                    let TrameViewUpdate::KeyCount { count, seq, t0_ms } = update;
                    let t1_ms = web_sys::window()
                        .and_then(|w| w.performance())
                        .map(|p| p.now())
                        .unwrap_or(0.0);
                    log::info!(
                        "[IPC] key_count={count} seq={seq} rtt={:.2}ms",
                        t1_ms - t0_ms
                    );
                }
                // TODO: submit display list to the Vello/wgpu render pipeline.
            }
            LogicEnvelope::PresenceUpdate {
                document_id: _,
                payload: _,
            } => {
                // Update collaborator cursor overlays.
                // self.request_frame();
            }
            LogicEnvelope::SessionReady {
                user_id: _,
                display_name: _,
                email: _,
            } => {
                // Navigate to the document dashboard.
                // self.router.navigate(Route::Dashboard);
                // self.request_frame();
            }
            LogicEnvelope::AccountSetupRequired => {
                // Navigate to the onboarding / passphrase setup screen.
                // self.router.navigate(Route::Setup);
                // self.request_frame();
            }
            LogicEnvelope::AccountCreated { recovery_words: _ } => {
                // Display recovery words modal (shown exactly once).
            }
            LogicEnvelope::DocumentList { documents: _ } => {
                // Populate the dashboard document list.
                // self.request_frame();
            }
            LogicEnvelope::DocumentCreated {
                document_id: _,
                doc_type: _,
            } => {
                // Open the newly created document.
            }
            LogicEnvelope::AuthError { message: _ } | LogicEnvelope::ApiError { .. } => {
                // Show error toast.
                // self.request_frame();
            }
            LogicEnvelope::ConnectionStatus {
                document_id: _,
                connected: _,
            } => {
                // Update online/offline indicator in the status bar.
                // self.request_frame();
            }
            _ => {
                // All other variants (sharing, media, channels, meetings, …)
                // will be routed to specific UI controllers when implemented.
            }
        }
    }

    // -- Step 2-7 - Masonry + Vello + GPU -------------------------------------

    /// Schedule a rAF callback if not already pending.
    fn request_frame(&mut self) {
        // if !self.pending_raf {
        //     self.pending_raf = true;
        //     let window = web_sys::window().unwrap();
        //     // closure holding Rc<RefCell<UiState>> - wired at bootstrap
        //     window.request_animation_frame(...).unwrap();
        // }
    }

    /// rAF render callback:
    ///
    /// ```text
    /// 1. drain_pending()                       (~0.1 ms)
    /// 2. Masonry on_event -> update             (~0.2 ms)
    ///    drain widget VecDeques -> send_to_worker (UiEnvelope batch)
    /// 3. Masonry layout (invalidated subtrees) (~0.5-2 ms)
    /// 4. AccessKit diff                        (~0.1 ms)
    /// 5. Masonry paint -> vello::Scene          (~0.5-1 ms)
    /// 6. Vello render -> wgpu intermediate      (~0.3-1 ms)
    /// 7. surface_texture.present()
    /// ```
    fn render(&mut self) {
        // self.pending_raf = false;

        // 1. Inbound queue
        self.drain_pending();

        // 2. Masonry event pass
        //    masonry_root.on_event(&mut event_cx, &winit_events);
        //    // After widgets emit events into their VecDeques:
        //    for cmd in widget_command_queue.borrow_mut().drain(..) {
        //        send_to_worker(&self.logic_worker, &cmd);
        //    }

        // 3. Layout
        //    masonry_root.layout(&mut layout_cx);

        // 4. Accessibility
        //    masonry_root.accessibility(&mut access_cx);

        // 5. Paint
        //    let mut scene = vello::Scene::new();
        //    masonry_root.paint(&mut paint_cx, &mut scene);

        // 6. GPU render
        //    vello_renderer.render_to_texture(&device, &queue, &scene, &view, &params);
        //    encoder.copy_texture_to_texture(&src, &dst, extent);

        // 7. Present
        //    surface_texture.present();
    }
}

// -----------------------------------------------------------------------------
// Entry point dispatcher + bootstrap
// -----------------------------------------------------------------------------

/// Entry point - behaviour differs between build modes.
///
/// unified (default, dev): one binary loaded three times. The JS bootstrap
/// passes `{ name: "logic" | "sync" }` to the Worker constructor; the main
/// thread has no name. `DedicatedWorkerGlobalScope::name()` is the discriminant.
///
/// standalone (release): this binary is only ever loaded by the UI Thread.
/// Workers have their own binaries and their own `#[wasm_bindgen(start)]`.
#[wasm_bindgen(start)]
pub fn main() {
    // Logging and panic hooks must be initialized in EVERY thread/worker separately.
    console_log::init_with_level(log::Level::Debug).ok();
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));

    #[cfg(feature = "unified")]
    {
        let global = js_sys::global();
        if let Ok(scope) = global.dyn_into::<DedicatedWorkerGlobalScope>() {
            match scope.name().as_str() {
                "logic" => ambre_logic_worker::start(),
                "sync"  => ambre_sync_worker::start(),
                name    => panic!("unknown worker name: {name}"),
            }
            return;
        }
    }
    ui_main();
}

// -----------------------------------------------------------------------------
// rAF loop
// -----------------------------------------------------------------------------

/// Schedule a single `requestAnimationFrame` callback that calls `render()` and
/// then reschedules itself.
///
/// Each frame allocates and `forget()`s a `Closure` — the browser holds it alive
/// for exactly one frame then drops it. This is the standard pattern for a
/// continuous rAF loop from WASM when the closure must capture an `Rc`.
///
/// Conditional rendering (skipping paint when there is no invalidation) will be
/// added once the Masonry dirty-flag plumbing is in place.
fn schedule_raf(state: Rc<RefCell<UiState>>) {
    let state_for_frame = Rc::clone(&state);
    let cb = Closure::<dyn FnMut(f64)>::new(move |_timestamp: f64| {
        state_for_frame.borrow_mut().render();
        schedule_raf(Rc::clone(&state_for_frame));
    });
    web_sys::window()
        .expect("no window in UI Thread")
        .request_animation_frame(cb.as_ref().unchecked_ref())
        .expect("requestAnimationFrame");
    cb.forget();
}

fn ui_main() {
    log::info!("Ambre UI Thread starting");

    // -- Spawn Workers (each is a separate WASM module, isolated linear memory) -

    // Workers are ES module workers. The name is the discriminant used by the
    // dispatcher in #[wasm_bindgen(start)] to route to the correct start().
    let logic_opts = WorkerOptions::new();
    logic_opts.set_type(WorkerType::Module);
    logic_opts.set_name("logic");

    let sync_opts = WorkerOptions::new();
    sync_opts.set_type(WorkerType::Module);
    sync_opts.set_name("sync");

    // Both modes use the same bootstrap filenames. The content differs:
    //   unified:    logic_worker.js imports ./ambre-app.js (same binary, "logic" role)
    //   standalone: logic_worker.js imports ./logic/logic_worker.js (dedicated binary)
    let logic_worker =
        Worker::new_with_options("./logic_worker.js", &logic_opts).expect("spawn Logic Worker");
    let sync_worker =
        Worker::new_with_options("./sync_worker.js", &sync_opts).expect("spawn Sync Worker");

    // -- MessageChannel: direct Logic <-> Sync (bypasses UI Thread) ------------
    //
    // port1 -> Logic Worker, port2 -> Sync Worker.
    // Ports are transferred inside the ready handlers below, not immediately,
    // because postMessage races WASM initialization (the workers set their
    // onmessage handler asynchronously after init() resolves).
    //
    // Handshake (per worker):
    //   1. Worker sets its onmessage handler.
    //   2. Worker posts "ready" to the UI Thread.
    //   3. UI Thread one-shot handler receives "ready" and transfers the port.
    //   4. UI Thread replaces the one-shot handler with the steady-state handler.
    // All logic lives in Rust; the .js bootstrap files stay as one-liners.

    let channel = MessageChannel::new().expect("create MessageChannel");

    let pending: Rc<RefCell<VecDeque<LogicEnvelope>>> = Rc::new(RefCell::new(VecDeque::new()));

    // -- Logic Worker ready handler --------------------------------------------
    {
        let port1 = channel.port1();
        let pending = Rc::clone(&pending);
        let logic_worker_c = logic_worker.clone();
        let on_ready = Closure::<dyn FnMut(MessageEvent)>::once(move |_: MessageEvent| {
            let init_msg = js_sys::Object::new();
            js_sys::Reflect::set(&init_msg, &"syncPort".into(), &port1).unwrap();
            let transfer = js_sys::Array::of1(&port1);
            logic_worker_c
                .post_message_with_transfer(&init_msg, &transfer)
                .expect("transfer syncPort to Logic Worker");

            // Replace one-shot handler with the steady-state LogicEnvelope handler.
            let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                if let Some(envelope) = extract_logic_envelope(&event) {
                    pending.borrow_mut().push_back(envelope);
                }
            });
            logic_worker_c.set_onmessage(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        });
        logic_worker.set_onmessage(Some(on_ready.as_ref().unchecked_ref()));
        on_ready.forget();
    }

    // -- Sync Worker ready handler ---------------------------------------------
    // The Sync Worker never messages the UI Thread directly; no steady-state
    // handler is needed on this side.
    {
        let port2 = channel.port2();
        let sync_worker_c = sync_worker.clone();
        let on_ready = Closure::<dyn FnMut(MessageEvent)>::once(move |_: MessageEvent| {
            let init_msg = js_sys::Object::new();
            js_sys::Reflect::set(&init_msg, &"logicPort".into(), &port2).unwrap();
            let transfer = js_sys::Array::of1(&port2);
            sync_worker_c
                .post_message_with_transfer(&init_msg, &transfer)
                .expect("transfer logicPort to Sync Worker");
        });
        sync_worker.set_onmessage(Some(on_ready.as_ref().unchecked_ref()));
        on_ready.forget();
    }

    // -- Build UiState ---------------------------------------------------------

    let state = Rc::new(RefCell::new(UiState {
        logic_worker,
        pending: Rc::clone(&pending),
    }));

    // -- Input events (wired when Masonry/Winit bootstrap is complete) ---------
    //
    // P1 - Scroll (< 15 ms):
    //   Portal.on_scroll -> UiEnvelope::ScrollTo { document_id, top_row, … }
    //   -> send_to_worker(&logic_worker, &envelope)
    //
    // P2 - Edit:
    //   Grid2D.on_double_click -> UiEnvelope::AppCommand { document_id, payload }
    //   -> send_to_worker(&logic_worker, &envelope)
    //
    // P3 - Session:
    //   LoginButton.on_click -> UiEnvelope::Unlock { passphrase }
    //   -> send_to_worker(&logic_worker, &envelope)

    // -- IPC smoke-test: keydown -> TrameController -> RTT log -----------------
    // Temporary DOM listener: exercises the full IPC round-trip before
    // Winit/Masonry input capture is wired. Replaced by on_event in
    // ambre-app/src/trame/ once the GPU surface is initialized.
    {
        let logic_worker = state.borrow().logic_worker.clone();
        let seq = Rc::new(Cell::new(0u64));
        let cb =
            Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |_: web_sys::KeyboardEvent| {
                let t0_ms = web_sys::window()
                    .and_then(|w| w.performance())
                    .map(|p| p.now())
                    .unwrap_or(0.0);
                let n = seq.get();
                seq.set(n + 1);
                let payload = postcard::to_allocvec(&TrameCommand::KeyPress { seq: n, t0_ms })
                    .expect("serialize TrameCommand");
                send_to_worker(
                    &logic_worker,
                    &UiEnvelope::AppCommand { document_id: IPC_TEST_DOC_ID, payload },
                );
            });
        web_sys::window()
            .expect("no window")
            .document()
            .expect("no document")
            .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())
            .expect("add keydown listener");
        cb.forget();
    }

    log::info!("Ambre UI Thread ready");

    // Start the requestAnimationFrame loop. Each frame drains the inbound
    // LogicEnvelope queue and runs the Masonry/Vello pipeline (steps 2-7 are
    // stubs until the GPU surface is wired).
    schedule_raf(state);

    // Hide the loading screen. Moves to after surface_texture.present() once
    // the GPU surface is wired, so the screen only disappears on a real frame.
    if let Some(window) = web_sys::window()
        && let Ok(f) = js_sys::Reflect::get(&window, &"ambreReady".into())
        && let Ok(f) = f.dyn_into::<js_sys::Function>()
    {
        let _ = f.call0(&window);
    }
}
