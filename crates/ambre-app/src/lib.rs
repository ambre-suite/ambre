// ambre-app/src/lib.rs
// ============================================================================
// UI Thread entry point — runs on the page's main JS thread (cdylib,
// #[wasm_bindgen(start)]).
//
// Responsibilities:
//   - Own HTMLCanvasElement, wgpu Device/Queue/Surface, QuadRenderer.
//   - Drive the requestAnimationFrame render loop.
//   - Capture keyboard input (arrow keys -> TrameCommand::ScrollViewport).
//   - Spawn the Logic and Sync threads via wasm_thread and own the channel
//     pair that connects them to this UI Thread.
//
// GPU bootstrap is async (wgpu::Adapter::request_device is a Future even on
// WASM). It runs via wasm_bindgen_futures::spawn_local and writes into
// `UiState::gpu: Option<GpuState>` once complete. The rAF loop is started
// after GPU init so Firefox doesn't throttle to 60 Hz from empty frames.
//
// IPC: post-Phase-3, four `flume` channels carry typed Rust values directly.
// No postcard on the inter-thread path. The application payloads
// (TrameCommand etc.) remain postcard-encoded internally because their
// schema is owned by the app crates and not part of the bus.
// ============================================================================

mod trame;

use ambre_bus::{IPC_TEST_DOC_ID, LogicEnvelope, LogicEvent, SyncCommand, UiEnvelope};
use ambre_trame::TrameViewUpdate;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

// -----------------------------------------------------------------------------
// GpuState
// -----------------------------------------------------------------------------

/// All wgpu resources needed to render one frame.
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
    /// Outbound channel to the Logic thread.
    logic_tx:              flume::Sender<UiEnvelope>,
    /// Inbound channel from the Logic thread. Drained at the top of each rAF
    /// frame via `try_iter()`.
    logic_rx:              flume::Receiver<LogicEnvelope>,
    gpu:                   Option<GpuState>,
    trame:                 trame::TrameUiState,
    initial_viewport_sent: bool,
    render_times:          VecDeque<f64>,
    render_fps:            f32,
    last_frame_ms:         f32,
    /// Time spent draining inbound `LogicEnvelope` messages this frame.
    last_drain_ms:         f32,
    /// Time spent in `draw_grid` (CPU-side: lyon tessellation, parley shaping,
    /// glyph atlas updates). Zero when the frame is not dirty.
    last_draw_ms:          f32,
    /// Time spent on the GPU path (`get_current_texture` + buffer upload +
    /// `flush` + `submit` + `present`).
    last_gpu_ms:           f32,
    /// Per-sub-phase breakdown of the last `draw_grid` call. Populated by
    /// `draw_grid` itself; consumed by the next frame's HUD render.
    draw_stats:            trame::renderer::DrawStats,
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
        // `try_iter` pulls every queued LogicEnvelope without blocking,
        // ending as soon as the channel is empty.
        while let Ok(envelope) = self.logic_rx.try_recv() {
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
        let t_drain = self.perf.now();
        self.drain_pending();
        let t_after_drain = self.perf.now();

        // rAF only fires after GPU init, so self.gpu is always Some here.
        let Some(ref mut gpu) = self.gpu else { return };

        if !self.initial_viewport_sent {
            self.initial_viewport_sent = true;
            self.dirty = true;
            let cmd     = self.trame.scroll_viewport_cmd();
            let payload = postcard::to_allocvec(&cmd).expect("serialize TrameCommand");
            let _ = self.logic_tx.send(UiEnvelope::AppCommand {
                document_id: IPC_TEST_DOC_ID,
                payload,
            });
        }

        Self::tick_fps(&mut self.render_times, &mut self.render_fps, raf_timestamp);

        if raf_timestamp - self.last_hud_update_ms >= 500.0 {
            self.last_hud_update_ms = raf_timestamp;
            self.dirty = true;
        }

        let t0 = self.perf.now();

        let upload = self.dirty;
        let t_draw_ms;
        if self.dirty {
            self.dirty = false;
            gpu.text_cache.reset_frame_stats();
            gpu.quad_renderer.begin_frame(gpu.width as f32, gpu.height as f32);
            let t_draw_start = self.perf.now();
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
                    self.last_drain_ms,
                    self.last_draw_ms,
                    self.last_gpu_ms,
                    &self.perf,
                    &mut self.draw_stats,
                );
            }
            gpu.text_cache.evict_if_oversized(5_000);
            t_draw_ms = (self.perf.now() - t_draw_start) as f32;
        } else {
            t_draw_ms = 0.0;
        }

        let t_gpu_start = self.perf.now();
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
        self.last_frame_ms  = (t2 - t0) as f32;
        self.last_drain_ms  = (t_after_drain - t_drain) as f32;
        self.last_draw_ms   = t_draw_ms;
        self.last_gpu_ms    = (t2 - t_gpu_start) as f32;
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

fn url_flag(flag: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .map(|q| {
            q.split(['?', '&'])
                .any(|part| part == flag || part.starts_with(&format!("{flag}=")))
        })
        .unwrap_or(false)
}

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
// Entry point
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

    ui_main();
}

// -----------------------------------------------------------------------------
// UI Thread main
// -----------------------------------------------------------------------------

fn ui_main() {
    tracing::info!("Ambre UI Thread starting");

    // Inter-thread channels.
    //   UI    --UiEnvelope--->  Logic
    //   Logic --LogicEnvelope-> UI
    //   Logic --SyncCommand--->  Sync
    //   Sync  --LogicEvent---->  Logic
    let (ui_to_logic_tx, ui_to_logic_rx) = flume::unbounded::<UiEnvelope>();
    let (logic_to_ui_tx, logic_to_ui_rx) = flume::unbounded::<LogicEnvelope>();
    let (logic_to_sync_tx, logic_to_sync_rx) = flume::unbounded::<SyncCommand>();
    let (sync_to_logic_tx, sync_to_logic_rx) = flume::unbounded::<LogicEvent>();

    // Spawn the Logic thread. Owns its half of every channel from here on.
    wasm_thread::Builder::new()
        .name("logic".to_owned())
        .spawn(move || {
            ambre_logic::run(ui_to_logic_rx, logic_to_ui_tx, logic_to_sync_tx, sync_to_logic_rx);
        })
        .expect("spawn Logic thread");

    // Spawn the Sync thread.
    wasm_thread::Builder::new()
        .name("sync".to_owned())
        .spawn(move || {
            ambre_sync::run(logic_to_sync_rx, sync_to_logic_tx);
        })
        .expect("spawn Sync thread");

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
        logic_tx:              ui_to_logic_tx,
        logic_rx:              logic_to_ui_rx,
        gpu:                   None,
        trame:                 trame::TrameUiState::new(canvas_w, canvas_h),
        initial_viewport_sent: false,
        render_times:          VecDeque::with_capacity(120),
        render_fps:            0.0,
        last_frame_ms:         0.0,
        last_drain_ms:         0.0,
        last_draw_ms:          0.0,
        last_gpu_ms:           0.0,
        draw_stats:            trame::renderer::DrawStats::default(),
        last_hud_update_ms:    0.0,
        text_enabled:          true,
        perf,
        dirty:                 true,
    }));

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
                    let _ = s.logic_tx.send(UiEnvelope::AppCommand {
                        document_id: IPC_TEST_DOC_ID,
                        payload,
                    });
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

    if let Some(window) = web_sys::window()
        && let Ok(f) = js_sys::Reflect::get(&window, &"ambreReady".into())
        && let Ok(f) = f.dyn_into::<js_sys::Function>()
    {
        let _ = f.call0(&window);
    }
}
