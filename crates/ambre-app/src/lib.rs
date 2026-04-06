// ambre-app/src/lib.rs
// ============================================================================
// UI Thread entry point -- main JS thread (cdylib, #[wasm_bindgen(start)]).
//
// Responsibilities:
//   - Own HTMLCanvasElement, wgpu Device/Queue/Surface, QuadRenderer.
//   - Drive the requestAnimationFrame render loop.
//   - Capture keyboard input (arrow keys -> TrameCommand::ScrollViewport).
//   - Spawn Logic Worker + Sync Worker; wire the MessageChannel between them.
//
// GPU bootstrap is async (wgpu::Adapter::request_device is a Future even on
// WASM). It runs via wasm_bindgen_futures::spawn_local and writes into
// `UiState::gpu: Option<GpuState>` once complete. The rAF loop is started
// immediately; frames before GPU init return early with no rendering.
//
// Render pipeline (each frame, once GPU is ready):
//   1. drain_pending()          -- apply incoming LogicEnvelopes
//   2. send initial ScrollViewport if GPU + Logic Worker are both ready
//   3. quad_renderer.begin_frame()
//   4. draw_grid() via WgpuSurface -- fills CPU-side vertex batch
//   5. quad_renderer.flush()    -- upload + single draw call + present
//
// IPC:
//   UiEnvelope   (postcard + ArrayBuffer transfer) -> Logic Worker
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
    let bytes    = postcard::to_allocvec(msg).expect("postcard serialize");
    let js_bytes = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    js_bytes.copy_from(&bytes);
    let ab       = js_bytes.buffer();
    let transfer = js_sys::Array::of1(&ab);
    worker
        .post_message_with_transfer(&ab, &transfer)
        .expect("postMessage to worker");
}

fn extract_logic_envelope(event: &MessageEvent) -> Option<LogicEnvelope> {
    let ab       = event.data().dyn_into::<js_sys::ArrayBuffer>().ok()?;
    let js_bytes = js_sys::Uint8Array::new(&ab);
    let mut bytes = vec![0u8; js_bytes.length() as usize];
    js_bytes.copy_to(&mut bytes);
    postcard::from_bytes(&bytes).ok()
}

// -----------------------------------------------------------------------------
// GpuState
// -----------------------------------------------------------------------------

/// All wgpu resources needed to render one frame.
///
/// Created asynchronously in `bootstrap_gpu()`; stored as `Option<GpuState>`
/// in `UiState`.
/// QuadRenderer owns the wgpu pipeline and writes directly to the surface.
struct GpuState {
    device:         wgpu::Device,
    queue:          wgpu::Queue,
    surface:        wgpu::Surface<'static>,
    surface_format: wgpu::TextureFormat,
    width:          u32,
    height:         u32,
    quad_renderer:  ambre_ui::QuadRenderer,
    font_cx:        parley::FontContext,
    text_cache:     ambre_ui::TextCache,
}

// -----------------------------------------------------------------------------
// UiState
// -----------------------------------------------------------------------------

struct UiState {
    logic_worker:          Worker,
    pending:               Rc<RefCell<VecDeque<LogicEnvelope>>>,
    gpu:                   Option<GpuState>,
    trame:                 trame::TrameUiState,
    logic_worker_ready:    bool,
    initial_viewport_sent: bool,
    render_times:          VecDeque<f64>,
    render_fps:            f32,
    last_frame_ms:         f32,
    /// rAF timestamp of the last HUD rebuild. We refresh the FPS overlay at most
    /// every 500ms so idle frames never trigger a full vertex rebuild.
    last_hud_update_ms:    f64,
    text_enabled:          bool,
    perf:                  web_sys::Performance,
    dirty:                 bool,
}

impl UiState {
    fn tick_fps(times: &mut VecDeque<f64>, fps: &mut f32, now: f64) {
        times.push_back(now);
        while times.front().is_some_and(|&t| now - t > 1000.0) {
            times.pop_front();
        }
        *fps = if times.len() >= 2 {
            (times.len() - 1) as f32 / ((now - times.front().unwrap()) / 1000.0) as f32
        } else {
            0.0
        };
    }

