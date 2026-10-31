// ambre-canvas/src/host.rs
// ============================================================================
// UI Thread host plumbing -- generic, app-agnostic GPU + rAF primitives.
//
//   GpuState            wgpu Device/Queue/Surface + QuadRenderer + text stack.
//   bootstrap_gpu       async wgpu init (WebGL2 by default, ?webgpu opt-in).
//   start_raf_loop      requestAnimationFrame driver over a frame callback.
//   GpuSettled          the token that orders the loop after the GPU init.
//   fetch_bytes         same-origin asset fetch (fonts).
//
// This module owns no application state and names no per-app view. The frame
// DRIVER (`FrameHost`), the closed `AppView` registry, and the DOM event wiring
// live in the launcher (`ambre-app`), which depends on every app crate. A closed
// view enum naming `TrameView` cannot live upstream here without a dependency
// cycle (ambre-spec-architecture.md). The launcher reads/writes the `GpuState`
// it gets from `bootstrap_gpu` and drives `start_raf_loop`.
//
// wasm32-only: surface creation from an HTMLCanvasElement
// (`wgpu::SurfaceTarget::Canvas`) and the web_sys glue do not exist on native.
// ============================================================================

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::{QuadRenderer, SurfaceSize, TextCache};
use ambre_core::{Px, Size, Texel};

// -----------------------------------------------------------------------------
// GpuState
// -----------------------------------------------------------------------------

/// All wgpu resources needed to render one frame.
pub struct GpuState {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub surface_format: wgpu::TextureFormat,
    /// The drawing buffer. Only its logical size is readable outside this
    /// crate (`CANVAS-044`).
    pub size: SurfaceSize,
    pub quad_renderer: QuadRenderer,
    pub font_cx: parley::FontContext,
    pub text_cache: TextCache,
}

impl GpuState {
    /// Fit the surface and the offscreens to `size`, in device pixels.
    pub fn configure(&mut self, size: SurfaceSize) {
        self.surface.configure(
            &self.device,
            &surface_config(self.surface_format, size.physical()),
        );
        self.quad_renderer.resize_offscreens(&self.device, size);
        self.size = size;
    }

    /// Reconfigure a lost or outdated swapchain at the current size. The
    /// offscreens survive, so no repaint is owed.
    pub fn reconfigure_surface(&self) {
        self.surface.configure(
            &self.device,
            &surface_config(self.surface_format, self.size.physical()),
        );
    }
}

/// The swapchain configuration for a buffer of `size` device pixels.
fn surface_config(
    format: wgpu::TextureFormat,
    size: Size<Px<Texel>>,
) -> wgpu::SurfaceConfiguration {
    wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: size.width.get(),
        height: size.height.get(),
        present_mode: wgpu::PresentMode::AutoNoVsync,
        desired_maximum_frame_latency: 2,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
    }
}

// -----------------------------------------------------------------------------
// rAF loop
// -----------------------------------------------------------------------------

/// Proof that GPU initialization has settled, whether it succeeded or not.
///
/// Only [`bootstrap_gpu`] mints one, and [`start_raf_loop`] takes it, so the
/// loop cannot start first (`CANVAS-004`). Firefox calibrates vsync on the
/// first frames; empty ones lock the loop to 60 Hz for good.
pub struct GpuSettled(());

/// Start the requestAnimationFrame loop, invoking `frame(now)` on every
/// frame, once the GPU has settled. `now` is the instant the browser hands
/// the frame callback.
///
/// The loop re-arms on every tick, whatever the frame did (`CANVAS-011`). A
/// static document still ticks, so an inbound event drains on the next frame.
/// Pausing is the frame's business ([`crate::frame::drive_frame`]).
///
/// One `Closure` is kept alive for the app lifetime and reused every frame,
/// with zero per-frame allocation. The naive allocate-and-forget pattern
/// leaks a closure per frame. At 240 Hz that grows the JS heap and makes
/// Firefox throttle rAF down to 60 Hz as self-protection.
pub fn start_raf_loop(
    mut frame: impl FnMut(ambre_bus::Instant<ambre_bus::PageLoad>) + 'static,
    _settled: GpuSettled,
) {
    let cb: Rc<FrameSlot> = Rc::new(RefCell::new(None));
    let cb_clone = Rc::clone(&cb);

    *cb.borrow_mut() = Some(Closure::new(move |ts: f64| {
        frame(ambre_bus::Instant::<ambre_bus::PageLoad>::new(ts));
        request_frame(&cb_clone);
    }));
    request_frame(&cb);

    std::mem::forget(cb);
}

/// Run `relay` every `period` while the page is hidden
/// ([`crate::frame::drive_hidden`]). The browser may space a hidden page's
/// timers further apart; the relay then runs that often. The interval lives
/// as long as the page.
pub fn start_hidden_relay(
    mut relay: impl FnMut() + 'static,
    period: ambre_bus::TimeSpan<ambre_bus::Ms>,
) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let document = window.document();
    let tick = Closure::<dyn FnMut()>::new(move || {
        let hidden = document.as_ref().is_some_and(web_sys::Document::hidden);
        crate::frame::drive_hidden(hidden, &mut relay);
    });
    if let Err(e) = window.set_interval_with_callback_and_timeout_and_arguments_0(
        tick.as_ref().unchecked_ref(),
        period.to_timer_i32(),
    ) {
        tracing::error!("hidden relay not started: {e:?}");
    }
    tick.forget();
}

