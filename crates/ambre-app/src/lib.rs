// ambre-app/src/lib.rs
// ============================================================================
// UI Thread entry point -- main JS thread (cdylib, #[wasm_bindgen(start)]).
//
// Responsibilities:
//   - Own HTMLCanvasElement, wgpu Device/Queue/Surface, Vello renderer.
//   - Drive the requestAnimationFrame render loop (60 FPS).
//   - Capture keyboard input (arrow keys → TrameCommand::ScrollViewport).
//   - Spawn Logic Worker + Sync Worker; wire the MessageChannel between them.
//
// GPU bootstrap is async (wgpu::Adapter::request_device is a Future even on
// WASM). It runs via wasm_bindgen_futures::spawn_local and writes into
// `UiState::gpu: Option<GpuState>` once complete. The rAF loop is started
// immediately; frames before GPU init return early with no rendering.
//
// Render pipeline (each frame, once GPU is ready):
//   1. drain_pending() -- apply incoming LogicEnvelopes
//   2. send initial ScrollViewport if GPU + Logic Worker are both ready
//   3. build vello::Scene via trame::renderer::draw_grid()
//   4. vello::Renderer::render_to_texture() -- GPU compute pass
//   5. TextureBlitter::copy()              -- blit to surface texture
//   6. SurfaceTexture::present()
//
// IPC:
//   UiEnvelope  (postcard + ArrayBuffer transfer) -> Logic Worker
//   LogicEnvelope (postcard + ArrayBuffer transfer) <- Logic Worker
// ============================================================================

mod trame;

use ambre_bus::{IPC_TEST_DOC_ID, LogicEnvelope, UiEnvelope};
use ambre_trame::TrameViewUpdate;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, MessageChannel, MessageEvent, Worker, WorkerOptions, WorkerType};

#[cfg(feature = "unified")]
use web_sys::DedicatedWorkerGlobalScope;

// -----------------------------------------------------------------------------
// Bridge helpers
// -----------------------------------------------------------------------------

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

fn extract_logic_envelope(event: &MessageEvent) -> Option<LogicEnvelope> {
    let ab = event.data().dyn_into::<js_sys::ArrayBuffer>().ok()?;
    let js_bytes = js_sys::Uint8Array::new(&ab);
    let mut bytes = vec![0u8; js_bytes.length() as usize];
    js_bytes.copy_to(&mut bytes);
    postcard::from_bytes(&bytes).ok()
}

// -----------------------------------------------------------------------------
// GpuState
// -----------------------------------------------------------------------------

/// All wgpu + Vello resources needed for one render frame.
///
/// Created asynchronously in `bootstrap_gpu()` after the HTML canvas is
/// available; stored as `Option<GpuState>` in `UiState`.
struct GpuState {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// wgpu surface backed by the HTML canvas element.
    surface: wgpu::Surface<'static>,
    /// Intermediate Rgba8Unorm texture that Vello renders into (compute shader).
    #[allow(dead_code)]
    target_texture: wgpu::Texture,
    target_view: wgpu::TextureView,
    /// Blits `target_view` → surface texture each frame.
    blitter: wgpu::util::TextureBlitter,
    renderer: vello::Renderer,
    scene: vello::Scene,
    #[allow(dead_code)]
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
}

// -----------------------------------------------------------------------------
// UiState
// -----------------------------------------------------------------------------

struct UiState {
    logic_worker: Worker,
    pending: Rc<RefCell<VecDeque<LogicEnvelope>>>,
    gpu: Option<GpuState>,
    trame: trame::TrameUiState,
    /// Set to true once the Logic Worker ready handshake completes (syncPort
    /// transferred, steady-state handler installed). The initial ScrollViewport
    /// is deferred until this flag is set to avoid sending during Phase-1 init.
    logic_worker_ready: bool,
    /// Set to true after the first ScrollViewport is sent on startup.
    initial_viewport_sent: bool,
}

