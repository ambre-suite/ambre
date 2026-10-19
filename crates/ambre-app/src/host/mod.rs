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
use ambre_canvas::{
    ContentRepaint, DensityWatch, FATAL_BACKDROP, FrameStats, GpuState, InputEvent, Layer,
    OverlayRepaint, Phases, RafControl, RepaintPlan, SurfaceSize, TextCache, TextInput, Tick,
    WgpuSurface, bootstrap_gpu, drive_frame, fatal_message, paint_fatal, size_backing_store,
    start_raf_loop,
};
use ambre_draw::{DrawSurface, Point, Rect, Size};
use ambre_toile::ui::ToileView;
use ambre_toile::ui::embed::{VisibleEmbed, VisibleWebEmbed};
use ambre_trame::ui::TrameView;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

// FrameHost and `run_with` live in sibling modules; both reach the shared types
// (AppView, DocSwitcher, helpers) via `use super::*`.
mod cell;
mod frame;
mod pipe;
mod probe;

pub(crate) use probe::{PageClock, mark};
mod murmure;
mod prose;
mod run;
mod web_embed;

pub use ambre_murmure::view::{MurmureView, THREAD_CANVAS};
pub(crate) use frame::send_faces;
pub use frame::{FrameHost, MakeChild};
pub use murmure::page_reads;
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
/// signal the toolbar/menu Paste button uses, since it cannot ride a native
/// `paste` event (`ambre-spec-clipboard.md`).
pub(crate) const AMBRE_CLIPBOARD_WEB_TYPE: &str = "web application/x-ambre";

/// How often a hidden page relays, in milliseconds (`CANVAS-075`). The
/// browser spaces a hidden page's timers to a second at least.
const HIDDEN_RELAY_PERIOD_MS: i32 = 1000;

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

/// The window is as wide as a desktop. The stylesheet decides, so a chord
/// shows on the canvas exactly where the shell shows one (`CHROME-057`).
pub(crate) fn desktop() -> bool {
    web_sys::window().is_some_and(|w| desktop_in(&w))
}

/// [`desktop`] for a given window, so a test can ask a frame of its own width.
/// It reads the sheet's `--desktop` token, which holds 1 at the desktop width.
pub(crate) fn desktop_in(window: &web_sys::Window) -> bool {
    let Some(root) = window.document().and_then(|d| d.document_element()) else {
        return false;
    };
    window
        .get_computed_style(&root)
        .ok()
        .flatten()
        .and_then(|style| style.get_property_value("--desktop").ok())
        .is_some_and(|value| value.trim() == "1")
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
        let Some((item, web)) = publish_item(
            &tsv,
            html.as_deref(),
            payload_b64.as_deref(),
            &token,
            clipboard_item,
        ) else {
            return;
        };
        let items = js_sys::Array::of1(&item);
        let written = wasm_bindgen_futures::JsFuture::from(clipboard.write(&items)).await;
        // Only a landed write of the `web ` types moves the token out of the
        // paste event's reach (`UI-THREAD-054`).
        published.set(web && written.is_ok());
    });
}

