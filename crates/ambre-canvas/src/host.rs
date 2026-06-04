// ambre-canvas/src/host.rs
// ============================================================================
// UI Thread host plumbing — generic, app-agnostic.
//
//   GpuState            wgpu Device/Queue/Surface + QuadRenderer + text stack.
//   bootstrap_gpu       async wgpu init (WebGL2 by default, ?webgpu opt-in).
//   start_raf_loop      requestAnimationFrame driver over a frame callback.
//   fetch_bytes         same-origin asset fetch (fonts).
//
// This module owns no application state. The per-app frame logic (what to draw,
// dirty-rect strategy) lives in the consuming crate, which passes a closure to
// `start_raf_loop` and reads/writes the `GpuState` it gets from `bootstrap_gpu`.
//
// wasm32-only: surface creation from an HTMLCanvasElement
// (`wgpu::SurfaceTarget::Canvas`) and the web_sys glue do not exist on native.
// ============================================================================

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::view::{DocView, FrameStats, InputEvent};
use crate::{ContentRepaint, Layer, OverlayRepaint, QuadRenderer, TextCache, WgpuSurface};

// -----------------------------------------------------------------------------
// GpuState
// -----------------------------------------------------------------------------

/// All wgpu resources needed to render one frame.
pub struct GpuState {
    pub device:         wgpu::Device,
    pub queue:          wgpu::Queue,
    pub surface:        wgpu::Surface<'static>,
    pub surface_format: wgpu::TextureFormat,
    pub width:          u32,
    pub height:         u32,
    pub quad_renderer:  QuadRenderer,
    pub font_cx:        parley::FontContext,
    pub text_cache:     TextCache,
}

// -----------------------------------------------------------------------------
// rAF loop
// -----------------------------------------------------------------------------