    fn drain_pending(&mut self) {
        let mut batch = VecDeque::new();
        std::mem::swap(&mut batch, &mut self.pending.borrow_mut());
        for envelope in batch {
            self.handle_logic_envelope(envelope);
        }
    }

    fn handle_resize(&mut self, width: u32, height: u32) {
        self.trame.resize(width as f64, height as f64);
        self.dirty = true;

        let Some(ref mut gpu) = self.gpu else { return };
        if gpu.width == width && gpu.height == height { return; }

        tracing::info!("GPU resize: {width}x{height}");

        gpu.surface.configure(&gpu.device, &wgpu::SurfaceConfiguration {
            usage:                        wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:                       gpu.surface_format,
            width,
            height,
            present_mode:                 wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode:                   wgpu::CompositeAlphaMode::Auto,
            view_formats:                 vec![],
        });

        gpu.width  = width;
        gpu.height = height;
        // QuadRenderer reads width/height from begin_frame() each frame --
        // no additional state to update here.
    }

    fn handle_logic_envelope(&mut self, envelope: LogicEnvelope) {
        if let LogicEnvelope::ViewportUpdate { document_id: _, payload } = envelope
            && let Ok(update) = postcard::from_bytes::<TrameViewUpdate>(&payload)
        {
            self.trame.on_viewport_data(update);
            self.dirty = true;
        }
    }

    fn render(&mut self, raf_timestamp: f64) {
        self.drain_pending();

        // rAF only fires after GPU init, so self.gpu is always Some here.
        let Some(ref mut gpu) = self.gpu else { return };

        if self.logic_worker_ready && !self.initial_viewport_sent {
            self.initial_viewport_sent = true;
            self.dirty = true;
            let cmd     = self.trame.scroll_viewport_cmd();
            let payload = postcard::to_allocvec(&cmd).expect("serialize TrameCommand");
            let worker  = self.logic_worker.clone();
            send_to_worker(
                &worker,
                &UiEnvelope::AppCommand { document_id: IPC_TEST_DOC_ID, payload },
            );
        }

        Self::tick_fps(&mut self.render_times, &mut self.render_fps, raf_timestamp);

        // Refresh the HUD (FPS overlay) at most every 500ms.
        if raf_timestamp - self.last_hud_update_ms >= 500.0 {
            self.last_hud_update_ms = raf_timestamp;
            self.dirty = true;
        }

        let t0 = self.perf.now();

        // -- Build CPU-side vertex batch (only when dirty) ---
        let upload = self.dirty;
        if self.dirty {
            self.dirty = false;
            gpu.text_cache.reset_frame_stats();
            gpu.quad_renderer.begin_frame(gpu.width as f32, gpu.height as f32);
            {
                let mut surface = ambre_ui::WgpuSurface::new(&mut gpu.quad_renderer);
                trame::renderer::draw_grid(
                    &mut surface,
                    &self.trame,
                    &mut gpu.font_cx,
                    &mut gpu.text_cache,
                    self.text_enabled,
                    self.render_fps,
                    self.last_frame_ms,
                );
            }
            gpu.text_cache.evict_if_oversized(5_000);
        }

        // -- GPU: upload (if dirty) + render pass + present ---
        let surface_tex = match gpu.surface.get_current_texture() {
            Ok(t)  => t,
            Err(e) => {
                tracing::error!("get_current_texture: {e:?}");
                return;
            }
        };
        let target_view  = surface_tex.texture.create_view(&Default::default());
        let mut encoder  = gpu.device.create_command_encoder(&Default::default());

        gpu.quad_renderer.flush(&gpu.device, &gpu.queue, &mut encoder, &target_view, upload);

        gpu.queue.submit(std::iter::once(encoder.finish()));
        surface_tex.present();

        let t2 = self.perf.now();
        self.last_frame_ms = (t2 - t0) as f32;
    }
}

