// ambre-app/src/host.rs
// ============================================================================
// UI Thread frame DRIVER -- the launcher-owned half of the canvas runtime.
//
//   AppView      the CLOSED registry of per-app views (one variant per document
//                type: `Trame(TrameView)`, future `Prose(ProseView)`, ...). It
//                names the concrete app views, so it must live downstream of the
//                app crates -- here, in the launcher -- not in `ambre-canvas`,
//                which is upstream of them (a closed enum naming `TrameView`
//                upstream would cycle; ambre-spec-architecture.md 6).
//   FrameHost    drives one `AppView` across the GPU and the rAF loop: owns the
//                `ambre_canvas::GpuState`, measures timings, encodes the layers
//                from the view's `RepaintPlan`, presents the swapchain, and
//                paints the fatal terminal screen.
//   run_with     UI-thread entry: build the view, install GPU, start the rAF
//                loop, and wire the DOM input/resize listeners to the host.
//   DocSwitcher  route-driven document swap handle (SPA navigation).
//
// `ambre-canvas` keeps the GPU PRIMITIVES (`GpuState`, `bootstrap_gpu`,
// `RafControl`, `start_raf_loop`, the quad/glyph pipeline) and the GPU-agnostic
// value types (`RepaintPlan`/`FrameStats`/`InputEvent`); this module composes
// them around the typed `AppView`.
//
// wasm32-only (the whole launcher is): web_sys glue + wgpu surface present.
// ============================================================================

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use parley::FontContext;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use ambre_bus::DocumentId;
use ambre_canvas::theme::{FONT_BODY, FONT_HEADING};
use ambre_canvas::{
    ContentRepaint, FrameStats, GpuState, InputEvent, Layer, OverlayRepaint, RafControl,
    RepaintPlan, TextCache, TextInput, WgpuSurface, bootstrap_gpu, fatal_message, start_raf_loop,
};
use ambre_draw::{Affine, Alignment, Brush, Color, DrawSurface, PositionedLayoutItem, Rect};
use ambre_toile::ui::ToileView;
use ambre_trame::ui::TrameView;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

// FrameHost and `run_with` live in sibling modules; both reach the shared types
// (AppView, DocSwitcher, helpers) via `use super::*`.
mod frame;
mod run;

pub use frame::FrameHost;
pub use run::run_with;

/// Private clipboard MIME type carrying the base64 postcard
/// [`ClipboardPayload`](ambre_bus::ClipboardPayload) for an internal round-trip
/// on the *synchronous* `copy`/`paste` events (Ctrl+C/Ctrl+V). Browsers that
/// reject it fall back to `text/plain` (`-ui.md` sec.5.E).
const AMBRE_CLIPBOARD_TYPE: &str = "application/x-ambre-trame";

/// The same payload under the `web ` prefix the *asynchronous* Clipboard API
/// (`navigator.clipboard.read`/`write`) requires for a custom type. This is the
/// channel the toolbar/menu Paste button uses, since it cannot ride a native
/// `paste` event (`-ui.md` sec.5.E).
pub(crate) const AMBRE_CLIPBOARD_WEB_TYPE: &str = "web application/x-ambre-trame";

/// Base64-encode bytes for transport as a clipboard string (UTF-8 safe).
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

/// Decode a base64 clipboard string back to bytes; `None` on malformed input.
pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    BASE64.decode(s).ok()
}