/// The item the asynchronous write publishes, and whether it carries the `web `
/// types. `build` makes an item from `(type, value)` pairs, or refuses it.
///
/// The first attempt carries the mirror, the HTML, the prefixed private bytes
/// and the prefixed token (`UI-THREAD-006`, `UI-THREAD-023`). Firefox rejects
/// the `web ` types in the item constructor (throwaway Firefox 156 run). The copy
/// event left only the token, so without a second attempt no other application
/// could paste the copy. The second carries the token in an HTML comment where
/// the view wrote HTML (`UI-THREAD-055`, `UI-THREAD-057`).
fn publish_item<T>(
    tsv: &str,
    html: Option<&str>,
    payload_b64: Option<&str>,
    token: &str,
    build: impl Fn(&[(&str, Option<&str>)]) -> Option<T>,
) -> Option<(T, bool)> {
    let text = ("text/plain", Some(tsv));
    let full = [
        text,
        ("text/html", html),
        (AMBRE_CLIPBOARD_WEB_TYPE, payload_b64),
        (AMBRE_CLIPBOARD_WEB_TOKEN_TYPE, Some(token)),
    ];
    if let Some(item) = build(&full) {
        return Some((item, true));
    }
    let marked = html.map(|h| ambre_bus::ClipboardBlob::mark_html(h, token));
    build(&[text, ("text/html", marked.as_deref())]).map(|item| (item, false))
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
#[allow(
    clippy::large_enum_variant,
    reason = "the page holds one view at a time, so a box would save nothing"
)]
pub enum AppView {
    /// A Trame (spreadsheet) document.
    Trame(TrameView),
    /// A Toile (whiteboard) document.
    Toile(ToileView),
    /// A Prose (text) document.
    Prose(ProseView),
    /// A Murmure page: its channels, not one document.
    Murmure(MurmureView),
}

impl AppView {
    /// Pump inbound state before painting (drain Logic->UI, set dirty flags).
    fn update(&mut self) {
        match self {
            AppView::Trame(v) => v.update(),
            AppView::Toile(v) => v.update(),
            AppView::Prose(v) => v.update(),
            AppView::Murmure(v) => v.update(),
        }
    }

    /// Decide what to repaint this frame and clear the dirty flags.
    fn plan_repaint(&mut self, now_ms: f64, hud_tick: bool) -> RepaintPlan {
        match self {
            AppView::Trame(v) => v.plan_repaint(now_ms, hud_tick),
            AppView::Toile(v) => v.plan_repaint(now_ms, hud_tick),
            AppView::Prose(v) => v.plan_repaint(now_ms, hud_tick),
            AppView::Murmure(v) => v.plan_repaint(now_ms, hud_tick),
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
            AppView::Murmure(v) => v.paint_content(surface, font_cx, text_cache, strip),
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
            AppView::Murmure(v) => v.paint_overlay(surface, font_cx, text_cache, stats),
        }
    }

    /// Canvas resized to `width` x `height` logical pixels (`CANVAS-044`).
    fn resize(&mut self, width: u32, height: u32) {
        match self {
            AppView::Trame(v) => v.resize(width, height),
            AppView::Toile(v) => v.resize(width, height),
            AppView::Prose(v) => v.resize(width, height),
            AppView::Murmure(v) => {
                v.resize(width, height);
                v.set_desktop(desktop());
            }
        }
    }

    /// Handle an input event; returns `true` to `preventDefault()`.
    fn handle_input(&mut self, ev: &InputEvent) -> bool {
        match self {
            AppView::Trame(v) => v.handle_input(ev),
            AppView::Toile(v) => v.handle_input(ev),
            AppView::Prose(v) => v.handle_input(ev),
            AppView::Murmure(v) => v.handle_input(ev),
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
            // The list's text is read-only: a cut leaves it where it is, and
            // copies nothing. The composer's draft is cut like any text.
            AppView::Murmure(v) => match v.clipboard(cut) {
                Some(text) => ambre_bus::CopyOutcome::Text(text),
                None => ambre_bus::CopyOutcome::Nothing,
            },
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
            AppView::Murmure(_) => None,
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
            // The composer takes the text mirror alone (the Murmure composer spec).
            AppView::Murmure(v) => v.paste(&blob.text),
        }
    }

    /// Text dropped at a canvas point. Only Prose takes it.
    fn drop_text(&mut self, x: f32, y: f32, text: String) -> bool {
        match self {
            AppView::Trame(_) | AppView::Toile(_) | AppView::Murmure(_) => false,
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
            AppView::Murmure(v) => {
                match phase {
                    TouchPhase::Down => v.touch_down(x, y, now_ms),
                    TouchPhase::Move => {
                        v.touch_move(x, y, now_ms);
                    }
                    TouchPhase::Up => {
                        v.touch_up(now_ms);
                    }
                    TouchPhase::Cancel => v.touch_cancel(),
                }
                true
            }
        }
    }

