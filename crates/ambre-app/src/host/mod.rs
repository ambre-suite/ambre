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

use ambre_bus::{DocType, PasteSource};
use ambre_canvas::{
    ContentRepaint, DensityWatch, FATAL_BACKDROP, FrameStats, GpuState, InputEvent, Layer,
    LayerRepaint, Phases, RafControl, RepaintPlan, SurfaceSize, TextCache, TextInput, Tick,
    WgpuSurface, bootstrap_gpu, drive_frame, fatal_message, paint_fatal, size_backing_store,
    start_raf_loop,
};
use ambre_chrome::commands::ClipboardWrite;
use ambre_core::id::{Document, Element, Id};
use ambre_core::{CanvasCss, Canvases, ClipboardBytes, Device, Payload, Px, StaticKey, View};
use ambre_draw::{Consumed, DrawSurface, Point, Rect, Size};
use ambre_logic::app::LogicCommand;
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
pub use murmure::{DraftKeeper, page_reads};
pub use prose::{ProseSignals, ProseView, ProseWiring};

/// Where a finger is in its gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    /// The browser took the gesture over (`KINETIC-010`).
    Cancel,
}
use ambre_bus::{Ms, TimeSpan};
use ambre_core::{Instant, PageLoad};
pub use run::run_with;

/// Open a web address in a fresh tab with `noopener,noreferrer`. The page can
/// neither reach back into this window nor learn where it came from. A
/// followed Toile link and a clicked Murmure link both open through it.
pub(crate) fn open_address(url: &ambre_core::LinkUrl) {
    if let Some(window) = web_sys::window() {
        let _ = window.open_with_url_and_target_and_features(
            url.as_str(),
            "_blank",
            "noopener,noreferrer",
        );
    }
}

/// The clipboard types Ambre reads and writes, one variant each
/// (`ambre-spec-clipboard.md`). The browser takes a type as its MIME name,
/// which [`ClipboardFormat::mime`] spells at the DOM call alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ClipboardFormat {
    /// The `text/plain` mirror every program reads.
    PlainText,
    /// The `text/html` a view wrote for another editor.
    Html,
    /// Private type carrying the base64 postcard
    /// [`ClipboardPayload`](ambre_trame::message::ClipboardPayload) for an internal
    /// round-trip on the *synchronous* `copy`/`paste` events (Ctrl+C/Ctrl+V).
    /// Browsers that reject it fall back to `text/plain`.
    Ambre,
    /// Synchronous same-tab ownership marker (`UI-THREAD-007`). Written into
    /// the clipboard event's `DataTransfer` *synchronously* during
    /// `copy`/`cut`, it always reaches the OS clipboard. The async write that
    /// follows replaces it. Chrome keeps it under the `web ` variant, which a
    /// `paste` event cannot see (`UI-THREAD-054`). Firefox refuses that
    /// variant and the write carries the standard types alone
    /// (`UI-THREAD-055`).
    ///
    /// On `paste`, a value here equal to this page's token proves the
    /// clipboard still holds THIS tab's own last copy. The view then pastes
    /// its in-memory `last_clip` at full fidelity. Any foreign copy (another
    /// app / tab) overwrites the clipboard and drops the marker, so its
    /// absence correctly means "not ours".
    AmbreToken,
    /// [`Self::Ambre`] under the `web ` prefix the *asynchronous* Clipboard
    /// API (`navigator.clipboard.read`/`write`) requires for a custom type.
    /// This is the signal the toolbar/menu Paste button uses, since it cannot
    /// ride a native `paste` event.
    WebAmbre,
    /// [`Self::AmbreToken`] under the `web ` prefix. The async publish writes
    /// it beside the payload so the whole-clipboard `write` never removes the
    /// token the copy event established (`UI-THREAD-023`). The Paste button
    /// reads it back to answer rung 2 of the fidelity ladder.
    WebAmbreToken,
}