/// Start the requestAnimationFrame loop, invoking `frame(timestamp)` once per
/// frame.
///
/// The naive recursive pattern (allocate closure → forget → repeat) leaks one
/// JS closure object per frame. At 240 Hz Firefox sees the JS heap grow, GC
/// pressure rises, and it throttles rAF down to 60 Hz as a self-protection
/// measure. Instead we keep one `Closure` alive for the lifetime of the app via
/// an `Rc<RefCell<Option<…>>>` so the same JS function object is reused every
/// frame with zero allocation overhead.
pub fn start_raf_loop(mut frame: impl FnMut(f64) + 'static) {
    #[allow(clippy::type_complexity)]
    let cb: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
    let cb_clone = Rc::clone(&cb);

    *cb.borrow_mut() = Some(Closure::new(move |ts: f64| {
        frame(ts);
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

    std::mem::forget(cb);
}

// -----------------------------------------------------------------------------
// GPU bootstrap (async)
// -----------------------------------------------------------------------------

pub async fn bootstrap_gpu(canvas: HtmlCanvasElement) -> Option<GpuState> {
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

    let mut quad_renderer = QuadRenderer::new(&device, format);
    // Size the ping-pong offscreens to the initial canvas; ResizeObserver
    // doesn't fire on startup so handle_resize wouldn't get a chance to
    // initialize them before the first render otherwise.
    quad_renderer.resize_offscreens(&device, width, height);

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
        text_cache: TextCache::new(1.0),
    })
}

// -----------------------------------------------------------------------------
// Font fetch helper
// -----------------------------------------------------------------------------

async fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
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
// FrameHost — generic per-frame driver
// -----------------------------------------------------------------------------

/// Drives one [`DocView`] across the GPU and the requestAnimationFrame loop.
///
/// App-agnostic: it owns the [`GpuState`], measures frame timings, decides which
/// layers to encode from the view's [`RepaintPlan`], and presents the swapchain.
/// Everything spreadsheet/document-specific lives behind the `DocView` trait.
///
/// [`RepaintPlan`]: crate::view::RepaintPlan
pub struct FrameHost<V: DocView> {
    gpu:  Option<GpuState>,
    view: V,
    perf: web_sys::Performance,

    render_times:       VecDeque<f64>,
    render_fps:         f32,
    last_frame_ms:      f32,
    last_drain_ms:      f32,
    last_draw_ms:       f32,
    last_gpu_ms:        f32,
    /// rAF timestamp of the last HUD rebuild. The overlay refreshes at most
    /// every 500 ms so idle frames never trigger a full vertex rebuild.
    last_hud_update_ms: f64,
}

impl<V: DocView> FrameHost<V> {
    /// Build a host around `view`. The GPU is installed later via
    /// [`install_gpu`] once async bootstrap completes.
    ///
    /// [`install_gpu`]: FrameHost::install_gpu
    pub fn new(view: V, perf: web_sys::Performance) -> Self {
        Self {
            gpu:  None,
            view,
            perf,
            render_times:       VecDeque::with_capacity(120),
            render_fps:         0.0,
            last_frame_ms:      0.0,
            last_drain_ms:      0.0,
            last_draw_ms:       0.0,
            last_gpu_ms:        0.0,
            last_hud_update_ms: 0.0,
        }
    }

    /// Hand the host its GPU resources once `bootstrap_gpu` has resolved.
    pub fn install_gpu(&mut self, gpu: GpuState) {
        self.gpu = Some(gpu);
    }

    /// Forward an input event to the view. Returns `true` if the caller should
    /// `preventDefault()` on the originating DOM event.
    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        self.view.handle_input(ev)
    }

    /// Reconfigure the GPU surface and notify the view of a canvas resize.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.view.resize(width, height);

        let Some(gpu) = self.gpu.as_mut() else { return };
        if gpu.width == width && gpu.height == height {
            return;
        }
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
        // Reallocate the ping-pong offscreens at the new size; previous content
        // is lost so the view's resize() must have flagged a full repaint.
        gpu.quad_renderer.resize_offscreens(&gpu.device, width, height);

        gpu.width  = width;
        gpu.height = height;
    }

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

    /// Render one frame. Wire this into [`start_raf_loop`].
    pub fn render(&mut self, raf_timestamp: f64) {
        let t_drain = self.perf.now();
        self.view.update();
        let t_after_drain = self.perf.now();

        // rAF only starts after GPU init, so `self.gpu` is always Some here.
        let Some(gpu) = self.gpu.as_mut() else { return };

        Self::tick_fps(&mut self.render_times, &mut self.render_fps, raf_timestamp);

        let mut hud_tick = false;
        if raf_timestamp - self.last_hud_update_ms >= 500.0 {
            self.last_hud_update_ms = raf_timestamp;
            hud_tick = true;
        }

        let plan = self.view.plan_repaint(hud_tick);
        let content_active   = !matches!(plan.content, ContentRepaint::Reuse);
        let overlay_redraw = matches!(plan.overlay, OverlayRepaint::Redraw);
        let any_dirty      = content_active || overlay_redraw;

        let stats = FrameStats {
            fps:      self.render_fps,
            frame_ms: self.last_frame_ms,
            drain_ms: self.last_drain_ms,
            draw_ms:  self.last_draw_ms,
            gpu_ms:   self.last_gpu_ms,
        };

        let t0 = self.perf.now();

        // -- Build CPU batches for whichever layers are being rebuilt. --------
        let t_draw_ms = if any_dirty {
            gpu.text_cache.reset_frame_stats();
            gpu.quad_renderer.begin_frame(gpu.width as f32, gpu.height as f32);
            let t_draw_start = self.perf.now();
            {
                let mut surface = WgpuSurface::new(&mut gpu.quad_renderer);
                if content_active {
                    surface.begin_layer(Layer::Content);
                    self.view.paint_content(
                        &mut surface, &mut gpu.font_cx, &mut gpu.text_cache, plan.content_strip,
                    );
                }
                if overlay_redraw {
                    surface.begin_layer(Layer::Overlay);
                    self.view.paint_overlay(
                        &mut surface, &mut gpu.font_cx, &mut gpu.text_cache, &stats,
                    );
                }
            }
            gpu.text_cache.evict_if_oversized(5_000);
            (self.perf.now() - t_draw_start) as f32
        } else {
            0.0
        };

        // -- GPU encoding + present. ------------------------------------------
        let t_gpu_start = self.perf.now();
        let surface_tex = match gpu.surface.get_current_texture() {
            Ok(t)  => t,
            Err(e) => {
                tracing::error!("get_current_texture: {e:?}");
                return;
            }
        };
        let target_view = surface_tex.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());

        gpu.quad_renderer.flush_dirty_rect(
            &gpu.device, &gpu.queue,
            &mut encoder, &target_view,
            plan.content, plan.overlay,
        );

        gpu.queue.submit(std::iter::once(encoder.finish()));
        surface_tex.present();

        let t2 = self.perf.now();
        self.last_frame_ms = (t2 - t0) as f32;
        self.last_drain_ms = (t_after_drain - t_drain) as f32;
        self.last_draw_ms  = t_draw_ms;
        self.last_gpu_ms   = (t2 - t_gpu_start) as f32;
    }
}

