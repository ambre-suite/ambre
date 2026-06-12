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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use ambre_draw::{Affine, Alignment, Brush, Color, DrawSurface, PositionedLayoutItem, Rect};

use crate::text_input::TextInput;
use crate::theme::{FONT_BODY, FONT_HEADING};
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

/// Handle to gate the rAF loop, so the chrome can freeze the canvas behind a
/// fullscreen `<dialog>`: while paused the loop keeps ticking but does no work
/// (no dispatch, no render), so the canvas holds its last frame -- a
/// `backdrop-filter: blur()` over it is then free and the GPU stays idle.
///
/// Backed by an `Arc<AtomicBool>` so it is `Send + Sync` and can ride inside a
/// Leptos `Callback`. (A literal `cancelAnimationFrame` would stop the loop
/// entirely, but resuming it needs main-thread access to the JS closure, which
/// is `!Send` and cannot cross a Leptos callback boundary; gating is the clean
/// equivalent here.)
#[derive(Clone)]
pub struct RafControl {
    running: Arc<AtomicBool>,
}

impl Default for RafControl {
    fn default() -> Self {
        Self::new()
    }
}

impl RafControl {
    pub fn new() -> Self {
        Self { running: Arc::new(AtomicBool::new(true)) }
    }

    /// Freeze the canvas: the loop ticks but skips all per-frame work.
    pub fn pause(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Resume rendering.
    pub fn resume(&self) {
        self.running.store(true, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

/// Start the requestAnimationFrame loop, invoking `frame(timestamp)` once per
/// frame while `control` is running (skipped while paused).
///
/// One `Closure` is kept alive for the app lifetime and reused every frame
/// (zero per-frame allocation): the naive allocate-and-forget pattern leaks a
/// closure per frame, which at 240 Hz grows the JS heap and makes Firefox
/// throttle rAF down to 60 Hz as self-protection.
pub fn start_raf_loop(mut frame: impl FnMut(f64) + 'static, control: RafControl) {
    #[allow(clippy::type_complexity)]
    let cb: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
    let cb_clone = Rc::clone(&cb);

    *cb.borrow_mut() = Some(Closure::new(move |ts: f64| {
        if control.is_running() {
            frame(ts);
        }
        // Always reschedule: a paused frame is just a flag check, and keeping
        // the loop alive is what lets `resume` take effect without re-arming.
        web_sys::window()
            .expect("no window")
            .request_animation_frame(
                cb_clone.borrow().as_ref().unwrap().as_ref().unchecked_ref(),
            )
            .expect("rAF");
    }));

    web_sys::window()
        .expect("no window")
        .request_animation_frame(cb.borrow().as_ref().unwrap().as_ref().unchecked_ref())
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
    // Bold / italic / bold-italic faces of the same "Noto Sans" family --
    // selected by FontWeight / FontStyle (real faces, not faux bold/oblique),
    // covering every combination of the bold and italic toggles.
    for (file, label) in [
        ("/fonts/NotoSans-Bold.ttf", "NotoSans-Bold.ttf"),
        ("/fonts/NotoSans-Italic.ttf", "NotoSans-Italic.ttf"),
        ("/fonts/NotoSans-BoldItalic.ttf", "NotoSans-BoldItalic.ttf"),
    ] {
        match fetch_bytes(file).await {
            Some(bytes) => {
                font_cx.collection.register_fonts(bytes.into(), None);
                tracing::info!("{label} loaded");
            }
            None => tracing::warn!("Failed to fetch {file}"),
        }
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
    /// Set once the fatal terminal screen has been painted, so the static
    /// message is encoded a single time rather than every subsequent frame.
    fatal_painted:      bool,
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
            fatal_painted:      false,
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

    /// Switch the active document on the live view, keeping the GPU surface.
    pub fn set_document(&mut self, document_id: ambre_bus::DocumentId) {
        self.view.set_document(document_id);
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
        // Fatal latch first: once set, the Logic thread feeding the view may be
        // aborted, so we must not keep draining it. Paint the terminal screen a
        // single time and then idle (the rAF loop keeps ticking harmlessly).
        if let Some(message) = crate::fatal::fatal_message() {
            if !self.fatal_painted {
                self.render_fatal(&message);
                self.fatal_painted = true;
            }
            return;
        }

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

    /// Paint the unrecoverable-error terminal screen: a dark backdrop with a
    /// centered title and the panic `message` below it. Encodes one full frame
    /// over the content layer; the caller paints it once (see `render`).
    ///
    /// This is the canvas counterpart of the `window.ambreFatal` DOM overlay
    /// (index.html). It runs when the UI/render thread is still alive -- a Logic
    /// panic or a Sync-worker death -- so the canvas shows a deliberate end state
    /// instead of freezing on the last good frame.
    fn render_fatal(&mut self, message: &str) {
        let Some(gpu) = self.gpu.as_mut() else { return };

        let w = gpu.width as f32;
        let h = gpu.height as f32;

        // Fixed colours (no theme dependency on the fatal path): a near-black
        // backdrop, a red title, and a dim grey body. ASCII strings only.
        const BACKDROP: Color = Color::from_rgba8(0x1a, 0x1a, 0x2e, 255);
        const TITLE:    Color = Color::from_rgba8(0xe0, 0x5a, 0x5a, 255);
        const BODY:     Color = Color::from_rgba8(0x9a, 0xa0, 0xa6, 255);
        const TITLE_TEXT: &str = "Ambre encountered a fatal error and must reload.";

        // Wrap the title/message to a centered column at most 640 logical px wide.
        let col_w = (w * 0.8).min(640.0);
        let origin_x = ((w as f64) - col_w as f64) / 2.0;

        gpu.quad_renderer.begin_frame(w, h);
        {
            let mut surface = WgpuSurface::new(&mut gpu.quad_renderer);
            surface.begin_layer(Layer::Content);
            surface.fill_rect(Rect::new(0.0, 0.0, w as f64, h as f64), BACKDROP);

            // Lay out both blocks first so we can vertically center the stack.
            let title = gpu.text_cache.get_or_build(
                &mut gpu.font_cx, TITLE_TEXT, FONT_HEADING, col_w, Alignment::Center, false, false,
            );
            let title_h = title.height() as f64;
            let body = gpu.text_cache.get_or_build(
                &mut gpu.font_cx, message, FONT_BODY, col_w, Alignment::Center, false, false,
            );
            let body_h = body.height() as f64;

            let gap = 16.0;
            let total_h = title_h + gap + body_h;
            let title_y = ((h as f64) - total_h) / 2.0;
            let body_y  = title_y + title_h + gap;

            draw_centered_text(&mut surface, &mut gpu.font_cx, &mut gpu.text_cache,
                TITLE_TEXT, FONT_HEADING, col_w, origin_x, title_y, TITLE);
            draw_centered_text(&mut surface, &mut gpu.font_cx, &mut gpu.text_cache,
                message, FONT_BODY, col_w, origin_x, body_y, BODY);
        }

        let surface_tex = match gpu.surface.get_current_texture() {
            Ok(t)  => t,
            Err(e) => { tracing::error!("fatal screen get_current_texture: {e:?}"); return; }
        };
        let target_view = surface_tex.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.quad_renderer.flush_dirty_rect(
            &gpu.device, &gpu.queue, &mut encoder, &target_view,
            ContentRepaint::Full, OverlayRepaint::Redraw,
        );
        gpu.queue.submit(std::iter::once(encoder.finish()));
        surface_tex.present();
    }
}

/// Draw `text` as centered glyph runs at `(origin_x, origin_y)` over `max_width`.
/// Mirrors the per-line glyph loop used by the Trame renderer.
#[allow(clippy::too_many_arguments)]
fn draw_centered_text(
    surface:   &mut WgpuSurface<'_>,
    font_cx:   &mut parley::FontContext,
    cache:     &mut TextCache,
    text:      &str,
    font_size: f32,
    max_width: f32,
    origin_x:  f64,
    origin_y:  f64,
    color:     Color,
) {
    if text.is_empty() {
        return;
    }
    let layout = cache.get_or_build(font_cx, text, font_size, max_width, Alignment::Center, false, false);
    let brush = Brush::Solid(color);
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
            let tx = origin_x + run.offset() as f64;
            let ty = origin_y + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
        }
    }
}

// -----------------------------------------------------------------------------
// DocSwitcher — route-driven document swap handle
// -----------------------------------------------------------------------------

/// A cloneable, type-erased handle that swaps the active document on the live
/// canvas without recreating the GPU surface. Returned by [`run_with`] so the
/// SPA shell (Leptos router) can point the persistent canvas at the routed
/// document on navigation. Main-thread only (`!Send`), like every canvas handle.
#[derive(Clone)]
pub struct DocSwitcher {
    set: Rc<dyn Fn(ambre_bus::DocumentId)>,
}

impl DocSwitcher {
    /// Point the canvas at `document_id`. A no-op if it is already current.
    pub fn set_document(&self, document_id: ambre_bus::DocumentId) {
        (self.set)(document_id);
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
/// `make_view` receives the canvas size in physical pixels plus the transparent
/// [`TextInput`] pipe (so the view can focus it and follow it to the caret), and
/// returns the view to drive. It runs on this (UI) thread before the GPU is ready.
pub fn run<V, F>(canvas_id: &str, make_view: F) -> DocSwitcher
where
    V: DocView + 'static,
    F: FnOnce(f64, f64, TextInput) -> V,
{
    run_with(canvas_id, make_view, || {}, RafControl::new())
}

/// Like [`run`], but invokes `pre_frame` once per animation frame on the UI
/// thread, immediately before the view's per-frame update. The launcher uses
/// this to pump the UI-thread command dispatcher (drain Logic->UI events,
/// mirror command state into the chrome signals, forward content to the view)
/// without this generic renderer knowing anything about commands or Leptos.
pub fn run_with<V, F, P>(
    canvas_id: &str,
    make_view: F,
    mut pre_frame: P,
    raf: RafControl,
) -> DocSwitcher
where
    V: DocView + 'static,
    F: FnOnce(f64, f64, TextInput) -> V,
    P: FnMut() + 'static,
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

    // The transparent <textarea> pipe (keyboard + IME + clipboard). Attached
    // under the canvas's positioned cell so a `transform` caret-follow is
    // measured against the same box; it renders nothing (see text_input.rs).
    let input_parent: web_sys::Element = canvas
        .parent_node()
        .expect("canvas has a parent cell")
        .dyn_into()
        .expect("canvas parent is an Element");
    let text_input = TextInput::create(&document, &input_parent);

    let view = make_view(canvas.width() as f64, canvas.height() as f64, text_input.clone());
    let host = Rc::new(RefCell::new(FrameHost::new(view, perf)));

    // -- Async GPU init, then start the rAF loop. -----------------------------
    {
        let host_c = Rc::clone(&host);
        let canvas_c = canvas.clone();
        let raf_c = raf.clone();
        // Start the rAF loop only after the GPU is ready so Firefox sees active
        // GPU frames from frame 1 (empty early frames make it lock to 60 Hz).
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(gpu) = bootstrap_gpu(canvas_c).await {
                host_c.borrow_mut().install_gpu(gpu);
            }
            // `pre_frame` is the route-independent relay (Logic<->Sync, the
            // dispatcher, the SPA route poll); it must run EVERY frame, even when
            // the canvas is frozen behind a modal or the landing overlay. So the
            // outer loop is always-on and only the render is gated by `raf_c`.
            start_raf_loop(
                move |ts| {
                    pre_frame();
                    if raf_c.is_running() {
                        host_c.borrow_mut().render(ts);
                    }
                },
                RafControl::new(),
            );
        });
    }

    // -- Keyboard input -> view. ----------------------------------------------
    {
        let host_c = Rc::clone(&host);
        let cb = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            move |event: web_sys::KeyboardEvent| {
                let consumed = host_c
                    .borrow_mut()
                    .handle_input(&InputEvent::Key {
                        key: event.key(),
                        ctrl: event.ctrl_key(),
                        alt: event.alt_key(),
                        shift: event.shift_key(),
                        meta: event.meta_key(),
                    });
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

    // -- Textarea pipe: committed text + IME composition -> view. -------------
    // `input` fires for latin typing / paste / autocomplete; outside a
    // composition we read the buffer, forward it as `Text`, and clear it so the
    // textarea never accumulates. During a composition the buffer is half-formed,
    // so we skip it and let the composition events drive the preview instead.
    {
        let host_c    = Rc::clone(&host);
        let composing = text_input.composing();
        let ta        = text_input.element().clone();
        let ta_read   = ta.clone();
        let cb = Closure::<dyn FnMut(web_sys::InputEvent)>::new(move |_event| {
            if composing.get() {
                return;
            }
            let value = ta_read.value();
            if !value.is_empty() {
                host_c.borrow_mut().handle_input(&InputEvent::Text { text: value });
                ta_read.set_value("");
            }
        });
        ta.add_event_listener_with_callback("input", cb.as_ref().unchecked_ref())
            .expect("add input listener");
        cb.forget();
    }

    // compositionstart: enter the composing window (gate the `input` listener).
    {
        let composing = text_input.composing();
        let cb = Closure::<dyn FnMut(web_sys::CompositionEvent)>::new(move |_event| {
            composing.set(true);
        });
        text_input
            .element()
            .add_event_listener_with_callback("compositionstart", cb.as_ref().unchecked_ref())
            .expect("add compositionstart listener");
        cb.forget();
    }

    // compositionupdate: forward the running preview; canvas renders it styled.
    {
        let host_c = Rc::clone(&host);
        let cb = Closure::<dyn FnMut(web_sys::CompositionEvent)>::new(
            move |event: web_sys::CompositionEvent| {
                let text = event.data().unwrap_or_default();
                host_c
                    .borrow_mut()
                    .handle_input(&InputEvent::CompositionUpdate { text });
            },
        );
        text_input
            .element()
            .add_event_listener_with_callback("compositionupdate", cb.as_ref().unchecked_ref())
            .expect("add compositionupdate listener");
        cb.forget();
    }

    // compositionend: commit the final text, leave the composing window, clear.
    {
        let host_c    = Rc::clone(&host);
        let composing = text_input.composing();
        let ta        = text_input.element().clone();
        let cb = Closure::<dyn FnMut(web_sys::CompositionEvent)>::new(
            move |event: web_sys::CompositionEvent| {
                composing.set(false);
                let text = event.data().unwrap_or_default();
                host_c
                    .borrow_mut()
                    .handle_input(&InputEvent::CompositionEnd { text });
                ta.set_value("");
            },
        );
        text_input
            .element()
            .add_event_listener_with_callback("compositionend", cb.as_ref().unchecked_ref())
            .expect("add compositionend listener");
        cb.forget();
    }

    // -- Canvas resize -> host (ResizeObserver). ------------------------------
    {
        let host_c    = Rc::clone(&host);
        let canvas_c  = canvas.clone();
        let canvas_c2 = canvas.clone();
        let cb = Closure::<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>::new(
            move |_entries, _observer| {
                if let Some((w, h)) = size_backing_store(&canvas_c2) {
                    host_c.borrow_mut().resize(w, h);
                }
            },
        );
        let observer = web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref())
            .expect("ResizeObserver");
        observer.observe(&canvas_c);
        cb.forget();
    }

    // -- DPI change -> host (matchMedia). --------------------------------------
    // When the window moves between displays of different density, the canvas
    // CSS size stays the same but devicePixelRatio changes. ResizeObserver does
    // not fire in that case, so we watch resolution changes separately.
    {
        let host_c    = Rc::clone(&host);
        let canvas_c  = canvas.clone();
        let cb = Closure::<dyn FnMut(web_sys::MediaQueryListEvent)>::new(move |_event| {
            if let Some((w, h)) = size_backing_store(&canvas_c) {
                host_c.borrow_mut().resize(w, h);
            }
        });
        if let Some(mql) = web_sys::window()
            .and_then(|w| w.match_media("(resolution)").ok())
            .flatten()
        {
            let _ = mql.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref());
        }
        cb.forget();
    }

    // -- Pointer events -> view. -----------------------------------------------
    {
        let host_c = Rc::clone(&host);
        let dpr = web_sys::window().map(|w| w.device_pixel_ratio()).unwrap_or(1.0);

        let cb = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(
            move |event: web_sys::PointerEvent| {
                let x = event.offset_x() as f64 * dpr;
                let y = event.offset_y() as f64 * dpr;
                let ev = match event.type_().as_str() {
                    "pointerdown" => InputEvent::PointerDown { x, y, button: event.button() },
                    "pointermove" => InputEvent::PointerMove { x, y },
                    "pointerup"   => InputEvent::PointerUp { x, y, button: event.button() },
                    _ => return,
                };
                let consumed = host_c.borrow_mut().handle_input(&ev);
                if consumed {
                    event.prevent_default();
                }
            },
        );
        for event_type in &["pointerdown", "pointermove", "pointerup"] {
            canvas
                .add_event_listener_with_callback(event_type, cb.as_ref().unchecked_ref())
                .expect("add pointer listener");
        }
        cb.forget();
    }

    // -- Double-click -> view. ------------------------------------------------
    // A separate `dblclick` listener so the platform's own double-click timing
    // (the OS setting) decides what counts as a double-click -- no hand-rolled
    // threshold. `MouseEvent::offset_*` is in CSS px; scale to physical like the
    // pointer path.
    {
        let host_c = Rc::clone(&host);
        let dpr = web_sys::window().map(|w| w.device_pixel_ratio()).unwrap_or(1.0);
        let cb = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(
            move |event: web_sys::MouseEvent| {
                let ev = InputEvent::DoubleClick {
                    x: event.offset_x() as f64 * dpr,
                    y: event.offset_y() as f64 * dpr,
                };
                if host_c.borrow_mut().handle_input(&ev) {
                    event.prevent_default();
                }
            },
        );
        canvas
            .add_event_listener_with_callback("dblclick", cb.as_ref().unchecked_ref())
            .expect("add dblclick listener");
        cb.forget();
    }

    // -- Wheel event -> view. -------------------------------------------------
    // The browser emits `wheel` for the mouse wheel *and* for two-finger
    // touchpad scrolling, so this single listener covers both. Deltas are in
    // CSS px (pixel mode) or lines/pages; scale to physical px like the pointer
    // path and forward `delta_mode` so the view can normalise lines/pages.
    {
        let host_c = Rc::clone(&host);
        let dpr = web_sys::window().map(|w| w.device_pixel_ratio()).unwrap_or(1.0);
        let cb = Closure::<dyn FnMut(web_sys::WheelEvent)>::new(
            move |event: web_sys::WheelEvent| {
                let ev = InputEvent::Wheel {
                    dx: event.delta_x() * dpr,
                    dy: event.delta_y() * dpr,
                    delta_mode: event.delta_mode(),
                    ctrl_key: event.ctrl_key(),
                };
                let consumed = host_c.borrow_mut().handle_input(&ev);
                if consumed {
                    event.prevent_default();
                }
            },
        );
        canvas
            .add_event_listener_with_callback("wheel", cb.as_ref().unchecked_ref())
            .expect("add wheel listener");
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

    // Hand the caller a route-driven document-swap handle. It captures the host
    // (kept alive by the rAF closure regardless), so swapping the active
    // document never touches the GPU surface.
    DocSwitcher {
        set: Rc::new(move |id| host.borrow_mut().set_document(id)),
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
