// ambre-app/src/host.rs
// ============================================================================
// UI Thread frame DRIVER -- the launcher-owned half of the canvas runtime.
//
//   AppView      the CLOSED registry of per-app views (one variant per document
//                type: `Trame(TrameView)`, future `Prose(ProseView)`, ...). It
//                names the concrete app views, so it must live downstream of the
//                app crates: here, in the launcher. Not in `ambre-canvas`, which
//                is upstream of them. A closed enum naming `TrameView` upstream
//                would cycle (ambre-spec-architecture.md).
//
//   FrameHost    drives one `AppView` across the GPU and the rAF loop. It owns
//                the `ambre_canvas::GpuState`, measures timings, encodes the
//                layers from the view's `RepaintPlan`, presents the swapchain,
//                and paints the fatal terminal screen.
//
//   run_with     UI-thread entry: build the view, install GPU, start the rAF
//                loop, and wire the DOM input/resize listeners to the host.
//   DocSwitcher  route-driven document swap handle (SPA navigation).
//
// `ambre-canvas` keeps the GPU PRIMITIVES (`GpuState`, `bootstrap_gpu`,
// `RafControl`, `start_raf_loop`, the quad/glyph pipeline) and the GPU-agnostic
// value types (`RepaintPlan`/`FrameStats`/`InputEvent`). This module composes
// them around the typed `AppView`.
//
// wasm32-only (the whole launcher is): web_sys glue + wgpu surface present.
// ============================================================================

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use parley::FontContext;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use ambre_bus::app::toile::ElementId;
use ambre_bus::{DocType, DocumentId, LogicCommand};
use ambre_canvas::theme::{FONT_BODY, FONT_HEADING};
use ambre_canvas::{
    ContentRepaint, FrameStats, GpuState, InputEvent, Layer, OverlayRepaint, RafControl,
    RepaintPlan, TextCache, TextInput, WgpuSurface, bootstrap_gpu, fatal_message, start_raf_loop,
};
use ambre_draw::{
    Affine, Alignment, Brush, Color, DrawSurface, Point, PositionedLayoutItem, Rect, Size,
};
use ambre_toile::ui::ToileView;
use ambre_toile::ui::embed::{VisibleEmbed, VisibleWebEmbed};
use ambre_trame::ui::TrameView;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

// FrameHost and `run_with` live in sibling modules; both reach the shared types
// (AppView, DocSwitcher, helpers) via `use super::*`.
mod frame;
mod probe;

pub(crate) use probe::mark;
mod prose;
mod run;
mod web_embed;

pub use frame::{FrameHost, MakeChild};
pub use prose::{ProseSignals, ProseView};

/// Where a finger is in its gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    /// The browser took the gesture over (`KINETIC-010`).
    Cancel,
}
pub use run::run_with;

/// Private clipboard MIME type carrying the base64 postcard
/// [`ClipboardPayload`](ambre_bus::ClipboardPayload) for an internal round-trip
/// on the *synchronous* `copy`/`paste` events (Ctrl+C/Ctrl+V). Browsers that
/// reject it fall back to `text/plain` (`ambre-spec-clipboard.md`).
const AMBRE_CLIPBOARD_TYPE: &str = "application/x-ambre";

/// The same payload under the `web ` prefix the *asynchronous* Clipboard API
/// (`navigator.clipboard.read`/`write`) requires for a custom type. This is the
/// channel the toolbar/menu Paste button uses, since it cannot ride a native
/// `paste` event (`ambre-spec-clipboard.md`).
pub(crate) const AMBRE_CLIPBOARD_WEB_TYPE: &str = "web application/x-ambre";

/// The ownership token under the `web ` prefix the asynchronous Clipboard API
/// demands. The async publish writes it beside the payload so the whole-clipboard
/// `write` never removes the token the copy event established (`UI-THREAD-023`).
/// The Paste button reads it back to answer rung 2 of the fidelity ladder.
pub(crate) const AMBRE_CLIPBOARD_WEB_TOKEN_TYPE: &str = "web application/x-ambre-token";