// -----------------------------------------------------------------------------
// rAF loop
// -----------------------------------------------------------------------------

/// Start the requestAnimationFrame loop using a single stable closure.
///
/// The naive recursive pattern (allocate closure → forget → repeat) leaks one
/// JS closure object per frame. At 240 Hz Firefox sees the JS heap grow, GC
/// pressure rises, and it throttles rAF down to 60 Hz as a self-protection
/// measure. Instead we keep one `Closure` alive for the lifetime of the app via
/// an `Rc<RefCell<Option<…>>>` so the same JS function object is reused every
/// frame with zero allocation overhead.
fn start_raf_loop(state: Rc<RefCell<UiState>>) {
    #[allow(clippy::type_complexity)]
    let cb: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
    let cb_clone = Rc::clone(&cb);

    *cb.borrow_mut() = Some(Closure::new(move |ts: f64| {
        state.borrow_mut().render(ts);
        web_sys::window()
            .expect("no window")
            .request_animation_frame(
                cb_clone
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .unchecked_ref(),
            )
            .expect("rAF");
    }));

    web_sys::window()
        .expect("no window")
        .request_animation_frame(
            cb.borrow().as_ref().unwrap().as_ref().unchecked_ref(),
        )
        .expect("rAF");

    // Keep the closure alive forever (one forget for the app lifetime).
    cb.borrow().as_ref().unwrap().as_ref().unchecked_ref::<js_sys::Function>();
    std::mem::forget(cb);
}

// -----------------------------------------------------------------------------
// GPU bootstrap (async)
// -----------------------------------------------------------------------------

async fn bootstrap_gpu(canvas: HtmlCanvasElement) -> Option<GpuState> {
    // WebGPU on Mesa/Intel Firefox has a compositor bug: the render pipeline
    // produces correct pixels (confirmed via GPU readback) but the browser
    // compositor silently fails to display the canvas on screen.
    //
    // This cannot be detected from JavaScript: `drawImage` reads the GPU buffer
    // directly and bypasses the compositor, so a probe canvas always looks correct.
    //
    // Strategy: default to GL (WebGL2) on WASM, which works universally.
    // Opt in to WebGPU with `?webgpu` in the URL (useful for Nvidia and future
    // Mesa versions where the bug is fixed).
    let use_webgpu = url_flag("webgpu");
    let backends = if use_webgpu {
        tracing::info!("WebGPU requested via URL flag");
        wgpu::Backends::BROWSER_WEBGPU
    } else {
        wgpu::Backends::GL
    };

    create_gpu_state(canvas, backends).await
}

/// Returns `true` if the current page URL contains `?webgpu` or `&webgpu`.
fn url_flag(flag: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .map(|q| {
            q.split(['?', '&'])
                .any(|part| part == flag || part.starts_with(&format!("{flag}=")))
        })
        .unwrap_or(false)
}