/// The one rAF closure the loop reuses, empty only while it is being built.
type FrameSlot = RefCell<Option<Closure<dyn FnMut(f64)>>>;

/// Schedule `slot`'s closure for the next frame. A worker has no window and
/// a refused request schedules nothing: the loop stops and says why, rather
/// than panicking the whole module.
fn request_frame(slot: &FrameSlot) {
    let Some(window) = web_sys::window() else {
        tracing::error!("rAF loop stopped: no window");
        return;
    };
    let slot = slot.borrow();
    let Some(closure) = slot.as_ref() else {
        return;
    };
    if let Err(e) = window.request_animation_frame(closure.as_ref().unchecked_ref()) {
        tracing::error!("rAF loop stopped: {e:?}");
    }
}

// -----------------------------------------------------------------------------
// GPU bootstrap (async)
// -----------------------------------------------------------------------------

/// Bring the GPU up on `canvas`, whose backing store `size` describes.
///
/// The token says the attempt is over, the state is `None` when it failed.
pub async fn bootstrap_gpu(
    canvas: HtmlCanvasElement,
    size: SurfaceSize,
) -> (GpuSettled, Option<GpuState>) {
    // WebGPU on Mesa/Intel Firefox has a compositor bug. The render pipeline
    // produces correct pixels (confirmed via GPU readback), but the browser
    // compositor silently fails to display the canvas on screen.
    //
    // This cannot be detected from JavaScript: `drawImage` reads the GPU buffer
    // directly and bypasses the compositor, so a probe canvas always looks correct.
    //
    // Strategy: default to GL (WebGL2) on WASM, which works universally. Opt in to
    // WebGPU with `?webgpu` in the URL (useful for Nvidia and future Mesa versions
    // where the bug is fixed).
    let use_webgpu = url_flag("webgpu");
    let backends = if use_webgpu {
        tracing::info!("WebGPU requested via URL flag");
        wgpu::Backends::BROWSER_WEBGPU
    } else {
        wgpu::Backends::GL
    };

    let gpu = create_gpu_state(canvas, size, backends).await;
    (GpuSettled(()), gpu)
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

async fn create_gpu_state(
    canvas: HtmlCanvasElement,
    size: SurfaceSize,
    backends: wgpu::Backends,
) -> Option<GpuState> {
    let Size { width, height, .. } = size.physical();

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    });

    let surface = match instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas)) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("create_surface failed: {e:?}");
            return None;
        }
    };

    let adapter = match instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
    {
        Ok(a) => a,
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
        Ok(dq) => dq,
        Err(e) => {
            tracing::error!("request_device failed: {e:?}");
            return None;
        }
    };

    let caps = surface.get_capabilities(&adapter);
    tracing::info!(
        "surface caps: formats={:?} alpha_modes={:?}",
        caps.formats,
        caps.alpha_modes
    );
    let preferred = caps
        .formats
        .iter()
        .find(|&&f| f == wgpu::TextureFormat::Bgra8Unorm || f == wgpu::TextureFormat::Rgba8Unorm);
    // A surface the adapter cannot present to has no format at all.
    let Some(&format) = preferred.or(caps.formats.first()) else {
        tracing::error!("surface reports no texture format");
        return None;
    };

    surface.configure(&device, &surface_config(format, size.physical()));

    let mut quad_renderer = QuadRenderer::new(&device, format);
    // Size the ping-pong offscreens to the initial canvas. ResizeObserver
    // doesn't fire on startup, so handle_resize would otherwise never
    // initialize them before the first render.
    quad_renderer.resize_offscreens(&device, size);

    // The base faces (Noto Sans + DejaVu) shared with every other text-capable
    // FontContext consumer; lazy scripts are fetched on demand at shape time.
    let font_cx = crate::fonts::font_context_from().await;

    let info = adapter.get_info();
    tracing::info!(
        "GPU initialized: {} ({:?}) | {width}x{height} format={format:?}",
        info.name,
        info.backend
    );

    Some(GpuState {
        device,
        queue,
        surface,
        surface_format: format,
        size,
        quad_renderer,
        font_cx,
        text_cache: TextCache::new(),
    })
}

// -----------------------------------------------------------------------------
// Font fetch helper
// -----------------------------------------------------------------------------

pub async fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    let window = web_sys::window()?;
    let resp_val = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
        .await
        .ok()?;
    let resp: web_sys::Response = resp_val.dyn_into().ok()?;
    if !resp.ok() {
        return None;
    }
    let ab_val = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().ok()?)
        .await
        .ok()?;
    let ab: js_sys::ArrayBuffer = ab_val.dyn_into().ok()?;
    Some(js_sys::Uint8Array::new(&ab).to_vec())
}