/// Synchronous same-tab ownership marker (`ambre-spec-clipboard.md`,
/// `UI-THREAD-007`). Written into the clipboard event's `DataTransfer`
/// *synchronously* during `copy`/`cut`, it always reaches the OS clipboard. The
/// async write that follows replaces it. Chrome keeps it under the `web `
/// variant, which a `paste` event cannot see (`UI-THREAD-054`). Firefox refuses
/// that variant and the write carries the standard types alone (`UI-THREAD-055`).
///
/// On `paste`, a value here equal to this page's token
/// proves the clipboard still holds THIS tab's own last copy. The view then
/// pastes its in-memory `last_clip` at full fidelity. Any foreign copy (another
/// app / tab) overwrites the clipboard and drops the marker, so its absence
/// correctly means "not ours".
const AMBRE_CLIPBOARD_TOKEN_TYPE: &str = "application/x-ambre-token";

/// Base64-encode bytes for transport as a clipboard string (UTF-8 safe).
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

/// Decode a base64 clipboard string back to bytes; `None` on malformed input.
pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    BASE64.decode(s).ok()
}

/// Best-effort asynchronous publish of a copied range to the OS clipboard via
/// `navigator.clipboard.write`. It carries the tab-separated `text/plain`, the
/// `text/html` a view wrote for another editor, the private `web ` payload type
/// and the `web ` ownership token. The write replaces
/// the whole clipboard, so it re-publishes everything the copy event established
/// rather than dropping the token a frame after the copy (`UI-THREAD-023`).
/// Where the browser refuses the `web ` types, it writes the standard ones
/// alone (`UI-THREAD-055`). Runs
/// in addition to the synchronous `setData`; a failure is ignored, and the
/// synchronous formats stay. A success replaces them, so `published` records
/// it and the next `paste` event reads the clipboard back (`UI-THREAD-054`).
pub(crate) fn async_write_clipboard(
    tsv: String,
    html: Option<String>,
    payload_b64: Option<String>,
    token: String,
    published: Rc<std::cell::Cell<bool>>,
) {
    let Some(clipboard) = web_sys::window().map(|w| w.navigator().clipboard()) else {
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        let text = ("text/plain", Some(tsv.as_str()));
        let marked = html
            .as_deref()
            .map(|h| ambre_bus::ClipboardBlob::mark_html(h, &token));
        let html = ("text/html", html.as_deref());
        let payload = (AMBRE_CLIPBOARD_WEB_TYPE, payload_b64.as_deref());
        let token = (AMBRE_CLIPBOARD_WEB_TOKEN_TYPE, Some(token.as_str()));
        // Firefox rejects the `web ` types in the ClipboardItem constructor
        // (throwaway Firefox 156 run). The copy event left only the token, so
        // without a second attempt no other application could paste the copy.
        // The standard write carries the token in an HTML comment where the
        // view wrote HTML (`UI-THREAD-055`, `UI-THREAD-056`).
        let (item, web) = match clipboard_item(&[text, html, payload, token]) {
            Some(item) => (item, true),
            None => match clipboard_item(&[text, ("text/html", marked.as_deref())]) {
                Some(item) => (item, false),
                None => return,
            },
        };
        let items = js_sys::Array::of1(&item);
        let written = wasm_bindgen_futures::JsFuture::from(clipboard.write(&items)).await;
        // Only a landed write of the `web ` types moves the token out of the
        // paste event's reach (`UI-THREAD-054`).
        published.set(web && written.is_ok());
    });
}

/// Build a ClipboardItem from `(type, value)` pairs, skipping absent values.
/// `None` when the browser refuses one of the types.
fn clipboard_item(entries: &[(&str, Option<&str>)]) -> Option<web_sys::ClipboardItem> {
    let record = js_sys::Object::new();
    for (kind, value) in entries {
        if let Some(value) = value {
            let promise = js_sys::Promise::resolve(&JsValue::from_str(value));
            js_sys::Reflect::set(&record, &JsValue::from_str(kind), &promise).ok()?;
        }
    }
    web_sys::ClipboardItem::new_with_record_from_str_to_str_promise(&record).ok()
}

/// The decoded result of an async clipboard read. The shell sends it to the
/// host, which routes it to the active view through the same ladder as the
/// native `paste` event (`UI-THREAD-024`). `token` is the ownership marker
/// read back; the host compares it against this page's token to answer rung 2
/// (`UI-THREAD-007`). The host names no application: it addresses the active
/// view (`UI-THREAD-025`).
pub(crate) struct ClipboardPaste {
    pub(crate) blob: ambre_bus::ClipboardBlob,
    pub(crate) token: Option<String>,
}