/// Initialise the full GPU stack on `canvas` using the given `backends`.
async fn create_gpu_state(canvas: HtmlCanvasElement, backends: wgpu::Backends) -> Option<GpuState> {
    let width  = canvas.width();
    let height = canvas.height();

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    });

    let surface = match instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas)) {
        Ok(s)  => s,
        Err(e) => { tracing::error!("create_surface failed: {e:?}"); return None; }
    };

    let adapter = match instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:       wgpu::PowerPreference::HighPerformance,
            compatible_surface:     Some(&surface),
            force_fallback_adapter: false,
        })
        .await
    {
        Ok(a)  => a,
        Err(e) => {
            tracing::error!("No GPU adapter: {e:?}");
            return None;
        }
    };

    let (device, queue) = match adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
            ..Default::default()
        })
        .await
    {
        Ok(dq)  => dq,
        Err(e)  => { tracing::error!("request_device failed: {e:?}"); return None; }
    };

    let caps   = surface.get_capabilities(&adapter);
    tracing::info!("surface caps: formats={:?} alpha_modes={:?}", caps.formats, caps.alpha_modes);
    let format = caps
        .formats
        .iter()
        .find(|&&f| f == wgpu::TextureFormat::Bgra8Unorm || f == wgpu::TextureFormat::Rgba8Unorm)
        .copied()
        .unwrap_or(caps.formats[0]);

    surface.configure(
        &device,
        &wgpu::SurfaceConfiguration {
            usage:                        wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode:                 wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode:                   wgpu::CompositeAlphaMode::Auto,
            view_formats:                 vec![],
        },
    );

    let quad_renderer = ambre_ui::QuadRenderer::new(&device, format);

    let mut font_cx = parley::FontContext::new();
    match fetch_bytes("/fonts/NotoSans-Regular.ttf").await {
        Some(bytes) => {
            font_cx.collection.register_fonts(bytes.into(), None);
            tracing::info!("NotoSans-Regular.ttf loaded");
        }
        None => tracing::warn!("Failed to fetch /fonts/NotoSans-Regular.ttf -- text disabled"),
    }

    let info = adapter.get_info();
    tracing::info!(
        "GPU initialized: {} ({:?}) | {width}x{height} format={format:?}",
        info.name, info.backend
    );

    Some(GpuState {
        device,
        queue,
        surface,
        surface_format: format,
        width,
        height,
        quad_renderer,
        font_cx,
        text_cache: ambre_ui::TextCache::new(1.0),
    })
}

// -----------------------------------------------------------------------------
// Font fetch helper
// -----------------------------------------------------------------------------

async fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    use wasm_bindgen::JsCast as _;
    let window   = web_sys::window()?;
    let resp_val =
        wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url)).await.ok()?;
    let resp: web_sys::Response = resp_val.dyn_into().ok()?;
    if !resp.ok() {
        return None;
    }
    let ab_val =
        wasm_bindgen_futures::JsFuture::from(resp.array_buffer().ok()?).await.ok()?;
    let ab: js_sys::ArrayBuffer = ab_val.dyn_into().ok()?;
    Some(js_sys::Uint8Array::new(&ab).to_vec())
}

// -----------------------------------------------------------------------------
// Entry point dispatcher
// -----------------------------------------------------------------------------

