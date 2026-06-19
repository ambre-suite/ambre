// ambre-canvas/src/host.rs
// ============================================================================
// UI Thread host plumbing — generic, app-agnostic GPU + rAF primitives.
//
//   GpuState            wgpu Device/Queue/Surface + QuadRenderer + text stack.
//   bootstrap_gpu       async wgpu init (WebGL2 by default, ?webgpu opt-in).
//   start_raf_loop      requestAnimationFrame driver over a frame callback.
//   RafControl          gate to freeze the rAF loop behind a modal/overlay.
//   fetch_bytes         same-origin asset fetch (fonts).
//
// This module owns no application state and names no per-app view. The frame
// DRIVER (`FrameHost`), the closed `AppView` registry, and the DOM event wiring
// live in the launcher (`ambre-app`), which depends on every app crate -- a
// closed view enum naming `TrameView` cannot live upstream here without a
// dependency cycle (ambre-spec-architecture.md 6). The launcher reads/writes the
// `GpuState` it gets from `bootstrap_gpu` and drives `start_raf_loop`.
//
// wasm32-only: surface creation from an HTMLCanvasElement
// (`wgpu::SurfaceTarget::Canvas`) and the web_sys glue do not exist on native.
// ============================================================================

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::{QuadRenderer, TextCache};

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
        // Alternate families for the formatting toolbar's font picker
        // (`-ui.md` sec.5bis): a serif ("DejaVu Serif") and a monospace
        // ("DejaVu Sans Mono"), selectable by family name.
        ("/fonts/DejaVuSerif.ttf", "DejaVuSerif.ttf"),
        ("/fonts/DejaVuSansMono.ttf", "DejaVuSansMono.ttf"),
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