/// Best-effort asynchronous publish of a copied range to the OS clipboard via
/// `navigator.clipboard.write`, carrying the tab-separated `text/plain` plus the
/// private `web ` payload type so the async-reading Paste button gets full
/// fidelity (styles + formula reindexing). Runs in addition to the synchronous
/// `setData` (which serves Ctrl+V); any failure is ignored -- Ctrl+V still works.
pub(crate) fn async_write_clipboard(tsv: String, payload_b64: Option<String>) {
    let Some(clipboard) = web_sys::window().map(|w| w.navigator().clipboard()) else {
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        // Build the `{ mime: Promise<string> }` record ClipboardItem expects.
        let record = js_sys::Object::new();
        let text_promise = js_sys::Promise::resolve(&JsValue::from_str(&tsv));
        let _ = js_sys::Reflect::set(&record, &JsValue::from_str("text/plain"), &text_promise);
        if let Some(b64) = payload_b64 {
            let p = js_sys::Promise::resolve(&JsValue::from_str(&b64));
            let _ = js_sys::Reflect::set(&record, &JsValue::from_str(AMBRE_CLIPBOARD_WEB_TYPE), &p);
        }
        let Ok(item) = web_sys::ClipboardItem::new_with_record_from_str_to_str_promise(&record)
        else {
            return;
        };
        let items = js_sys::Array::of1(&item);
        let _ = wasm_bindgen_futures::JsFuture::from(clipboard.write(&items)).await;
    });
}

// -----------------------------------------------------------------------------
// AppView — the closed per-app view registry
// -----------------------------------------------------------------------------

/// The active document's view. A closed sum over document types: adding an app
/// is one variant here plus its inherent driver methods below (no trait object,
/// no `Box<dyn Any>`, exhaustiveness checked by the compiler -- symmetric to
/// `AppCommand`/`DocState`). The driver (`FrameHost`) dispatches over it by
/// `match`; each variant's view exposes the same inherent method set.
pub enum AppView {
    /// A Trame (spreadsheet) document.
    Trame(TrameView),
    /// A Toile (whiteboard) document.
    Toile(ToileView),
}

impl AppView {
    /// Pump inbound state before painting (drain Logic->UI, set dirty flags).
    fn update(&mut self) {
        match self {
            AppView::Trame(v) => v.update(),
            AppView::Toile(v) => v.update(),
        }
    }

    /// Decide what to repaint this frame and clear the dirty flags.
    fn plan_repaint(&mut self, hud_tick: bool) -> RepaintPlan {
        match self {
            AppView::Trame(v) => v.plan_repaint(hud_tick),
            AppView::Toile(v) => v.plan_repaint(hud_tick),
        }
    }

    /// Paint the persistent content layer (`strip` = scroll repaint rect).
    fn paint_content(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        strip: Option<Rect>,
    ) {
        match self {
            AppView::Trame(v) => v.paint_content(surface, font_cx, text_cache, strip),
            AppView::Toile(v) => v.paint_content(surface, font_cx, text_cache, strip),
        }
    }

    /// Paint the transient overlay layer (`stats` feeds the debug HUD).
    fn paint_overlay(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        stats: &FrameStats,
    ) {
        match self {
            AppView::Trame(v) => v.paint_overlay(surface, font_cx, text_cache, stats),
            AppView::Toile(v) => v.paint_overlay(surface, font_cx, text_cache, stats),
        }
    }

    /// Canvas resized to `width` x `height` physical pixels.
    fn resize(&mut self, width: u32, height: u32) {
        match self {
            AppView::Trame(v) => v.resize(width, height),
            AppView::Toile(v) => v.resize(width, height),
        }
    }

    /// Handle an input event; returns `true` to `preventDefault()`.
    fn handle_input(&mut self, ev: &InputEvent) -> bool {
        match self {
            AppView::Trame(v) => v.handle_input(ev),
            AppView::Toile(v) => v.handle_input(ev),
        }
    }

    /// Build the clipboard contents for a `copy`/`cut`, or `None` to let the
    /// browser default run. Returns the OS-interop text plus an optional typed
    /// [`ClipboardPayload`](ambre_bus::ClipboardPayload) (present only for a grid
    /// range, carrying styles + relative-formula reindexing for an internal
    /// round-trip). `cut` deletes the source as a side effect.
    fn clipboard_write(
        &mut self,
        cut: bool,
    ) -> Option<(String, Option<ambre_bus::ClipboardPayload>)> {
        match self {
            AppView::Trame(v) => v.clipboard_write(cut),
            AppView::Toile(v) => v.clipboard_write(cut),
        }
    }