impl UiState {
    // -- Inbound queue --------------------------------------------------------

    fn drain_pending(&mut self) {
        let mut batch = VecDeque::new();
        std::mem::swap(&mut batch, &mut self.pending.borrow_mut());
        for envelope in batch {
            self.handle_logic_envelope(envelope);
        }
    }

    fn handle_logic_envelope(&mut self, envelope: LogicEnvelope) {
        if let LogicEnvelope::ViewportUpdate { document_id: _, payload } = envelope
            && let Ok(update) = postcard::from_bytes::<TrameViewUpdate>(&payload)
        {
            self.trame.on_viewport_data(update);
        }
    }

    // -- Render ---------------------------------------------------------------

    fn render(&mut self) {
        self.drain_pending();

        // Send the initial viewport request on the first frame where both the
        // GPU and the Logic Worker are ready. Doing this here (rather than in
        // the GPU-init async block) avoids a race where the message is sent
        // before the Logic Worker has finished its Phase-1 handshake.
        if self.gpu.is_some() && self.logic_worker_ready && !self.initial_viewport_sent {
            self.initial_viewport_sent = true;
            let cmd = self.trame.scroll_viewport_cmd();
            let payload = postcard::to_allocvec(&cmd).expect("serialize TrameCommand");
            let logic_worker = self.logic_worker.clone();
            send_to_worker(
                &logic_worker,
                &UiEnvelope::AppCommand { document_id: IPC_TEST_DOC_ID, payload },
            );
        }

        let Some(ref mut gpu) = self.gpu else { return };

        gpu.scene.reset();
        trame::renderer::draw_grid(&mut gpu.scene, &self.trame);

        let params = vello::RenderParams {
            base_color: vello::peniko::Color::WHITE,
            width: gpu.width,
            height: gpu.height,
            antialiasing_method: vello::AaConfig::Area,
        };

        // render_to_texture needs &mut Renderer + &Device + &Queue + &Scene + &TextureView.
        // Destructure gpu so the borrow checker sees them as independent field borrows.
        {
            let GpuState { device, queue, target_view, renderer, scene, .. } = &mut *gpu;
            if let Err(e) =
                renderer.render_to_texture(device, queue, scene, target_view, &params)
            {
                tracing::error!("vello render_to_texture: {e:?}");
                return;
            }
        }

        let surface_tex = match gpu.surface.get_current_texture() {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("get_current_texture: {e:?}");
                return;
            }
        };
        let dst_view = surface_tex.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.blitter.copy(&gpu.device, &mut encoder, &gpu.target_view, &dst_view);
        gpu.queue.submit(std::iter::once(encoder.finish()));
        surface_tex.present();
    }
}

// -----------------------------------------------------------------------------
// rAF loop
// -----------------------------------------------------------------------------

fn schedule_raf(state: Rc<RefCell<UiState>>) {
    let state_for_frame = Rc::clone(&state);
    let cb = Closure::<dyn FnMut(f64)>::new(move |_: f64| {
        state_for_frame.borrow_mut().render();
        schedule_raf(Rc::clone(&state_for_frame));
    });
    web_sys::window()
        .expect("no window in UI Thread")
        .request_animation_frame(cb.as_ref().unchecked_ref())
        .expect("requestAnimationFrame");
    cb.forget();
}

// -----------------------------------------------------------------------------
// GPU bootstrap (async)
// -----------------------------------------------------------------------------