// -----------------------------------------------------------------------------
// AppView -- the closed per-app view registry
// -----------------------------------------------------------------------------

/// The active document's view. A closed sum over document types: adding an app
/// is one variant here plus its inherent driver methods below. No trait object,
/// no `Box<dyn Any>`, exhaustiveness checked by the compiler, symmetric to
/// `AppCommand`/`DocState`. The driver (`FrameHost`) dispatches over it by
/// `match`; each variant's view exposes the same inherent method set.
pub enum AppView {
    /// A Trame (spreadsheet) document.
    Trame(TrameView),
    /// A Toile (whiteboard) document.
    Toile(ToileView),
    /// A Prose (text) document.
    Prose(ProseView),
}

impl AppView {
    /// Pump inbound state before painting (drain Logic->UI, set dirty flags).
    fn update(&mut self) {
        match self {
            AppView::Trame(v) => v.update(),
            AppView::Toile(v) => v.update(),
            AppView::Prose(v) => v.update(),
        }
    }

    /// Decide what to repaint this frame and clear the dirty flags.
    fn plan_repaint(&mut self, now_ms: f64, hud_tick: bool) -> RepaintPlan {
        match self {
            AppView::Trame(v) => v.plan_repaint(now_ms, hud_tick),
            AppView::Toile(v) => v.plan_repaint(now_ms, hud_tick),
            AppView::Prose(v) => v.plan_repaint(now_ms, hud_tick),
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
            AppView::Prose(v) => v.paint_content(surface, font_cx, text_cache, strip),
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
            AppView::Prose(v) => v.paint_overlay(surface, font_cx, text_cache, stats),
        }
    }

    /// Canvas resized to `width` x `height` physical pixels.
    fn resize(&mut self, width: u32, height: u32) {
        match self {
            AppView::Trame(v) => v.resize(width, height),
            AppView::Toile(v) => v.resize(width, height),
            AppView::Prose(v) => v.resize(width, height),
        }
    }

    /// Handle an input event; returns `true` to `preventDefault()`.
    fn handle_input(&mut self, ev: &InputEvent) -> bool {
        match self {
            AppView::Trame(v) => v.handle_input(ev),
            AppView::Toile(v) => v.handle_input(ev),
            AppView::Prose(v) => v.handle_input(ev),
        }
    }

    /// Resolve a `copy`/`cut` synchronously inside the native clipboard event
    /// ([`CopyOutcome`](ambre_bus::CopyOutcome)). `Text` is for an in-cell/inline
    /// selection the view holds. `Deferred` is for a payload being built on the
    /// Logic thread, drained a frame later by
    /// [`poll_clipboard`](Self::poll_clipboard). `Nothing` lets the browser
    /// default proceed. Each app applies its own cut policy behind this call.
    fn clipboard_write(&mut self, cut: bool) -> ambre_bus::CopyOutcome {
        match self {
            AppView::Trame(v) => v.clipboard_write(cut),
            AppView::Toile(v) => v.clipboard_write(cut),
            AppView::Prose(v) => v.clipboard_write(cut),
        }
    }

    /// Drain any clipboard blob built since the last frame, from an authoritative
    /// `Clipboard` reply. The host can then mirror it to the OS clipboard within
    /// the transient user-activation window. `None` when no copy is pending.
    fn poll_clipboard(&mut self) -> Option<ambre_bus::ClipboardBlob> {
        match self {
            AppView::Trame(v) => v.poll_clipboard(),
            AppView::Toile(v) => v.poll_clipboard(),
            AppView::Prose(v) => v.poll_clipboard(),
        }
    }