impl ClipboardFormat {
    /// The MIME name, for the call into the clipboard API.
    pub(crate) const fn mime(self) -> &'static str {
        match self {
            Self::PlainText => "text/plain",
            Self::Html => "text/html",
            Self::Ambre => "application/x-ambre",
            Self::AmbreToken => "application/x-ambre-token",
            Self::WebAmbre => "web application/x-ambre",
            Self::WebAmbreToken => "web application/x-ambre-token",
        }
    }
}

/// How often a hidden page relays, in milliseconds (`CANVAS-075`). The
/// browser spaces a hidden page's timers to a second at least.
const HIDDEN_RELAY_PERIOD: TimeSpan<Ms> = TimeSpan::<Ms>::new(1000);

/// Base64-encode bytes for transport as a clipboard string (UTF-8 safe).
/// Where a mouse, pointer or drag event landed on its canvas. The DOM
/// reports the offset in whole CSS px, which a view lays out in
/// (`CANVAS-032`).
pub(crate) fn offset_of(event: &web_sys::MouseEvent) -> ambre_core::Point<Px<View>> {
    ambre_core::Point::new(
        Px::<View>::new(event.offset_x() as f32),
        Px::<View>::new(event.offset_y() as f32),
    )
}

pub(crate) fn base64_encode(bytes: &Payload<ClipboardBytes>) -> String {
    BASE64.encode(bytes.as_bytes())
}

/// Decode a base64 clipboard string back to bytes; `None` on malformed input.
pub(crate) fn base64_decode(s: &str) -> Option<Payload<ClipboardBytes>> {
    BASE64.decode(s).ok().map(Payload::<ClipboardBytes>::new)
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
    blob: ambre_bus::ClipboardBlob,
    token: ambre_bus::ClipToken,
    published: Rc<std::cell::Cell<bool>>,
) {
    let Some(clipboard) = web_sys::window().map(|w| w.navigator().clipboard()) else {
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        let Some(Published { item, web }) = publish_item(&blob, &token, clipboard_item) else {
            return;
        };
        let items = js_sys::Array::of1(&item);
        let written = wasm_bindgen_futures::JsFuture::from(clipboard.write(&items)).await;
        // Only a landed write of the `web ` types moves the token out of the
        // paste event's reach (`UI-THREAD-054`).
        published.set(web == WebTypes::Carried && written.is_ok());
    });
}

/// Whether a published clipboard item carries the `web ` types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebTypes {
    /// The browser refused them: the item holds the text and the marked HTML.
    Dropped,
    /// The item holds the private bytes and the token beside the text.
    Carried,
}

/// The item an asynchronous clipboard write publishes.
#[derive(Debug)]
struct Published<T> {
    item: T,
    web: WebTypes,
}

/// The item the asynchronous write publishes, and whether it carries the `web `
/// types. `build` makes an item from `(type, value)` pairs, or refuses it. The
/// private bytes travel as base64, a clipboard string.
///
/// The first attempt carries the mirror, the HTML, the prefixed private bytes
/// and the prefixed token (`UI-THREAD-006`, `UI-THREAD-023`). Firefox rejects
/// the `web ` types in the item constructor (throwaway Firefox 156 run). The copy
/// event left only the token, so without a second attempt no other application
/// could paste the copy. The second carries the token in an HTML comment where
/// the view wrote HTML (`UI-THREAD-055`, `UI-THREAD-057`).
fn publish_item<T>(
    blob: &ambre_bus::ClipboardBlob,
    token: &ambre_bus::ClipToken,
    build: impl Fn(&[(ClipboardFormat, Option<&str>)]) -> Option<T>,
) -> Option<Published<T>> {
    let html = blob.html.as_deref();
    let payload = blob.ambre.as_ref().map(base64_encode);
    let text = (ClipboardFormat::PlainText, Some(blob.text.as_str()));
    let full = [
        text,
        (ClipboardFormat::Html, html),
        (ClipboardFormat::WebAmbre, payload.as_deref()),
        (ClipboardFormat::WebAmbreToken, Some(token.as_str())),
    ];
    if let Some(item) = build(&full) {
        return Some(Published {
            item,
            web: WebTypes::Carried,
        });
    }
    let marked = html.map(|h| ambre_bus::ClipboardBlob::mark_html(h, token));
    build(&[text, (ClipboardFormat::Html, marked.as_deref())]).map(|item| Published {
        item,
        web: WebTypes::Dropped,
    })
}