async fn bootstrap_gpu(canvas: HtmlCanvasElement) -> GpuState {
    let width = canvas.width();
    let height = canvas.height();

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        ..Default::default()
    });

    // HtmlCanvasElement is 'static (JS GC-managed, no Rust lifetime).
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
        .expect("create wgpu surface from canvas");

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
        .expect("no WebGPU adapter -- is WebGPU enabled in this browser?");

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .expect("request_device failed");

    // Choose a surface format compatible with Vello (prefers Rgba8Unorm).
    let caps = surface.get_capabilities(&adapter);
    let format = caps
        .formats
        .iter()
        .find(|&&f| {
            f == wgpu::TextureFormat::Rgba8Unorm || f == wgpu::TextureFormat::Bgra8Unorm
        })
        .copied()
        .unwrap_or(caps.formats[0]);

    surface.configure(
        &device,
        &wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
        },
    );

    // Intermediate texture: Vello renders into this (compute shader requirement).
    let target_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vello_target"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let target_view = target_texture.create_view(&Default::default());
    let blitter = wgpu::util::TextureBlitter::new(&device, format);

    let renderer = vello::Renderer::new(
        &device,
        vello::RendererOptions {
            use_cpu: false,
            antialiasing_support: vello::AaSupport::area_only(),
            num_init_threads: std::num::NonZeroUsize::new(1),
            pipeline_cache: None,
        },
    )
    .expect("create vello Renderer");

    tracing::info!("GPU initialized: {width}x{height} format={format:?}");

    GpuState {
        device,
        queue,
        surface,
        target_texture,
        target_view,
        blitter,
        renderer,
        scene: vello::Scene::new(),
        format,
        width,
        height,
    }
}

// -----------------------------------------------------------------------------
// Entry point dispatcher
// -----------------------------------------------------------------------------

#[wasm_bindgen(start)]
pub fn main() {
    tracing_wasm::set_as_global_default();
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));

    #[cfg(feature = "unified")]
    {
        let global = js_sys::global();
        if let Ok(scope) = global.dyn_into::<DedicatedWorkerGlobalScope>() {
            match scope.name().as_str() {
                "logic" => ambre_logic_worker::start(),
                "sync" => ambre_sync_worker::start(),
                name => panic!("unknown worker name: {name}"),
            }
            return;
        }
    }
    ui_main();
}

// -----------------------------------------------------------------------------
// UI Thread main
// -----------------------------------------------------------------------------