    /// Service a `paste`: a typed Ambre payload (internal round-trip) when
    /// present, else the OS plain text.
    fn paste_clipboard(&mut self, payload: Option<ambre_bus::ClipboardPayload>, text: String) {
        match self {
            AppView::Trame(v) => {
                if v.is_editing() {
                    // In-cell edit: route through the same text-insertion path as
                    // typing, so the paste lands at the caret (the typed payload,
                    // if any, is meaningless inside a text field).
                    v.handle_input(&InputEvent::Text { text });
                } else if let Some(payload) = payload {
                    v.grid_paste_payload(payload);
                } else {
                    v.grid_paste_text(&text);
                }
            }
            AppView::Toile(v) => v.paste_clipboard(payload, text),
        }
    }

    /// Swap the active document in place, keeping the GPU surface.
    fn set_document(&mut self, document_id: DocumentId) {
        match self {
            AppView::Trame(v) => v.set_document(document_id),
            AppView::Toile(v) => v.set_document(document_id),
        }
    }
}

// -----------------------------------------------------------------------------
// FrameHost — per-frame driver over an AppView
// -----------------------------------------------------------------------------

/// Drives one [`AppView`] across the GPU and the requestAnimationFrame loop.
///
/// App-agnostic at this level: it owns the [`GpuState`], measures frame timings,
/// decides which layers to encode from the view's [`RepaintPlan`], and presents
/// the swapchain. Everything document-specific lives behind the `AppView` match.
/// Draw `text` as centered glyph runs at `(origin_x, origin_y)` over `max_width`.
/// Mirrors the per-line glyph loop used by the Trame renderer.
#[allow(clippy::too_many_arguments)]
fn draw_centered_text(
    surface: &mut WgpuSurface<'_>,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    text: &str,
    font_size: f32,
    max_width: f32,
    origin_x: f64,
    origin_y: f64,
    color: Color,
) {
    if text.is_empty() {
        return;
    }
    let layout = cache.get_or_build(
        font_cx,
        text,
        font_size,
        max_width,
        Alignment::Center,
        false,
        false,
    );
    let brush = Brush::Solid(color);
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = origin_x + run.offset() as f64;
            let ty = origin_y + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
        }
    }
}

// -----------------------------------------------------------------------------
// DocSwitcher — route-driven document swap handle
// -----------------------------------------------------------------------------

/// A cloneable handle that swaps the active document on the live canvas without
/// recreating the GPU surface. Returned by [`run_with`] so the SPA shell (Leptos
/// router) can point the persistent canvas at the routed document on navigation.
/// Main-thread only (`!Send`), like every canvas handle.
#[derive(Clone)]
pub struct DocSwitcher {
    set: Rc<dyn Fn(DocumentId)>,
}

impl DocSwitcher {
    /// Point the canvas at `document_id`. A no-op if it is already current.
    pub fn set_document(&self, document_id: DocumentId) {
        (self.set)(document_id);
    }
}

// -----------------------------------------------------------------------------
// run_with — UI thread entry point
// -----------------------------------------------------------------------------