    /// Service a `paste` from the shared [`ClipboardBlob`](ambre_bus::ClipboardBlob)
    /// transport; each view decodes its own app-specific payload behind the opaque
    /// bytes. `own_copy` is true when the clipboard still carries this tab's
    /// synchronous ownership token. The view then pastes its in-memory `last_clip`
    /// at full fidelity, rather than trusting the (possibly failed) OS mirror.
    fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, own_copy: bool) {
        match self {
            AppView::Trame(v) => v.paste_clipboard(blob, own_copy),
            AppView::Toile(v) => v.paste_clipboard(blob, own_copy),
            AppView::Prose(v) => v.paste_clipboard(blob, own_copy),
        }
    }

    /// Text dropped at a canvas point. Only Prose takes it.
    fn drop_text(&mut self, x: f32, y: f32, text: String) -> bool {
        match self {
            AppView::Trame(_) | AppView::Toile(_) => false,
            AppView::Prose(v) => v.drop_text(x, y, text),
        }
    }

    /// A finger on the canvas. Every application takes it apart from the
    /// mouse.
    fn touch(&mut self, phase: TouchPhase, x: f32, y: f32, now_ms: f64) -> bool {
        match self {
            AppView::Toile(v) => {
                let (x, y) = (f64::from(x), f64::from(y));
                match phase {
                    TouchPhase::Down => v.touch_down(x, y, now_ms),
                    TouchPhase::Move => v.touch_move(x, y, now_ms),
                    TouchPhase::Up => v.touch_up(x, y, now_ms),
                    TouchPhase::Cancel => v.touch_cancel(),
                }
            }
            AppView::Trame(v) => {
                match phase {
                    TouchPhase::Down => v.touch_down(x, y, now_ms),
                    TouchPhase::Move => v.touch_move(x, y, now_ms),
                    TouchPhase::Up => v.touch_up(now_ms),
                    TouchPhase::Cancel => v.touch_cancel(),
                }
                true
            }
            AppView::Prose(v) => v.touch(phase, x, y, now_ms),
        }
    }

    /// Whether a consumed press captures the pointer, and a finger pans
    /// rather than selects. Every application does.
    fn captures_pointer(&self) -> bool {
        true
    }

    /// Swap the active document in place, keeping the GPU surface.
    fn set_document(&mut self, document_id: DocumentId) {
        match self {
            AppView::Trame(v) => v.set_document(document_id),
            AppView::Toile(v) => v.set_document(document_id),
            AppView::Prose(v) => v.set_document(document_id),
        }
    }

    /// The nested `doc_embed`s this view wants painted, projected to screen space
    /// (`toile-embeds`, "Render"). Only a Toile embeds documents; every
    /// other app returns an empty set, so `FrameHost` owns no children for it.
    fn visible_embeds(&self) -> Vec<VisibleEmbed> {
        match self {
            AppView::Toile(v) => v.visible_embeds(),
            AppView::Trame(_) | AppView::Prose(_) => Vec::new(),
        }
    }

    /// The web pages this view wants framed, projected to screen space
    /// (`TOILE-088`). Only a Toile embeds a page; every other app returns an
    /// empty set, so the shell mounts no frame for it.
    fn visible_web_embeds(&self) -> Vec<VisibleWebEmbed> {
        match self {
            AppView::Toile(v) => v.visible_web_embeds(),
            AppView::Trame(_) | AppView::Prose(_) => Vec::new(),
        }
    }

    /// The embed element the last input event was routed into
    /// (`toile-embeds`, "Input focus"), for `FrameHost` to forward the remapped
    /// event to. The element rather than its document: two embeds of one
    /// document are two child views (`TOILE-UI-097`). `None` off a Toile, or
    /// when the board itself consumed the event.
    fn child_route(&self) -> Option<ElementId> {
        match self {
            AppView::Toile(v) => v.child_route(),
            AppView::Trame(_) | AppView::Prose(_) => None,
        }
    }

    /// Map a physical screen point into the focused embed's child pixel space
    /// (`toile-embeds`). `None` off a Toile or when nothing is focused.
    fn embed_to_child_space(&self, screen: Point) -> Option<Point> {
        match self {
            AppView::Toile(v) => v.embed_to_child_space(screen),
            AppView::Trame(_) | AppView::Prose(_) => None,
        }
    }
}

// -----------------------------------------------------------------------------
// FrameHost -- per-frame driver over an AppView
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
// DocSwitcher -- route-driven document swap handle
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
/// (detached / display:none). The caller then skips the resize.
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

// -- Tests (wasm tier) -------------------------------------------------------
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