fn ui_main() {
    tracing::info!("Ambre UI Thread starting");

    let logic_opts = WorkerOptions::new();
    logic_opts.set_type(WorkerType::Module);
    logic_opts.set_name("logic");

    let sync_opts = WorkerOptions::new();
    sync_opts.set_type(WorkerType::Module);
    sync_opts.set_name("sync");

    let logic_worker =
        Worker::new_with_options("./logic_worker.js", &logic_opts).expect("spawn Logic Worker");
    let sync_worker =
        Worker::new_with_options("./sync_worker.js", &sync_opts).expect("spawn Sync Worker");

    let channel = MessageChannel::new().expect("create MessageChannel");
    let pending: Rc<RefCell<VecDeque<LogicEnvelope>>> = Rc::new(RefCell::new(VecDeque::new()));

    // -- Read canvas dimensions -----------------------------------------------

    let canvas: HtmlCanvasElement = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document")
        .get_element_by_id("ambre-canvas")
        .expect("no #ambre-canvas element")
        .dyn_into()
        .expect("#ambre-canvas is not a canvas");

    let canvas_w = canvas.width() as f64;
    let canvas_h = canvas.height() as f64;

    // -- Build UiState --------------------------------------------------------
    // State is created before the on_ready closures so they can capture
    // Rc::clone(&state) and flip logic_worker_ready once the handshake completes.

    let state = Rc::new(RefCell::new(UiState {
        logic_worker,
        pending: Rc::clone(&pending),
        gpu: None,
        trame: trame::TrameUiState::new(canvas_w, canvas_h),
        logic_worker_ready: false,
        initial_viewport_sent: false,
    }));

    // -- Logic Worker ready handshake -----------------------------------------
    {
        let port1 = channel.port1();
        let pending = Rc::clone(&pending);
        let state_c = Rc::clone(&state);
        let logic_worker_c = state.borrow().logic_worker.clone();
        let logic_worker_c2 = logic_worker_c.clone();
        let on_ready = Closure::<dyn FnMut(MessageEvent)>::once(move |_: MessageEvent| {
            let init_msg = js_sys::Object::new();
            js_sys::Reflect::set(&init_msg, &"syncPort".into(), &port1).unwrap();
            let transfer = js_sys::Array::of1(&port1);
            logic_worker_c
                .post_message_with_transfer(&init_msg, &transfer)
                .expect("transfer syncPort to Logic Worker");

            // Phase-1 complete: install the steady-state message handler and
            // mark the Logic Worker as ready so render() sends the first viewport.
            let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                if let Some(envelope) = extract_logic_envelope(&event) {
                    pending.borrow_mut().push_back(envelope);
                }
            });
            logic_worker_c2.set_onmessage(Some(cb.as_ref().unchecked_ref()));
            cb.forget();

            state_c.borrow_mut().logic_worker_ready = true;
        });
        state.borrow().logic_worker.set_onmessage(Some(on_ready.as_ref().unchecked_ref()));
        on_ready.forget();
    }

    // -- Sync Worker ready handshake ------------------------------------------
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

    // -- Async GPU init -------------------------------------------------------
    //
    // wasm_bindgen_futures::spawn_local schedules the future as a JS microtask.
    // It runs on the same single thread as the rAF loop (no Send/Sync needed).
    // Once the GPU is ready it writes into state.gpu; render() will detect that
    // both gpu.is_some() and logic_worker_ready are true on the next rAF frame
    // and send the first ScrollViewport automatically.
    {
        let state_c = Rc::clone(&state);
        wasm_bindgen_futures::spawn_local(async move {
            // Re-fetch the canvas inside the async block (closures can't easily
            // capture web_sys::HtmlCanvasElement across await points due to
            // !Send, but this is single-threaded so it's safe).
            let canvas: HtmlCanvasElement = web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .get_element_by_id("ambre-canvas")
                .unwrap()
                .dyn_into()
                .unwrap();

            let gpu = bootstrap_gpu(canvas).await;
            state_c.borrow_mut().gpu = Some(gpu);
        });
    }

    // -- Arrow key input → TrameCommand::ScrollViewport -----------------------
    {
        let state_c = Rc::clone(&state);
        let cb = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            move |event: web_sys::KeyboardEvent| {
                let (dr, dc): (i32, i32) = match event.key().as_str() {
                    "ArrowUp" => (-1, 0),
                    "ArrowDown" => (1, 0),
                    "ArrowLeft" => (0, -1),
                    "ArrowRight" => (0, 1),
                    _ => return,
                };
                event.prevent_default(); // prevent page scroll

                let mut state = state_c.borrow_mut();
                if let Some(cmd) = state.trame.move_cursor(dr, dc) {
                    let payload =
                        postcard::to_allocvec(&cmd).expect("serialize TrameCommand");
                    let logic_worker = state.logic_worker.clone();
                    send_to_worker(
                        &logic_worker,
                        &UiEnvelope::AppCommand {
                            document_id: IPC_TEST_DOC_ID,
                            payload,
                        },
                    );
                }
            },
        );
        web_sys::window()
            .expect("no window")
            .document()
            .expect("no document")
            .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())
            .expect("add keydown listener");
        cb.forget();
    }

    tracing::info!("Ambre UI Thread ready");

    // Start the rAF loop. Frames before GPU init return immediately without
    // rendering (UiState::render() guards on self.gpu.is_some()).
    schedule_raf(state);

    // Hide the loading screen after GPU init (ambreReady is called from JS).
    // For now we call it immediately; it will move to after the first GPU frame
    // once the surface is properly initialized.
    if let Some(window) = web_sys::window()
        && let Ok(f) = js_sys::Reflect::get(&window, &"ambreReady".into())
        && let Ok(f) = f.dyn_into::<js_sys::Function>()
    {
        let _ = f.call0(&window);
    }
}