// -----------------------------------------------------------------------------
// run — UI thread entry point
// -----------------------------------------------------------------------------

/// Run the UI thread: build the application view, drive it through a
/// [`FrameHost`] on the requestAnimationFrame loop, and forward DOM input and
/// resize events to it.
///
/// Symmetric to `ambre_logic::run` / `ambre_sync::run`, but **generic** over the
/// application's [`DocView`]: the caller (the launcher crate) injects the
/// concrete view via `make_view`, so this stays free of any app dependency and
/// no dependency cycle is introduced.
///
/// `make_view` receives the canvas size in physical pixels and returns the view
/// to drive. It runs on this (UI) thread before the GPU is ready.
pub fn run<V, F>(canvas_id: &str, make_view: F)
where
    V: DocView + 'static,
    F: FnOnce(f64, f64) -> V,
{
    tracing::info!("Ambre UI thread starting");

    let document = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document");
    let canvas: HtmlCanvasElement = document
        .get_element_by_id(canvas_id)
        .unwrap_or_else(|| panic!("no #{canvas_id} element"))
        .dyn_into()
        .expect("element is not a canvas");

    let perf = web_sys::window()
        .expect("no window")
        .performance()
        .expect("no Performance API");

    // Size the backing store to the canvas cell before the first frame. The
    // chrome (ambre-chrome AppShell) creates this `<canvas>` at its CSS layout
    // size; left untouched it would default to 300x150. Reading `client_width`
    // forces the pending layout, so the dimensions are exact. Subsequent cell
    // resizes are handled by the resize listener below.
    size_backing_store(&canvas);

    let view = make_view(canvas.width() as f64, canvas.height() as f64);
    let host = Rc::new(RefCell::new(FrameHost::new(view, perf)));

    // -- Async GPU init, then start the rAF loop. -----------------------------
    {
        let host_c = Rc::clone(&host);
        let canvas_c = canvas.clone();
        // Start the rAF loop only after the GPU is ready so Firefox sees active
        // GPU frames from frame 1 (empty early frames make it lock to 60 Hz).
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(gpu) = bootstrap_gpu(canvas_c).await {
                host_c.borrow_mut().install_gpu(gpu);
            }
            start_raf_loop(move |ts| host_c.borrow_mut().render(ts));
        });
    }

    // -- Keyboard input -> view. ----------------------------------------------
    {
        let host_c = Rc::clone(&host);
        let cb = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            move |event: web_sys::KeyboardEvent| {
                let consumed = host_c
                    .borrow_mut()
                    .handle_input(&InputEvent::Key { key: event.key() });
                if consumed {
                    event.prevent_default();
                }
            },
        );
        document
            .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())
            .expect("add keydown listener");
        cb.forget();
    }

    // -- Canvas resize -> host. -----------------------------------------------
    // Window-driven for now (cell-driven ResizeObserver is the bootstrap step).
    {
        let host_c   = Rc::clone(&host);
        let canvas_c = canvas.clone();
        let cb = Closure::<dyn FnMut()>::new(move || {
            if let Some((w, h)) = size_backing_store(&canvas_c) {
                host_c.borrow_mut().resize(w, h);
            }
        });
        web_sys::window()
            .expect("no window")
            .set_onresize(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    }

    tracing::info!("Ambre UI thread ready");

    // Notify the page that the UI thread is up (optional JS hook).
    if let Some(window) = web_sys::window()
        && let Ok(f) = js_sys::Reflect::get(&window, &"ambreReady".into())
        && let Ok(f) = f.dyn_into::<js_sys::Function>()
    {
        let _ = f.call0(&window);
    }
}

/// Set a canvas backing store to its CSS layout size scaled by the device pixel
/// ratio, so rendering is crisp on HiDPI displays. Returns the new physical
/// `(width, height)` in device pixels, or `None` if the cell has zero area
/// (detached / display:none) -- in which case the caller skips the resize.
fn size_backing_store(canvas: &HtmlCanvasElement) -> Option<(u32, u32)> {
    let dpr = web_sys::window().map(|w| w.device_pixel_ratio()).unwrap_or(1.0);
    let w = (canvas.client_width() as f64 * dpr).round() as u32;
    let h = (canvas.client_height() as f64 * dpr).round() as u32;
    if w == 0 || h == 0 {
        return None;
    }
    canvas.set_width(w);
    canvas.set_height(h);
    Some((w, h))
}