/// Build a ClipboardItem from `(type, value)` pairs, skipping absent values.
/// `None` when the browser refuses one of the types.
fn clipboard_item(entries: &[(ClipboardFormat, Option<&str>)]) -> Option<web_sys::ClipboardItem> {
    let record = js_sys::Object::new();
    for (kind, value) in entries {
        if let Some(value) = value {
            let promise = js_sys::Promise::resolve(&JsValue::from_str(value));
            js_sys::Reflect::set(&record, &JsValue::from_str(kind.mime()), &promise).ok()?;
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
    pub(crate) token: Option<ambre_bus::ClipToken>,
}

// -----------------------------------------------------------------------------
// AppView -- the closed per-app view registry
// -----------------------------------------------------------------------------

/// The active document's view. A closed sum over document types: adding an app
/// is one variant here plus its inherent driver methods below. No trait object,
/// no `Box<dyn Any>`, exhaustiveness checked by the compiler, symmetric to
/// `AppCommand`/`DocState`. The driver (`FrameHost`) dispatches over it by
/// `match`; each variant's view exposes the same inherent method set.
#[expect(
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
    fn plan_repaint(
        &mut self,
        now: Instant<PageLoad>,
        hud_tick: ambre_canvas::HudTick,
    ) -> RepaintPlan {
        match self {
            AppView::Trame(v) => v.plan_repaint(now, hud_tick),
            AppView::Toile(v) => v.plan_repaint(now, hud_tick),
            AppView::Prose(v) => v.plan_repaint(now, hud_tick),
            AppView::Murmure(v) => v.plan_repaint(now, hud_tick),
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

    /// Canvas resized to `size` logical pixels (`CANVAS-044`).
    fn resize(&mut self, size: ambre_core::Size<Px<CanvasCss>>) {
        match self {
            AppView::Trame(v) => v.resize(size),
            AppView::Toile(v) => v.resize(size),
            AppView::Prose(v) => v.resize(size),
            AppView::Murmure(v) => {
                v.resize(size);
                v.set_width(ambre_murmure::view::WindowWidth::desktop_if(desktop()));
            }
        }
    }

    /// Handle an input event; `Consumed::Yes` asks for `preventDefault()`.
    fn handle_input(&mut self, ev: &InputEvent) -> Consumed {
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
    fn clipboard_write(&mut self, write: ClipboardWrite) -> ambre_bus::CopyOutcome {
        match self {
            AppView::Trame(v) => v.clipboard_write(write),
            AppView::Toile(v) => v.clipboard_write(write),
            AppView::Prose(v) => v.clipboard_write(write),
            // The list's text is read-only: a cut leaves it where it is, and
            // copies nothing. The composer's draft is cut like any text.
            AppView::Murmure(v) => match v.clipboard(write) {
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
    /// bytes. `source` is `Own` when the clipboard still carries this tab's
    /// synchronous ownership token. The view then pastes its in-memory `last_clip`
    /// at full fidelity, rather than trusting the (possibly failed) OS mirror.
    fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, source: PasteSource) {
        match self {
            AppView::Trame(v) => v.paste_clipboard(blob, source),
            AppView::Toile(v) => v.paste_clipboard(blob, source),
            AppView::Prose(v) => v.paste_clipboard(blob, source),
            // The composer takes the text mirror alone (the Murmure composer spec).
            AppView::Murmure(v) => v.paste(&blob.text),
        }
    }

    /// Text dropped at a canvas point. Only Prose takes it.
    fn drop_text(&mut self, at: ambre_core::Point<Px<View>>, text: String) -> Consumed {
        match self {
            AppView::Trame(_) | AppView::Toile(_) | AppView::Murmure(_) => Consumed::No,
            AppView::Prose(v) => v.drop_text(at, text),
        }
    }

    /// A finger on the canvas. Every application takes it apart from the
    /// mouse.
    fn touch(
        &mut self,
        phase: TouchPhase,
        at: ambre_core::Point<Px<View>>,
        now: Instant<PageLoad>,
    ) -> Consumed {
        match self {
            AppView::Toile(v) => match phase {
                TouchPhase::Down => v.touch_down(at, now),
                TouchPhase::Move => v.touch_move(at, now),
                TouchPhase::Up => v.touch_up(at, now),
                TouchPhase::Cancel => v.touch_cancel(),
            },
            AppView::Trame(v) => {
                match phase {
                    TouchPhase::Down => v.touch_down(at, now),
                    TouchPhase::Move => v.touch_move(at, now),
                    TouchPhase::Up => v.touch_up(now),
                    TouchPhase::Cancel => v.touch_cancel(),
                }
                Consumed::Yes
            }
            AppView::Prose(v) => v.touch(phase, at, now),
            AppView::Murmure(v) => {
                match phase {
                    TouchPhase::Down => v.touch_down(at, now),
                    TouchPhase::Move => {
                        v.touch_move(at, now);
                    }
                    TouchPhase::Up => {
                        v.touch_up(now);
                    }
                    TouchPhase::Cancel => v.touch_cancel(),
                }
                Consumed::Yes
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
    fn set_document(&mut self, document_id: Id<Document>) {
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
    fn child_route(&self) -> Option<Id<Element>> {
        match self {
            AppView::Toile(v) => v.child_route(),
            AppView::Trame(_) | AppView::Prose(_) | AppView::Murmure(_) => None,
        }
    }

    /// The ids of the canvases this view shows beside the content canvas, each
    /// in a cell of its own (`CANVAS-070`). Only Murmure shows one: its thread.
    fn cell_ids(&self) -> &'static [StaticKey<Canvases>] {
        match self {
            AppView::Murmure(_) => &[THREAD_CANVAS],
            AppView::Trame(_) | AppView::Toile(_) | AppView::Prose(_) => &[],
        }
    }

    /// The view of the cell named `id`, at `size` logical pixels. `None` for
    /// an id this view does not show.
    fn cell_view(
        &self,
        id: StaticKey<Canvases>,
        size: ambre_core::Size<Px<CanvasCss>>,
    ) -> Option<AppView> {
        match self {
            AppView::Murmure(v) if id == THREAD_CANVAS => Some(AppView::Murmure(v.region(size))),
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
    set: Rc<dyn Fn(Id<Document>)>,
}

impl DocSwitcher {
    /// Point the canvas at `document_id`. A no-op if it is already current.
    pub fn set_document(&self, document_id: Id<Document>) {
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
        ClipboardFormat, FrameHost, Published, WebTypes, base64_decode, base64_encode,
        clipboard_item, publish_item,
    };
    use ambre_core::{ClipboardBytes, Instant, PageLoad, Payload};
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
            let bytes = Payload::<ClipboardBytes>::new(bytes);
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
        let mut fps = ambre_core::FrameRate::new(-1.0);
        FrameHost::tick_fps(&mut times, &mut fps, Instant::<PageLoad>::new(0.0));
        // A single sample cannot establish a rate.
        assert_eq!(fps, ambre_core::FrameRate::ZERO);
    }

    #[wasm_bindgen_test]
    fn tick_fps_computes_rate_over_window() {
        let mut times = VecDeque::new();
        let mut fps = ambre_core::FrameRate::ZERO;
        // 60 frames spaced 1/60 s over (just under) a second -> ~60 FPS.
        for i in 0..60 {
            FrameHost::tick_fps(
                &mut times,
                &mut fps,
                Instant::<PageLoad>::new(i as f64 * 1000.0 / 60.0),
            );
        }
        let fps = fps.get();
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
    ) -> (ambre_bus::ClipboardBlob, Option<ambre_bus::ClipToken>) {
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
        let blob = ambre_bus::ClipboardBlob {
            text: "a\tb".to_owned(),
            html: Some("<p>a</p>".to_owned()),
            ambre: Some(ambre_core::Payload::<ambre_core::ClipboardBytes>::new(
                bytes.to_vec(),
            )),
            ..ambre_bus::ClipboardBlob::default()
        };
        let Published { item, web } = publish_item(
            &blob,
            &ambre_bus::ClipToken::new("ambre-1".to_owned()).unwrap(),
            clipboard_item,
        )
        .expect("an item");
        assert_eq!(web, WebTypes::Carried, "Chrome takes the `web ` types");
        assert_eq!(
            types(&item),
            vec![
                ClipboardFormat::Html.mime(),
                ClipboardFormat::PlainText.mime(),
                ClipboardFormat::WebAmbre.mime(),
                ClipboardFormat::WebAmbreToken.mime(),
            ]
        );
        let (blob, token) = read_back(&item).await;
        assert_eq!(blob.text, "a\tb");
        assert_eq!(blob.html.as_deref(), Some("<p>a</p>"));
        assert_eq!(
            blob.ambre
                .as_ref()
                .map(ambre_core::Payload::<ambre_core::ClipboardBytes>::as_bytes),
            Some(&bytes[..])
        );
        assert_eq!(
            token.as_ref().map(ambre_bus::ClipToken::as_str),
            Some("ambre-1")
        );
    }

    /// `UI-THREAD-055`: where the browser refuses the `web ` types, the second
    /// attempt still publishes the mirror and the HTML, the token in the HTML.
    #[wasm_bindgen_test]
    async fn a_browser_refusing_the_web_types_still_gets_the_mirror_and_the_html() {
        let refusing = |entries: &[(ClipboardFormat, Option<&str>)]| {
            let web = entries
                .iter()
                .any(|(kind, value)| value.is_some() && kind.mime().starts_with("web "));
            if web { None } else { clipboard_item(entries) }
        };
        let blob = ambre_bus::ClipboardBlob {
            text: "a".to_owned(),
            html: Some("<p>a</p>".to_owned()),
            ambre: Some(ambre_core::Payload::<ambre_core::ClipboardBytes>::new(
                vec![1, 2, 3],
            )),
            ..ambre_bus::ClipboardBlob::default()
        };
        let Published { item, web } = publish_item(
            &blob,
            &ambre_bus::ClipToken::new("ambre-2".to_owned()).unwrap(),
            refusing,
        )
        .expect("the second attempt");
        assert_eq!(web, WebTypes::Dropped);
        assert_eq!(
            types(&item),
            vec![
                ClipboardFormat::Html.mime(),
                ClipboardFormat::PlainText.mime()
            ]
        );
        let (blob, token) = read_back(&item).await;
        assert_eq!(blob.text, "a");
        assert_eq!(blob.ambre, None);
        assert_eq!(
            token.as_ref().map(ambre_bus::ClipToken::as_str),
            Some("ambre-2"),
            "the token rides in the HTML"
        );
    }

    #[wasm_bindgen_test]
    fn tick_fps_evicts_samples_older_than_one_second() {
        let mut times = VecDeque::new();
        let mut fps = ambre_core::FrameRate::ZERO;
        FrameHost::tick_fps(&mut times, &mut fps, Instant::<PageLoad>::new(0.0));
        FrameHost::tick_fps(&mut times, &mut fps, Instant::<PageLoad>::new(5000.0)); // 5 s later
        // The 0.0 sample is > 1 s old and must be evicted, leaving one sample.
        assert_eq!(times.len(), 1);
        assert_eq!(fps, ambre_core::FrameRate::ZERO);
    }
}