#[wasm_bindgen(start)]
pub fn main() {
    tracing_wasm::set_as_global_default_with_config(
        tracing_wasm::WASMLayerConfigBuilder::new()
            .set_max_level(if cfg!(debug_assertions) {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            })
            .build(),
    );
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
    let pending: Rc<RefCell<VecDeque<LogicEnvelope>>> =
        Rc::new(RefCell::new(VecDeque::new()));

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

    let perf = web_sys::window()
        .expect("no window")
        .performance()
        .expect("no Performance API");

    let state = Rc::new(RefCell::new(UiState {
        logic_worker,
        pending:               Rc::clone(&pending),
        gpu:                   None,
        trame:                 trame::TrameUiState::new(canvas_w, canvas_h),
        logic_worker_ready:    false,
        initial_viewport_sent: false,
        render_times:          VecDeque::with_capacity(120),
        render_fps:            0.0,
        last_frame_ms:         0.0,
        last_hud_update_ms:    0.0,
        text_enabled:          true,
        perf,
        dirty:                 true,
    }));

    // -- Logic Worker ready handshake -----------------------------------------
    {
        let port1         = channel.port1();
        let pending       = Rc::clone(&pending);
        let state_c       = Rc::clone(&state);
        let worker_c      = state.borrow().logic_worker.clone();
        let worker_c2     = worker_c.clone();
        let on_ready = Closure::<dyn FnMut(MessageEvent)>::once(move |_: MessageEvent| {
            let init_msg = js_sys::Object::new();
            js_sys::Reflect::set(&init_msg, &"syncPort".into(), &port1).unwrap();
            let transfer = js_sys::Array::of1(&port1);
            worker_c
                .post_message_with_transfer(&init_msg, &transfer)
                .expect("transfer syncPort to Logic Worker");

            let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                if let Some(envelope) = extract_logic_envelope(&event) {
                    pending.borrow_mut().push_back(envelope);
                }
            });
            worker_c2.set_onmessage(Some(cb.as_ref().unchecked_ref()));
            cb.forget();

            state_c.borrow_mut().logic_worker_ready = true;
        });
        state.borrow().logic_worker.set_onmessage(Some(on_ready.as_ref().unchecked_ref()));
        on_ready.forget();
    }

    // -- Sync Worker ready handshake ------------------------------------------
    {
        let port2      = channel.port2();
        let sync_c     = sync_worker.clone();
        let on_ready = Closure::<dyn FnMut(MessageEvent)>::once(move |_: MessageEvent| {
            let init_msg = js_sys::Object::new();
            js_sys::Reflect::set(&init_msg, &"logicPort".into(), &port2).unwrap();
            let transfer = js_sys::Array::of1(&port2);
            sync_c
                .post_message_with_transfer(&init_msg, &transfer)
                .expect("transfer logicPort to Sync Worker");
        });
        sync_worker.set_onmessage(Some(on_ready.as_ref().unchecked_ref()));
        on_ready.forget();
    }

    // -- Async GPU init -------------------------------------------------------
    {
        let state_c = Rc::clone(&state);
        // Start the rAF loop only after the GPU is ready so Firefox sees active
        // GPU frames from frame 1. If rAF starts before GPU init, the empty
        // early-return frames (<0.1ms, no GPU work) cause Firefox to calibrate
        // its vsync-source to 60 Hz and it stays there for the tab's lifetime.
        wasm_bindgen_futures::spawn_local(async move {
            let canvas: HtmlCanvasElement = web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .get_element_by_id("ambre-canvas")
                .unwrap()
                .dyn_into()
                .unwrap();
            state_c.borrow_mut().gpu = bootstrap_gpu(canvas).await;
            start_raf_loop(state_c);
        });
    }

    // -- Arrow key input -> TrameCommand::ScrollViewport ----------------------
    {
        let state_c = Rc::clone(&state);
        let cb = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            move |event: web_sys::KeyboardEvent| {
                let (dr, dc): (i32, i32) = match event.key().as_str() {
                    "ArrowUp"    => (-1,  0),
                    "ArrowDown"  => ( 1,  0),
                    "ArrowLeft"  => ( 0, -1),
                    "ArrowRight" => ( 0,  1),
                    "t" | "T" => {
                        let mut s = state_c.borrow_mut();
                        s.text_enabled ^= true;
                        s.dirty = true;
                        return;
                    }
                    _ => return,
                };
                event.prevent_default();

                let mut s = state_c.borrow_mut();
                s.dirty = true;
                if let Some(cmd) = s.trame.move_cursor(dr, dc) {
                    let payload = postcard::to_allocvec(&cmd).expect("serialize TrameCommand");
                    let worker  = s.logic_worker.clone();
                    send_to_worker(
                        &worker,
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

    // -- Canvas resize -> reconfigure GPU surface ----------------------------
    {
        let state_c  = Rc::clone(&state);
        let canvas_c = canvas.clone();
        let cb = Closure::<dyn FnMut()>::new(move || {
            let w = canvas_c.client_width()  as u32;
            let h = canvas_c.client_height() as u32;
            if w > 0 && h > 0 {
                canvas_c.set_width(w);
                canvas_c.set_height(h);
                state_c.borrow_mut().handle_resize(w, h);
            }
        });
        web_sys::window()
            .expect("no window")
            .set_onresize(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    }

    tracing::info!("Ambre UI Thread ready");
    // rAF loop is started inside the GPU bootstrap spawn_local above,
    // after the GPU is fully initialized.

    if let Some(window) = web_sys::window()
        && let Ok(f) = js_sys::Reflect::get(&window, &"ambreReady".into())
        && let Ok(f) = f.dyn_into::<js_sys::Function>()
    {
        let _ = f.call0(&window);
    }
}