    /// Whether a consumed press captures the pointer, and a finger pans
    /// rather than selects. Every application does.
    fn captures_pointer(&self) -> bool {
        true
    }

    /// Whether a press began a drag across read-only text. The view leaves
    /// the press to the browser, yet the host captures the pointer, so the
    /// drag keeps its moves past the canvas edge.
    fn drags_text(&self) -> bool {
        match self {
            AppView::Murmure(v) => v.drags_text(),
            AppView::Trame(_) | AppView::Toile(_) | AppView::Prose(_) => false,
        }
    }

    /// Whether the page's text pipe writes into this view now. Only a Murmure
    /// region shares the pipe with another canvas.
    fn takes_text(&self) -> bool {
        match self {
            AppView::Murmure(v) => v.composer().focused(),
            AppView::Trame(_) | AppView::Toile(_) | AppView::Prose(_) => false,
        }
    }

    /// Another canvas of the page took the text pipe.
    fn leave_text(&mut self) {
        match self {
            AppView::Murmure(v) => v.leave_composer(),
            AppView::Trame(_) | AppView::Toile(_) | AppView::Prose(_) => {}
        }
    }

    /// Swap the active document in place, keeping the GPU surface.
    fn set_document(&mut self, document_id: DocumentId) {
        match self {
            AppView::Trame(v) => v.set_document(document_id),
            AppView::Toile(v) => v.set_document(document_id),
            AppView::Prose(v) => v.set_document(document_id),
            AppView::Murmure(v) => v.set_document(document_id),
        }
    }

    /// The nested `doc_embed`s this view wants painted, projected to screen space
    /// (`toile-embeds`, "Render"). Only a Toile embeds documents; every
    /// other app returns an empty set, so `FrameHost` owns no children for it.
    fn visible_embeds(&self) -> Vec<VisibleEmbed> {
        match self {
            AppView::Toile(v) => v.visible_embeds(),
            AppView::Trame(_) | AppView::Prose(_) | AppView::Murmure(_) => Vec::new(),
        }
    }

    /// The web pages this view wants framed, projected to screen space
    /// (`TOILE-088`). Only a Toile embeds a page; every other app returns an
    /// empty set, so the shell mounts no frame for it.
    fn visible_web_embeds(&self) -> Vec<VisibleWebEmbed> {
        match self {
            AppView::Toile(v) => v.visible_web_embeds(),
            AppView::Trame(_) | AppView::Prose(_) | AppView::Murmure(_) => Vec::new(),
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
            AppView::Trame(_) | AppView::Prose(_) | AppView::Murmure(_) => None,
        }
    }

    /// The ids of the canvases this view shows beside the content canvas, each
    /// in a cell of its own (`CANVAS-070`). Only Murmure shows one: its thread.
    fn cell_ids(&self) -> &'static [&'static str] {
        match self {
            AppView::Murmure(_) => &[THREAD_CANVAS],
            AppView::Trame(_) | AppView::Toile(_) | AppView::Prose(_) => &[],
        }
    }

    /// The view of the cell named `id`, at `width` x `height` logical pixels.
    /// `None` for an id this view does not show.
    fn cell_view(&self, id: &str, width: f64, height: f64) -> Option<AppView> {
        match self {
            AppView::Murmure(v) if id == THREAD_CANVAS => {
                Some(AppView::Murmure(v.region(width, height)))
            }
            _ => None,
        }
    }

    /// Map a logical screen point into the focused embed's child pixel space
    /// (`toile-embeds`). `None` off a Toile or when nothing is focused.
    fn embed_to_child_space(&self, screen: Point) -> Option<Point> {
        match self {
            AppView::Toile(v) => v.embed_to_child_space(screen),
            AppView::Trame(_) | AppView::Prose(_) | AppView::Murmure(_) => None,
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
///
/// Any element inside an open `<dialog>` counts too. A modal makes the page
/// inert to the pointer, but a document listener still hears its keys. An
/// arrow key on a dialog button would otherwise move the selection behind it.
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
                || el.closest("dialog[open]").ok().flatten().is_some()
        }
        None => false,
    }
}