/// Run the UI thread: build the application view, drive it through a
/// [`FrameHost`] on the requestAnimationFrame loop, and forward DOM input and
/// resize events to it.
///
/// `make_view` receives the canvas size in physical pixels plus the transparent
/// [`TextInput`] pipe (so the view can focus it and follow it to the caret), and
/// returns the [`AppView`] to drive. It runs on this (UI) thread before the GPU
/// is ready. `pre_frame` runs once per animation frame immediately before the
/// view's update -- the launcher pumps its route-independent relay there (the
/// Logic<->Sync relay, the dispatcher, the SPA route poll), so this driver knows
/// nothing about commands or Leptos. `raf` gates only the render (a modal /
/// landing overlay freezes the canvas), never the relay.
/// True when keydown `target` is an editable DOM element OTHER than the canvas
/// text `pipe`: an `<input>` / `<textarea>` / `<select>`, or a contenteditable
/// host. Such elements own their own keyboard, so the grid's document-level
/// keydown listener must not also process the event. The pipe itself is exempt:
/// the grid intentionally reads navigation/edit keys from it via this listener.
fn is_other_editable(target: &web_sys::EventTarget, pipe: &web_sys::HtmlTextAreaElement) -> bool {
    if target
        .dyn_ref::<web_sys::HtmlTextAreaElement>()
        .is_some_and(|ta| ta == pipe)
    {
        return false;
    }
    match target.dyn_ref::<web_sys::HtmlElement>() {
        Some(el) => {
            matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT")
                || el.is_content_editable()
        }
        None => false,
    }
}

/// Set a canvas backing store to its CSS layout size scaled by the device pixel
/// ratio, so rendering is crisp on HiDPI displays. Returns the new physical
/// `(width, height)` in device pixels, or `None` if the cell has zero area
/// (detached / display:none) -- in which case the caller skips the resize.
fn size_backing_store(canvas: &HtmlCanvasElement) -> Option<(u32, u32)> {
    let dpr = web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .unwrap_or(1.0);
    let w = (canvas.client_width() as f64 * dpr).round() as u32;
    let h = (canvas.client_height() as f64 * dpr).round() as u32;
    if w == 0 || h == 0 {
        return None;
    }
    canvas.set_width(w);
    canvas.set_height(h);
    Some((w, h))
}

// ── Tests (wasm tier) ───────────────────────────────────────────────────────
// Pure helpers only (base64 transport codec + the FPS sliding window). The crate
// is `#![cfg(target_arch = "wasm32")]`, so these run under the wasm target via
// the `wasm-bindgen-test-runner` (node, no DOM). See ambre-spec-testing.md.
#[cfg(test)]
mod tests {
    use super::{FrameHost, base64_decode, base64_encode};
    use std::collections::VecDeque;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn base64_roundtrips() {
        for bytes in [
            Vec::new(),
            vec![0u8],
            vec![1, 2, 3, 4, 5],
            vec![0, 255, 128, 64, 32, 16, 8],
        ] {
            assert_eq!(base64_decode(&base64_encode(&bytes)), Some(bytes));
        }
    }

    #[wasm_bindgen_test]
    fn base64_decode_rejects_malformed() {
        assert_eq!(base64_decode("!!!not base64!!!"), None);
    }

    #[wasm_bindgen_test]
    fn tick_fps_needs_two_samples() {
        let mut times = VecDeque::new();
        let mut fps = -1.0f32;
        FrameHost::tick_fps(&mut times, &mut fps, 0.0);
        // A single sample cannot establish a rate.
        assert_eq!(fps, 0.0);
    }

    #[wasm_bindgen_test]
    fn tick_fps_computes_rate_over_window() {
        let mut times = VecDeque::new();
        let mut fps = 0.0f32;
        // 60 frames spaced 1/60 s over (just under) a second -> ~60 FPS.
        for i in 0..60 {
            FrameHost::tick_fps(&mut times, &mut fps, i as f64 * 1000.0 / 60.0);
        }
        assert!((fps - 60.0).abs() < 1.0, "expected ~60 fps, got {fps}");
    }

    #[wasm_bindgen_test]
    fn tick_fps_evicts_samples_older_than_one_second() {
        let mut times = VecDeque::new();
        let mut fps = 0.0f32;
        FrameHost::tick_fps(&mut times, &mut fps, 0.0);
        FrameHost::tick_fps(&mut times, &mut fps, 5000.0); // 5 s later
        // The 0.0 sample is > 1 s old and must be evicted, leaving one sample.
        assert_eq!(times.len(), 1);
        assert_eq!(fps, 0.0);
    }
}
