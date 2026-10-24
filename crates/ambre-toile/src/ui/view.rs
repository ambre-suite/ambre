// ambre-toile/src/ui/view.rs
// ============================================================================
// ToileView: the Toile app's `AppView` variant. It owns the local view state
// (camera, selection, active tool, last DisplayViewport) and the UI<->Logic
// queues. It exposes the inherent driver methods the `FrameHost` dispatches
// every frame (`ambre-app/src/host`). Those methods are update, plan_repaint,
// paint_content, paint_overlay, resize, handle_input, clipboard and
// set_document.
//
// Mirrors `ambre_trame::ui::TrameView` but with a true world<->screen camera
// (`ViewportTransform`) instead of a virtual-scroll grid. Holds NO document data
// -- every mutation leaves as a `ToileCommand`; the board lives in `LoroDoc` on
// the Logic thread (`toile-board-rendering`).
// ============================================================================

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use ambre_bus::app::toile::CommentAnchor;
use ambre_bus::app::toile::{
    ClipboardPayload as ToileClipboardPayload, ConnectorId, CorrelationToken, DisplayConnector,
    DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, ElementId, ElementKind, End,
    LinkTarget, PresencePayload, PresentMode, StatsRequestKind, ThreadId, ToileCommand, ToileError,
    ToileEvent, Viewport,
};

/// What the user is told when an edit is lost with its target
/// (`TOILE-UI-387`, `TOILE-UI-388`, `TOILE-UI-389`).
const EDIT_LOST: &str = "Someone deleted what you were editing. Your edit was not saved.";
/// What the user is told when a frame entry finds no frame to act on
/// (`TOILE-UI-153`).
const NO_FRAME_CONTAINS: &str = "No frame contains the selection, so nothing was added.";
const NO_FRAME_HOLDS: &str = "The selection is in no frame, so nothing was removed.";
/// What the user is told when a fit to the selection has nothing to fit
/// (`TOILE-UI-160`).
const NOTHING_TO_ZOOM: &str = "Nothing is selected, so there is nothing to zoom to.";
use ambre_bus::{
    AppCommand, AppEvent, DocumentId, LogicCommand, LossyOutcome, Outbox, SendOutcome, UiEvent,
};
use ambre_canvas::{
    BusyPublisher, ContentRepaint, FrameStats, InputEvent, NoticePublisher, OverlayRepaint,
    PerfPublisher, PerfSnapshot, PointerButton, RepaintPlan, TextCache, TextInput, Theme,
    ThemeControl, WHEEL_LINE_PX, WheelDelta,
};
use ambre_core::{CssPx, CssVector};
use ambre_draw::kinetic::{Release, ScrollStep};
use ambre_draw::{FontContext, Point, Rect, Vec2};

use crate::logic::animate::AnimatedElement;
use crate::logic::text_merge::merge_text;

use super::arrow;
#[cfg(target_arch = "wasm32")]
use super::comments::CommentIntent;
use super::comments::{self, CommentDraft};
#[cfg(target_arch = "wasm32")]
use super::deck::{self, DeckIntent};
use super::edit::{EditIntent, EditTarget, InlineEditor};
use super::embed;
use super::hit::hit_topmost;
use super::kinetic::FingerRole;
use super::label;
use super::link::{self, LinkAction};
use super::presence::{PeerCursors, PresenceSender};
use super::present::Transition;
use super::reactions;
use super::render;
use super::tool::{
    GestureCursor, Preview, Tool, ToolMachine, ToolOutcome, is_group, resolve_group_target,
    world_rect_to_screen,
};
use super::viewport::ViewportTransform;

// Context-menu bridge (`toile-menus`): the DOM `ContextMenu` reads the
// published selection scope and writes back the chosen `CommandId`. Both ends
// are leptos signals, so the handles and their access traits are wasm-only.
use ambre_bus::PasteSource;
use ambre_commands::{ClipboardWrite, CommandId, Modifiers};
#[cfg(target_arch = "wasm32")]
use leptos::prelude::{GetUntracked, RwSignal, Set};

/// Hand one child-session control command to the Logic thread, and report
/// whether it left (`TOILE-UI-338`, `TOILE-UI-339`).
///
/// A free function rather than a method: the reconciliation borrows the session
/// registry mutably, so the sender it calls cannot also borrow the whole view.
/// The answer is what makes the reconciliation self-healing -- the registry
/// records no refused command, so a later frame offers it again.
///
/// `fatal` reports success on a send nobody made. The view is tearing down and
/// no later frame will retry. A `false` here would keep a dead registry asking
/// for ever instead of letting it go.
fn send_session(
    to_logic: &ambre_bus::conduit::Sender<LogicCommand>,
    fatal: bool,
    cmd: LogicCommand,
) -> bool {
    if fatal {
        return true;
    }
    match to_logic.try_send(cmd) {
        Ok(()) => true,
        Err(ambre_bus::conduit::TrySendError::Full(_)) => {
            // Neither an open nor a close is worth blocking the UI thread for.
            // Both are worth a warning, and both come back on a later frame.
            tracing::warn!("Toile: UI->Logic channel full, embed session command deferred");
            false
        }
        Err(ambre_bus::conduit::TrySendError::Disconnected(_)) => {
            tracing::error!("Toile: UI->Logic channel disconnected");
            false
        }
    }
}

/// The zoom rate per pixel of wheel `dy`, a wheel notch and a pinch alike: a
/// step zooms by the factor `exp(-dy * ZOOM_PER_WHEEL_PX)`.
const ZOOM_PER_WHEEL_PX: f64 = 0.0018;

/// A wheel `|dy|` at or above this (logical px, a pixel-mode delta) reads as
/// a discrete mouse notch rather than a fine trackpad glide. This is the auto
/// heuristic's mouse threshold (`toile-tools`).
const WHEEL_MOUSE_NOTCH_PX: CssPx = CssPx::new(40.0);

/// A point of the canvas, as the shared kinetic model reads it.
fn view_point(screen: Point) -> ambre_core::ViewPoint {
    ambre_core::ViewPoint::new(
        ambre_core::ViewPx::new(screen.x as f32),
        ambre_core::ViewPx::new(screen.y as f32),
    )
}

/// A point of the open editor's layout, from the `f64` the camera mapped it to.
fn doc_point(local: Point) -> ambre_core::DocPoint {
    ambre_core::DocPoint::new(
        ambre_core::DocPx::new(local.x as f32),
        ambre_core::DocPx::new(local.y as f32),
    )
}

/// The screen point a pointer event names, in the `f64` the camera maps.
fn on_screen(at: ambre_core::ViewPoint) -> Point {
    Point::new(f64::from(at.x.get()), f64::from(at.y.get()))
}

/// The device a wheel event reads as (`toile-viewport`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WheelDevice {
    /// A notched mouse wheel: the wheel zooms.
    Mouse,
    /// A trackpad: two fingers pan.
    Trackpad,
}

/// Pure device classifier for one wheel event in `Auto` mode
/// (`toile-viewport`).
/// `None` = signal too weak to change the latched guess. Three signals, strongest first. A line or page
/// step is a classic wheel. A non-zero `dx` is a two-axis trackpad glide. A
/// large `|dy|` with no `dx` is a discrete mouse notch.
///
/// The deltas are pixels, read from the event's own unit first.
fn wheel_device(delta: WheelDelta, px: CssVector) -> Option<WheelDevice> {
    if delta.is_step() {
        Some(WheelDevice::Mouse)
    } else if px.dx != CssPx::ZERO {
        Some(WheelDevice::Trackpad)
    } else if px.dy.abs() >= WHEEL_MOUSE_NOTCH_PX {
        Some(WheelDevice::Mouse)
    } else {
        None
    }
}

/// How the wheel navigates the board (`toile-viewport`). A whiteboard is a 2D
/// infinite plane, so the natural gesture differs by device.
///
/// * On a mouse the wheel zooms, map-like.
/// * On a trackpad two-finger scroll pans, and pinch zooms.
/// * `Auto` classifies the device per event from the `WheelEvent` signature.
/// * `Mouse` and `Trackpad` force a mode and freeze that detection: the user
///   chose in the View menu, and the view obeys.
///
/// Dragging always pans as well (Hand tool / Space-drag).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum PointerMode {
    /// Detect the device per event; fall back to trackpad behaviour until a
    /// confident mouse signature latches (the gentler wrong default).
    #[default]
    Auto,
    /// The wheel always zooms about the cursor, with or without Ctrl.
    Mouse,
    /// The wheel pans; only Ctrl+wheel / pinch zooms.
    Trackpad,
}

/// Keyboard-nudge step in world units: a fine 1-unit tap, or a coarse 10-unit
/// jump with Shift held (`toile-elements`).
const NUDGE_SMALL: f64 = 1.0;
const NUDGE_LARGE: f64 = 10.0;
/// Diagonal offset applied to a `Ctrl/Cmd+D` duplicate so the copy is visibly
/// distinct from its source (`toile-clipboard`), matching the alt-drag
/// nudge.
const DUPLICATE_NUDGE: f64 = 10.0;

/// Inline-edit caret blink half-period: the caret shows for one interval,
/// hides for the next (`toile-tools`). ~530 ms is the familiar desktop
/// cadence.
const CARET_BLINK_MS: ambre_core::PageMillis = ambre_core::PageMillis::new(530.0);

/// How many trailing samples of an in-flight freehand stroke ride a presence
/// sample (`TOILE-UI-217`). The relay forwards presence and forgets it, and a
/// stroke can run to thousands of samples. A peer needs to see where the pen
/// is, and the finished stroke reaches them as a CRDT delta when it commits.
const LIVE_STROKE_TAIL: usize = 64;

/// Max press-to-release travel (screen px) that still reads as a click, not a
/// drag, for click-to-edit (`toile-text-editing`). Beyond it the gesture is a
/// move, and the view arms no edit.
const CLICK_SLOP: f64 = 4.0;