// -- Tests (wasm tier) -------------------------------------------------------
// Pure helpers only (base64 transport codec + the FPS sliding window). The crate
// is `#![cfg(target_arch = "wasm32")]`, so these run under the wasm target via
// the `wasm-bindgen-test-runner` (node, no DOM). See ambre-spec-testing.md.
#[cfg(test)]
mod tests {
    use super::{
        AMBRE_CLIPBOARD_WEB_TOKEN_TYPE, AMBRE_CLIPBOARD_WEB_TYPE, FrameHost, base64_decode,
        base64_encode, clipboard_item, publish_item,
    };
    use std::collections::VecDeque;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// `CANVAS-011`: the loop keeps ticking while the frame draws nothing, so an
    /// event queued meanwhile is drained on the next frame. It starts only once
    /// the GPU bootstrap hands over its token (`CANVAS-004`).
    #[wasm_bindgen_test]
    async fn the_loop_ticks_while_the_content_is_static() {
        use std::cell::Cell;
        use std::rc::Rc;
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a page");
        let canvas: web_sys::HtmlCanvasElement =
            wasm_bindgen::JsCast::dyn_into(document.create_element("canvas").expect("a canvas"))
                .expect("a canvas element");
        let (settled, _gpu) =
            ambre_canvas::bootstrap_gpu(canvas, ambre_canvas::SurfaceSize::UNSIZED).await;
        let ticks = Rc::new(Cell::new(0u32));
        let seen = Rc::clone(&ticks);
        // A frame with nothing to draw: it only counts.
        ambre_canvas::start_raf_loop(move |_| seen.set(seen.get() + 1), settled);
        let wait = js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window()
                .expect("a window")
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 300);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(wait).await;
        assert!(ticks.get() >= 3, "{} ticks in 300 ms", ticks.get());
    }

    /// A key aimed at a control inside an open dialog stays with the dialog.
    /// So an arrow key on a sort dialog button cannot move the selection
    /// behind it. The same button outside a dialog, and the pipe, still reach
    /// the view.
    #[wasm_bindgen_test]
    fn a_key_inside_an_open_dialog_stays_there() {
        use wasm_bindgen::JsCast;
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a page");
        let body = document.body().expect("a body");
        let make = |tag: &str| document.create_element(tag).expect("an element");
        let pipe: web_sys::HtmlTextAreaElement = make("textarea").dyn_into().expect("a textarea");
        let dialog: web_sys::HtmlDialogElement = make("dialog").dyn_into().expect("a dialog");
        let inside = make("button");
        let outside = make("button");
        dialog.append_child(&inside).expect("append");
        for node in [pipe.as_ref(), dialog.as_ref(), outside.as_ref()] {
            body.append_child(node).expect("append");
        }
        assert!(!super::is_other_editable(&inside, &pipe), "closed");
        dialog.show_modal().expect("a modal");
        assert!(super::is_other_editable(&inside, &pipe), "open");
        assert!(!super::is_other_editable(&outside, &pipe));
        assert!(!super::is_other_editable(&pipe, &pipe));
        dialog.close();
        for node in [pipe.as_ref(), dialog.as_ref(), outside.as_ref()] {
            body.remove_child(node).expect("remove");
        }
    }

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

    /// `CANVAS-035` and `CANVAS-045`: the watch queries the density it starts
    /// on, so it fires when that density is left. Each change runs the resize
    /// once and leaves a fresh query armed, and the spent one stays silent.
    #[wasm_bindgen_test]
    fn a_density_change_resizes_and_rearms_the_watch() {
        use ambre_canvas::DensityWatch;
        use std::cell::Cell;
        use std::rc::Rc;
        let resized = Rc::new(Cell::new(0));
        let count = Rc::clone(&resized);
        let watch = DensityWatch::arm(Rc::new(move || count.set(count.get() + 1)));
        let change = || web_sys::Event::new("change").expect("an event");

        let first = watch.armed().expect("a watch is armed");
        assert!(first.media().contains("dppx"), "{}", first.media());
        assert!(first.matches(), "the query holds the current density");
        first.dispatch_event(&change()).expect("dispatched");
        assert_eq!(resized.get(), 1, "one change, one resize");

        let second = watch.armed().expect("re-armed");
        assert!(second != first, "a fresh query");
        first.dispatch_event(&change()).expect("dispatched");
        assert_eq!(resized.get(), 1, "the spent query stays silent");
        second.dispatch_event(&change()).expect("dispatched");
        assert_eq!(resized.get(), 2, "the fresh query fires in turn");
    }

    /// Read an item back the way the Paste button does.
    async fn read_back(
        item: &web_sys::ClipboardItem,
    ) -> (ambre_bus::ClipboardBlob, Option<String>) {
        let items: wasm_bindgen::JsValue = js_sys::Array::of1(item).into();
        let read = js_sys::Promise::resolve(&items);
        crate::platform::read_clipboard_blob(read)
            .await
            .expect("the item reads back")
    }

    fn types(item: &web_sys::ClipboardItem) -> Vec<String> {
        let mut types: Vec<String> = item.types().iter().filter_map(|t| t.as_string()).collect();
        types.sort();
        types
    }

    /// `UI-THREAD-006` and `UI-THREAD-023`: where the browser takes the `web `
    /// types, the asynchronous item carries the mirror, the HTML, the private
    /// bytes and the token. Read back, each holds what the copy published.
    #[wasm_bindgen_test]
    async fn the_asynchronous_item_carries_everything_the_copy_published() {
        let bytes = [1u8, 2, 3];
        let (item, web) = publish_item(
            "a\tb",
            Some("<p>a</p>"),
            Some(&base64_encode(&bytes)),
            "ambre-1",
            clipboard_item,
        )
        .expect("an item");
        assert!(web, "Chrome takes the `web ` types");
        assert_eq!(
            types(&item),
            vec![
                "text/html",
                "text/plain",
                AMBRE_CLIPBOARD_WEB_TYPE,
                AMBRE_CLIPBOARD_WEB_TOKEN_TYPE,
            ]
        );
        let (blob, token) = read_back(&item).await;
        assert_eq!(blob.text, "a\tb");
        assert_eq!(blob.html.as_deref(), Some("<p>a</p>"));
        assert_eq!(blob.ambre.as_deref(), Some(&bytes[..]));
        assert_eq!(token.as_deref(), Some("ambre-1"));
    }

    /// `UI-THREAD-055`: where the browser refuses the `web ` types, the second
    /// attempt still publishes the mirror and the HTML, the token in the HTML.
    #[wasm_bindgen_test]
    async fn a_browser_refusing_the_web_types_still_gets_the_mirror_and_the_html() {
        let refusing = |entries: &[(&str, Option<&str>)]| {
            let web = entries
                .iter()
                .any(|(kind, value)| value.is_some() && kind.starts_with("web "));
            if web { None } else { clipboard_item(entries) }
        };
        let (item, web) = publish_item("a", Some("<p>a</p>"), Some("AQID"), "ambre-2", refusing)
            .expect("the second attempt");
        assert!(!web);
        assert_eq!(types(&item), vec!["text/html", "text/plain"]);
        let (blob, token) = read_back(&item).await;
        assert_eq!(blob.text, "a");
        assert_eq!(blob.ambre, None);
        assert_eq!(
            token.as_deref(),
            Some("ambre-2"),
            "the token rides in the HTML"
        );
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