/// Hand `data` to the browser as a download named `filename`
/// (`TOILE-IO-085`). A `Blob` behind an object URL, clicked through a detached
/// anchor: the same path Trame's CSV export takes, because the browser offers no
/// other. Best effort -- the view logs a blob the browser refuses, never fails.
#[cfg(target_arch = "wasm32")]
fn trigger_download(filename: &str, data: &[u8]) {
    use wasm_bindgen::JsCast;
    let array = js_sys::Uint8Array::from(data);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let blob = match web_sys::Blob::new_with_u8_array_sequence(&parts) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Toile: download blob failed: {e:?}");
            return;
        }
    };
    let url = match web_sys::Url::create_object_url_with_blob(&blob) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("Toile: download URL failed: {e:?}");
            return;
        }
    };
    let anchor = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.create_element("a").ok())
        .and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok());
    if let Some(anchor) = anchor {
        anchor.set_href(&url);
        anchor.set_download(filename);
        anchor.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

/// Host-build stub: no DOM to download into outside the browser. Kept so the
/// exhaustive event dispatch compiles in the headless tests. A host test reads
/// back each file the view handed over (`PDF-014`).
#[cfg(not(target_arch = "wasm32"))]
fn trigger_download(filename: &str, data: &[u8]) {
    #[cfg(test)]
    DOWNLOADS.with(|d| d.borrow_mut().push((filename.to_owned(), data.to_vec())));
    #[cfg(not(test))]
    let _ = (filename, data);
}

#[cfg(all(test, not(target_arch = "wasm32")))]
thread_local! {
    /// Each file the host stub was handed, in order: its name and its bytes.
    pub(crate) static DOWNLOADS: std::cell::RefCell<Vec<(String, Vec<u8>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub struct ToileView {
    document_id: DocumentId,
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
    from_logic: ambre_bus::conduit::Receiver<UiEvent>,
    /// The lossless send path: it keeps an edit a full queue could not take,
    /// and offers it again on a later frame (`BUS-011`).
    outbox: Outbox,
    /// The busy level the chrome reads while the outbox holds anything
    /// (`BUS-016`).
    busy_pub: BusyPublisher,
    /// What the shell shows the user when the board cannot show it, such as
    /// an edit lost with its element (`TOILE-UI-387`).
    notices: NoticePublisher,

    /// The render-thread camera (world <-> screen), updated locally on pan/zoom.
    cam: ViewportTransform,
    /// The last culled frame received from Logic (what to paint).
    display: DisplayViewport,

    /// Active tool + the gesture state machine (Select + creation tools).
    machine: ToolMachine,
    /// Local selection (never persisted; a peer's selection is presence,
    /// `toile-selection`).
    selection: Vec<ElementId>,
    /// Full display data of the current selection, off-screen included, fetched
    /// from Logic via `RequestSelectionData` (`TOILE-UI-070`, `TOILE-UI-134`).
    /// The culled [`display`](Self::display) omits the selection past the edge,
    /// so the menu scope, the style surfaces and the inline editor read this.
    selection_data: Vec<DisplayElement>,
    /// The selected ids a frame holds, from the same reply. The menu offers
    /// removal from a frame by this, whatever the boxes (`TOILE-UI-384`).
    selection_framed: Vec<ElementId>,
    /// A select-all waits for Logic's answer to `RequestBoardIds`, which names
    /// every element on the board, off-screen included (`TOILE-UI-033`). The
    /// list is asked for at each select-all: one read at the open would miss
    /// every element created since.
    select_all_pending: bool,
    /// The selected arrow (`toile-connectors`). Exclusive with
    /// [`selection`](Self::selection): an arrow has no box, so it never joins a
    /// marquee, a group or a transform, and selecting one clears the other.
    arrow: Option<ConnectorId>,
    /// An endpoint drag in flight: the arrow, the dragged end, and where the
    /// pointer is. The write happens on release (`toile-connectors`).
    arrow_drag: Option<(ConnectorId, End)>,
    /// Entered group scope (`toile-groups` / `toile-selection`):
    /// while `Some(g)`, a click selects a member inside `g` rather than the whole
    /// group. Entered by double-clicking a grouped element (peels one nesting level
    /// per double-click), left by `Escape` or a document swap. Pure-UI state, never
    /// synced.
    group_scope: Option<ElementId>,
    /// Live gesture preview (marquee / rubber-band / pen / stroke), world space.
    preview: Option<Preview>,

    /// Find-on-board highlight state (`toile-find`), driven by
    /// `FindResults` from Logic. `find_hits` are all matching ids (only those in
    /// the culled set get an outline); `find_focus` is the current match. Local
    /// view state, never synced and never an undo item
    /// (`toile-find`).
    find_hits: Vec<ElementId>,
    find_focus: Option<ElementId>,
    /// The focus id awaiting a `SelectionStats` reply so the camera can recenter
    /// on it (`toile-find`). `FindResults` carries no geometry and the focus may
    /// be off-screen. So the view asks Logic for its bounds via
    /// `RequestSelectionStats`, and recenters when the answer lands.
    /// Distinguishes a find-driven stats request from a future status-bar one.
    pending_find_focus: Option<ElementId>,

    /// Active inline text editor (sticky / text body, `toile-text-editing`).
    /// `None` when not editing. The view queues font-free edit intents here at
    /// input time and applies them during paint, when the `FontContext` is
    /// available (see `edit.rs`).
    editor: Option<InlineEditor>,
    /// The target of the last write an editor sent, kept until the board
    /// refuses it as gone (`TOILE-UI-389`).
    edit_written: Option<EditTarget>,
    /// The world box of the caption under edit, as last read on screen. A
    /// culled arrow keeps its editor at this box (`TOILE-UI-386`).
    edit_caption_box: Cell<Option<Rect>>,
    /// The frame whose title is under edit, as the board last answered, off
    /// screen included. The title editor clears the selection, so it fetches
    /// its frame on its own, and a pan keeps it open (`TOILE-UI-390`).
    edit_frame: Option<DisplayElement>,
    /// A freshly-created element (Sticky / Text tool) whose editor must open once
    /// the matching `DisplayViewport` has landed (nothing knows its geometry
    /// before then). Consumed at the end of `update`.
    edit_pending_open: Option<ElementId>,
    /// The `correlate` token the Sticky / Text tool's `CreateElement` carried,
    /// so the deferred open claims that element and no other (`TOILE-UI-194`).
    /// `None` when no placement awaits its reply.
    edit_open_on_create: Option<CorrelationToken>,
    /// The token the next placement's `CreateElement.correlate` carries.
    next_create_token: CorrelationToken,
    /// Deferred close: the editor commits its buffer as `SetText` and closes on
    /// the next paint (so any intents typed since the last paint flush first).
    edit_closing: bool,
    /// Where the last paint laid out the editor's caret, in canvas pixels. The
    /// pipe sits there while a text is edited (`CHROME-007`).
    caret_drawn: Option<ambre_draw::Rect>,
    /// A queued edit intent / open / close marked the editor overlay dirty
    /// (`toile-text-editing`).
    edit_dirty: bool,
    /// Whether a pointer is down inside the editor (text selection drag).
    edit_pointer_down: bool,
    /// Click-to-edit arming (Miro-style, `toile-text-editing`): set on a left press
    /// that lands on an already-selected Sticky / Text element (id + press
    /// position). A release near that position with the selection unchanged enters
    /// edit; a drag past the slop (a move gesture) clears it. `None` when the press
    /// did not qualify.
    click_edit_candidate: Option<(ElementId, Point)>,

    /// Last pointer position in screen (logical) pixels -- the zoom anchor.
    last_pointer: Point,
    /// While panning (space-drag / Hand tool): the previous screen position.
    panning: Option<Point>,
    /// The shared kinetic scroll: wheel glides, finger pans and the motion
    /// either leaves (`1-Foundations/ambre-spec-kinetic-scroll.md`).
    kinetic: ambre_draw::kinetic::Kinetic,
    /// The role of the finger down, if any.
    finger: Option<FingerRole>,
    /// The edge pan of a held gesture (`TOILE-UI-380`).
    edge: ambre_draw::kinetic::EdgeScroll,
    /// Space held: a drag pans in any tool (`toile-viewport`).
    space_down: bool,

    /// Wheel navigation preference (`toile-viewport`): `Auto` (default), or a
    /// forced `Mouse` / `Trackpad` set from the View menu.
    pointer_mode: PointerMode,
    /// Latched device guess while `pointer_mode == Auto`. Starts `false` (trackpad,
    /// the gentler wrong default, `toile-viewport`) and flips to `true` on a
    /// confident mouse-wheel signature, back on a two-axis trackpad glide.
    detected_mouse: bool,

    text_input: TextInput,
    /// The view is a board embedded in another document, which owns the pipe.
    embedded: bool,
    perf_pub: PerfPublisher,
    theme_ctl: ThemeControl,
    theme: Theme,

    // Dirty-flag bookkeeping (`toile-board-rendering`).
    needs_full: bool,
    /// A change drawn on the overlay alone, such as the selection: the frame
    /// redraws the overlay and reuses the content (`TOILE-UI-271`).
    overlay_dirty: bool,
    viewport_changed: bool,
    data_dirty: bool,
    /// Coalesced viewport request: at most one `SetViewport` per frame.
    pending_viewport: Option<Viewport>,
    /// The version of the last frame the view accepted. An older one is a
    /// stale answer, and the view drops it (`BUS-017`).
    accepted_version: ambre_bus::ViewportVersion,
    initial_viewport_sent: bool,
    fatal: bool,

    /// A [`ClipboardBlob`] built from a `ToileEvent::Clipboard` reply, awaiting the
    /// host's per-frame [`poll_clipboard`](ToileView::poll_clipboard) which mirrors
    /// it to the OS clipboard inside the transient-activation window
    /// (`toile-clipboard`).
    pending_clipboard: Option<ambre_bus::ClipboardBlob>,
    /// The last authoritative clipboard blob copied this session, kept in
    /// memory for a full-fidelity same-tab paste (`toile-clipboard`). That
    /// paste never depends on the OS custom MIME type surviving the round-trip.
    last_clip: Option<ambre_bus::ClipboardBlob>,

    /// Selection scope published to the DOM context menu each right-click
    /// (`toile-menus`): the view writes it, the chrome's entry `Memo`
    /// reads it to omit inapplicable rows. Leptos signal, so wasm-only.
    #[cfg(target_arch = "wasm32")]
    ctx_scope: RwSignal<super::menu::CtxScope>,
    /// The context-menu command the shell forwards back into the view
    /// (`toile-menus`). `update` drains the menu id and resolves it against the
    /// live selection. The Logic thread never sees the selection, so the view
    /// attaches it. Wasm-only.
    #[cfg(target_arch = "wasm32")]
    menu_cmd: RwSignal<Option<CommandId>>,
    /// The active tool, shared with the DOM tool palette
    /// (`toile-tools`). The palette buttons and the bare-key tool chords both
    /// write it, and the view applies it each frame in `update`. Tool selection
    /// is UI-only, canvas-local state (`toile-tools`), never a `ToileCommand`.
    /// Leptos signal, so wasm-only.
    #[cfg(target_arch = "wasm32")]
    active_tool: RwSignal<Tool>,
    /// Position-dependent canvas cursor, shared with the chrome
    /// (`toile-tools`). The base cursor is a pure function of the active tool
    /// (`cursor_css`). A resting hover over a selection edge or corner
    /// overrides it with a directional resize or grab cursor. So does any
    /// resize, rotate or move in flight. The view writes it (`publish_cursor`),
    /// and the chrome's cursor `Effect` prefers it over `cursor_css` when
    /// `Some`. Leptos signal, so wasm-only.
    #[cfg(target_arch = "wasm32")]
    cursor: RwSignal<Option<super::palette::CanvasCursor>>,
    /// Live zoom factor shared with the DOM zoom control and status bar
    /// (`toile-menus` / `toile-status-bar`): `1.0` = 100%. The view publishes
    /// the camera's zoom, and a preset the chip writes moves the camera.
    /// Wasm-only.
    #[cfg(target_arch = "wasm32")]
    zoom: RwSignal<ambre_chrome::ZoomFactor>,
    /// The zoom this view last published. A signal reading anything else was
    /// written by the chip, so the camera follows it. Wasm-only.
    #[cfg(target_arch = "wasm32")]
    zoom_published: ambre_core::ZoomFactor,
    /// Status-bar selection aggregates (`toile-status-bar`), shared with the
    /// DOM band. The view clears it to `None` on an empty selection: an
    /// empty-ids stats request would mean "whole board" to Logic. It must not
    /// go through the round trip. On a non-empty selection the shell fills it
    /// from the `SelectionStats` reply. Leptos signal, so wasm-only.
    #[cfg(target_arch = "wasm32")]
    sel_stats: RwSignal<Option<super::status::SelStats>>,
    /// The selection's common style published to the DOM style inspector
    /// (`toile-styles`). The view derives it from its selection x last display
    /// and writes it, alongside `ctx_scope`. The inspector reads it to show
    /// values and mixed-state. `None` hides the panel. Leptos signal, so
    /// wasm-only.
    #[cfg(target_arch = "wasm32")]
    style_model: RwSignal<Option<super::style::StyleModel>>,
    /// A whole `ElementStyle` the inspector emits back
    /// (`toile-styles`). `update` drains it and turns it into one
    /// `RestyleElements` over the live selection. The command carries the whole
    /// register (`toile-styles`), and the view attaches the ids the DOM lacks.
    /// Wasm-only.
    #[cfg(target_arch = "wasm32")]
    restyle: RwSignal<Option<ambre_bus::app::toile::ElementStyle>>,
    /// The content-anchored floating selection toolbar's placement + flags
    /// (`toile-styles`), republished every frame so the bar follows the
    /// selection as the board pans and zooms (transform-synced). `None` hides
    /// it: an empty selection, an active gesture (move/resize/rotate), a pan,
    /// or an open inline editor. Leptos signal, so wasm-only.
    #[cfg(target_arch = "wasm32")]
    float_bar: RwSignal<Option<super::floatbar::FloatBar>>,
    /// A single-field style edit the floating bar emits back
    /// (`toile-styles`). `update` drains it and folds it into one
    /// `RestyleElements` over the live selection, the same seam as `restyle`.
    /// The changed field rides on top of the freshly-derived common style.
    /// Wasm-only.
    #[cfg(target_arch = "wasm32")]
    quick_style: RwSignal<Option<super::floatbar::StyleEdit>>,
    /// The comment panel's signals (`toile-comments`): the view publishes the
    /// list and the open thread it read back from Logic, and drains the panel's
    /// intents. The panel holds nothing of its own (`TOILE-UI-023`). Wasm-only.
    #[cfg(target_arch = "wasm32")]
    comments: super::comments::CommentSignals,
    /// The deck panel's signals (`toile-presentation`): the view publishes the
    /// frames it read back from Logic and drains the panel's intents. The panel
    /// holds nothing of its own. Wasm-only.
    #[cfg(target_arch = "wasm32")]
    deck: super::deck::DeckSignals,
    /// A file the shell read for the board (`toile-import-export`): a drop on
    /// the canvas, or a pick from the File menu. Drained each frame in `update`
    /// and sent to the importer at the world point under the drop
    /// (`TOILE-IO-023`). The shell owns the dialog and the bytes; the view owns
    /// the camera. Wasm-only.
    #[cfg(target_arch = "wasm32")]
    dropped_file: RwSignal<Option<super::interop::DroppedFile>>,
    /// An embed the shell resolved for the board (`TOILE-086`, `TOILE-087`): a
    /// web address the user typed, or a document they picked. Drained each
    /// frame like [`Self::dropped_file`], and for the same reason. The shell
    /// owns both dialogs and the view owns the camera, so only the view can say
    /// where on the board the answer lands. Wasm-only.
    #[cfg(target_arch = "wasm32")]
    placed_embed: RwSignal<Option<super::embed::PlacedEmbed>>,
    /// A link edit the inspector emitted, drained in `update` and applied to the
    /// single selection as one `SetLink` (`toile-links`). The DOM panel cannot
    /// name the element, so the view attaches it.
    #[cfg(target_arch = "wasm32")]
    set_link: RwSignal<Option<super::link::LinkEdit>>,
    /// A followed link the board cannot perform itself: the shell opens the tab
    /// or the document (`TOILE-065`, `TOILE-114`). Write-only from here.
    #[cfg(target_arch = "wasm32")]
    followed_link: RwSignal<Option<super::link::ShellNav>>,
    /// The present mode and the find results, published for the chrome. The
    /// view keeps both, so it alone writes these signals (`UI-THREAD-003`).
    /// Wasm-only.
    #[cfg(target_arch = "wasm32")]
    present_mode: RwSignal<ambre_bus::app::toile::PresentMode>,
    #[cfg(target_arch = "wasm32")]
    find_result: RwSignal<Option<ambre_bus::app::toile::FindResults>>,

    /// The element that anchors a thread the panel opened, awaiting the
    /// `SelectionStats` reply that carries its bounds. The camera then comes to
    /// it (`TOILE-UI-309`). The same reply as the find focus, and the view
    /// reads it after. `cfg`-agnostic so the dispatch arm stays uniform.
    pending_comment_focus: Option<ElementId>,

    /// Set while a Fit / Zoom-to-selection awaits the `SelectionStats` reply whose
    /// bounds it will frame (`toile-viewport`). The reply treats both the same
    /// way, fitting the returned bounds, so a flag suffices. The view reads it
    /// before the find-focus recenter that shares the same stats reply.
    /// `cfg`-agnostic so the dispatch arm stays uniform.
    pending_fit: bool,

    /// The selection last reported to the status bar (`toile-status-bar`).
    /// `plan_repaint` re-requests `SelectionStats` whenever the live selection
    /// differs. The band's count and bounds then track every selection change
    /// (marquee, click, nudge) without a request threaded through each
    /// mutation site.
    last_stats_sel: Vec<ElementId>,
    /// A board change landed while something is selected, so the selection's
    /// data may be stale: its lock, its box, its frame (`TOILE-UI-384`).
    /// `plan_repaint` asks again once, whatever number of changes landed.
    selection_stale: bool,
    /// The next `SelectionData` answers a new selection, so it is announced.
    /// A refresh of the same selection is not (`selection_stale`).
    announce_next_data: bool,

    /// The render-thread mirror of remote peer presence (`toile-presence`):
    /// cursors, selection ghosts, live strokes and laser trails, keyed by
    /// canonical `UserId`. Fed by `ToileEvent::PeerPresence` and painted in the
    /// overlay pass. SYNC P4 gates end-to-end multi-user, because it resolves
    /// the `pub_key` -> `UserId` that keys an inbound sample. Synthetic events
    /// exercise the render pass meanwhile.
    presence: PeerCursors,
    /// The broadcast schedule for this peer's own presence
    /// (`toile-presence`, `TOILE-UI-073`): it decides whether the sample built
    /// this frame is worth sending, and holds nothing else.
    presence_out: PresenceSender,

    /// Presentation focus mode (`toile-presentation`): driven by
    /// `PresentModeChanged`, it makes the content pass hide the board grid so the
    /// audience sees only the slide. Pure-UI, never synced.
    present: PresentMode,
    /// The in-flight slide transition (`toile-presentation`), built on a
    /// `SlideChanged` event and advanced each `plan_repaint` until done. It
    /// glides the camera to the active frame, and it drives the Smart Animate
    /// content overrides. `None` when the deck is idle.
    transition: Option<Transition>,
    /// Smart Animate overrides to paint this frame, keyed by element id, rebuilt
    /// from the active `transition` each frame and cleared when it completes. The
    /// content pass looks each element up here before falling back to its static
    /// display (`toile-presentation`).
    anim_overrides: HashMap<ElementId, AnimatedElement>,

    /// Live transform overrides for the in-flight Select gesture
    /// (`toile-tools`). These are the resized, rotated or moved elements
    /// painted at their preview transform before the command commits on
    /// release. Rebuilt each pointer event from `machine.gesture_overrides`,
    /// cleared on release, cancel or tool switch. The drag stays 100% local and
    /// sends no `send_edit`, so the preview is instant and never touches the
    /// CRDT. The single command still commits on `pointer_up`. Mutually
    /// exclusive with `anim_overrides`: a slide transition and a manual gesture
    /// never run together.
    gesture_overrides: HashMap<ElementId, AnimatedElement>,

    /// True between committing a transform gesture on release and the arrival of
    /// the recomputed `ViewportData` that carries the new geometry
    /// (`toile-tools`). While set, the view holds `gesture_overrides` as an
    /// *optimistic* preview, so the object keeps its released position and
    /// size. Without it the object snaps back to the pre-commit geometry for
    /// the frames the Logic round-trip takes, which reads as a flicker. The
    /// next display update clears the flag and the overrides together.
    gesture_commit_pending: bool,

    /// Decoded image pixels keyed by media hash, passed to the content walk so
    /// `Image` elements paint real pixels (`toile-media`). Empty
    /// until the media cache resolves a hash: an unresolved image paints a
    /// placeholder box. The screen-side population (from `MediaDownloaded`) lands
    /// with the media cache slice; the field is here so the walk has a stable
    /// source today.
    images: render::ImageStore,
    /// Addresses already asked for and not yet answered. The view asks for a
    /// picture once rather than on every frame that fails to paint it. An
    /// address the document cannot resolve never gets an answer and stays here.
    /// That is what stops the view asking again for a picture that is not there
    /// (`toile-media`).
    media_requested: HashSet<ambre_core::ContentHash>,
    /// A media request the queue dropped. The next frame offers it again
    /// (`BUS-024`): a viewport reply may never come to ask for it.
    media_retry: bool,

    /// Which `doc_embed` holds input focus, and the click-vs-drag arbitration
    /// that decides it (`toile-embeds`, "Input focus"). View-local,
    /// never CRDT. While an embed holds focus, `FrameHost` maps canvas input
    /// into that child and calls the child view's own `handle_input`. This view
    /// forwards nothing itself, because it cannot name the child's command type
    /// (`toile-embeds`, "Command routing").
    embed_focus: embed::EmbedFocus,
    /// The child sync sessions behind the visible `doc_embed`s, with the close
    /// debounce that keeps a pan from thrashing them (`toile-embeds`,
    /// "Opening the child").
    embed_sessions: embed::EmbedSessions,
    /// The child document that took the last input event
    /// (`toile-embeds`, "Input focus"), or `None` when the board
    /// consumed it. Set by `handle_input`, read once by `FrameHost` right after,
    /// which forwards the (coordinate-remapped) event to that child's own view.
    /// View-local, never CRDT.
    child_route: Option<ElementId>,

    /// The latest frame timestamp (ms), captured in `plan_repaint`, used to blink
    /// the inline-edit caret (`toile-text-editing`). Paint has no clock of its own,
    /// so the per-frame hook stashes it here.
    frame_now_ms: ambre_core::PageTime,
}

/// The topmost element under a canvas point, through the camera
/// (`UI-THREAD-001`).
impl ambre_draw::HitTest for ToileView {
    type Target = ElementId;

    fn hit_test(&self, at: ambre_core::ViewPoint) -> Option<ElementId> {
        let at = Point::new(f64::from(at.x.get()), f64::from(at.y.get()));
        self.element_at(self.cam.screen_to_world() * at)
    }
}

impl ToileView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        document_id: DocumentId,
        to_logic: ambre_bus::conduit::Sender<LogicCommand>,
        from_logic: ambre_bus::conduit::Receiver<UiEvent>,
        canvas_width: f64,
        canvas_height: f64,
        text_input: TextInput,
        // When embedded as a `doc_embed` child (`toile-embeds`), the child view
        // must not grab the shared text pipe on construction. The top-level
        // document owns the caret until the user clicks into the embed.
        // A top-level view passes `Standalone` and focuses immediately.
        embedding: ambre_canvas::Embedding,
        perf_pub: PerfPublisher,
        busy_pub: BusyPublisher,
        notices: NoticePublisher,
        theme_ctl: ThemeControl,
        #[cfg(target_arch = "wasm32")] ctx_scope: RwSignal<super::menu::CtxScope>,
        #[cfg(target_arch = "wasm32")] menu_cmd: RwSignal<Option<CommandId>>,
        #[cfg(target_arch = "wasm32")] active_tool: RwSignal<Tool>,
        #[cfg(target_arch = "wasm32")] cursor: RwSignal<Option<super::palette::CanvasCursor>>,
        #[cfg(target_arch = "wasm32")] zoom: RwSignal<ambre_chrome::ZoomFactor>,
        #[cfg(target_arch = "wasm32")] sel_stats: RwSignal<Option<super::status::SelStats>>,
        #[cfg(target_arch = "wasm32")] style_model: RwSignal<Option<super::style::StyleModel>>,
        #[cfg(target_arch = "wasm32")] restyle: RwSignal<
            Option<ambre_bus::app::toile::ElementStyle>,
        >,
        #[cfg(target_arch = "wasm32")] float_bar: RwSignal<Option<super::floatbar::FloatBar>>,
        #[cfg(target_arch = "wasm32")] quick_style: RwSignal<Option<super::floatbar::StyleEdit>>,
        #[cfg(target_arch = "wasm32")] comments: super::comments::CommentSignals,
        #[cfg(target_arch = "wasm32")] deck: super::deck::DeckSignals,
        #[cfg(target_arch = "wasm32")] dropped_file: RwSignal<Option<super::interop::DroppedFile>>,
        #[cfg(target_arch = "wasm32")] placed_embed: RwSignal<Option<super::embed::PlacedEmbed>>,
        #[cfg(target_arch = "wasm32")] set_link: RwSignal<Option<super::link::LinkEdit>>,
        #[cfg(target_arch = "wasm32")] followed_link: RwSignal<Option<super::link::ShellNav>>,
        #[cfg(target_arch = "wasm32")] present_mode: RwSignal<ambre_bus::app::toile::PresentMode>,
        #[cfg(target_arch = "wasm32")] find_result: RwSignal<
            Option<ambre_bus::app::toile::FindResults>,
        >,
    ) -> Self {
        let embedded = embedding == ambre_canvas::Embedding::Embedded;
        if !embedded {
            text_input.name_content(ambre_canvas::ContentKind::Board);
            text_input.focus();
            // No editor is open yet, so a bare key arms a tool (`TOILE-UI-393`).
            text_input.set_typing(false);
        }
        let theme = theme_ctl.get();
        Self {
            document_id,
            outbox: Outbox::new(to_logic.clone()),
            busy_pub,
            notices,
            to_logic,
            from_logic,
            cam: ViewportTransform::new(canvas_width, canvas_height),
            display: DisplayViewport {
                version: ambre_bus::ViewportVersion(0),
                grid_unit: super::grid::BASE_GRID,
                reactions: Default::default(),
                pins: Vec::new(),
                viewport: Viewport::default(),
                nodes: Vec::new(),
            },
            machine: ToolMachine::new(),
            selection: Vec::new(),
            selection_data: Vec::new(),
            selection_framed: Vec::new(),
            select_all_pending: false,
            arrow: None,
            arrow_drag: None,
            group_scope: None,
            preview: None,
            find_hits: Vec::new(),
            find_focus: None,
            pending_find_focus: None,
            editor: None,
            edit_written: None,
            edit_caption_box: Cell::new(None),
            edit_frame: None,
            edit_pending_open: None,
            edit_open_on_create: None,
            next_create_token: CorrelationToken::FIRST,
            edit_closing: false,
            caret_drawn: None,
            edit_dirty: false,
            edit_pointer_down: false,
            click_edit_candidate: None,
            last_pointer: Point::new(canvas_width / 2.0, canvas_height / 2.0),
            panning: None,
            kinetic: Default::default(),
            finger: None,
            edge: ambre_draw::kinetic::EdgeScroll::default(),
            space_down: false,
            pointer_mode: PointerMode::default(),
            detected_mouse: false,
            text_input,
            embedded,
            perf_pub,
            theme_ctl,
            theme,
            needs_full: true,
            overlay_dirty: false,
            viewport_changed: false,
            data_dirty: false,
            pending_viewport: None,
            accepted_version: ambre_bus::ViewportVersion(0),
            initial_viewport_sent: false,
            fatal: false,
            pending_clipboard: None,
            last_clip: None,
            #[cfg(target_arch = "wasm32")]
            ctx_scope,
            #[cfg(target_arch = "wasm32")]
            menu_cmd,
            #[cfg(target_arch = "wasm32")]
            active_tool,
            #[cfg(target_arch = "wasm32")]
            cursor,
            #[cfg(target_arch = "wasm32")]
            zoom,
            #[cfg(target_arch = "wasm32")]
            zoom_published: ambre_core::ZoomFactor::ONE,
            #[cfg(target_arch = "wasm32")]
            sel_stats,
            #[cfg(target_arch = "wasm32")]
            style_model,
            #[cfg(target_arch = "wasm32")]
            restyle,
            #[cfg(target_arch = "wasm32")]
            float_bar,
            #[cfg(target_arch = "wasm32")]
            quick_style,
            #[cfg(target_arch = "wasm32")]
            comments,
            #[cfg(target_arch = "wasm32")]
            deck,
            #[cfg(target_arch = "wasm32")]
            dropped_file,
            #[cfg(target_arch = "wasm32")]
            placed_embed,
            #[cfg(target_arch = "wasm32")]
            set_link,
            #[cfg(target_arch = "wasm32")]
            followed_link,
            #[cfg(target_arch = "wasm32")]
            present_mode,
            #[cfg(target_arch = "wasm32")]
            find_result,
            pending_comment_focus: None,
            pending_fit: false,
            last_stats_sel: Vec::new(),
            selection_stale: false,
            announce_next_data: false,
            presence: PeerCursors::default(),
            presence_out: PresenceSender::default(),
            present: PresentMode::Editing,
            transition: None,
            anim_overrides: HashMap::new(),
            gesture_overrides: HashMap::new(),
            gesture_commit_pending: false,
            images: render::ImageStore::new(),
            media_requested: HashSet::new(),
            media_retry: false,
            embed_focus: embed::EmbedFocus::default(),
            embed_sessions: embed::EmbedSessions::default(),
            child_route: None,
            frame_now_ms: ambre_bus::PageTime(0.0),
        }
    }

    // -- Command emission ------------------------------------------------------

    /// Ask the Logic thread for the pixels of every picture the new display list
    /// names and this view does not hold (`toile-media`).
    ///
    /// Driven from the viewport reply rather than from the render walk. The walk
    /// takes the store by shared reference and paints. A paint that sends
    /// commands is a paint that can change what the next paint does. The display
    /// list holds only what survived the cull, so a board of two hundred
    /// photographs transfers the ones on screen.
    fn request_missing_media(&mut self) {
        let wanted =
            render::missing_media(&self.display.nodes, &self.images, &self.media_requested);
        if wanted.is_empty() {
            return;
        }
        // Mark the requests in flight only once they left: a dropped request
        // that stayed marked is a picture nothing ever asks for again
        // (`BUS-024`). A dropped one goes out again on the next frame.
        if self.send_lossy(ToileCommand::RequestMedia {
            hashes: wanted.clone(),
        }) {
            for h in wanted {
                self.media_requested.insert(h);
            }
        } else {
            self.media_retry = true;
        }
    }

    /// Offer one lossy command to the Logic thread (`BUS-023`).
    ///
    /// Returns whether the command reached the queue. A full one drops it and
    /// nothing holds it. The caller offers the request again on a later frame
    /// (`BUS-024`), or undoes what it recorded for it. Every drop is logged: a
    /// board that stops re-culling leaves no trace of why (`TOILE-UI-313`). The
    /// bus gate refuses a command that changes the document, so an edit offered
    /// here stops instead of dropping. Never blocks the UI thread.
    fn send_lossy(&self, cmd: ToileCommand) -> bool {
        if self.fatal {
            return false;
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: AppCommand::Toile(cmd),
        };
        match ambre_bus::send_lossy(&self.to_logic, msg) {
            LossyOutcome::Sent => true,
            LossyOutcome::Dropped => {
                tracing::warn!("Toile: UI->Logic channel full, the lossy request was dropped");
                false
            }
            LossyOutcome::Refused => {
                tracing::error!("Toile: a document mutation was offered to the lossy class");
                false
            }
            LossyOutcome::Disconnected => {
                tracing::error!("Toile: UI->Logic channel disconnected");
                false
            }
        }
    }

    /// Send a mutating command (edits) through the lossless outbox.
    ///
    /// On a full queue the outbox holds the command for a later frame
    /// (`BUS-011`), and the board refuses input until the queue drains
    /// (`BUS-016`). The return says whether the command is on its way, so
    /// a caller mirroring it into the view state can decline (`BUS-018`).
    fn send_edit(&mut self, cmd: ToileCommand) -> bool {
        if self.fatal {
            return false;
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: AppCommand::Toile(cmd),
        };
        match self.outbox.send(msg) {
            SendOutcome::Sent => true,
            SendOutcome::Held => {
                tracing::warn!("Toile: UI->Logic channel full, edit held for a later frame");
                true
            }
            SendOutcome::Refused => {
                tracing::error!("Toile: the outbox is full, edit refused");
                false
            }
            SendOutcome::Disconnected => {
                tracing::error!("Toile: UI->Logic channel disconnected");
                false
            }
        }
    }

    /// Offer the held lossless commands to the queue, then publish the busy
    /// level the chrome shows and the input gate reads (`BUS-016`).
    ///
    /// Called from the frame hook. A disconnected queue is fatal here as
    /// everywhere else: the Logic thread is gone, and it answers nothing.
    fn flush_outbox(&mut self) {
        if self.fatal {
            return;
        }
        if self.outbox.flush() == SendOutcome::Disconnected {
            self.signal_fatal("Logic thread disconnected");
            return;
        }
        if self.busy_pub.publish(self.outbox.is_busy()) && !self.outbox.is_busy() {
            tracing::info!("Toile: the UI->Logic channel cleared, input resumes");
        }
    }

    /// Whether the board is refusing input because the outbox holds an edit.
    pub fn is_busy(&self) -> bool {
        self.outbox.is_busy()
    }

    /// Queue a coalesced `SetViewport` for the current camera (flushed in `update`).
    fn queue_viewport(&mut self) {
        self.pending_viewport = Some(self.cam.to_bus_viewport());
        self.viewport_changed = true;
    }

    fn signal_fatal(&mut self, message: &str) {
        if !self.fatal {
            self.fatal = true;
            ambre_canvas::signal_fatal(message);
        }
    }

    // -- Event intake ----------------------------------------------------------

    /// Canvas-side reaction to one `ToileEvent`. Exhaustive: a new variant must
    /// fail the build here rather than be dropped silently.
    fn dispatch(&mut self, ev: ToileEvent) {
        match ev {
            // The pixels behind an address this view asked for (`toile-media`).
            // They land in the store the render walk consults, and the next
            // frame paints the picture instead of the placeholder.
            ToileEvent::MediaDecoded {
                hash,
                width,
                height,
                pixels,
            } => {
                self.media_requested.remove(&hash);
                self.images.insert(
                    hash,
                    render::DecodedImage {
                        pixels,
                        width,
                        height,
                    },
                );
                self.data_dirty = true;
                self.needs_full = true;
            }
            ToileEvent::ViewportData(dv) => {
                if dv.version < self.accepted_version {
                    return;
                }
                self.accepted_version = dv.version;
                self.display = dv;
                self.data_dirty = true;
                self.request_missing_media();
                // The recomputed frame now carries the committed transform, so drop
                // the optimistic post-release preview (`toile-tools`): the
                // real geometry takes over seamlessly, with no snap-back flicker.
                if self.gesture_commit_pending && !self.machine.is_active() {
                    self.gesture_overrides.clear();
                    self.gesture_commit_pending = false;
                    self.needs_full = true;
                }
            }
            // Authoritative copy reply (`toile-clipboard`): the Logic thread
            // built a self-contained, id-free snapshot of the selection from the
            // whole board (no viewport limit). Stash it in memory for a
            // full-fidelity same-tab paste and mark it pending so the host mirrors
            // it to the OS clipboard next frame. Paste into a foreign app is out of
            // MVP, so the `text/plain` mirror is empty (the private Ambre bytes
            // carry it all).
            ToileEvent::Clipboard(payload) => {
                let blob = ambre_bus::ClipboardBlob {
                    text: String::new(),
                    html: None,
                    ambre: postcard::to_allocvec(&payload).ok(),
                    plain: false,
                };
                self.last_clip = Some(blob.clone());
                self.pending_clipboard = Some(blob);
            }
            // A Sticky / Text tool just created an element: capture its id so the
            // inline editor opens on it once its `DisplayViewport` lands
            // (`toile-text-editing`).
            ToileEvent::ElementCreated { id, correlate } => {
                if correlate.is_some_and(|token| self.edit_open_on_create == Some(token)) {
                    self.edit_open_on_create = None;
                    self.edit_pending_open = Some(id);
                }
            }
            // Find results (`toile-find`): stash the highlight set, and request
            // the focus element's geometry so the camera can recenter on it.
            // The reply may concern an off-screen match absent from `display`.
            // The panel's counter and step origin are published here.
            ToileEvent::FindResults(fr) => {
                #[cfg(target_arch = "wasm32")]
                self.find_result.set(Some(fr.clone()));
                self.find_hits = fr.hits;
                self.find_focus = fr.focus.clone();
                if let Some(focus) = fr.focus {
                    self.pending_find_focus = Some(focus.clone());
                    self.send_edit(ToileCommand::RequestSelectionStats {
                        ids: vec![focus],
                        kind: StatsRequestKind::FindFocus,
                    });
                }
                self.needs_full = true;
            }
            // A `SelectionStats` reply names the request it answers
            // (`TOILE-UI-108`). A band refresh never consumes a pending fit,
            // and a fit never centres on a find focus.
            ToileEvent::SelectionStats(stats) => match stats.kind {
                // A pending Fit / Zoom-to-selection consumes the reply and frames
                // the returned bounds (`toile-viewport`).
                StatsRequestKind::Fit => {
                    if std::mem::take(&mut self.pending_fit)
                        && let Some(bounds) = stats.bounds
                    {
                        self.cam.fit_bounds(bounds, super::viewport::FIT_PAD);
                        self.needs_full = true;
                        self.queue_viewport();
                    }
                }
                // A find step recenters on its focus element, possibly off-screen
                // (`toile-find`).
                StatsRequestKind::FindFocus => {
                    if self.pending_find_focus.take().is_some()
                        && let Some(bounds) = stats.bounds
                    {
                        self.cam.center = bounds.center();
                        self.needs_full = true;
                        self.queue_viewport();
                    }
                }
                // A thread the panel opened recenters on its anchor element
                // (`TOILE-UI-309`).
                StatsRequestKind::CommentFocus => {
                    if self.pending_comment_focus.take().is_some()
                        && let Some(bounds) = stats.bounds
                    {
                        self.cam.center = bounds.center();
                        self.needs_full = true;
                        self.queue_viewport();
                    }
                }
                // The status bar mirrors the band reply in the shell; the view
                // does nothing with it.
                StatsRequestKind::Band => {}
            },
            // Full display data of the selection, off-screen included
            // (`TOILE-UI-070`, `TOILE-UI-134`): cache it for the menu scope, the
            // style surfaces and the inline editor.
            ToileEvent::SelectionData {
                mut elements,
                framed,
                missing,
            } => {
                // An element a peer deleted leaves the selection, so no later
                // command names it (`TOILE-UI-385`).
                super::tool::drop_missing(&mut self.selection, &missing);
                self.drop_lost_editor(&missing);
                // The frame whose title is under edit is not selected: its
                // data is the editor's alone (`TOILE-UI-390`).
                if let Some(frame) = self
                    .editor
                    .as_ref()
                    .and_then(|ed| ed.target().frame_title())
                    && !self.selection.contains(frame)
                    && let Some(at) = elements.iter().position(|e| &e.id == frame)
                {
                    self.edit_frame = Some(elements.remove(at));
                }
                self.selection_data = elements;
                self.selection_framed = framed;
                self.needs_full = true;
                if std::mem::take(&mut self.announce_next_data) {
                    self.announce_selection();
                }
            }
            // Arrows the board lost. A selected arrow leaves the selection and
            // a caption editor on one closes; an arrow merely culled from the
            // frame keeps both (`TOILE-UI-386`, `TOILE-UI-388`).
            ToileEvent::ArrowsGone(gone) => {
                if self.arrow.as_ref().is_some_and(|c| gone.contains(c)) {
                    self.arrow = None;
                    self.arrow_drag = None;
                    self.needs_full = true;
                }
                self.drop_lost_caption_editor(&gone);
            }
            // Every element id on the board, for select-all (`TOILE-UI-033`).
            ToileEvent::BoardIds(ids) => {
                if std::mem::take(&mut self.select_all_pending) {
                    self.arrow = None;
                    self.selection = ids;
                    self.overlay_dirty = true;
                }
            }
            // The selection's data may now be stale: its lock, its box, its
            // frame. `update` asks again once (`TOILE-UI-384`).
            ToileEvent::BoardChanged => {
                self.selection_stale = !self.selection.is_empty();
                // The frame under a title editor may have moved or gone: ask
                // again, so a peer's delete closes the editor (`TOILE-UI-390`).
                if let Some(frame) = self
                    .editor
                    .as_ref()
                    .and_then(|ed| ed.target().frame_title())
                    .cloned()
                {
                    self.send_edit(ToileCommand::RequestSelectionData { ids: vec![frame] });
                }
            }
            // Remote presence (`toile-presence`): adopt the whole live set, so
            // the overlay drops a peer absent from it with no departure signal
            // (`PRESENCE-009`, `TOILE-UI-075`). The sender's canonical `UserId`
            // keys each entry (spoof-proof, `TOILE-177`).
            ToileEvent::PeerPresence(peers) => {
                self.presence.adopt(peers, self.frame_now_ms);
                self.data_dirty = true;
            }
            ToileEvent::Error(e) => {
                tracing::warn!("Toile rejected: {e}");
                self.notice_refused_edit(&e);
                // A rejected transform never yields a new frame; drop the
                // optimistic preview so the object reverts to its real (unchanged)
                // geometry.
                if self.gesture_commit_pending {
                    self.gesture_overrides.clear();
                    self.gesture_commit_pending = false;
                    self.needs_full = true;
                }
            }
            // A slide change (`toile-presentation`): start
            // a transition that glides the camera to the active frame and Smart
            // Animates its content. The camera target is the fitted frame rect; a
            // slide with no resolvable frame keeps the camera where it is
            // (index-only move). Advanced each `plan_repaint` against the frame
            // clock.
            ToileEvent::SlideChanged(change) => {
                let cam_from = self.cam;
                // The length rides the event: the deck's own `transition_ms`,
                // read off the document by the Logic thread (`TOILE-FEAT-028`).
                let dur = change.transition;
                let (to_center, to_zoom) = change
                    .to_rect
                    .and_then(|r| self.cam.fit_target(r, super::viewport::FIT_PAD))
                    .unwrap_or((self.cam.center, self.cam.zoom));
                self.transition = Some(Transition::new(
                    cam_from,
                    to_center,
                    to_zoom,
                    change.from,
                    change.to,
                    dur,
                ));
                self.needs_full = true;
            }
            // Present (focus) mode toggled (`toile-presentation`): the
            // content pass hides the grid while `present`. Leaving it drops any
            // in-flight transition so the board returns to its plain camera +
            // full-opacity content. The mode is published for the presenter
            // panel and the editing surfaces.
            ToileEvent::PresentModeChanged(mode) => {
                #[cfg(target_arch = "wasm32")]
                self.present_mode.set(mode);
                self.present = mode;
                if mode.is_presenting() {
                    self.leave_editing();
                } else {
                    self.transition = None;
                    self.anim_overrides.clear();
                }
                self.needs_full = true;
            }
            // A local reaction changed the tally (`toile-reactions`).
            // The tally rides in `DisplayViewport.reactions`, derived on the
            // Logic thread, so the view requests a fresh viewport for the
            // current camera. The resulting `ViewportData` carries the updated
            // badges. (Remote reactions already push a fresh `ViewportData`
            // via `apply_remote`.)
            ToileEvent::ReactionsUpdated(_) => self.queue_viewport(),
            // "Follow presenter" role changes (`toile-presentation`) are
            // chrome-panel state, not camera-pass state: the presenter panel
            // reflects them via its own signals. The WebGPU view has nothing to
            // repaint for these.
            ToileEvent::PresentBroadcastChanged(_) | ToileEvent::FollowingChanged(_) => {}
            // An arrow lost an end (`TOILE-044`). It is not painted any more
            // (`TOILE-166`), so the board itself cannot show it. The shell
            // raises the notice from this same event.
            ToileEvent::DanglingConnector(_) | ToileEvent::UndoState { .. } => {}
            // A foreign-file import landed (`toile-import-export`). The
            // accompanying `ViewportData` already triggered the repaint; log the
            // lossy-import summary so a dropped construct is visible in
            // diagnostics. UI toasts are the app shell's job.
            ToileEvent::ImportReport { imported, skipped } => {
                tracing::info!(
                    "Toile import: {imported} elements, {} construct(s) dropped",
                    skipped.len()
                );
            }
            ToileEvent::ImportFailed(reason) => {
                tracing::warn!("Toile import failed: {reason}");
            }
            // A thread changed (`toile-comments`). The pins ride the display
            // viewport, so ask for a fresh one. The panel holds no state of its
            // own, so re-read the list and the thread it shows
            // (`TOILE-UI-310`).
            ToileEvent::CommentsUpdated(thread) => {
                self.queue_viewport();
                self.reread_comments(&thread);
            }
            // A thread takes the focus (`toile-comments`): the comment tool
            // just dropped it, or a click landed on its pin. Open the panel on
            // it (`TOILE-UI-082`, `TOILE-UI-307`).
            ToileEvent::CommentThreadOpened(thread) => self.open_thread(thread),
            // The board-wide list, and one whole thread: the answers to a
            // `ReadComments`. Both go straight to the panel's signals, which are
            // the panel's only source (`TOILE-UI-023`).
            ToileEvent::CommentList(list) => self.publish_comment_list(list),
            ToileEvent::CommentThreadData(thread) => self.publish_comment_thread(thread),
            // The board's frames: the answer to a `ReadDeck`. Straight to the
            // deck panel's signals, its only source (`TOILE-FEAT-026`).
            ToileEvent::DeckData(view) => self.publish_deck(view),
            // The deck moved under this viewer (`toile-presentation`): a write,
            // a history step, a remote delta or a deleted frame. Re-read what
            // the panel shows; it keeps no answer of its own.
            ToileEvent::DeckChanged => self.reread_deck(),
            // The note on screen moved under this viewer (`TOILE-UI-316`). The
            // presenter panel is chrome, and the shell mirrors the body into
            // its signal; the board itself draws no note.
            ToileEvent::NotesChanged { .. } => {}
            // An export produced a file (`toile-import-export`). The Logic
            // thread holds the board and serialized the bytes; the UI thread
            // owns the DOM, so this arm triggers the download (`TOILE-IO-085`).
            // Nothing on the board changed.
            ToileEvent::ExportComplete {
                filename,
                data,
                skipped,
            } => {
                // What the file could not express (`TOILE-IO-025`). The board
                // is unchanged; the user is told what left it behind.
                for line in &skipped {
                    tracing::warn!("Toile: the export skipped {line}");
                }
                trigger_download(&filename, &data);
            }
            // Nothing to download: the notice is the shell's (`TOILE-IO-022`).
            ToileEvent::ExportFailed(reason) => {
                tracing::warn!("Toile: export failed: {reason}");
            }
            // Both schema notices are the chrome's to surface (`notify_event`):
            // the canvas has nothing to repaint for either. The board never
            // applied a refused register, so the pixels on screen are already
            // correct. The walk drew an unreadable register with the default
            // style, and redrawing it would produce the same pixels again.
            ToileEvent::DocumentRefused(reason) => {
                tracing::warn!("Toile: document refused: {reason}");
            }
            ToileEvent::UndecodableRecords => {
                tracing::warn!("Toile: a register could not be decoded");
            }
        }
    }

    // -- Input handling --------------------------------------------------------

    fn handle_wheel(&mut self, delta: WheelDelta, mods: Modifiers) -> bool {
        let ctrl_key = mods.primary;
        // A line or page delta moves the board by the pixels it is worth, as
        // in every other view (`KINETIC-015`).
        let line = CssVector::new(WHEEL_LINE_PX, WHEEL_LINE_PX);
        let page = CssVector::new(
            CssPx::new(self.cam.size.width as f32),
            CssPx::new(self.cam.size.height as f32),
        );
        let px = delta.to_px(line, page);
        // The pointer mode decides zoom against pan (`toile-viewport`). Ctrl
        // always zooms, and so does a trackpad pinch, which the browser reports
        // as a ctrl-wheel.
        let zoom = match self.pointer_mode {
            PointerMode::Mouse => true,
            PointerMode::Trackpad => ctrl_key,
            PointerMode::Auto => {
                self.classify_wheel(delta, px);
                ctrl_key || self.detected_mouse
            }
        };
        if zoom {
            // A zoom never coasts, and stops a coast under way.
            self.kinetic.stop();
            // Zoom about the cursor. A trackpad pinch and a mouse notch both land
            // here; only `dy` drives the factor.
            let factor = (-f64::from(px.dy.get()) * ZOOM_PER_WHEEL_PX).exp();
            self.cam.zoom_about(self.last_pointer, factor);
        } else {
            // Two-finger scroll pans the 2D plane in both axes. The model
            // applies the step one for one, and lets a glide coast once it
            // pauses; a notch never coasts (`KINETIC-002`, `KINETIC-009`).
            let now = self.clock_ms();
            super::kinetic::pan_by_wheel(&mut self.cam, &mut self.kinetic, px, delta.mode(), now);
        }
        self.queue_viewport();
        true
    }

    /// Classify the device behind a wheel event while in `Auto` mode, and
    /// update the latched guess (`detected_mouse`). Three signals, strongest
    /// first. A line or page step is a classic wheel, so a mouse. A non-zero `dx` is a two-axis trackpad glide. A large `|dy|` with
    /// no `dx` is a discrete mouse notch. An ambiguous fine glide leaves the
    /// latch unchanged (`toile-viewport`).
    fn classify_wheel(&mut self, delta: WheelDelta, px: CssVector) {
        if let Some(device) = wheel_device(delta, px) {
            self.detected_mouse = device == WheelDevice::Mouse;
        }
    }

    fn apply_outcome(&mut self, out: ToolOutcome) {
        match out {
            ToolOutcome::None => {}
            ToolOutcome::SelectionChanged(ids) => {
                self.selection = ids;
                // The selection is drawn on the overlay alone (`TOILE-UI-271`).
                self.overlay_dirty = true;
            }
            ToolOutcome::Command(mut cmd) => {
                // A Sticky / Text placement opens the inline editor on the new
                // element once its id comes back via `ElementCreated`
                // (`toile-text-editing`). The token ties the reply to this
                // command, so no other create can claim it (`TOILE-UI-194`).
                if let ToileCommand::CreateElement {
                    kind: ElementKind::Sticky | ElementKind::Text,
                    correlate,
                    ..
                } = &mut cmd
                {
                    let token = self.next_create_token;
                    self.next_create_token = token.next();
                    *correlate = Some(token);
                    self.edit_open_on_create = Some(token);
                    // A placement is a one-shot action. Revert to Select. The
                    // just-created note then carries the selection and is ready
                    // to edit, rather than the next click dropping another note
                    // (`toile-tools` / feedback). The editor opens on the new
                    // id when `ElementCreated` lands.
                    self.set_tool(Tool::Select);
                }
                // A transform commit (release of a move, resize or rotate)
                // keeps its optimistic preview until the recomputed frame
                // lands. The object then holds its released geometry instead
                // of snapping back for a frame.
                if matches!(
                    &cmd,
                    ToileCommand::MoveElements { .. }
                        | ToileCommand::ResizeElement { .. }
                        | ToileCommand::RotateElement { .. }
                ) {
                    self.gesture_commit_pending = true;
                }
                self.send_edit(cmd);
            }
            // The comment tool dropped a thread (`TOILE-UI-042`): open the
            // composer where the click landed, so the user types there
            // (`TOILE-UI-307`). The post writes the thread, and nothing before
            // it: a thread starts with its first message.
            ToolOutcome::Comment(anchor) => {
                let at = self.css_point(self.last_pointer);
                self.open_composer(CommentDraft { anchor, at });
                // A drop is a one-shot action, like a sticky placement.
                self.set_tool(Tool::Select);
            }
        }
        // The machine's live state yields the overlay preview on each event.
        self.preview = self.machine.preview();
        self.rebuild_gesture_overrides();
    }

    /// Rebuild the live gesture overrides from the machine's in-flight
    /// transform (`toile-tools`). Each resized, rotated or moved element paints
    /// at its preview bounds and rotation, and keeps its committed opacity and
    /// fill. Empty when no gesture is in flight, after a release or a cancel,
    /// which clears the preview.
    ///
    /// The content pass paints the overrides, so a rebuild that held or left
    /// one repaints the content (`TOILE-UI-271`).
    fn rebuild_gesture_overrides(&mut self) {
        let live = self.machine.gesture_overrides(&self.display);
        // No live gesture: clear -- unless a commit is pending, where the
        // overrides are the optimistic post-release preview held until the new
        // frame lands.
        if live.is_empty() {
            if !self.gesture_commit_pending && !self.gesture_overrides.is_empty() {
                self.gesture_overrides.clear();
                self.needs_full = true;
            }
            return;
        }
        self.needs_full = true;
        // A fresh gesture supersedes any optimistic post-commit hold.
        self.gesture_commit_pending = false;
        self.gesture_overrides.clear();
        for (id, bounds, rotation) in live {
            let (opacity, fill) = self
                .find_element(&id)
                .map(|e| (e.style.opacity, e.style.fill))
                .unwrap_or((1.0, None));
            self.gesture_overrides.insert(
                id.clone(),
                AnimatedElement {
                    id,
                    bounds,
                    rotation,
                    opacity,
                    fill,
                },
            );
        }
    }

    // -- Nested document embeds (`toile-embeds`) --------------------

    /// The visible `doc_embed`s in screen space, for `FrameHost`
    /// (`toile-embeds`, "Render").
    /// It owns the child `AppView`s. It paints each one clipped to
    /// `screen_rect` over this view's content pass. For the focused one it maps
    /// canvas input into the child and calls the child's own `handle_input`.
    /// This view can do neither, because both need `AppView`, which it cannot
    /// name (ARCH P2).
    pub fn visible_embeds(&self) -> Vec<embed::VisibleEmbed> {
        embed::visible_embeds(&self.display, &self.cam, self.embed_focus.current())
    }

    /// The web `embed`s in screen space, for the shell to place its frames over
    /// (`TOILE-087`, `TOILE-088`). A web page is DOM, and the canvas has no way
    /// to hold one. The board paints its placeholder box and publishes where
    /// the page goes, exactly as it publishes its nested documents.
    pub fn visible_web_embeds(&self) -> Vec<embed::VisibleWebEmbed> {
        embed::visible_web_embeds(&self.display, &self.cam, self.embed_focus.current())
    }

    /// Map a screen point into the focused embed's child pixel space, if the
    /// focused element is still in the culled frame. `FrameHost` calls this to
    /// rewrite pointer coordinates before handing an event to the child.
    pub fn embed_to_child_space(&self, screen: Point) -> Option<Point> {
        let id = self.embed_focus.current()?;
        let elem = self.find_element(id)?;
        Some(embed::to_child_space(elem, &self.cam, screen))
    }

    /// The `doc_embed` element currently holding input focus
    /// (`toile-embeds`). `None` when the board itself has focus, or when the
    /// focused element is not an embed.
    ///
    /// The **element**, not the document it points at. Two embeds of one
    /// document are two child views, each sized to its own box. Only the one
    /// under the pointer may take the event (`TOILE-UI-097`).
    pub fn focused_child(&self) -> Option<ElementId> {
        let id = self.embed_focus.current()?;
        matches!(self.find_element(id)?.geom, DisplayGeom::DocEmbed { .. }).then(|| id.clone())
    }

    /// The child the last `handle_input` routed into, for `FrameHost` to forward
    /// the (coordinate-remapped) event to (`toile-embeds`, "Input
    /// focus"). Cleared at the start of every `handle_input`, so it reflects only
    /// the latest event.
    pub fn child_route(&self) -> Option<ElementId> {
        self.child_route.clone()
    }

    /// Whether `screen` lands inside the focused embed's (rotated) quad. Used to
    /// decide that a pointer event belongs to the child rather than the board.
    fn screen_in_focused_embed(&self, screen: Point, id: &ElementId) -> bool {
        let Some(e) = self.find_element(id) else {
            return false;
        };
        let world = self.cam.screen_to_world() * screen;
        e.bounds.contains(e.transform.inverse() * world)
    }

    /// Decide whether the just-arrived event belongs to the focused embed `id`,
    /// and so goes to the child instead of the board (`toile-embeds`, "Input
    /// focus"). A keystroke, a text run and a composition go to the child while
    /// it holds focus. `Escape` never does, because the board needs it to blur
    /// (see `handle_key`). A pointer event goes to the child only while the
    /// cursor is inside its quad. A click that lands outside still returns
    /// focus to the board.
    ///
    /// A wheel carries no position of its own, so the last pointer position
    /// binds it (`TOILE-UI-329`). Capturing it unconditionally left the board
    /// impossible to pan or zoom while any embed held focus. The wheel scrolled
    /// the child wherever the cursor was, and only a click outside gave the
    /// board its own navigation back.
    fn route_to_child(&self, ev: &InputEvent, id: &ElementId) -> bool {
        embed::routes_to_child(ev, self.last_pointer, |p| {
            self.screen_in_focused_embed(p, id)
        })
    }

    /// A press landed at `world` / `screen`: arm a possible focus grant if it is
    /// over an unfocused `doc_embed`. Consumes nothing -- the board's own
    /// gesture proceeds regardless, and the release settles the grant.
    fn arm_embed_press(&mut self, world: Point, screen: Point) {
        let hit = embed::embed_at(&self.display, world).map(|e| &e.id);
        self.embed_focus.on_press(hit, screen);
    }

    /// A release landed at `screen`: settle the armed press (a click focuses the
    /// embed, a release elsewhere returns focus to the board). The click that
    /// focuses does not reach the child (`toile-embeds`, "Input focus"). So
    /// entering a document never lands a stray edit on the first cell it
    /// happens to hit.
    fn settle_embed_press(&mut self, screen: Point) {
        if self.embed_focus.on_release(screen) {
            self.needs_full = true;
        }
    }

    /// Return focus to the board (`Escape`, a document swap). Reports whether
    /// anything held focus.
    fn blur_embed(&mut self) -> bool {
        let had = self.embed_focus.blur();
        if had {
            self.needs_full = true;
        }
        had
    }

    /// Open every `doc_embed` that entered the culled set and close those that
    /// have stayed out past the debounce (`toile-embeds`, "Opening the
    /// child"). Driven from `plan_repaint`, the one per-frame hook carrying the
    /// frame clock.
    fn drive_embed_lifecycle(&mut self, now_ms: ambre_core::PageTime) {
        // The session set records only what the queue actually took, so the
        // next frame offers a refused command again rather than losing it
        // (`TOILE-UI-338`, `TOILE-UI-339`). `fatal` is the one case that
        // reports success on a send nobody made. The view is tearing down and
        // no later frame will run, so retrying for ever would be the leak.
        let (to_logic, fatal) = (&self.to_logic, self.fatal);
        self.embed_sessions.sync(&self.display, now_ms, |cmd| {
            send_session(to_logic, fatal, cmd)
        });
        // Focus cannot outlive the element that held it. An embed a peer
        // deleted, or one scrolled far enough away to close, leaves the board
        // with nothing to route input into.
        let present: Vec<ElementId> = self
            .display
            .nodes
            .iter()
            .filter_map(|n| match n {
                DisplayNode::Element(e) => Some(e.id.clone()),
                _ => None,
            })
            .collect();
        if self.embed_focus.retain_if(|id| present.contains(id)) {
            self.needs_full = true;
        }
    }

    /// Close every child session this view opened (`toile-embeds`:
    /// "Closing the Toile closes every child immediately, no debounce").
    fn close_all_embeds(&mut self) {
        let (to_logic, fatal) = (&self.to_logic, self.fatal);
        self.embed_sessions
            .close_all(|cmd| send_session(to_logic, fatal, cmd));
        self.embed_focus.blur();
    }

    /// The time on the frame clock's timeline, for an input that carries
    /// none.
    fn clock_ms(&self) -> ambre_core::PageTime {
        #[cfg(target_arch = "wasm32")]
        if let Some(perf) = web_sys::window().and_then(|w| w.performance()) {
            return ambre_core::PageTime(perf.now());
        }
        self.frame_now_ms
    }

    /// The pick floor in world units at the current zoom (`TOILE-UI-225`).
    fn pick(&self) -> f64 {
        super::hit::pick_floor(&self.cam)
    }

    /// The topmost element under a world point.
    fn element_at(&self, world: Point) -> Option<ElementId> {
        hit_topmost(&self.display.nodes, world, self.pick())
    }

    /// A finger went down. It stops any motion left. On the empty board with
    /// the select tool, or with the hand, it pans (`TOILE-UI-378`);
    /// otherwise it acts as the mouse.
    pub fn touch_down(&mut self, at: ambre_core::ViewPoint, now_ms: ambre_core::PageTime) -> bool {
        self.kinetic.stop();
        let screen = on_screen(at);
        let world = self.cam.screen_to_world() * screen;
        let grabbable = self.element_at(world).is_some()
            || self
                .machine
                .hover_handle(&self.selection, &self.display, &self.cam, screen)
                .is_some();
        let role = super::kinetic::finger_role(self.machine.tool(), grabbable);
        self.finger = Some(role);
        if role == FingerRole::Pans {
            self.last_pointer = screen;
            self.kinetic.press(at, now_ms);
            return true;
        }
        self.handle_input(&InputEvent::PointerDown {
            at,
            button: PointerButton::Primary,
            shift: false,
            ctrl: false,
        })
    }

    /// A finger moved: a panning finger drags the board past the tap slop,
    /// any other acts as the mouse.
    pub fn touch_move(&mut self, at: ambre_core::ViewPoint, now_ms: ambre_core::PageTime) -> bool {
        match self.finger {
            Some(FingerRole::Pans) => {
                self.last_pointer = on_screen(at);
                if let Some(step) = self.kinetic.sample(at, now_ms) {
                    super::kinetic::move_view(&mut self.cam, step);
                    self.queue_viewport();
                }
                true
            }
            Some(FingerRole::ActsAsMouse) => self.handle_input(&InputEvent::PointerMove { at }),
            None => false,
        }
    }

    /// A finger lifted. A pan released with speed keeps the board moving; a
    /// finger that never left the tap slop is a click where it went down.
    pub fn touch_up(&mut self, at: ambre_core::ViewPoint, now_ms: ambre_core::PageTime) -> bool {
        match self.finger.take() {
            Some(FingerRole::Pans) => {
                if let Release::Tap { at } = self.kinetic.release(now_ms) {
                    self.handle_input(&InputEvent::PointerDown {
                        at,
                        button: PointerButton::Primary,
                        shift: false,
                        ctrl: false,
                    });
                    self.handle_input(&InputEvent::PointerUp {
                        at,
                        button: PointerButton::Primary,
                        ctrl: false,
                    });
                }
                true
            }
            Some(FingerRole::ActsAsMouse) => self.handle_input(&InputEvent::PointerUp {
                at,
                button: PointerButton::Primary,
                ctrl: false,
            }),
            None => false,
        }
    }

    /// The browser cancelled the finger (`KINETIC-010`). A pan ends with no
    /// click and no flick. A finger acting as the mouse ends its gesture where
    /// it last was, as a release would (`KINETIC-011`).
    pub fn touch_cancel(&mut self) -> bool {
        match self.finger.take() {
            Some(FingerRole::Pans) => {
                self.kinetic.cancel();
                true
            }
            Some(FingerRole::ActsAsMouse) => self.handle_input(&InputEvent::PointerUp {
                at: view_point(self.last_pointer),
                button: PointerButton::Primary,
                ctrl: false,
            }),
            None => false,
        }
    }

    /// Pan the board under a gesture held at an edge, once a frame, and let
    /// the gesture follow (`TOILE-UI-380`). The first tick only starts the
    /// clock.
    fn step_edge_pan(&mut self, now_ms: ambre_core::PageTime) {
        let held = self.machine.is_active() || self.arrow_drag.is_some();
        let speed = held.then(|| super::kinetic::edge_pan_speed(&self.cam, self.last_pointer));
        let Some(step) = self.edge.tick(speed, now_ms) else {
            return;
        };
        super::kinetic::move_view(&mut self.cam, step);
        self.queue_viewport();
        self.handle_pointer_move(self.last_pointer);
    }

    /// Follow the link under a primary press, if any (`toile-links`). The badge
    /// always follows. The element follows only when it asks to: a modifier, or
    /// presentation mode, where editing is off.
    fn follow_link_under(&mut self, screen: Point, mods: Modifiers) -> bool {
        let world = self.cam.screen_to_world() * screen;
        let target = self.badge_at(screen).or_else(|| {
            link::wants_follow(mods, self.present)
                .then(|| link::link_at(&self.display, world, self.pick()).map(|(_, l)| l))
                .flatten()
        });
        let Some(link) = target else {
            return false;
        };
        self.follow_link(LinkAction::of(&link));
        true
    }

    /// Drop what editing left on screen as a presentation begins
    /// (`TOILE-UI-083`). An open editor commits on the next paint, an embed
    /// gives focus back, and a gesture in flight ends with no write.
    fn leave_editing(&mut self) {
        self.request_close_editor();
        self.blur_embed();
        self.machine.cancel();
        self.preview = None;
        self.gesture_overrides.clear();
        self.panning = None;
        self.selection.clear();
        self.arrow = None;
        self.arrow_drag = None;
    }

    /// The input a presentation still takes (`TOILE-UI-083`). The wheel moves
    /// the camera, a primary press follows a link, and a release or a move
    /// ends what began before. Every other input reaches no board handler.
    fn present_input(&mut self, ev: &InputEvent) -> bool {
        match ev {
            InputEvent::Wheel { delta, ctrl_key } => self.handle_wheel(
                *delta,
                Modifiers::from_event(*ctrl_key, false, false, false),
            ),
            InputEvent::PointerDown {
                at,
                button,
                shift,
                ctrl,
            } => {
                let screen = on_screen(*at);
                self.last_pointer = screen;
                if *button == PointerButton::Primary {
                    self.follow_link_under(
                        screen,
                        Modifiers::from_event(*ctrl, false, *shift, false),
                    );
                }
                true
            }
            InputEvent::PointerMove { at } => self.handle_pointer_move(on_screen(*at)),
            InputEvent::PointerUp { at, .. } => self.handle_pointer_up(on_screen(*at)),
            InputEvent::KeyUp { key } => self.handle_key_up(key),
            InputEvent::DoubleClick { .. } => true,
            InputEvent::Key { .. }
            | InputEvent::Text { .. }
            | InputEvent::CompositionUpdate { .. }
            | InputEvent::CompositionEnd { .. } => false,
        }
    }

    fn handle_pointer_down(
        &mut self,
        screen: Point,
        button: PointerButton,
        mods: Modifiers,
    ) -> bool {
        self.last_pointer = screen;
        // A press in the canvas returns keyboard focus to the transparent text
        // pipe (Trame parity, `trame-cell-editing`). After the user touches the
        // DOM chrome, for example to pick a sticky colour in the palette, focus
        // sits on a chrome button. Without this, the board would feel one click
        // behind: the first canvas click would only refocus, and the second
        // would act. The gesture still runs in this same call, so a single
        // click both refocuses and places.
        self.text_input.focus();
        // Right button: select under the cursor and publish the menu scope
        // (`toile-menus`). The DOM `contextmenu` listener opens the menu;
        // pointerdown fires first, so the scope signal is fresh when the entry
        // `Memo` recomputes.
        if button == PointerButton::Secondary {
            self.select_all_pending = false;
            self.handle_right_click(screen);
            return true;
        }
        // Follow a link (`toile-links`), before any gesture. A click on the
        // badge always follows. A click on the element follows only when it
        // asks to: a modifier, or presentation mode, where editing is off.
        // A plain click with the select tool falls through and selects, which is
        // what keeps a linked element editable (`TOILE-112`).
        if button == PointerButton::Primary && self.follow_link_under(screen, mods) {
            return true;
        }
        // Space-drag or Hand tool or middle button: begin a pan.
        if self.space_down
            || self.machine.tool() == Tool::Hand
            || button == PointerButton::Auxiliary
        {
            self.panning = Some(screen);
            return true;
        }
        // A pin answers before the board does (`TOILE-UI-082`): a click on it
        // opens its thread, and the element under it stays unselected
        // (`TOILE-UI-308`). Only the Select tool picks pins -- a creation tool
        // still places where it clicks.
        if let comments::BoardPress::Thread(thread) = comments::press_target(
            self.machine.tool() == Tool::Select,
            button,
            &self.display.pins,
            &self.cam.world_to_screen(),
            screen,
        ) {
            self.open_thread(thread);
            self.publish_cursor(screen);
            return true;
        }
        let world = self.cam.screen_to_world() * screen;
        // Arm a possible embed focus (`toile-embeds`). Consumes nothing: the
        // board's own gesture runs below either way, and the release settles
        // the grant. A drag from here moves the embed instead of entering it.
        self.arm_embed_press(world, screen);
        // Capture the selection before the click settles it, to detect a press on
        // an already-selected element (the click-to-edit trigger,
        // `toile-text-editing`).
        // A press on an endpoint handle of the selected arrow starts a re-aim
        // and consumes the event. The board's own gesture must not also run
        // (`toile-connectors`).
        if button == PointerButton::Primary
            && self.machine.tool() == Tool::Select
            && self.grab_arrow_handle(world)
        {
            self.publish_cursor(screen);
            self.needs_full = true;
            return true;
        }
        // A press that the tool takes changes the selection. A select-all
        // answer that comes after it must not overwrite it (`TOILE-UI-396`).
        self.select_all_pending = false;
        let prev_selection = self.selection.clone();
        let out = self.machine.pointer_down(
            world,
            screen,
            &self.selection,
            &self.display,
            &self.cam,
            self.group_scope.as_ref(),
        );
        self.apply_outcome(out);
        // Click-to-edit (Miro-style, `toile-text-editing`): a left press on a
        // Sticky / Text element that was ALREADY the sole selection arms an edit.
        // A first click only selects, and arms nothing, because the element did
        // not carry the selection before. A second click enters edit on
        // release, unless a drag turns it into a move.
        //
        // Settle the arrow selection against what the click landed on. An
        // element always wins, because the user sees it on top. An arrow is
        // selected only where the press found nothing else. A prior element
        // selection does not stop it, and the arrow empties it
        // (`TOILE-UI-350`, `TOILE-UI-354`).
        if button == PointerButton::Primary && self.machine.tool() == Tool::Select {
            self.arrow = if self.machine.pressed_empty_board() {
                label::connector_at(&self.display.nodes, world, self.pick())
            } else {
                None
            };
            if self.arrow.is_some() {
                self.selection.clear();
            }
        }
        self.click_edit_candidate = (button == PointerButton::Primary
            && self.machine.tool() == Tool::Select)
            .then(|| match self.selection.as_slice() {
                [id] if *prev_selection == *self.selection && self.is_text_editable(id) => {
                    Some((id.clone(), screen))
                }
                _ => None,
            })
            .flatten();
        self.publish_cursor(screen);
        self.overlay_dirty = true;
        true
    }

    // -- The selected arrow (`toile-connectors`) ------------------------------

    /// Start a re-aim when `world` presses an endpoint handle of the selected
    /// arrow. The handle radius is a screen distance, so it is taken back into
    /// world units by the zoom. A handle is equally easy to grab at any scale.
    fn grab_arrow_handle(&mut self, world: Point) -> bool {
        let Some(id) = self.arrow.clone() else {
            return false;
        };
        let Some(path) = arrow::path_of(&self.display, &id) else {
            return false;
        };
        let tol = arrow::HANDLE_R / self.cam.zoom.get();
        let Some(end) = arrow::handle_at(path, world, tol) else {
            return false;
        };
        // The preview runs from the end that stays put to the pointer, which is
        // the same rubber band the connector tool draws.
        let anchored = match arrow::endpoints(path) {
            Some((from, to)) => match end {
                End::From => to,
                End::To => from,
            },
            None => world,
        };
        self.preview = Some(Preview::Connector {
            from: anchored,
            to: world,
        });
        self.arrow_drag = Some((id, end));
        true
    }

    /// Drag the grabbed endpoint. Returns whether a re-aim is in flight.
    fn drag_arrow_handle(&mut self, world: Point) -> bool {
        if self.arrow_drag.is_none() {
            return false;
        }
        if let Some(Preview::Connector { to, .. }) = self.preview.as_mut() {
            *to = world;
        }
        self.needs_full = true;
        true
    }

    /// Finish a re-aim: the endpoint moves to the element under the pointer. A
    /// release over empty board leaves the arrow where it was rather than
    /// breaking it. Returns whether a re-aim was in flight.
    fn release_arrow_handle(&mut self, world: Point) -> bool {
        let Some((id, end)) = self.arrow_drag.take() else {
            return false;
        };
        self.preview = None;
        if let Some(cmd) = arrow::retarget_on_release(&id, end, &self.display, world, self.pick()) {
            self.send_edit(cmd);
        }
        self.needs_full = true;
        true
    }

    /// Whether an element id resolves to an inline-editable Sticky / Text element.
    fn is_text_editable(&self, id: &ElementId) -> bool {
        self.find_element(id)
            .is_some_and(|e| matches!(e.kind, ElementKind::Sticky | ElementKind::Text))
    }

    // -- Context menu bridge (`toile-menus`) -------------------------

    /// React to a right-click. Resolve the topmost hit to its group-selection
    /// target (`toile-menus`) and, unless it is already part of the current
    /// selection, select just it. An empty hit clears the selection (empty-canvas
    /// menu). Then publish the selection scope for the DOM menu. Emits no command
    /// and starts no gesture.
    fn handle_right_click(&mut self, screen: Point) {
        let world = self.cam.screen_to_world() * screen;
        match self.element_at(world) {
            Some(hit) => {
                let target = resolve_group_target(hit, &self.display, self.group_scope.as_ref());
                if !self.selection.contains(&target) {
                    self.selection = vec![target];
                }
            }
            None => self.selection.clear(),
        }
        // The context menu addresses elements only, so a right-click drops the
        // arrow rather than offering entries that would do nothing to it
        // (`toile-connectors` section 6).
        self.arrow = None;
        self.overlay_dirty = true;
        self.publish_ctx_scope();
    }

    /// Publish the current selection scope to the DOM context menu
    /// (`toile-menus`). Wasm-only (leptos signal); a host no-op keeps
    /// the input path `cfg`-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn publish_ctx_scope(&self) {
        let scope = super::menu::ctx_scope_for(
            &self.selection,
            self.group_scope.as_ref(),
            &self.selection_data,
            &self.selection_framed,
            &self.display,
        );
        self.ctx_scope.set(scope);
        // The style inspector shares the same publish point
        // (`toile-menus`): both are pure functions of the
        // selection x its whole data from Logic.
        self.style_model.set(match self.arrow.as_ref() {
            Some(c) if self.selection.is_empty() => {
                super::style::arrow_style_scope(c, &self.display)
            }
            _ => super::style::style_scope_for(&self.selection, &self.selection_data),
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_ctx_scope(&self) {}

    /// Tell the keyboard pipe whether a key on it is typed text
    /// (`TOILE-UI-393`). It is while an editor is open, or while a document
    /// embedded in the board holds the focus. Otherwise a bare key arms a tool.
    /// An embedded board leaves the pipe to the document around it.
    fn sync_pipe_typing(&self) {
        if self.embedded {
            return;
        }
        let typing = self.editor.is_some() || self.embed_focus.current().is_some();
        if typing != self.text_input.typing() {
            self.text_input.set_typing(typing);
        }
    }

    /// Whether a key on the keyboard pipe is typed text (`TOILE-UI-393`).
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn pipe_typing(&self) -> bool {
        self.text_input.typing()
    }

    /// Announce the selection through the shared live region (`CHROME-023`).
    /// A lone element waits for its data, which carries its kind and text.
    fn announce_selection(&self) {
        if let Some(text) =
            super::a11y::selection_announcement(&self.selection, &self.selection_data)
        {
            self.text_input.announce(&text);
        }
    }

    /// Clear the status-bar aggregates (`toile-status-bar`) on an empty
    /// selection. Wasm-only (leptos signal); a host no-op keeps `plan_repaint`
    /// cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn clear_sel_stats(&self) {
        self.sel_stats.set(None);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn clear_sel_stats(&self) {}

    /// Drain a context-menu command the shell forwarded and apply it against the
    /// live selection (`toile-status-bar`). Wasm-only; a host no-op keeps
    /// `update` cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn drain_menu_cmd(&mut self) {
        let Some(id) = self.menu_cmd.get_untracked() else {
            return;
        };
        self.menu_cmd.set(None);
        self.apply_menu_command(id);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_menu_cmd(&mut self) {}

    /// Drain a whole-style edit the inspector emitted and apply it to the live
    /// selection as one `RestyleElements` (`toile-styles` /
    /// `toile-styles`). The DOM inspector cannot name the selection, so
    /// the view attaches it here. Wasm-only; a host no-op keeps `update`
    /// cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn drain_restyle(&mut self) {
        let Some(style) = self.restyle.get_untracked() else {
            return;
        };
        self.restyle.set(None);
        self.apply_restyle(style);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_restyle(&mut self) {}

    /// Apply a whole style from the inspector to the live selection, locked
    /// members included: the refusal belongs to the write (`TOILE-UI-182`).
    /// The selection itself is left as it was (`TOILE-UI-176`).
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(dead_code, reason = "the inspector signal that calls it is wasm-only")
    )]
    fn apply_restyle(&mut self, style: ambre_bus::app::toile::ElementStyle) {
        if self.selection.is_empty() {
            // An arrow takes the same edit through its own command
            // (`toile-connectors`).
            if let Some(id) = self.arrow.clone() {
                self.send_edit(ToileCommand::RestyleConnector { id, style });
                self.needs_full = true;
            }
            return;
        }
        self.send_edit(ToileCommand::RestyleElements {
            ids: self.selection.clone(),
            style,
        });
        self.needs_full = true;
    }

    /// Republish the floating selection toolbar's placement + flags
    /// (`toile-styles`). Transform-synced: recomputed every frame
    /// from the live selection x camera so the bar tracks pan/zoom. Hidden
    /// (`None`) on an empty selection, an active gesture (move/resize/rotate), a
    /// pan, or an open inline editor. Written only on change to avoid needless
    /// reactive churn while idle. Wasm-only; a host no-op keeps `update`
    /// cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn publish_float_bar(&self) {
        let next = self.float_bar_at(canvas_client_offset());
        if self.float_bar.get_untracked() != next {
            self.float_bar.set(next);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_float_bar(&self) {}

    /// The floating bar this frame shows, with the canvas at `offset` in the
    /// page. None while a move, a resize or a rotate is in flight, so the bar
    /// does not follow the preview (`TOILE-UI-170`).
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(dead_code, reason = "the bar signal that reads it is wasm-only")
    )]
    fn float_bar_at(
        &self,
        canvas_origin: ambre_core::CssPoint,
    ) -> Option<super::floatbar::FloatBar> {
        if self.selection.is_empty()
            || self.machine.is_active()
            || self.panning.is_some()
            || self.editor.is_some()
        {
            return None;
        }
        super::floatbar::build_floatbar(
            &self.selection,
            &self.selection_data,
            &self.cam,
            canvas_origin,
        )
    }

    /// Drain a single-field style edit the floating bar emitted (`toile-styles`)
    /// and apply it to the live selection as one whole-register `RestyleElements`
    /// (`toile-styles`). The base is the selection's freshly-derived common style
    /// so the untouched attributes are preserved. Only the emitted field is
    /// overwritten, so back-to-back bar edits never clobber one another. The view
    /// attaches the ids the DOM bar cannot name. Wasm-only; a host no-op keeps
    /// `update` cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn drain_quick_style(&mut self) {
        let Some(edit) = self.quick_style.get_untracked() else {
            return;
        };
        self.quick_style.set(None);
        self.apply_quick_style(edit);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_quick_style(&mut self) {}

    /// Apply one bar edit to the live selection. The base is the selection's
    /// style as it stands now, so two quick edits compose (`TOILE-UI-179`).
    /// The selection itself is left as it was (`TOILE-UI-181`).
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(dead_code, reason = "the bar signal that calls it is wasm-only")
    )]
    fn apply_quick_style(&mut self, edit: super::floatbar::StyleEdit) {
        if self.selection.is_empty() {
            if let Some(id) = self.arrow.clone()
                && let Some(model) = super::style::arrow_style_scope(&id, &self.display)
            {
                let mut style = model.base;
                edit.apply_to(&mut style);
                self.send_edit(ToileCommand::RestyleConnector { id, style });
                self.needs_full = true;
            }
            return;
        }
        let mut style = super::style::style_scope_for(&self.selection, &self.selection_data)
            .map(|m| m.base)
            .unwrap_or_default();
        // Overwrite only the one field the bar changed, on the freshly-derived
        // common style -- host-tested in `ui::floatbar` (no-clobber property).
        edit.apply_to(&mut style);
        self.send_edit(ToileCommand::RestyleElements {
            ids: self.selection.clone(),
            style,
        });
        self.needs_full = true;
    }

    // -- Presence (`toile-presence`) ---------------------------------------

    /// Build this peer's presence sample from live view state: the pointer in
    /// world space, the selection, the in-flight freehand stroke and the
    /// highlighter point. The sample replaces the peer's presence whole on every
    /// receiver, so a field that is over reads as absent rather than as stale
    /// (`TOILE-178`).
    fn presence_sample(&self) -> PresencePayload {
        PresencePayload {
            // The name is stamped by the Logic thread, which holds the session
            // (`TOILE-UI-076`).
            name: None,
            cursor: Some(self.cam.screen_to_world() * self.last_pointer),
            selection: self.selection.clone(),
            // The tail alone: a sample rides the wire on every gesture frame,
            // and a long stroke would resend its whole history each time. What
            // a peer needs is where the pen is now; the finished stroke arrives
            // as a delta when it commits.
            live_stroke: self.machine.live_stroke().map(|pts| {
                pts.iter()
                    .rev()
                    .take(LIVE_STROKE_TAIL)
                    .rev()
                    .cloned()
                    .collect()
            }),
            laser: self.machine.laser(),
            // The presentation broadcast rides its own sample, emitted by the
            // Logic thread on a slide change (`toile-presentation`).
            present_slide: None,
        }
    }

    /// Send this peer's sample if it changed or the heartbeat is due
    /// (`TOILE-UI-073`). Lossy: a dropped sample is corrected by the next one,
    /// and presence must never displace an edit on a full queue.
    fn broadcast_presence(&mut self, now_ms: ambre_core::PageTime) {
        if let Some(payload) = self.presence_out.due(self.presence_sample(), now_ms) {
            // A dropped sample is not re-offered: the next one carries a fresher
            // pointer, and a queue of stale samples is worse than a gap.
            let _ = self.send_lossy(ToileCommand::Presence { payload, now_ms });
        }
    }

    // -- Comments bridge (`toile-comments`) --------------------------------

    /// Open `thread` in the panel: ask Logic for it whole (the board carries no
    /// body, `TOILE-205`) and show the panel (`TOILE-UI-082`).
    fn open_thread(&mut self, thread: ThreadId) {
        self.show_comments_panel();
        self.send_edit(ToileCommand::ReadComments {
            thread: Some(thread),
        });
    }

    /// Re-read what the panel shows after `thread` changed (`TOILE-UI-310`):
    /// the board-wide list always, and the open thread when it is that one. The
    /// panel keeps no answer of its own (`TOILE-UI-023`).
    #[cfg(target_arch = "wasm32")]
    fn reread_comments(&mut self, thread: &ThreadId) {
        let panel = self.comments.panel.get_untracked();
        let showing = self.comments.open.get_untracked().map(|t| t.id);
        self.reread_comments_shown(panel, showing.as_ref(), thread);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn reread_comments(&mut self, _thread: &ThreadId) {}

    /// The re-read itself, over what the panel shows: whether it is open, and
    /// the thread it shows whole. A closed panel reads nothing.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(dead_code, reason = "the panel signals that feed it are wasm-only")
    )]
    fn reread_comments_shown(
        &mut self,
        panel: bool,
        showing: Option<&ThreadId>,
        thread: &ThreadId,
    ) {
        if !panel {
            return;
        }
        self.send_edit(ToileCommand::ReadComments { thread: None });
        if showing == Some(thread) {
            self.send_edit(ToileCommand::ReadComments {
                thread: Some(thread.clone()),
            });
        }
    }

    /// Show the panel and refresh its board-wide list.
    #[cfg(target_arch = "wasm32")]
    fn show_comments_panel(&mut self) {
        self.comments.panel.set(true);
        self.send_edit(ToileCommand::ReadComments { thread: None });
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn show_comments_panel(&mut self) {}

    /// Publish the board-wide list the panel renders.
    #[cfg(target_arch = "wasm32")]
    fn publish_comment_list(&mut self, list: Vec<ambre_bus::app::toile::CommentSummary>) {
        self.comments.list.set(list);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_comment_list(&mut self, _list: Vec<ambre_bus::app::toile::CommentSummary>) {}

    /// Publish the one thread the panel shows whole.
    #[cfg(target_arch = "wasm32")]
    fn publish_comment_thread(&mut self, thread: ambre_bus::app::toile::CommentThread) {
        self.comments.open.set(Some(thread));
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_comment_thread(&mut self, _thread: ambre_bus::app::toile::CommentThread) {}

    /// Open the composer the comment tool dropped (`TOILE-UI-307`).
    #[cfg(target_arch = "wasm32")]
    fn open_composer(&mut self, draft: CommentDraft) {
        self.comments.draft.set(Some(draft));
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn open_composer(&mut self, _draft: CommentDraft) {}

    /// Drop everything the panel shows: a route change leaves the board whose
    /// threads these are.
    #[cfg(target_arch = "wasm32")]
    fn clear_comments(&mut self) {
        self.comments.panel.set(false);
        self.comments.list.set(Vec::new());
        self.comments.open.set(None);
        self.comments.draft.set(None);
        self.comments.intent.set(None);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn clear_comments(&mut self) {}

    /// Drop everything the deck panel shows: a closed document leaves no deck.
    #[cfg(target_arch = "wasm32")]
    fn clear_deck(&mut self) {
        self.deck.panel.set(false);
        self.deck.view.set(Default::default());
        self.deck.intent.set(None);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn clear_deck(&mut self) {}

    /// Drain the intent the panel or the composer emitted this frame. The DOM
    /// names no camera and holds no command queue, so the view carries each
    /// one out (`TOILE-UI-023`). Wasm-only; a host no-op keeps `update`
    /// cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn drain_comment_intent(&mut self) {
        let Some(intent) = self.comments.intent.get_untracked() else {
            return;
        };
        self.comments.intent.set(None);
        match intent {
            CommentIntent::Post(body) => {
                // An empty post is not a thread: leave the composer open rather
                // than close it on a stray Enter.
                if body.is_empty() {
                    return;
                }
                let Some(draft) = self.comments.draft.get_untracked() else {
                    return;
                };
                self.comments.draft.set(None);
                self.send_edit(ToileCommand::CreateComment {
                    anchor: draft.anchor,
                    body,
                    ts: wall_clock_ms(),
                });
            }
            CommentIntent::Discard => self.comments.draft.set(None),
            CommentIntent::Reply { thread, body } => {
                if body.is_empty() {
                    return;
                }
                self.send_edit(ToileCommand::ReplyComment {
                    thread,
                    body,
                    ts: wall_clock_ms(),
                });
            }
            CommentIntent::Resolve { thread, resolved } => {
                self.send_edit(ToileCommand::ResolveThread { thread, resolved });
            }
            CommentIntent::Focus { thread, anchor } => {
                self.focus_thread(&anchor);
                self.open_thread(thread);
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_comment_intent(&mut self) {}

    // -- Deck bridge (`toile-presentation`) --------------------------------

    /// Show the deck panel and read the board's frames into it.
    #[cfg(target_arch = "wasm32")]
    fn show_deck_panel(&mut self) {
        self.deck.panel.set(true);
        self.send_edit(ToileCommand::ReadDeck);
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(dead_code, reason = "the menu bar that calls it is wasm-only")]
    fn show_deck_panel(&mut self) {}

    /// Re-read the deck after it changed, while the panel is open.
    #[cfg(target_arch = "wasm32")]
    fn reread_deck(&mut self) {
        let panel = self.deck.panel.get_untracked();
        self.reread_deck_shown(panel);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn reread_deck(&mut self) {}

    /// The re-read itself, over whether the panel is open
    /// (`TOILE-FEAT-098`). A closed panel reads nothing.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(dead_code, reason = "the panel signal that feeds it is wasm-only")
    )]
    fn reread_deck_shown(&mut self, panel: bool) {
        if panel {
            self.send_edit(ToileCommand::ReadDeck);
        }
    }

    /// Publish the frames the panel renders.
    #[cfg(target_arch = "wasm32")]
    fn publish_deck(&mut self, view: ambre_bus::app::toile::DeckView) {
        self.deck.view.set(view);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_deck(&mut self, _view: ambre_bus::app::toile::DeckView) {}

    /// Drain the intent the deck panel emitted this frame. Each edit answers
    /// with the whole new order, which is the only shape `SetSlideOrder` takes.
    /// An edit the model refuses (a frame already on the deck, a rank past its
    /// end) sends nothing, so it costs no undo item.
    #[cfg(target_arch = "wasm32")]
    fn drain_deck_intent(&mut self) {
        let Some(intent) = self.deck.intent.get_untracked() else {
            return;
        };
        self.deck.intent.set(None);
        let order: Vec<ambre_bus::app::toile::FrameId> = self
            .deck
            .view
            .get_untracked()
            .slides
            .into_iter()
            .map(|f| f.id)
            .collect();
        let next = match intent {
            DeckIntent::Add(frame) => deck::add(&order, &frame),
            DeckIntent::Remove(at) => deck::remove(&order, at),
            DeckIntent::Shift(at, to) => deck::shift(&order, at, to),
            DeckIntent::Goto(at) => {
                self.send_edit(ToileCommand::GotoSlide(at));
                return;
            }
            // The engine refuses a non-frame and a name that changed nothing,
            // so the panel sends what was typed and reads the answer back
            // (`TOILE-250`, `TOILE-251`).
            DeckIntent::Rename(frame, name) => {
                self.send_edit(ToileCommand::RenameFrame { frame, name });
                return;
            }
        };
        if let Some(order) = next {
            self.send_edit(ToileCommand::SetSlideOrder { order });
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_deck_intent(&mut self) {}

    /// Bring the camera to a thread's anchor (`TOILE-UI-309`). A point anchor is
    /// its own target. An element anchor may sit outside the culled set. The
    /// view asks Logic for its bounds and recentres when the answer lands, the
    /// same round trip as the find focus.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(dead_code, reason = "the panel intent that calls it is wasm-only")
    )]
    fn focus_thread(&mut self, anchor: &CommentAnchor) {
        match anchor {
            CommentAnchor::Point { x, y } => {
                self.cam.center = Point::new(*x, *y);
                self.needs_full = true;
                self.queue_viewport();
            }
            CommentAnchor::Element(id) => {
                self.pending_comment_focus = Some(id.clone());
                self.send_edit(ToileCommand::RequestSelectionStats {
                    ids: vec![id.clone()],
                    kind: StatsRequestKind::CommentFocus,
                });
            }
        }
    }

    /// A canvas screen point in the viewport a `position: fixed` DOM overlay is
    /// placed in, the same mapping as the floating selection toolbar. Both are
    /// CSS pixels (`CANVAS-032`), so only the canvas offset applies.
    #[cfg(target_arch = "wasm32")]
    fn css_point(&self, screen: Point) -> Point {
        let origin = canvas_client_offset();
        Point::new(
            f64::from(origin.x.get()) + screen.x,
            f64::from(origin.y.get()) + screen.y,
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn css_point(&self, screen: Point) -> Point {
        screen
    }

    /// Apply the tool the DOM palette / keyboard selected (`toile-tools`). If the
    /// shared signal differs from the machine's current tool, switch (which
    /// cancels any in-flight gesture and marks a full repaint). Wasm-only; a host
    /// no-op keeps `update` cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn apply_active_tool(&mut self) {
        let tool = self.active_tool.get_untracked();
        if tool != self.machine.tool() {
            self.set_tool(tool);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn apply_active_tool(&mut self) {}

    /// Trade the zoom with the shared `zoom` signal (`toile-tools`). A value
    /// the view did not publish is a preset the chip picked: the camera zooms
    /// to it about the screen center. Then the camera's zoom is published, on
    /// change only, so the readout tracks pan and zoom. Wasm-only; a host
    /// no-op keeps `update` cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn publish_zoom(&mut self) {
        let asked = self.zoom.get_untracked().get();
        if (asked - self.zoom_published.get()).abs() > f64::EPSILON {
            self.cam.zoom_by_center(asked / self.cam.zoom.get());
            self.needs_full = true;
            self.queue_viewport();
        }
        let (lo, hi) = (super::viewport::MIN_ZOOM, super::viewport::MAX_ZOOM);
        let shown = self.cam.zoom.clamped(lo, hi);
        self.zoom_published = shown;
        if self.zoom.get_untracked() != shown {
            self.zoom.set(shown);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_zoom(&mut self) {}

    /// Turn a menu `CommandId` into board mutation(s). `Paste` and `Select all`
    /// are UI-only intents serviced here (the same-session clipboard / local
    /// selection). Every other id is resolved against the selection by
    /// `menu::resolve_menu_command` and emitted. The shell reaches it through
    /// `drain_menu_cmd` on wasm; the host tests call it directly.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(dead_code, reason = "the menu signal that calls it is wasm-only")
    )]
    fn apply_menu_command(&mut self, id: CommandId) {
        use super::menu_bar as mb;

        // Top menu bar / zoom control ids (`toile-menus`). Undo / redo and the
        // presentation entry point resolve to a direct board command via the
        // host-tested `menu_bar_edit`. The zoom actions below are local-transform
        // intents that never reach Logic (only the resulting `SetViewport` cull
        // request does).
        if let Some(cmd) = mb::menu_bar_edit(id) {
            self.send_edit(cmd);
            return;
        }
        match id {
            // The View menu's comments entry (`toile-comments` section 4): open
            // the panel on the whole board. UI-only, like the zoom actions.
            mb::COMMENTS => {
                self.show_comments_panel();
                return;
            }
            // The Present menu's deck entry (`toile-presentation` section 4):
            // open the ordering panel. UI-only, like the comments entry.
            mb::DECK => {
                self.show_deck_panel();
                return;
            }
            mb::ZOOM_IN => {
                self.cam.zoom_by_center(super::viewport::ZOOM_STEP);
                self.needs_full = true;
                self.queue_viewport();
                return;
            }
            mb::ZOOM_OUT => {
                self.cam.zoom_by_center(1.0 / super::viewport::ZOOM_STEP);
                self.needs_full = true;
                self.queue_viewport();
                return;
            }
            mb::ZOOM_RESET => {
                self.cam.reset_zoom();
                self.needs_full = true;
                self.queue_viewport();
                return;
            }
            mb::ZOOM_FIT => {
                // Fit the whole board: ask Logic for the union of every element's
                // bounds (empty ids = whole-board extent) and frame it on reply.
                self.pending_fit = true;
                self.send_edit(ToileCommand::RequestSelectionStats {
                    ids: Vec::new(),
                    kind: StatsRequestKind::Fit,
                });
                return;
            }
            mb::ZOOM_TO_SELECTION => {
                if self.selection.is_empty() {
                    self.notices.publish(NOTHING_TO_ZOOM);
                } else {
                    self.pending_fit = true;
                    self.send_edit(ToileCommand::RequestSelectionStats {
                        ids: self.selection.clone(),
                        kind: StatsRequestKind::Fit,
                    });
                }
                return;
            }
            // Wheel navigation preference (`toile-viewport`): pure view-local
            // state, no viewport change and no Logic round-trip. A forced mode
            // overrides auto-detect.
            mb::POINTER_AUTO => {
                self.pointer_mode = PointerMode::Auto;
                return;
            }
            mb::POINTER_MOUSE => {
                self.pointer_mode = PointerMode::Mouse;
                return;
            }
            mb::POINTER_TRACKPAD => {
                self.pointer_mode = PointerMode::Trackpad;
                return;
            }
            _ => {}
        }

        // Clearing a link needs the selection, which a menu id does not carry,
        // so the view attaches it here (`toile-links`). Setting one goes to the
        // shell, which owns the chooser.
        if id == mb::LINK_CLEAR {
            if let [target] = self.selection.as_slice() {
                self.send_edit(ToileCommand::SetLink {
                    id: target.clone(),
                    link: None,
                });
                self.needs_full = true;
            }
            return;
        }
        if id == super::menu::SELECT_ALL {
            self.select_all();
            return;
        }
        if id == super::menu::PASTE {
            // Menu paste uses the in-memory same-session blob; a cross-tab paste
            // still rides the native Ctrl+V `paste` DOM event
            // (`toile-viewport`).
            if let Some(blob) = self.last_clip.clone() {
                self.paste_clipboard(blob, PasteSource::Own);
            }
            return;
        }
        // A removal the selection's membership cannot honour says so, rather
        // than send a command Logic would answer with nothing (`TOILE-UI-153`).
        if id == super::menu::REMOVE_FROM_FRAME
            && !self
                .selection
                .iter()
                .any(|s| self.selection_framed.contains(s))
        {
            self.notices.publish(NO_FRAME_HOLDS);
            return;
        }
        // With an arrow selected the element selection is empty, so the ordinary
        // resolution would answer nothing. An arrow has a table of its own
        // (`toile-connectors`).
        let cmds = match self.arrow.as_ref() {
            Some(c) if self.selection.is_empty() => arrow::arrow_command_for(id, c),
            _ => match super::menu::resolve_menu_command(
                id,
                &self.selection,
                self.group_scope.as_ref(),
                &self.selection_data,
                &self.display,
            ) {
                Some(cmds) => cmds,
                // An id the resolver does not know is refused, never treated as
                // a view-only intent (`TOILE-UI-152`).
                None => {
                    tracing::warn!("Toile: unknown menu command id {id:?}");
                    return;
                }
            },
        };
        if id == super::menu::ADD_TO_FRAME && cmds.is_empty() {
            self.notices.publish(NO_FRAME_CONTAINS);
            return;
        }
        // A refused command ends the sequence: a cut whose copy went nowhere
        // deletes nothing (`UI-THREAD-045`).
        let mut sent_all = true;
        for cmd in cmds {
            if !self.send_edit(cmd) {
                sent_all = false;
                break;
            }
        }
        // The entry does what its `Del` chord does, so the deleted ids leave
        // the selection here too (`TOILE-UI-052`).
        if id == super::menu::DELETE && sent_all && self.arrow.is_none() {
            self.selection.clear();
        }
        self.needs_full = true;
    }

    /// Select every displayed element (local, never synced). Shared by `Ctrl+A`
    /// and the context-menu "Select all" entry (`toile-selection`).
    fn select_all(&mut self) {
        // The whole board, not the culled viewport (`TOILE-UI-033`): Logic
        // answers the ids, and the answer selects them.
        if self.send_edit(ToileCommand::RequestBoardIds) {
            self.select_all_pending = true;
        }
    }

    fn handle_pointer_move(&mut self, screen: Point) -> bool {
        self.last_pointer = screen;
        if let Some(prev) = self.panning {
            // A mouse held on the board drags it one for one and leaves no
            // flick (`KINETIC-001`).
            self.cam
                .pan_by_screen(ScrollStep::dragged(view_point(prev), view_point(screen)));
            self.panning = Some(screen);
            self.queue_viewport();
            return true;
        }
        if self.arrow_drag.is_some() {
            let world = self.cam.screen_to_world() * screen;
            self.drag_arrow_handle(world);
            self.publish_cursor(screen);
            return true;
        }
        if self.machine.is_active() {
            let world = self.cam.screen_to_world() * screen;
            let out = self.machine.pointer_move(world);
            self.apply_outcome(out);
            self.publish_cursor(screen);
            self.overlay_dirty = true;
            return true;
        }
        // At rest: refresh the directional resize cursor from the hovered edge /
        // corner (`toile-tools`). No repaint -- only the DOM
        // cursor changes.
        self.publish_cursor(screen);
        false
    }

    fn handle_pointer_up(&mut self, screen: Point) -> bool {
        self.last_pointer = screen;
        // Consume any click-to-edit arming this release, whatever path is taken.
        let edit_candidate = self.click_edit_candidate.take();
        if self.panning.take().is_some() {
            return true;
        }
        // Settle the embed focus armed at press time (`toile-embeds`):
        // a click on an embed enters it, a release anywhere else returns focus to
        // the board. Runs before the gesture end below so a click that only
        // focuses still lets the Select tool finish its selection normally.
        self.settle_embed_press(screen);
        if self.arrow_drag.is_some() {
            let world = self.cam.screen_to_world() * screen;
            self.release_arrow_handle(world);
            self.publish_cursor(screen);
            return true;
        }
        let mut consumed = false;
        if self.machine.is_active() {
            let world = self.cam.screen_to_world() * screen;
            let out = self.machine.pointer_up(world, &self.display);
            self.apply_outcome(out);
            self.publish_cursor(screen);
            self.overlay_dirty = true;
            consumed = true;
        }
        // Click-to-edit (Miro-style, `toile-text-editing`): a press on an
        // already-selected Sticky / Text element released without a drag (a click,
        // not a move) enters inline edit. Checked after the gesture settles, so a
        // real move does not trigger it and the selection is still the armed
        // element.
        if let Some((id, down)) = edit_candidate
            && self.editor.is_none()
            && self.selection.as_slice() == std::slice::from_ref(&id)
            && (screen - down).hypot() <= CLICK_SLOP
        {
            self.open_editor(id);
            self.needs_full = true;
            return true;
        }
        consumed
    }

    fn handle_double_click(&mut self, screen: Point) -> bool {
        self.last_pointer = screen;
        let world = self.cam.screen_to_world() * screen;
        // Select tool: a double-click on a Sticky / Text element opens the inline
        // editor on it (`toile-text-editing`), taking precedence over group
        // entering.
        if self.machine.tool() == Tool::Select {
            let nodes = &self.display.nodes;
            let edit = super::tool::double_click_edit(nodes, screen, &self.cam, |hit| {
                self.find_element(hit)
                    .is_some_and(|e| matches!(e.kind, ElementKind::Sticky | ElementKind::Text))
            });
            match edit {
                Some(super::tool::DoubleClickEdit::FrameTitle(frame)) => {
                    self.open_frame_title_editor(frame);
                    return true;
                }
                Some(super::tool::DoubleClickEdit::Body(hit)) => {
                    self.open_editor(hit);
                    return true;
                }
                Some(super::tool::DoubleClickEdit::Caption(c)) => {
                    self.open_label_editor(c);
                    self.needs_full = true;
                    return true;
                }
                None => {}
            }
        }
        let out = self.machine.double_click(world);
        // Select tool: a double-click on a grouped element enters its scope,
        // peeling one nesting level so the next click selects a member
        // (`toile-groups` / `toile-selection`). Only when the
        // tool produced nothing of its own, for example a pen finish.
        if matches!(out, ToolOutcome::None)
            && self.machine.tool() == Tool::Select
            && let Some(hit) = self.element_at(world)
        {
            let target =
                resolve_group_target(hit.clone(), &self.display, self.group_scope.as_ref());
            if is_group(&target, &self.display) {
                self.group_scope = Some(target.clone());
                // Select the member one level inside the freshly-entered scope.
                self.selection = vec![resolve_group_target(hit, &self.display, Some(&target))];
            }
        }
        self.apply_outcome(out);
        self.needs_full = true;
        true
    }

    /// Switch the active tool (driven by the chrome tool palette, `toile-tools`,
    /// or by the view itself, for example Escape reverting to Select,
    /// `toile-tools`). Publishes the shared active-tool signal so the palette's
    /// pressed state follows a view-initiated switch. Without this,
    /// `apply_active_tool` would read the stale signal next frame and re-arm the
    /// old tool.
    pub fn set_tool(&mut self, tool: Tool) {
        self.machine.set_tool(tool);
        self.preview = None;
        self.gesture_overrides.clear();
        self.gesture_commit_pending = false;
        self.publish_active_tool();
        self.needs_full = true;
    }

    /// Mirror the machine's active tool back into the shared signal
    /// (`toile-tools`). Wasm-only (leptos signal); a host no-op
    /// keeps `set_tool` cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn publish_active_tool(&self) {
        if self.active_tool.get_untracked() != self.machine.tool() {
            self.active_tool.set(self.machine.tool());
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_active_tool(&self) {}

    /// Publish the position-dependent canvas cursor (`toile-tools`). An in-flight
    /// resize / rotate / move shows its stable directional / grab cursor. At rest,
    /// a hover over a single selection's edge / corner shows the matching resize
    /// cursor. Otherwise `None` lets the chrome fall back to the tool's base
    /// cursor. Wasm-only (leptos signal); a host no-op keeps callers cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn publish_cursor(&self, screen: Point) {
        let cursor = self.cursor_at(screen);
        if self.cursor.get_untracked() != cursor {
            self.cursor.set(cursor);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_cursor(&self, _screen: Point) {}

    /// The cursor for the pointer at `screen`. Pure over the machine + selection +
    /// camera. Only `publish_cursor` (wasm) consumes it; kept compiled on the host
    /// so the mapping type-checks there too.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn cursor_at(&self, screen: Point) -> Option<super::palette::CanvasCursor> {
        // A single selected element rotates its handles, so the directional resize
        // cursor rotates with them (`toile-tools`).
        let theta = self
            .selection
            .first()
            .map(|id| super::tool::element_rotation(&self.display, id))
            .unwrap_or(ambre_core::Radians::ZERO);
        let css = if let Some(g) = self.machine.gesture_cursor() {
            match g {
                GestureCursor::Handle(h) => super::palette::resize_cursor(h, theta),
                GestureCursor::Move => std::borrow::Cow::Borrowed("grabbing"),
            }
        } else {
            let h = self
                .machine
                .hover_handle(&self.selection, &self.display, &self.cam, screen)?;
            super::palette::resize_cursor(h, theta)
        };
        Some(super::palette::CanvasCursor(css))
    }

    /// Release of a key the board held. `Space` is the only one: it arms the
    /// pan, and the release gives the board back to its tool (`TOILE-UI-256`).
    /// A pan already under the pointer is ended with it, so the drag does not
    /// outlive the key that started it.
    fn handle_key_up(&mut self, key: &str) -> bool {
        if !matches!(key, " " | "Spacebar") {
            return false;
        }
        self.space_down = false;
        if self.machine.tool() != Tool::Hand {
            self.panning = None;
        }
        true
    }

    /// Keyboard shortcuts (`toile-tools`). Modifiers ride the event:
    /// `ctrl || meta` is the command chord (Ctrl on Windows/Linux, Cmd on macOS).
    /// Ctrl+C/X/V are intentionally NOT decoded here -- they ride the native
    /// clipboard events (Phase A). Returns `true` when the key was consumed (host
    /// `preventDefault`).
    fn handle_key(&mut self, key: &str, mods: Modifiers) -> bool {
        let cmd = mods.primary;
        let shift = mods.shift;
        // Command chords first, so a bare-key fallthrough never shadows them.
        if cmd {
            match key {
                // Undo / redo (`toile-undo`). Ctrl+Shift+Z and
                // Ctrl+Y both redo.
                "z" | "Z" if shift => {
                    self.send_edit(ToileCommand::Redo);
                }
                "z" | "Z" => {
                    self.send_edit(ToileCommand::Undo);
                }
                "y" | "Y" => {
                    self.send_edit(ToileCommand::Redo);
                }
                // Duplicate the selection with a small diagonal nudge
                // (`toile-undo`).
                "d" | "D" if !self.selection.is_empty() => {
                    self.send_edit(ToileCommand::DuplicateElements {
                        ids: self.selection.clone(),
                        delta: Vec2::new(DUPLICATE_NUDGE, DUPLICATE_NUDGE),
                    });
                }
                // Group / ungroup the selection (`toile-groups`).
                // One command for the whole selection, so one undo item
                // (`TOILE-094`).
                "g" | "G" if shift => {
                    if let Some(cmd) = super::menu::ungroup_chord(&self.selection) {
                        self.send_edit(cmd);
                    }
                }
                "g" | "G" if !self.selection.is_empty() => {
                    self.send_edit(ToileCommand::GroupElements {
                        ids: self.selection.clone(),
                    });
                }
                // Local select-all over every displayed element (connectors are
                // selected with their endpoints, not directly).
                "a" | "A" => self.select_all(),
                _ => return false,
            }
            self.needs_full = true;
            return true;
        }
        // Type-to-edit: with a single sticky/text selected and no editor open, a
        // printable character enters edit and REPLACES the body (Trame-cell
        // parity, feedback). Double-click edits the existing body instead. Space
        // is excluded (it pans). The character is NOT consumed, so the textarea
        // still emits the `input` event that inserts it over the selected-all
        // body in the now-open editor.
        if key.chars().count() == 1
            && key != " "
            && let Some(id) = self.single_editable_selection()
        {
            self.open_editor(id);
            self.push_intent(EditIntent::SelectAll);
            return false;
        }
        match key {
            " " | "Spacebar" => {
                self.space_down = true;
                true
            }
            "Escape" => {
                // Leaving a focused embed is the whole action: the board's
                // selection survives. Escape steps out one level at a time rather
                // than also clearing what was selected (`toile-embeds`).
                //
                // This arm is reachable while an embed holds focus: `FrameHost`
                // routes every input into the focused child EXCEPT `Escape`,
                // which it hands to this view. Otherwise the key that exits the
                // child would be swallowed by it.
                if self.blur_embed() {
                    return true;
                }
                // Cancel any in-flight gesture and revert an armed creation tool
                // to Select so it stops silently placing elements
                // (`toile-tools`). Republish the shared signal when the
                // tool changed, so the palette follows.
                if self.machine.escape() {
                    self.publish_active_tool();
                }
                self.preview = None;
                self.gesture_overrides.clear();
                self.gesture_commit_pending = false;
                if !self.selection.is_empty() {
                    self.selection.clear();
                }
                // A select-all still in flight would select again once its
                // answer comes (`TOILE-UI-396`).
                self.select_all_pending = false;
                // An arrow deselects on the same Escape, and an in-flight re-aim
                // is abandoned without a write (`toile-connectors`).
                self.arrow = None;
                self.arrow_drag = None;
                // Leave any entered group scope (`toile-groups`):
                // the next click selects whole groups again.
                self.group_scope = None;
                self.needs_full = true;
                true
            }
            "Delete" | "Backspace" => {
                if !self.selection.is_empty() {
                    let ids = std::mem::take(&mut self.selection);
                    self.send_edit(ToileCommand::DeleteElements { ids });
                    self.needs_full = true;
                } else if let Some(id) = self.arrow.take() {
                    self.send_edit(ToileCommand::DeleteConnector { id });
                    self.needs_full = true;
                }
                true
            }
            // Nudge the selection by 1 world unit (10 with Shift),
            // `toile-elements`.
            "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" if !self.selection.is_empty() => {
                let step = if shift { NUDGE_LARGE } else { NUDGE_SMALL };
                let delta = match key {
                    "ArrowUp" => Vec2::new(0.0, -step),
                    "ArrowDown" => Vec2::new(0.0, step),
                    "ArrowLeft" => Vec2::new(-step, 0.0),
                    _ => Vec2::new(step, 0.0),
                };
                self.send_edit(ToileCommand::MoveElements {
                    ids: self.selection.clone(),
                    delta,
                });
                self.needs_full = true;
                true
            }
            _ => false,
        }
    }

    // -- Inline text editor (`toile-text-editing`) -----------------------------

    /// Find an element by id, in the selection's whole data first (off-screen
    /// included) and then the culled set. The editor and the single-selection
    /// shortcuts read the selection data, which survives a pan off screen
    /// (`TOILE-UI-070`, `TOILE-UI-210`).
    fn find_element(&self, id: &ElementId) -> Option<&DisplayElement> {
        self.selection_data
            .iter()
            .find(|e| &e.id == id)
            .or_else(|| {
                self.display.nodes.iter().find_map(|n| match n {
                    DisplayNode::Element(e) if &e.id == id => Some(e),
                    _ => None,
                })
            })
    }

    /// Optimistically set an element's body text in the local display. The
    /// static render then matches a just-committed value before the model
    /// round-trip delivers a fresh viewport (avoids a one-frame flash of the
    /// old body, `toile-text-editing`). The displayed body may carry a peer's
    /// edit since the editor opened on `base`. So the echo is the merge Logic
    /// computes, and those words stay on screen (`TOILE-UI-382`).
    fn patch_element_text(&mut self, id: &ElementId, base: &str, text: &str) {
        if let Some(DisplayNode::Element(e)) = self
            .display
            .nodes
            .iter_mut()
            .find(|n| matches!(n, DisplayNode::Element(e) if &e.id == id))
        {
            match &mut e.geom {
                DisplayGeom::Sticky(t) | DisplayGeom::Text(t) => *t = merge_text(base, t, text),
                _ => {}
            }
        }
    }

    /// The text origin (top-left screen corner + padding) and wrap width in
    /// screen pixels of a world box. Also its full screen rect, for hit tests.
    fn edit_geometry_of(&self, world: Rect) -> (Point, f32, Rect) {
        let sb = world_rect_to_screen(world, &self.cam);
        let pad = InlineEditor::pad(self.cam.zoom);
        let origin = Point::new(sb.x0 + pad, sb.y0 + pad);
        let wrap_w = (sb.width() - 2.0 * pad).max(1.0) as f32;
        (origin, wrap_w, sb)
    }

    /// The edited element's text origin, wrap width and screen rect.
    fn edit_geometry(&self, e: &DisplayElement) -> (Point, f32, Rect) {
        self.edit_geometry_of(e.bounds)
    }

    /// A displayed connector by id (linear over the culled set, like
    /// [`find_element`](Self::find_element)).
    fn find_connector(&self, id: &ConnectorId) -> Option<&DisplayConnector> {
        self.display.nodes.iter().find_map(|n| match n {
            DisplayNode::Connector(c) if &c.id == id => Some(c),
            _ => None,
        })
    }

    /// The screen geometry the active editor writes into, plus the rotation its
    /// overlay rides. An element's box carries the element's angle; a connector
    /// caption's box is the arrow's mid-arc box and never rotates
    /// (`toile-connectors`). `None` when the target has left the culled set.
    fn edit_target_geometry(&self) -> Option<(Point, f32, Rect, ambre_core::Radians)> {
        match self.editor.as_ref()?.target() {
            EditTarget::Element(id) => {
                let e = self.find_element(id)?;
                let (origin, wrap_w, sb) = self.edit_geometry(e);
                // Rotation from the element's own transform, not the culled
                // display, so a pan off screen keeps the editor open
                // (`TOILE-UI-210`).
                let c = e.transform.as_coeffs();
                let rot = ambre_core::Radians::new(c[1].atan2(c[0]));
                Some((origin, wrap_w, sb, rot))
            }
            EditTarget::ConnectorLabel(id) => {
                // A culled arrow is still on the board: the caption keeps the
                // box it last read, until the board says the arrow is gone
                // (`TOILE-UI-386`).
                let live = self
                    .find_connector(id)
                    .and_then(|c| label::label_rect(&c.path));
                if live.is_some() {
                    self.edit_caption_box.set(live);
                }
                let world = live.or(self.edit_caption_box.get())?;
                let (origin, wrap_w, sb) = self.edit_geometry_of(world);
                Some((origin, wrap_w, sb, ambre_core::Radians::ZERO))
            }
            // The title band is screen space, and the editor takes world
            // coordinates, so the band is projected back through the camera
            // (`TOILE-261`). Padding rides outside it, so the caret sits where
            // the painted glyphs did.
            EditTarget::FrameTitle(id) => {
                // A culled frame is still on the board: the editor reads it
                // from its own fetch, until the board says it is gone
                // (`TOILE-UI-390`).
                let e = self
                    .find_element(id)
                    .or_else(|| self.edit_frame.as_ref().filter(|e| &e.id == id))?;
                let band = self.frame_title_band(e)?;
                let pad = InlineEditor::pad(self.cam.zoom);
                let s2w = self.cam.screen_to_world();
                let world = Rect::from_points(
                    s2w * Point::new(band.x0 - pad, band.y0 - pad),
                    s2w * Point::new(band.x1 + pad, band.y1 + pad),
                );
                let (origin, wrap_w, sb) = self.edit_geometry_of(world);
                Some((origin, wrap_w, sb, ambre_core::Radians::ZERO))
            }
        }
    }

    /// The screen-space band a frame's title occupies, or a band of the same
    /// shape when it carries none. An editor has to open on a frame the user is
    /// naming for the first time (`TOILE-261`).
    /// `None` only if the placeholder label were ever empty.
    fn frame_title_band(&self, e: &DisplayElement) -> Option<Rect> {
        let title = match &e.geom {
            DisplayGeom::Frame(t) => t.as_str(),
            _ => "",
        };
        let (label, _) = render::frame_title_label(title);
        render::frame_title_box(world_rect_to_screen(e.bounds, &self.cam), label)
    }

    /// Optimistically set a frame's title in the local display, so the band
    /// does not flash its previous name before the round trip returns.
    fn patch_frame_title(&mut self, id: &ElementId, title: &str) {
        if let Some(DisplayNode::Element(e)) = self
            .display
            .nodes
            .iter_mut()
            .find(|n| matches!(n, DisplayNode::Element(e) if &e.id == id))
            && let DisplayGeom::Frame(t) = &mut e.geom
        {
            *t = title.trim().to_owned();
        }
    }

    /// Optimistically set a connector's caption in the local display, the
    /// counterpart of [`patch_element_text`](Self::patch_element_text), and
    /// merged the same way (`TOILE-UI-382`).
    fn patch_connector_label(&mut self, id: &ConnectorId, base: &str, text: &str) {
        if let Some(DisplayNode::Connector(c)) = self
            .display
            .nodes
            .iter_mut()
            .find(|n| matches!(n, DisplayNode::Connector(c) if &c.id == id))
        {
            c.label = merge_text(base, &c.label, text);
        }
    }

    /// Open the inline editor on a connector's caption (`toile-connectors`).
    /// No-op for an arrow that is not on screen.
    fn open_label_editor(&mut self, id: ConnectorId) {
        let Some(c) = self.find_connector(&id) else {
            return;
        };
        let text = c.label.clone();
        self.edit_caption_box.set(label::label_rect(&c.path));
        self.editor = Some(InlineEditor::open_label(id, &text, self.theme.palette()));
        // A caption is not an element: nothing is selected while it is edited,
        // and the previous selection would otherwise take the next Delete.
        self.selection.clear();
        self.edit_closing = false;
        self.edit_pointer_down = false;
        self.edit_dirty = true;
        self.needs_full = true;
        self.text_input.clear();
        self.text_input.focus();
    }

    /// Open the inline editor on a frame's title (`TOILE-261`). No-op for a
    /// frame that has left the culled set.
    fn open_frame_title_editor(&mut self, id: ElementId) {
        let Some(e) = self.find_element(&id) else {
            return;
        };
        let DisplayGeom::Frame(title) = &e.geom else {
            return;
        };
        let title = title.clone();
        let frame = e.clone();
        self.edit_frame = Some(frame);
        // The selection is cleared below, so the frame is fetched on its own.
        // The answer outlives a pan, and names a frame a peer deleted
        // (`TOILE-UI-390`).
        self.send_edit(ToileCommand::RequestSelectionData {
            ids: vec![id.clone()],
        });
        // The band is a screen size, and the editor's own size is world units
        // scaled by the zoom at paint. Dividing here opens the edited text at
        // the size the band reads today. A zoom during the edit then drifts,
        // which `toile-frames` section 6 records.
        let base_size = (render::FRAME_TITLE_PX / self.cam.zoom.get()) as f32;
        self.editor = Some(InlineEditor::open_frame_title(
            id,
            &title,
            base_size,
            self.theme.palette(),
        ));
        // A title is not the frame: nothing is selected while it is edited, so
        // the next Delete cannot take the frame the user is naming.
        self.selection.clear();
        self.edit_closing = false;
        self.edit_pointer_down = false;
        self.edit_dirty = true;
        self.needs_full = true;
        self.text_input.clear();
        self.text_input.focus();
    }

    /// The single selected Sticky / Text element, if the selection is exactly one
    /// such element (the target of type-to-edit, `toile-text-editing`). `None` for
    /// an empty, multi, or non-text selection.
    fn single_editable_selection(&self) -> Option<ElementId> {
        let [id] = self.selection.as_slice() else {
            return None;
        };
        self.find_element(id)
            .filter(|e| matches!(e.kind, ElementKind::Sticky | ElementKind::Text))
            .map(|_| id.clone())
    }

    /// Open the inline editor on a Sticky / Text element (`toile-text-editing`).
    /// No-op for any other kind or an unknown id.
    fn open_editor(&mut self, id: ElementId) {
        let editor = {
            let Some(e) = self.find_element(&id) else {
                return;
            };
            if !matches!(e.kind, ElementKind::Sticky | ElementKind::Text) {
                return;
            }
            let host = if matches!(e.kind, ElementKind::Sticky) {
                super::walk::TextHost::Sticky
            } else {
                super::walk::TextHost::Text
            };
            let text = match &e.geom {
                DisplayGeom::Sticky(t) | DisplayGeom::Text(t) => t.clone(),
                _ => String::new(),
            };
            InlineEditor::open(e, &text, self.theme.palette(), host)
        };
        self.editor = Some(editor);
        // The edited element is the selection, so leaving edit (Escape) leaves it
        // selected (`toile-text-editing`, feedback) and the style/float chrome
        // tracks it.
        self.selection = vec![id];
        self.edit_closing = false;
        self.edit_pointer_down = false;
        self.edit_dirty = true;
        // Suppress the element's static text in the content pass while editing.
        self.needs_full = true;
        self.text_input.clear();
        self.text_input.focus();
    }

    /// Close an editor whose element or frame the selection answer reports
    /// missing (`TOILE-UI-387`). The body editor is the selection, so the
    /// refetch a board change triggers covers it. A title editor clears the
    /// selection and refetches its frame on its own (`TOILE-UI-390`).
    fn drop_lost_editor(&mut self, missing: &[ElementId]) {
        if self
            .editor
            .as_ref()
            .and_then(|ed| ed.target().element().or(ed.target().frame_title()))
            .is_some_and(|id| missing.contains(id))
        {
            self.close_lost_editor();
        }
    }

    /// Close a caption editor whose arrow the board lost (`TOILE-UI-388`).
    /// The board names the arrows it lost, so one panned out of the frame
    /// keeps its editor (`TOILE-UI-386`).
    fn drop_lost_caption_editor(&mut self, gone: &[ConnectorId]) {
        if self
            .editor
            .as_ref()
            .and_then(|ed| ed.target().connector())
            .is_some_and(|c| gone.contains(c))
        {
            self.close_lost_editor();
        }
    }

    /// Close the editor with no write, and have the shell say why. The
    /// deferred close would name a target the board lost, and the typed text
    /// would vanish in silence.
    fn close_lost_editor(&mut self) {
        self.editor = None;
        self.edit_closing = false;
        self.edit_pointer_down = false;
        // Repaint content: the static pass no longer suppresses a body.
        self.needs_full = true;
        self.notices.publish(EDIT_LOST);
    }

    /// Tell the user when the board refuses the editor's last write because
    /// its target is gone (`TOILE-UI-389`). This catches what the two drops
    /// above cannot see: a title, or a delete that lands before the first
    /// selection answer. It speaks once per write.
    fn notice_refused_edit(&mut self, error: &ToileError) {
        let named = match (&self.edit_written, error) {
            (
                Some(EditTarget::Element(id) | EditTarget::FrameTitle(id)),
                ToileError::NoSuchElement(gone),
            ) => id == gone,
            (Some(EditTarget::ConnectorLabel(id)), ToileError::NoSuchConnector(gone)) => id == gone,
            _ => false,
        };
        if named {
            self.edit_written = None;
            self.notices.publish(EDIT_LOST);
        }
    }

    /// Request a deferred commit + close: the buffer is written back as `SetText`
    /// on the next paint (after a final intent flush).
    fn request_close_editor(&mut self) {
        if self.editor.is_some() {
            self.edit_closing = true;
            self.edit_dirty = true;
        }
    }

    /// Map a screen point into the edited element's local editor frame. `None`
    /// when the point falls outside the element, where a click closes the edit.
    fn edit_local_point(&self, screen: Point) -> Option<Point> {
        let (origin, _w, sb, rot) = self.edit_target_geometry()?;
        // Unrotate the pointer about the element center into its axis-aligned
        // frame, so clicks land on the rotated in-edit text. This mirrors the
        // overlay rotation in `drive_editor` (`toile-text-editing`).
        let p = if rot != ambre_core::Radians::ZERO {
            render::rotate_about(sb.center(), -rot) * screen
        } else {
            screen
        };
        sb.contains(p)
            .then(|| Point::new(p.x - origin.x, p.y - origin.y))
    }

    /// Route an input event to the active editor. Returns `Some(consumed)` when
    /// the editor decides the event's fate, or `None` to let normal handling run
    /// (chords like Ctrl+Z, wheel pan/zoom keep working while editing).
    fn edit_input(&mut self, ev: &InputEvent) -> Option<bool> {
        match ev {
            InputEvent::Text { text } => {
                self.push_intent(EditIntent::Insert(text.clone()));
                Some(true)
            }
            InputEvent::CompositionUpdate { text } => {
                self.push_intent(EditIntent::Compose(text.clone()));
                Some(true)
            }
            InputEvent::CompositionEnd { text } => {
                self.push_intent(EditIntent::ComposeEnd(text.clone()));
                Some(true)
            }
            InputEvent::Key {
                key,
                ctrl,
                alt,
                shift,
                meta,
            } => Some(self.edit_key(key, Modifiers::from_event(*ctrl, *alt, *shift, *meta))),
            // The editor holds no key across frames, so a release means nothing
            // to it. Normal handling runs, where the board lowers its pan flag.
            InputEvent::KeyUp { .. } => None,
            InputEvent::PointerDown { at, .. } => {
                self.last_pointer = on_screen(*at);
                match self.edit_local_point(self.last_pointer) {
                    Some(local) => {
                        self.push_intent(EditIntent::PointerDown(doc_point(local)));
                        self.edit_pointer_down = true;
                    }
                    None => self.request_close_editor(),
                }
                Some(true)
            }
            InputEvent::PointerMove { at } => {
                self.last_pointer = on_screen(*at);
                if self.edit_pointer_down
                    && let Some(local) = self.edit_local_point(self.last_pointer)
                {
                    self.push_intent(EditIntent::PointerDrag(doc_point(local)));
                    return Some(true);
                }
                Some(self.edit_pointer_down)
            }
            InputEvent::PointerUp { .. } => {
                let was = self.edit_pointer_down;
                self.edit_pointer_down = false;
                Some(was)
            }
            InputEvent::DoubleClick { at } => {
                match self.edit_local_point(on_screen(*at)) {
                    Some(local) => self.push_intent(EditIntent::WordAt(doc_point(local))),
                    None => self.request_close_editor(),
                }
                Some(true)
            }
            // Wheel pans/zooms the board under the editor (it follows the element
            // each paint); let the normal handler run. A dropped file is the
            // board's, not the text's: the same fall-through.
            InputEvent::Wheel { .. } => None,
        }
    }

    /// Editing key map. Returns `true` when consumed (host `preventDefault`).
    /// Character-producing keys return `false` so the browser still emits the
    /// `input` event that lands as `InputEvent::Text` (the actual insertion).
    fn edit_key(&mut self, key: &str, mods: Modifiers) -> bool {
        let cmd = mods.primary;
        let shift = mods.shift;
        // Ctrl/Alt+Arrow jumps by word; Ctrl+A selects all inside the editor.
        if cmd {
            match key {
                "a" | "A" => {
                    self.push_intent(EditIntent::SelectAll);
                    return true;
                }
                // Other chords (Ctrl+Z/C/X/V...) are not editor actions: let the
                // board handler / native clipboard have them.
                _ => return false,
            }
        }
        let word = mods.alt; // Alt+Arrow = word motion (Ctrl is reserved for chords).
        match key {
            "Escape" => self.request_close_editor(),
            "Enter" => self.push_intent(EditIntent::Newline),
            "Backspace" => self.push_intent(EditIntent::Backspace),
            "Delete" => self.push_intent(EditIntent::Delete),
            "ArrowLeft" => self.push_intent(EditIntent::Left {
                select: shift,
                word,
            }),
            "ArrowRight" => self.push_intent(EditIntent::Right {
                select: shift,
                word,
            }),
            "ArrowUp" => self.push_intent(EditIntent::Up { select: shift }),
            "ArrowDown" => self.push_intent(EditIntent::Down { select: shift }),
            "Home" => self.push_intent(EditIntent::LineStart { select: shift }),
            "End" => self.push_intent(EditIntent::LineEnd { select: shift }),
            // A character key: do NOT consume, so the textarea emits `input` and
            // the char arrives as `InputEvent::Text`. Returning false also skips
            // the board shortcut handler (guarded by `editor.is_some()`).
            _ => return false,
        }
        true
    }

    /// Queue an editor intent and mark the overlay dirty.
    fn push_intent(&mut self, intent: EditIntent) {
        if let Some(ed) = self.editor.as_mut() {
            ed.push(intent);
            self.edit_dirty = true;
        }
    }

    /// Flush queued intents, render the editor, and perform a deferred commit +
    /// close. Runs in the paint pass, where `font_cx` is available. Returns the
    /// id currently under edit (for content-pass text suppression).
    fn drive_editor<S: ambre_draw::DrawSurface>(
        &mut self,
        surface: &mut S,
        font_cx: &mut FontContext,
    ) {
        self.caret_drawn = None;
        let Some(target) = self.editor.as_ref().map(|ed| ed.target().clone()) else {
            return;
        };
        // Resolve the edited target's screen geometry (may have scrolled out).
        let geom = self.edit_target_geometry();

        match geom {
            Some((origin, wrap_w, sb, rot)) => {
                let zoom = self.cam.zoom;
                let palette = self.theme.palette();
                // Blink the caret on a ~1-second cycle so it visibly signals "you can
                // type here" (`toile-text-editing`). `editor.is_some()` keeps the
                // overlay redrawing every frame, so the toggle animates.
                let caret_on = (self.frame_now_ms.since_load() / CARET_BLINK_MS) as i64 % 2 == 0;
                // The guard above found an editor, and nothing since closed it.
                let Some(ed) = self.editor.as_mut() else {
                    return;
                };
                ed.flush(font_cx, wrap_w, zoom);
                // Always paint the body -- including the closing frame. The
                // content pass already ran this frame with the element's static
                // text suppressed, the editor being still live. Skipping the
                // overlay here would leave one blank frame before the static text
                // returns next frame. Painting the committed text bridges that
                // gap. The caret + IME anchor are for active editing only, so they
                // are gated off while closing.
                let rotated = rot != ambre_core::Radians::ZERO;
                if rotated {
                    surface.push_transform(render::rotate_about(sb.center(), rot));
                }
                // A caption is edited over whatever the arrow crosses, so the
                // box gets the same plate the static caption paints. That plate
                // is at its full width, which is also the wrap width being
                // typed into (`toile-connectors`).
                if target.connector().is_some() {
                    render::paint_label_plate(surface, sb, palette);
                }
                ed.render(surface, origin, palette, caret_on && !self.edit_closing);
                if rotated {
                    surface.pop_transform();
                }
                if !self.edit_closing
                    && let Some(caret) = ed.caret_local()
                {
                    // The editor's local caret, moved to the screen pixels the
                    // pipe wants, so the IME box anchors there (`CANVAS-032`).
                    let spot = ambre_draw::Rect::new(
                        origin.x + caret.x0,
                        origin.y + caret.y0,
                        origin.x + caret.x1,
                        origin.y + caret.y1,
                    );
                    let height = ambre_core::ViewPx::new(spot.height().max(1.0) as f32);
                    self.text_input.set_caret(view_point(spot.origin()), height);
                    self.caret_drawn = Some(spot);
                }
            }
            // Element gone/culled: commit whatever the last flush produced.
            None => self.edit_closing = true,
        }

        if self.edit_closing {
            let committed = self
                .editor
                .as_ref()
                .map(|ed| (ed.committed_text(), ed.opened_text().to_owned()));
            if let Some((Some(text), base)) = committed {
                // Optimistic local update: reflect the committed text in the
                // static pass immediately. Dropping the editor then does not
                // flash the pre-edit body for the frames before the CRDT
                // round-trip returns a fresh viewport (`toile-viewport`). The
                // incoming viewport carries the same text.
                self.edit_written = Some(target.clone());
                match target {
                    EditTarget::Element(id) => {
                        // A locked element's write is refused (`TOILE-033`). The
                        // local echo must not paint a body the write will refuse
                        // (`TOILE-UI-209`): the screen keeps the committed body.
                        // The base lets Logic keep a peer's edit that landed
                        // while the editor was open (`TOILE-UI-381`).
                        let locked = self.find_element(&id).is_some_and(|e| e.locked);
                        if !locked {
                            self.patch_element_text(&id, &base, &text);
                        }
                        self.send_edit(ToileCommand::SetText { id, base, text });
                    }
                    EditTarget::ConnectorLabel(id) => {
                        // The base keeps a peer's caption edit that landed
                        // while the editor was open (`TOILE-263`).
                        self.patch_connector_label(&id, &base, &text);
                        self.send_edit(ToileCommand::SetConnectorLabel { id, base, text });
                    }
                    // A title commits as a rename, and the engine trims it and
                    // refuses a no-op (`TOILE-249`, `TOILE-250`).
                    EditTarget::FrameTitle(id) => {
                        self.patch_frame_title(&id, &text);
                        self.send_edit(ToileCommand::RenameFrame {
                            frame: id,
                            name: text,
                        });
                    }
                }
            }
            self.editor = None;
            self.edit_closing = false;
            self.edit_pointer_down = false;
            // Repaint content so the element's static text returns.
            self.needs_full = true;
        }
    }
}

// -----------------------------------------------------------------------------
// Inherent driver methods dispatched by `AppView` in `ambre-app`.
// -----------------------------------------------------------------------------

impl ToileView {
    pub fn update(&mut self) {
        // Offer the held edits to the queue first: the Logic thread drained
        // while the frame was out. The board stays busy for as long as one
        // command waits (`BUS-011`, `BUS-016`).
        self.flush_outbox();
        loop {
            match self.from_logic.try_recv() {
                Ok(UiEvent::Viewport {
                    event: AppEvent::Toile(te),
                    ..
                }) => self.dispatch(te),
                Ok(UiEvent::FatalError { message }) => self.signal_fatal(&message),
                // Other UiEvent variants (session, CRUD, connection) are not yet
                // consumed by the Toile view; ignore them for now.
                Ok(_) => {}
                Err(ambre_bus::conduit::TryRecvError::Empty) => break,
                Err(ambre_bus::conduit::TryRecvError::Disconnected) => {
                    self.signal_fatal("Logic thread disconnected");
                    break;
                }
            }
        }

        // Apply any context-menu command the shell forwarded this frame
        // (`toile-menus`).
        self.drain_menu_cmd();

        // Apply any style edit the inspector emitted this frame
        // (`toile-styles`).
        self.drain_restyle();

        // Apply any quick-colour pick the floating bar emitted
        // (`toile-styles`).
        self.drain_quick_style();

        // Apply the comment panel's / composer's intent this frame
        // (`toile-comments`).
        self.drain_comment_intent();
        self.drain_dropped_file();
        self.drain_placed_embed();
        self.drain_set_link();
        self.drain_deck_intent();

        // Apply the tool the DOM palette / keyboard selected this frame
        // (`toile-tools`).
        self.apply_active_tool();

        // Publish the live zoom % to the DOM zoom control / status bar
        // (`toile-menus`).
        self.publish_zoom();

        // Republish the floating selection toolbar, transform-synced
        // (`toile-styles`).
        self.publish_float_bar();

        // A freshly-created Sticky / Text element: open its editor once its
        // geometry has landed, keeping the deferred open until it does
        // (`TOILE-UI-195`). A culled-but-created element is not dropped.
        if let Some(id) = self.edit_pending_open.clone()
            && self.find_element(&id).is_some()
        {
            self.edit_pending_open = None;
            self.open_editor(id);
        }

        // First frame: request the initial culled viewport for the boot camera.
        if !self.initial_viewport_sent {
            self.initial_viewport_sent = true;
            self.needs_full = true;
            self.pending_viewport = Some(self.cam.to_bus_viewport());
        }

        // Flush the coalesced viewport request (at most one per frame). A full
        // queue takes none, and the request goes back into the slot. The next
        // frame offers it again: dropped outright, it leaves the board painting
        // the previous culled set until the camera moves (`TOILE-UI-275`,
        // `BUS-024`).
        if let Some(vp) = self.pending_viewport.take()
            && !self.send_lossy(ToileCommand::SetViewport(vp))
        {
            self.pending_viewport = Some(vp);
        }
        if std::mem::take(&mut self.media_retry) {
            self.request_missing_media();
        }

        // Pick up a theme switch from the chrome toggle.
        let theme = self.theme_ctl.get();
        if theme != self.theme {
            self.theme = theme;
            self.needs_full = true;
        }
    }

    /// # Panics
    ///
    /// When the in-flight presentation transition vanishes between the guard
    /// that finds it and the step that advances it. Both run on this thread,
    /// with nothing in between.
    pub fn plan_repaint(&mut self, now_ms: ambre_core::PageTime, hud_tick: bool) -> RepaintPlan {
        // Stash the frame clock for paint-time use (inline-edit caret blink,
        // `toile-text-editing`).
        self.frame_now_ms = now_ms;
        self.sync_pipe_typing();
        // Open / close the nested documents behind the visible `doc_embed`s
        // (`toile-embeds`). Driven here rather than in `update` because
        // this is the one per-frame hook carrying the frame clock the close
        // debounce needs.
        self.drive_embed_lifecycle(now_ms);

        // Advance an in-flight presentation transition
        // (`toile-presentation`): glide the camera and
        // rebuild the Smart Animate overrides from the frame clock, until it
        // completes. Each active frame is a full repaint (the camera and content
        // both change) and re-culls so elements entering the fitted frame are
        // painted. The cull asks for both ends of the glide at once. An element
        // only the previous slide holds therefore stays in the set, and fades
        // out whole (`TOILE-UI-085`).
        if let Some(transition) = self.transition.as_mut() {
            let covering = transition.covering_viewport(&self.cam);
            let frame = transition.advance(now_ms);
            self.cam = frame.cam;
            self.anim_overrides = frame
                .overrides
                .into_iter()
                .map(|a| (a.id.clone(), a))
                .collect();
            self.needs_full = true;
            if frame.done {
                self.transition = None;
                self.anim_overrides.clear();
                self.queue_viewport();
            } else {
                self.pending_viewport = Some(covering);
                self.viewport_changed = true;
            }
        }

        // A flick or a glide keeps the board moving (`KINETIC-004`), and a
        // gesture held at an edge pans it (`TOILE-UI-380`).
        if super::kinetic::coast(&mut self.cam, &mut self.kinetic, now_ms) {
            self.queue_viewport();
        }
        self.step_edge_pan(now_ms);

        // Keep the status-bar aggregates (`toile-status-bar`) in step with the
        // selection. On a change, ask Logic for the fresh count/bounds. The reply
        // is mirrored into the band's signal by the shell; this view ignores it
        // unless a Fit / find-focus is pending. Cheap -- fires only when the
        // selection differs.
        if self.selection != self.last_stats_sel {
            self.last_stats_sel = self.selection.clone();
            if self.selection.is_empty() {
                // Empty selection: clear the band directly. An empty-ids request
                // means "whole board" to Logic (used by Fit), never "nothing".
                self.clear_sel_stats();
                self.selection_data.clear();
                self.selection_framed.clear();
                self.announce_selection();
            } else {
                self.send_edit(ToileCommand::RequestSelectionStats {
                    ids: self.selection.clone(),
                    kind: StatsRequestKind::Band,
                });
                // Fetch the whole selection's display data, off-screen included
                // (`TOILE-UI-070`, `TOILE-UI-134`). The menu scope, the style
                // surfaces and the inline editor read this, not the culled set.
                self.send_edit(ToileCommand::RequestSelectionData {
                    ids: self.selection.clone(),
                });
                self.announce_next_data = true;
            }
            self.selection_stale = false;
        } else if std::mem::take(&mut self.selection_stale) && !self.selection.is_empty() {
            // The same selection after a board change: fetch its data again.
            // The menu and the panels then read the lock, the box and the
            // frame it has now (`TOILE-UI-384`).
            self.send_edit(ToileCommand::RequestSelectionData {
                ids: self.selection.clone(),
            });
        }

        // Broadcast this peer's gesture (`TOILE-UI-073`). Once per frame, so a
        // pointer stream that outruns the display costs one sample. Also on a
        // heartbeat while nothing changes, which is what keeps this client in
        // the others' live set and advances the Logic thread's only clock.
        self.broadcast_presence(now_ms);

        let any_dirty = self.needs_full
            || self.overlay_dirty
            || self.viewport_changed
            || self.data_dirty
            || self.machine.is_active()
            || self.edit_dirty
            || self.editor.is_some()
            || hud_tick;

        // Phase 0 has no blit-scroll optimization: any content change is a full
        // repaint. The board is transformed on the GPU for pan/zoom, so this is
        // cheap for the common idle/gesture case where nothing repaints at all.
        let content = if self.needs_full || self.data_dirty || self.viewport_changed {
            ContentRepaint::Full
        } else {
            ContentRepaint::Reuse
        };
        let overlay = if any_dirty {
            OverlayRepaint::Redraw
        } else {
            OverlayRepaint::Reuse
        };

        self.needs_full = false;
        self.overlay_dirty = false;
        self.viewport_changed = false;
        self.data_dirty = false;
        self.edit_dirty = false;

        RepaintPlan {
            content,
            content_strip: None,
            overlay,
            backdrop: self.theme.palette().content_bg,
        }
    }

    pub fn paint_content<S: ambre_draw::DrawSurface>(
        &mut self,
        surface: &mut S,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        _strip: Option<Rect>,
    ) {
        render::paint_content(
            surface,
            &self.display,
            &self.cam,
            self.theme.palette(),
            font_cx,
            text_cache,
            self.editor.as_ref().map(|e| e.target()),
            &self.images,
            // A manual gesture preview and a slide transition are mutually
            // exclusive (`toile-tools`); prefer the gesture override while
            // one is in flight.
            if self.gesture_overrides.is_empty() {
                &self.anim_overrides
            } else {
                &self.gesture_overrides
            },
            self.present,
        );
    }

    pub fn paint_overlay<S: ambre_draw::DrawSurface>(
        &mut self,
        surface: &mut S,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        stats: &FrameStats,
    ) {
        render::paint_overlay(
            surface,
            &self.selection,
            &self.display,
            &self.cam,
            self.theme.palette(),
            self.preview.as_ref(),
            &self.find_hits,
            self.find_focus.as_ref(),
            &self.gesture_overrides,
            self.arrow.as_ref(),
            self.present,
        );

        // Remote presence overlay (`toile-presence`): cursors, selection
        // ghosts, live strokes and laser trails, above content, in each peer's
        // cross-app colour. A presentation shows the slide alone, so the peers'
        // pointers stay off it (`TOILE-UI-083`).
        if !self.present.is_presenting() && !self.presence.is_empty() {
            self.presence.paint(
                surface,
                &self.display,
                &self.cam,
                font_cx,
                text_cache,
                self.frame_now_ms,
            );
        }

        // Reaction badges (`toile-reactions`): a chip row under each
        // element carrying a tally. Always visible (edit + present), above content
        // and presence.
        reactions::paint_reactions(
            surface,
            &self.display,
            &self.cam,
            font_cx,
            text_cache,
            self.theme.palette(),
        );

        // Comment pins (`toile-comments`): painted last of the board overlays,
        // so no content and no other overlay hides one (`TOILE-UI-306`). They
        // paint in screen space, so a pin keeps one size through the zoom
        // (`TOILE-UI-022`).
        comments::paint_pins(
            surface,
            &self.display,
            &self.cam.world_to_screen(),
            font_cx,
            text_cache,
            self.theme.palette(),
        );

        // Inline text editor: flush queued intents, draw caret/selection/glyphs,
        // and perform a deferred commit + close (`toile-text-editing`). Drawn
        // above content.
        self.drive_editor(surface, font_cx);

        self.perf_pub.publish(PerfSnapshot {
            fps: stats.fps,
            frame_ms: stats.frame_ms,
            drain_ms: stats.drain_ms,
            draw_ms: stats.draw_ms,
            gpu_ms: stats.gpu_ms,
            ..Default::default()
        });
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.cam.set_size(width as f64, height as f64);
        self.needs_full = true;
        self.queue_viewport();
    }

    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        // Clear the child route first, so a stale value never survives an event
        // the board consumes early, such as an open inline editor below.
        // `FrameHost` reads `child_route()` after every `handle_input`.
        self.child_route = None;
        // The outbox holds an edit the Logic thread has not taken, so the board
        // stops accepting input until the queue drains (`BUS-016`). Consuming
        // the event is what makes the refusal visible: a gesture that fell
        // through to the browser would scroll the page under a frozen board.
        if self.is_busy() {
            return true;
        }
        // The chrome arms a tool and runs a palette command at once, and the
        // board reads both at its next frame. An input that comes first must
        // still see them, so the board reads them now (`TOILE-UI-394`,
        // `TOILE-UI-395`).
        self.apply_active_tool();
        self.drain_menu_cmd();
        // A presentation shows the slide and nothing to edit (`TOILE-UI-083`).
        // It answers before the editor and the embed, which it closed on entry.
        if self.present.is_presenting() {
            return self.present_input(ev);
        }
        // While an inline editor is open it gets first refusal on input
        // (`toile-tools`). `Some(consumed)` settles the event; `None` falls
        // through to the board handlers below (chords, wheel pan/zoom keep working
        // mid-edit).
        if self.editor.is_some()
            && let Some(consumed) = self.edit_input(ev)
        {
            return consumed;
        }
        // Nested-document capture (`toile-embeds`, "Input focus"): while an embed
        // holds focus, input aimed at it belongs to the child, not the board. We
        // record the route and consume the event without running any board
        // handler. `FrameHost` reads `child_route()` and forwards the remapped
        // event into the child's own view. The focusing click itself is NOT
        // captured (focus is still on the board at that point) and `Escape` never
        // is.
        if let Some(focused) = self.embed_focus.current().cloned()
            && self.route_to_child(ev, &focused)
        {
            if let InputEvent::PointerDown { at, .. }
            | InputEvent::PointerUp { at, .. }
            | InputEvent::PointerMove { at }
            | InputEvent::DoubleClick { at } = ev
            {
                self.last_pointer = on_screen(*at);
            }
            self.child_route = self.focused_child();
            return true;
        }
        match ev {
            InputEvent::Wheel { delta, ctrl_key } => self.handle_wheel(
                *delta,
                Modifiers::from_event(*ctrl_key, false, false, false),
            ),
            InputEvent::PointerDown {
                at,
                button,
                shift,
                ctrl,
            } => self.handle_pointer_down(
                on_screen(*at),
                *button,
                Modifiers::from_event(*ctrl, false, *shift, false),
            ),
            InputEvent::PointerMove { at } => self.handle_pointer_move(on_screen(*at)),
            InputEvent::PointerUp { at, .. } => self.handle_pointer_up(on_screen(*at)),
            InputEvent::Key {
                key,
                ctrl,
                alt,
                shift,
                meta,
            } => self.handle_key(key, Modifiers::from_event(*ctrl, *alt, *shift, *meta)),
            InputEvent::KeyUp { key } => self.handle_key_up(key),
            InputEvent::DoubleClick { at } => self.handle_double_click(on_screen(*at)),
            InputEvent::Text { .. }
            | InputEvent::CompositionUpdate { .. }
            | InputEvent::CompositionEnd { .. } => false,
        }
    }

    // -- Interop bridge (`toile-import-export`) ----------------------------

    /// Drain the file the shell read for us this frame (`TOILE-IO-020`). The
    /// shell owns the file dialog and the drop listener. The view owns the
    /// camera, so it is the one place that can say where the drop landed in the
    /// world (`TOILE-IO-023`). Wasm-only; a host no-op keeps `update`
    /// cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn drain_dropped_file(&mut self) {
        let Some(file) = self.dropped_file.get_untracked() else {
            return;
        };
        self.dropped_file.set(None);
        // A file with no drop point came from the picker: it lands in the middle
        // of what the viewer is looking at, never off screen.
        let screen = file.at.unwrap_or(Point::new(
            self.cam.size.width / 2.0,
            self.cam.size.height / 2.0,
        ));
        let at = self.cam.screen_to_world() * screen;
        let cmd = match file.mime.as_str() {
            "image/svg+xml" => ToileCommand::ImportSvg {
                bytes: file.bytes,
                at,
            },
            m if m.starts_with("image/") => ToileCommand::ImportImage {
                bytes: file.bytes,
                mime: ambre_core::MediaType::from(m),
                at,
            },
            // The shell asks `ui::interop::accepts` before it reads a file. A
            // type reaching here at all is a bug on that side, not a user error.
            // Drop it rather than send the board something no converter reads.
            other => {
                tracing::warn!("Toile: no converter reads {other}");
                return;
            }
        };
        self.send_edit(cmd);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_dropped_file(&mut self) {}

    /// Drain the embed the shell resolved for us this frame (`TOILE-086`,
    /// `TOILE-087`). The address has already been through
    /// [`WebAddress::parse`](ambre_bus::app::toile::WebAddress::parse) on the
    /// shell's side. The dialog refuses what it cannot accept. The user learns
    /// of a bad address while it is still on screen, rather than after a box
    /// lands on the board. Placing writes a reference and nothing else, so it
    /// never touches the document it points at (`TOILE-225`). Wasm-only.
    #[cfg(target_arch = "wasm32")]
    fn drain_placed_embed(&mut self) {
        let Some(placed) = self.placed_embed.get_untracked() else {
            return;
        };
        self.placed_embed.set(None);
        self.send_edit(super::embed::placement(placed, &self.cam));
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_placed_embed(&mut self) {}

    /// Drain a link edit the inspector emitted and apply it to the single
    /// selection as one `SetLink` (`toile-links`). A link is a property of one
    /// element: a chooser cannot mean "these five all point here". A
    /// multi-selection is ignored rather than fanned out. Wasm-only.
    #[cfg(target_arch = "wasm32")]
    fn drain_set_link(&mut self) {
        let Some(edit) = self.set_link.get_untracked() else {
            return;
        };
        self.set_link.set(None);
        let [id] = self.selection.as_slice() else {
            return;
        };
        self.send_edit(ToileCommand::SetLink {
            id: id.clone(),
            link: edit.0,
        });
        self.needs_full = true;
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_set_link(&mut self) {}

    /// Hand an outbound navigation to the shell. Wasm-only; on the host the
    /// board simply has nowhere to hand it.
    #[cfg(target_arch = "wasm32")]
    fn publish_shell_nav(&self, nav: super::link::ShellNav) {
        self.followed_link.set(Some(nav));
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_shell_nav(&self, _nav: super::link::ShellNav) {}

    /// Follow a link (`toile-links`). Writes nothing to the document
    /// (`TOILE-063`) and enters no undo history (`TOILE-115`). A jump asks the
    /// board for the frame's box and fits the camera to the answer, and the two
    /// outbound kinds go to the shell.
    ///
    /// The frame is *asked for* rather than read off the painted element. A link
    /// to a frame exists to reach one that is off screen, and an off-screen
    /// element is not in the culled frame. A frame the board no longer holds
    /// answers no bounds, and the fit then does nothing, which is the silent
    /// refusal `TOILE-064` asks for.
    fn follow_link(&mut self, action: LinkAction) {
        match action {
            LinkAction::GoToFrame(frame) => {
                self.pending_fit = true;
                self.send_edit(ToileCommand::RequestSelectionStats {
                    ids: vec![frame],
                    kind: StatsRequestKind::Fit,
                });
            }
            other => {
                if let Some(nav) = other.shell_nav() {
                    self.publish_shell_nav(nav);
                }
            }
        }
    }

    /// The linked element whose badge `screen` lands on, if any. The badge is a
    /// screen-space square outside the element's top-right corner, so the test
    /// runs in screen space too.
    fn badge_at(&self, screen: Point) -> Option<LinkTarget> {
        link::linked(&self.display)
            .find(|e| link::badge_rect(world_rect_to_screen(e.bounds, &self.cam)).contains(screen))
            .and_then(|e| e.link.clone())
    }

    /// Resolve a copy/cut synchronously inside the native clipboard event
    /// (`toile-clipboard`). The clipboard is authoritative. With a non-empty
    /// selection we emit [`ToileCommand::CopyElements`] and return
    /// [`CopyOutcome::Deferred`]; Logic builds the id-free payload from the whole
    /// board and replies with `ToileEvent::Clipboard`. The payload is mirrored to
    /// the OS clipboard next frame by [`poll_clipboard`](Self::poll_clipboard). A
    /// cut appends [`ToileCommand::DeleteElements`] on the same selection
    /// immediately after -- queue order guarantees Logic captured the payload
    /// before the delete. An empty selection returns [`CopyOutcome::Nothing`].
    pub fn clipboard_write(&mut self, write: ClipboardWrite) -> ambre_bus::CopyOutcome {
        // An open inline edit takes precedence over the board selection: copy /
        // cut act on the text selection, not the elements (`toile-text-editing`).
        // `selected_text` is font-free, so it resolves synchronously here; a cut
        // queues a font-bound `DeleteSelection` intent applied at the next paint
        // flush.
        if let Some(ed) = self.editor.as_mut() {
            let Some(text) = ed.selected_text() else {
                return ambre_bus::CopyOutcome::Nothing;
            };
            if write == ClipboardWrite::Cut {
                ed.push(EditIntent::DeleteSelection);
                self.edit_dirty = true;
            }
            return ambre_bus::CopyOutcome::Text(text);
        }
        if self.selection.is_empty() {
            return ambre_bus::CopyOutcome::Nothing;
        }
        let ids = self.selection.clone();
        // A copy that went nowhere is no copy, and its cut deletes nothing
        // (`UI-THREAD-045`).
        if !self.send_edit(ToileCommand::CopyElements { ids: ids.clone() }) {
            return ambre_bus::CopyOutcome::Nothing;
        }
        if write == ClipboardWrite::Cut {
            self.send_edit(ToileCommand::DeleteElements { ids });
            self.selection.clear();
            self.needs_full = true;
        }
        ambre_bus::CopyOutcome::Deferred
    }

    /// Drain the clipboard blob built from the last `ToileEvent::Clipboard` reply.
    /// The host can then mirror it to the OS clipboard within the transient user
    /// activation window (`toile-clipboard`). Yields the blob at most once per
    /// copy.
    pub fn poll_clipboard(&mut self) -> Option<ambre_bus::ClipboardBlob> {
        self.pending_clipboard.take()
    }

    /// Service a paste from the shared [`ClipboardBlob`](ambre_bus::ClipboardBlob)
    /// transport (`toile-clipboard`). An `Own` source still carries the synchronous
    /// ownership token. It prefers the in-memory `last_clip` at
    /// full fidelity, whatever the asynchronous OS write did. Otherwise it
    /// decodes the blob's Ambre bytes. Either way,
    /// `PasteElements` lands the id-free payload at the cursor's world position. A
    /// blob with no decodable Ambre payload is ignored (plain-text paste into a
    /// board is out of MVP).
    pub fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, from: PasteSource) {
        // An open inline edit consumes the paste as text at the caret
        // (`toile-clipboard`). An Ambre element payload is meaningless inside a
        // text field, so route the plain-text mirror through the same insertion
        // path as typing.
        if self.editor.is_some() {
            if !blob.text.is_empty() {
                self.push_intent(EditIntent::Insert(blob.text));
            }
            return;
        }
        let source = ambre_bus::ClipboardBlob::source(self.last_clip.as_ref(), &blob, from);
        let Some(payload) = source
            .ambre
            .as_ref()
            .filter(|_| !source.plain)
            .and_then(|bytes| postcard::from_bytes::<ToileClipboardPayload>(bytes).ok())
        else {
            return;
        };
        let at = self.cam.screen_to_world() * self.last_pointer;
        self.send_edit(ToileCommand::PasteElements { payload, at });
    }

    /// Swap the active document in place (SPA route change): reset document-scoped
    /// state, keep the plumbing and canvas size, and re-fetch the viewport.
    pub fn set_document(&mut self, document_id: DocumentId) {
        if self.document_id == document_id {
            return;
        }
        // Closing the board closes every child immediately, no debounce
        // (`toile-embeds`). The embeds belong to the board we are leaving, and
        // holding their sessions open for 5 s past a route change would leak
        // them.
        self.close_all_embeds();
        self.document_id = document_id;
        self.display = DisplayViewport {
            version: ambre_bus::ViewportVersion(0),
            grid_unit: super::grid::BASE_GRID,
            reactions: Default::default(),
            pins: Vec::new(),
            viewport: Viewport::default(),
            nodes: Vec::new(),
        };
        self.selection.clear();
        self.group_scope = None;
        self.preview = None;
        self.find_hits.clear();
        self.find_focus = None;
        self.pending_find_focus = None;
        self.pending_comment_focus = None;
        self.clear_comments();
        self.clear_deck();
        self.editor = None;
        self.edit_pending_open = None;
        self.edit_open_on_create = None;
        self.edit_closing = false;
        self.edit_dirty = false;
        self.edit_pointer_down = false;
        self.machine.cancel();
        self.presence.clear();
        self.presence_out.reset();
        self.present = PresentMode::Editing;
        self.transition = None;
        self.anim_overrides.clear();
        self.gesture_overrides.clear();
        self.gesture_commit_pending = false;
        self.initial_viewport_sent = false;
        self.pending_viewport = None;
        self.accepted_version = ambre_bus::ViewportVersion(0);
        self.needs_full = true;
        // Keep the transparent text pipe focused so the keyboard works on the
        // freshly-routed board without a click.
        self.text_input.focus();
    }
}

/// The author's wall clock in milliseconds, stamped on a comment command
/// (`toile-comments` section 5). The Logic thread holds no clock, and nothing
/// orders by this one (`TOILE-FEAT-012`): the reader sees it, and that is all.
#[cfg(target_arch = "wasm32")]
fn wall_clock_ms() -> ambre_core::Timestamp {
    // `Date.now()` is a whole, non-negative count of milliseconds.
    ambre_core::Timestamp(js_sys::Date::now() as u64)
}

/// The canvas element's top-left client offset (CSS px), so a canvas-local anchor
/// maps into viewport coordinates for a `position: fixed` overlay
/// (`toile-board-rendering`).
/// The canvas sits below the header / toolbar chrome, so this offset is non-zero.
/// Off the web, or before the canvas mounts, it is `(0, 0)`.
#[cfg(target_arch = "wasm32")]
fn canvas_client_offset() -> ambre_core::CssPoint {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".ambre-canvas").ok().flatten())
        .map(|el| {
            let r = el.get_bounding_client_rect();
            // The DOM measures in `f64`; a CSS pixel is an `f32`.
            ambre_core::CssPoint::new(CssPx::new(r.left() as f32), CssPx::new(r.top() as f32))
        })
        .unwrap_or_default()
}

// Host-only: the view drives a detached text pipe, which the browser build
// replaces by its DOM input elements.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
#[path = "view_tests.rs"]
mod view_tests;

#[cfg(test)]
mod wheel_tests {
    use super::{WHEEL_MOUSE_NOTCH_PX, WheelDevice, wheel_device};
    use ambre_canvas::{WheelDelta, WheelDeltaMode};
    use ambre_core::{CssPx, CssVector};

    /// A notched step, in line units.
    const LINE: WheelDelta = WheelDelta::new(0.0, 1.0, WheelDeltaMode::Line);
    /// A pixel glide.
    const GLIDE: WheelDelta = WheelDelta::pixels(0.0, 1.0);

    #[test]
    fn line_or_page_delta_mode_is_a_mouse() {
        // A classic wheel reports line (1) or page (2) units.
        assert_eq!(
            wheel_device(LINE, CssVector::new(CssPx::new(0.0), CssPx::new(120.0))),
            Some(WheelDevice::Mouse)
        );
        assert_eq!(
            wheel_device(LINE, CssVector::new(CssPx::new(0.0), CssPx::new(600.0))),
            Some(WheelDevice::Mouse)
        );
    }

    #[test]
    fn horizontal_component_is_a_trackpad() {
        // Mice have no horizontal wheel axis; a non-zero dx means two fingers.
        assert_eq!(
            wheel_device(GLIDE, CssVector::new(CssPx::new(4.0), CssPx::new(12.0))),
            Some(WheelDevice::Trackpad)
        );
    }

    #[test]
    fn large_vertical_notch_is_a_mouse() {
        assert_eq!(
            wheel_device(GLIDE, CssVector::new(CssPx::new(0.0), WHEEL_MOUSE_NOTCH_PX)),
            Some(WheelDevice::Mouse)
        );
    }

    #[test]
    fn fine_vertical_glide_is_ambiguous() {
        // A small pixel-mode dy with no dx could be either; keep the latch.
        assert_eq!(
            wheel_device(GLIDE, CssVector::new(CssPx::new(0.0), CssPx::new(6.0))),
            None
        );
    }
}
