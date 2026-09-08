// ambre-toile/src/ui/view.rs
// ============================================================================
// ToileView: the Toile app's `AppView` variant. Owns the local view state
// (camera, selection, active tool, last DisplayViewport) and the UI<->Logic
// channels, and exposes the inherent driver methods the `FrameHost` dispatches
// every frame (`ambre-app/src/host`): update / plan_repaint / paint_content /
// paint_overlay / resize / handle_input / clipboard / set_document.
//
// Mirrors `ambre_trame::ui::TrameView` but with a true world<->screen camera
// (`ViewportTransform`) instead of a virtual-scroll grid. Holds NO document data
// -- every mutation leaves as a `ToileCommand`; the board lives in `LoroDoc` on
// the Logic thread (`toile-board-rendering`).
// ============================================================================

use std::collections::HashMap;

#[cfg(target_arch = "wasm32")]
use ambre_bus::app::toile::CommentAnchor;
use ambre_bus::app::toile::{
    ClipboardPayload as ToileClipboardPayload, DisplayElement, DisplayGeom, DisplayNode,
    DisplayViewport, ElementId, ElementKind, PresencePayload, ThreadId, ToileCommand, ToileEvent,
    Viewport,
};
use ambre_bus::{AppCommand, AppEvent, DocumentId, LogicCommand, UiEvent};
use ambre_canvas::{
    ContentRepaint, FrameStats, InputEvent, OverlayRepaint, PerfPublisher, PerfSnapshot,
    RepaintPlan, TextCache, TextInput, Theme, ThemeControl, WgpuSurface,
};
use ambre_draw::{BoundingBox, FontContext, Point, Rect, Vec2};

use crate::logic::animate::AnimatedElement;

#[cfg(target_arch = "wasm32")]
use super::comments::CommentIntent;
use super::comments::{self, CommentDraft};
#[cfg(target_arch = "wasm32")]
use super::deck::{self, DeckIntent};
use super::edit::{EditIntent, InlineEditor};
use super::embed;
use super::hit::hit_topmost;
use super::presence::{PeerCursors, PresenceSender};
use super::present::{DEFAULT_TRANSITION_MS, Transition};
use super::reactions;
use super::render;
use super::tool::{
    GestureCursor, Preview, Tool, ToolMachine, ToolOutcome, is_group, resolve_group_target,
    world_rect_to_screen,
};
use super::viewport::ViewportTransform;

// Context-menu bridge (`toile-menus`): the DOM `ContextMenu` reads the
// published selection scope and writes back the chosen `CommandId`; both are
// leptos signals, so the handles and their access traits are wasm-only.
#[cfg(target_arch = "wasm32")]
use ambre_commands::CommandId;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::{GetUntracked, RwSignal, Set};

/// One notch of the mouse wheel maps to this zoom factor (wheel / pinch).
const ZOOM_PER_WHEEL_PX: f64 = 0.0018;

/// A wheel `|dy|` at or above this (physical px, `delta_mode == pixel`) reads as a
/// discrete mouse notch rather than a fine trackpad glide -- the auto heuristic's
/// mouse threshold (`toile-tools`).
const WHEEL_MOUSE_NOTCH_PX: f64 = 40.0;

/// Pure device classifier for one wheel event in `Auto` mode
/// (`toile-viewport`).
/// `Some(true)` = mouse, `Some(false)` = trackpad, `None` = signal too weak to
/// change the latched guess. Signals, strongest first: a non-pixel `delta_mode`
/// is a classic line/page wheel; a non-zero `dx` is a two-axis trackpad glide; a
/// large `|dy|` with no `dx` is a discrete mouse notch.
fn wheel_device(dx: f64, dy: f64, delta_mode: u32) -> Option<bool> {
    if delta_mode != 0 {
        Some(true)
    } else if dx != 0.0 {
        Some(false)
    } else if dy.abs() >= WHEEL_MOUSE_NOTCH_PX {
        Some(true)
    } else {
        None
    }
}

/// How the wheel navigates the board (`toile-viewport`). A whiteboard is a 2D
/// infinite plane, so the natural gestures differ by device: on a mouse the wheel
/// should zoom (map-like), on a trackpad two-finger scroll should pan and pinch
/// should zoom. `Auto` classifies the device per event from the `WheelEvent`
/// signature; `Mouse` / `Trackpad` force a mode and freeze that detection (the
/// user chose in the View menu, we obey). Pan is always also available by
/// dragging (Hand tool / Space-drag).
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

/// Inline-edit caret blink half-period (ms): the caret shows for one interval,
/// hides for the next (`toile-tools`). ~530 ms is the familiar desktop
/// cadence.
const CARET_BLINK_MS: f64 = 530.0;

/// How many trailing samples of an in-flight freehand stroke ride a presence
/// frame (`TOILE-UI-217`). Presence is relayed and forgotten, and a stroke can
/// run to thousands of samples: a peer needs to see where the pen is, and the
/// finished stroke reaches them as a CRDT delta when it commits.
const LIVE_STROKE_TAIL: usize = 64;

/// Max press-to-release travel (screen px) still treated as a click, not a drag,
/// for click-to-edit (`toile-text-editing`): beyond it the gesture is a move and
/// edit is not armed.
const CLICK_SLOP: f64 = 4.0;

/// Hand `data` to the browser as a download named `filename`
/// (`TOILE-IO-085`). A `Blob` behind an object URL, clicked through a detached
/// anchor: the same path Trame's CSV export takes, because the browser offers no
/// other. Best effort -- a blob the browser refuses is logged, never fatal.
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
/// exhaustive event dispatch compiles in the headless tests.
#[cfg(not(target_arch = "wasm32"))]
fn trigger_download(_filename: &str, _data: &[u8]) {}

pub struct ToileView {
    document_id: DocumentId,
    to_logic: flume::Sender<LogicCommand>,
    from_logic: flume::Receiver<UiEvent>,

    /// The render-thread camera (world <-> screen), updated locally on pan/zoom.
    cam: ViewportTransform,
    /// The last culled frame received from Logic (what to paint).
    display: DisplayViewport,

    /// Active tool + the gesture state machine (Select + creation tools).
    machine: ToolMachine,
    /// Local selection (never persisted; a peer's selection is presence,
    /// `toile-selection`).
    selection: Vec<ElementId>,
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
    /// on it (`toile-find`). `FindResults` carries no geometry and
    /// the focus may be off-screen, so the view asks Logic for its bounds via
    /// `RequestSelectionStats` and recenters when the answer lands. Distinguishes
    /// a find-driven stats request from a future status-bar one.
    pending_find_focus: Option<ElementId>,

    /// Active inline text editor (sticky / text body, `toile-text-editing`).
    /// `None` when not editing. Font-free edit intents are queued here at input
    /// time and applied during paint, when the `FontContext` is available (see
    /// `edit.rs`).
    editor: Option<InlineEditor>,
    /// A freshly-created element (Sticky / Text tool) whose editor must open once
    /// the matching `DisplayViewport` has landed (its geometry is only known
    /// then). Consumed at the end of `update`.
    edit_pending_open: Option<ElementId>,
    /// Set when the Sticky / Text tool just emitted a `CreateElement`, so the
    /// next `ElementCreated` id is captured into `edit_pending_open`.
    edit_open_on_create: bool,
    /// Deferred close: the editor commits its buffer as `SetText` and closes on
    /// the next paint (so any intents typed since the last paint flush first).
    edit_closing: bool,
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

    /// Last pointer position in screen (physical) pixels -- the zoom anchor.
    last_pointer: Point,
    /// While panning (space-drag / Hand tool): the previous screen position.
    panning: Option<Point>,
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
    perf_pub: PerfPublisher,
    theme_ctl: ThemeControl,
    theme: Theme,

    // Dirty-flag bookkeeping (`toile-board-rendering`).
    needs_full: bool,
    viewport_changed: bool,
    data_dirty: bool,
    /// Coalesced viewport request: at most one `SetViewport` per frame.
    pending_viewport: Option<Viewport>,
    initial_viewport_sent: bool,
    fatal: bool,

    /// A [`ClipboardBlob`] built from a `ToileEvent::Clipboard` reply, awaiting the
    /// host's per-frame [`poll_clipboard`](ToileView::poll_clipboard) which mirrors
    /// it to the OS clipboard inside the transient-activation window
    /// (`toile-clipboard`).
    pending_clipboard: Option<ambre_bus::ClipboardBlob>,
    /// The last authoritative clipboard blob copied this session, kept in memory
    /// for a full-fidelity same-tab paste that never depends on the OS custom MIME
    /// type surviving the round-trip (`toile-clipboard`).
    last_clip: Option<ambre_bus::ClipboardBlob>,

    /// Selection scope published to the DOM context menu each right-click
    /// (`toile-menus`): the view writes it, the chrome's entry `Memo`
    /// reads it to omit inapplicable rows. Leptos signal, so wasm-only.
    #[cfg(target_arch = "wasm32")]
    ctx_scope: RwSignal<super::menu::CtxScope>,
    /// The context-menu command the shell forwards back into the view
    /// (`toile-menus`): the menu id is drained in `update` and resolved
    /// against the live selection (the Logic thread never sees the selection, so
    /// the view attaches it). Wasm-only.
    #[cfg(target_arch = "wasm32")]
    menu_cmd: RwSignal<Option<CommandId>>,
    /// The active tool, shared with the DOM tool palette
    /// (`toile-tools`): the palette buttons and the bare-key tool
    /// chords both write it, and the view applies it each frame in `update`. Tool
    /// selection is UI-only, canvas-local state (`toile-tools`), never a
    /// `ToileCommand`. Leptos signal, so wasm-only.
    #[cfg(target_arch = "wasm32")]
    active_tool: RwSignal<Tool>,
    /// Position-dependent canvas cursor, shared with the chrome
    /// (`toile-tools`).
    /// The base cursor is a pure function of the active tool (`cursor_css`), but a
    /// resting hover over a selection edge / corner -- and any resize / rotate /
    /// move in flight -- overrides it with a directional resize / grab cursor. The
    /// view writes it (`publish_cursor`); the chrome's cursor `Effect` prefers it
    /// over `cursor_css` when `Some`. Leptos signal, so wasm-only.
    #[cfg(target_arch = "wasm32")]
    cursor: RwSignal<Option<std::borrow::Cow<'static, str>>>,
    /// Live zoom factor published to the DOM zoom control / status bar each frame
    /// (`toile-menus` / `toile-status-bar`): `1.0` = 100%.
    /// The view owns the camera, so it is the source; the chip mirrors it. Written
    /// only on change. Wasm-only.
    #[cfg(target_arch = "wasm32")]
    zoom: RwSignal<f64>,
    /// Status-bar selection aggregates (`toile-status-bar`), shared with the
    /// DOM band. The view clears it to `None` on an empty selection (an empty-ids
    /// stats request would mean "whole board" to Logic, so it must not go through
    /// the round trip); a non-empty selection is filled by the shell from the
    /// `SelectionStats` reply. Leptos signal, so wasm-only.
    #[cfg(target_arch = "wasm32")]
    sel_stats: RwSignal<Option<super::status::SelStats>>,
    /// The selection's common style published to the DOM style inspector
    /// (`toile-styles`): the view derives it from its selection x
    /// last display and writes it (alongside `ctx_scope`); the inspector reads it
    /// to show values and mixed-state. `None` hides the panel. Leptos signal, so
    /// wasm-only.
    #[cfg(target_arch = "wasm32")]
    style_model: RwSignal<Option<super::style::StyleModel>>,
    /// A whole `ElementStyle` the inspector emits back
    /// (`toile-styles`): drained in `update` and turned into one
    /// `RestyleElements` over the live selection (the register is written whole,
    /// `toile-styles`; the view attaches the ids the DOM lacks).
    /// Wasm-only.
    #[cfg(target_arch = "wasm32")]
    restyle: RwSignal<Option<ambre_bus::app::toile::ElementStyle>>,
    /// The content-anchored floating selection toolbar's placement + flags
    /// (`toile-styles`), republished every frame so the bar
    /// follows the selection as the board pans / zooms (transform-synced). `None`
    /// hides it: an empty selection, an active gesture (move/resize/rotate), a pan,
    /// or an open inline editor. Leptos signal, so wasm-only.
    #[cfg(target_arch = "wasm32")]
    float_bar: RwSignal<Option<super::floatbar::FloatBar>>,
    /// A single-field style edit the floating bar emits back
    /// (`toile-styles`):
    /// drained in `update` and folded into one `RestyleElements` over the live
    /// selection (the changed field applied on top of the freshly-derived common
    /// style), the same seam as `restyle`. Wasm-only.
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

    /// The element a thread the panel opened is anchored to, awaiting the
    /// `SelectionStats` reply that carries its bounds so the camera can come to
    /// it (`TOILE-UI-309`). The same channel as the find focus, and checked
    /// after it. `cfg`-agnostic so the dispatch arm stays uniform.
    pending_comment_focus: Option<ElementId>,

    /// Set while a Fit / Zoom-to-selection awaits the `SelectionStats` reply whose
    /// bounds it will frame (`toile-viewport`). The reply treats
    /// both the same way (fit the returned bounds), so a flag suffices; it is
    /// checked before the find-focus recenter that shares the same stats channel.
    /// `cfg`-agnostic so the dispatch arm stays uniform.
    pending_fit: bool,

    /// The selection last reported to the status bar (`toile-status-bar`).
    /// `plan_repaint` re-requests `SelectionStats` whenever the live selection
    /// differs, so the band's count/bounds track every selection change (marquee,
    /// click, nudge) without threading a request through each mutation site.
    last_stats_sel: Vec<ElementId>,

    /// The render-thread mirror of remote peer presence (`toile-presence`):
    /// cursors, selection ghosts, live strokes and laser trails, keyed by
    /// canonical `UserId`. Fed by `ToileEvent::PeerPresence` and painted in the
    /// overlay pass. End-to-end multi-user is gated on SYNC P4 (the `pub_key` ->
    /// `UserId` resolution that keys inbound frames); the render pass is exercised
    /// against synthetic events.
    presence: PeerCursors,
    /// The broadcast schedule for this peer's own presence
    /// (`toile-presence`, `TOILE-UI-073`): it decides whether the sample built
    /// this frame is worth sending, and holds nothing else.
    presence_out: PresenceSender,

    /// Presentation focus mode (`toile-presentation`): driven by
    /// `PresentModeChanged`, it makes the content pass hide the board grid so the
    /// audience sees only the slide. Pure-UI, never synced.
    present: bool,
    /// The in-flight slide transition (`toile-presentation`), built on a
    /// `SlideChanged` event and advanced each
    /// `plan_repaint` until done: it glides the camera to the active frame and
    /// drives the Smart Animate content overrides. `None` when the deck is idle.
    transition: Option<Transition>,
    /// Smart Animate overrides to paint this frame, keyed by element id, rebuilt
    /// from the active `transition` each frame and cleared when it completes. The
    /// content pass looks each element up here before falling back to its static
    /// display (`toile-presentation`).
    anim_overrides: HashMap<ElementId, AnimatedElement>,

    /// Live transform overrides for the in-flight Select gesture
    /// (`toile-tools`): the resized / rotated / moved element(s) painted at
    /// their preview transform before the command commits on release. Rebuilt each
    /// pointer event from `machine.gesture_overrides`, cleared on release / cancel
    /// / tool switch. It is 100% local (no `send_edit` during the drag), so the
    /// preview is instant and never touches the CRDT; the single command still
    /// commits on `pointer_up`. Mutually exclusive with `anim_overrides` (a slide
    /// transition and a manual gesture never run together).
    gesture_overrides: HashMap<ElementId, AnimatedElement>,

    /// True between committing a transform gesture on release and the arrival of
    /// the recomputed `ViewportData` that carries the new geometry
    /// (`toile-tools`). While set, `gesture_overrides` is held as an
    /// *optimistic* preview so the object keeps its released position/size instead
    /// of snapping back to the pre-commit geometry for the frames the Logic
    /// round-trip takes -- which read as a flicker. Cleared (with the overrides) on
    /// the next display update.
    gesture_commit_pending: bool,

    /// Decoded image pixels keyed by media hash, passed to the content walk so
    /// `Image` elements paint real pixels (`toile-media`). Empty
    /// until the media cache resolves a hash: an unresolved image paints a
    /// placeholder box. The screen-side population (from `MediaDownloaded`) lands
    /// with the media cache slice; the field is here so the walk has a stable
    /// source today.
    images: render::ImageStore,

    /// Which `doc_embed` holds input focus, and the click-vs-drag arbitration
    /// that decides it (`toile-embeds`, "Input focus"). View-local,
    /// never CRDT. While an embed is focused, `FrameHost` maps canvas input into
    /// that child and calls the child view's own `handle_input`; this view forwards
    /// nothing itself, because it cannot name the child's command type
    /// (`toile-embeds`, "Command routing").
    embed_focus: embed::EmbedFocus,
    /// The child sync sessions behind the visible `doc_embed`s, with the close
    /// debounce that keeps a pan from thrashing them (`toile-embeds`,
    /// "Opening the child").
    embed_sessions: embed::EmbedSessions,
    /// The child document the last input event was routed into
    /// (`toile-embeds`, "Input focus"), or `None` when the board
    /// consumed it. Set by `handle_input`, read once by `FrameHost` right after,
    /// which forwards the (coordinate-remapped) event to that child's own view.
    /// View-local, never CRDT.
    child_route: Option<DocumentId>,

    /// The latest frame timestamp (ms), captured in `plan_repaint`, used to blink
    /// the inline-edit caret (`toile-text-editing`). Paint has no clock of its own,
    /// so the per-frame hook stashes it here.
    frame_now_ms: f64,
}

impl ToileView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        document_id: DocumentId,
        to_logic: flume::Sender<LogicCommand>,
        from_logic: flume::Receiver<UiEvent>,
        canvas_width: f64,
        canvas_height: f64,
        text_input: TextInput,
        // When embedded as a `doc_embed` child (`toile-embeds`), the
        // child view must not grab the shared text pipe on construction: the
        // top-level document owns the caret until the user clicks into the embed.
        // A top-level view passes `false` and focuses immediately, as before.
        embedded: bool,
        perf_pub: PerfPublisher,
        theme_ctl: ThemeControl,
        #[cfg(target_arch = "wasm32")] ctx_scope: RwSignal<super::menu::CtxScope>,
        #[cfg(target_arch = "wasm32")] menu_cmd: RwSignal<Option<CommandId>>,
        #[cfg(target_arch = "wasm32")] active_tool: RwSignal<Tool>,
        #[cfg(target_arch = "wasm32")] cursor: RwSignal<Option<std::borrow::Cow<'static, str>>>,
        #[cfg(target_arch = "wasm32")] zoom: RwSignal<f64>,
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
    ) -> Self {
        if !embedded {
            text_input.focus();
        }
        let theme = theme_ctl.get();
        Self {
            document_id,
            to_logic,
            from_logic,
            cam: ViewportTransform::new(canvas_width, canvas_height),
            display: DisplayViewport {
                reactions: Default::default(),
                pins: Vec::new(),
                viewport: Viewport::default(),
                nodes: Vec::new(),
            },
            machine: ToolMachine::new(),
            selection: Vec::new(),
            group_scope: None,
            preview: None,
            find_hits: Vec::new(),
            find_focus: None,
            pending_find_focus: None,
            editor: None,
            edit_pending_open: None,
            edit_open_on_create: false,
            edit_closing: false,
            edit_dirty: false,
            edit_pointer_down: false,
            click_edit_candidate: None,
            last_pointer: Point::new(canvas_width / 2.0, canvas_height / 2.0),
            panning: None,
            space_down: false,
            pointer_mode: PointerMode::default(),
            detected_mouse: false,
            text_input,
            perf_pub,
            theme_ctl,
            theme,
            needs_full: true,
            viewport_changed: false,
            data_dirty: false,
            pending_viewport: None,
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
            pending_comment_focus: None,
            pending_fit: false,
            last_stats_sel: Vec::new(),
            presence: PeerCursors::default(),
            presence_out: PresenceSender::default(),
            present: false,
            transition: None,
            anim_overrides: HashMap::new(),
            gesture_overrides: HashMap::new(),
            gesture_commit_pending: false,
            images: render::ImageStore::new(),
            embed_focus: embed::EmbedFocus::default(),
            embed_sessions: embed::EmbedSessions::default(),
            child_route: None,
            frame_now_ms: 0.0,
        }
    }

    // -- Command emission ------------------------------------------------------

    /// Send a lossy command (viewport requests): dropped when the channel is full
    /// (the next coalesced request supersedes it). Never blocks the UI thread.
    fn send_lossy(&self, cmd: ToileCommand) {
        if self.fatal {
            return;
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: AppCommand::Toile(cmd),
        };
        if let Err(flume::TrySendError::Disconnected(_)) = self.to_logic.try_send(msg) {
            tracing::error!("Toile: UI->Logic channel disconnected");
        }
    }

    /// Send a mutating command (edits). Warns (does not panic) on backpressure.
    fn send_edit(&self, cmd: ToileCommand) {
        if self.fatal {
            return;
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: AppCommand::Toile(cmd),
        };
        match self.to_logic.try_send(msg) {
            Ok(()) => {}
            Err(flume::TrySendError::Full(_)) => {
                tracing::warn!("Toile: UI->Logic channel full, edit dropped");
            }
            Err(flume::TrySendError::Disconnected(_)) => {
                tracing::error!("Toile: UI->Logic channel disconnected");
            }
        }
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
            ToileEvent::ViewportData(dv) => {
                self.display = dv;
                self.data_dirty = true;
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
                    ambre: postcard::to_allocvec(&payload).ok(),
                };
                self.last_clip = Some(blob.clone());
                self.pending_clipboard = Some(blob);
            }
            // A Sticky / Text tool just created an element: capture its id so the
            // inline editor opens on it once its `DisplayViewport` lands
            // (`toile-text-editing`).
            ToileEvent::ElementCreated(id) => {
                if self.edit_open_on_create {
                    self.edit_open_on_create = false;
                    self.edit_pending_open = Some(id);
                }
            }
            // Find results (`toile-find`): stash the highlight set
            // and request the focus element's geometry so the camera can recenter
            // on it (the reply may concern an off-screen match absent from
            // `display`).
            ToileEvent::FindResults(fr) => {
                self.find_hits = fr.hits;
                self.find_focus = fr.focus.clone();
                if let Some(focus) = fr.focus {
                    self.pending_find_focus = Some(focus.clone());
                    self.send_edit(ToileCommand::RequestSelectionStats { ids: vec![focus] });
                }
                self.needs_full = true;
            }
            // A `SelectionStats` reply we asked for to recenter on a find focus
            // (`toile-find`). Move the camera center onto the
            // element's bounds center and re-cull; ignore replies not tied to a
            // pending find focus (a future status-bar request,
            // `toile-find`).
            ToileEvent::SelectionStats(stats) => {
                // A pending Fit / Zoom-to-selection consumes the reply first
                // (`toile-viewport`): frame the returned bounds
                // (center + scale).
                if std::mem::take(&mut self.pending_fit) {
                    if let Some(bounds) = stats.bounds {
                        self.cam.fit_bounds(bounds, super::viewport::FIT_PAD);
                        self.needs_full = true;
                        self.queue_viewport();
                    }
                } else if (self.pending_find_focus.take().is_some()
                    || self.pending_comment_focus.take().is_some())
                    && let Some(bounds) = stats.bounds
                {
                    // The same channel serves the find focus and a thread the
                    // panel opened, whose anchor element may sit off-screen
                    // (`TOILE-UI-309`).
                    self.cam.center = bounds.center();
                    self.needs_full = true;
                    self.queue_viewport();
                }
            }
            // Remote presence (`toile-presence`): adopt the whole live set, so a
            // peer absent from it stops being painted with no departure signal
            // (`PRESENCE-009`, `TOILE-UI-075`). Each entry is keyed by the
            // sender's canonical `UserId` (spoof-proof, `TOILE-177`).
            ToileEvent::PeerPresence(peers) => {
                self.presence.adopt(peers, self.frame_now_ms);
                self.data_dirty = true;
            }
            ToileEvent::Error(e) => {
                tracing::warn!("Toile rejected: {e}");
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
                    DEFAULT_TRANSITION_MS,
                ));
                self.needs_full = true;
            }
            // Present (focus) mode toggled (`toile-presentation`): the
            // content pass hides the grid while `present`. Leaving it drops any
            // in-flight transition so the board returns to its plain camera +
            // full-opacity content.
            ToileEvent::PresentModeChanged(present) => {
                self.present = present;
                if !present {
                    self.transition = None;
                    self.anim_overrides.clear();
                }
                self.needs_full = true;
            }
            // A local reaction changed the tally (`toile-reactions`).
            // The tally rides in `DisplayViewport.reactions`, derived on the Logic
            // thread, so we request a fresh viewport for the current camera: the
            // resulting `ViewportData` carries the updated badges. (Remote
            // reactions already push a fresh `ViewportData` via `apply_remote`.)
            ToileEvent::ReactionsUpdated(_) => self.queue_viewport(),
            // "Follow presenter" role changes (`toile-presentation`) are
            // chrome-panel state, not camera-pass state: the presenter panel
            // reflects them via its own signals. The WebGPU view has nothing to
            // repaint for these.
            ToileEvent::PresentBroadcastChanged(_) | ToileEvent::FollowingChanged(_) => {}
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
            // viewport, so ask for a fresh one; and the panel holds no state of
            // its own, so re-read the list and the thread it shows
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
            // An export produced a file (`toile-import-export`). The Logic
            // thread holds the board and serialized the bytes; the UI thread
            // owns the DOM, so the download is triggered here (`TOILE-IO-085`).
            // Nothing on the board changed.
            ToileEvent::ExportComplete { filename, data } => {
                trigger_download(&filename, &data);
            }
        }
    }

    // -- Input handling --------------------------------------------------------

    fn handle_wheel(&mut self, dx: f64, dy: f64, delta_mode: u32, ctrl_key: bool) -> bool {
        // Zoom vs pan is decided by the pointer mode (`toile-viewport`). Ctrl
        // (or a trackpad pinch, which the browser reports as a ctrl-wheel) always
        // zooms.
        let zoom = match self.pointer_mode {
            PointerMode::Mouse => true,
            PointerMode::Trackpad => ctrl_key,
            PointerMode::Auto => {
                self.classify_wheel(dx, dy, delta_mode);
                ctrl_key || self.detected_mouse
            }
        };
        if zoom {
            // Zoom about the cursor. A trackpad pinch and a mouse notch both land
            // here; only `dy` drives the factor.
            let factor = (-dy * ZOOM_PER_WHEEL_PX).exp();
            self.cam.zoom_about(self.last_pointer, factor);
        } else {
            // Two-finger scroll pans the 2D plane in both axes.
            self.cam.pan_by_screen(Vec2::new(dx, dy));
        }
        self.queue_viewport();
        true
    }

    /// Classify the device behind a wheel event while in `Auto` mode and update the
    /// latched guess (`detected_mouse`). Signals, strongest first: a non-pixel
    /// `delta_mode` is a classic line/page wheel (mouse); a non-zero `dx` is a
    /// two-axis trackpad glide; a large `|dy|` with no `dx` is a discrete mouse
    /// notch. Ambiguous fine glides leave the latch unchanged
    /// (`toile-viewport`).
    fn classify_wheel(&mut self, dx: f64, dy: f64, delta_mode: u32) {
        if let Some(mouse) = wheel_device(dx, dy, delta_mode) {
            self.detected_mouse = mouse;
        }
    }

    fn apply_outcome(&mut self, out: ToolOutcome) {
        match out {
            ToolOutcome::None => {}
            ToolOutcome::SelectionChanged(ids) => {
                self.selection = ids;
                self.needs_full = true;
            }
            ToolOutcome::Command(cmd) => {
                // A Sticky / Text placement opens the inline editor on the new
                // element once its id comes back via `ElementCreated`
                // (`toile-text-editing`).
                if let ToileCommand::CreateElement {
                    kind: ElementKind::Sticky | ElementKind::Text,
                    ..
                } = &cmd
                {
                    self.edit_open_on_create = true;
                    // A placement is a one-shot action: revert to Select so the
                    // just-created note is selected and ready to edit, rather than
                    // the next click dropping another note (`toile-tools` /
                    // feedback). The editor opens on the new id when
                    // `ElementCreated` lands.
                    self.set_tool(Tool::Select);
                }
                // A transform commit (release of a move / resize / rotate) keeps
                // its optimistic preview until the recomputed frame lands, so the
                // object holds its released geometry instead of snapping back for
                // a frame.
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
            // (`TOILE-UI-307`). Nothing is written until they post: a thread is
            // born with its first message.
            ToolOutcome::Comment(anchor) => {
                let at = self.css_point(self.last_pointer);
                self.open_composer(CommentDraft { anchor, at });
                // A drop is a one-shot action, like a sticky placement.
                self.set_tool(Tool::Select);
            }
        }
        // The overlay preview is derived from live machine state each event.
        self.preview = self.machine.preview();
        self.rebuild_gesture_overrides();
    }

    /// Rebuild the live gesture overrides from the machine's in-flight transform
    /// (`toile-tools`): the resized / rotated / moved element(s) painted at
    /// their preview bounds / rotation, keeping each one's committed opacity /
    /// fill. Empty when no gesture is in flight (after release / cancel), which
    /// clears the preview.
    fn rebuild_gesture_overrides(&mut self) {
        let live = self.machine.gesture_overrides(&self.display);
        // No live gesture: clear -- unless a commit is pending, where the
        // overrides are the optimistic post-release preview held until the new
        // frame lands.
        if live.is_empty() {
            if !self.gesture_commit_pending {
                self.gesture_overrides.clear();
            }
            return;
        }
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
    /// It owns the child `AppView`s: it paints each one clipped to `screen_rect`
    /// over this view's content pass, and -- for the focused one -- maps canvas
    /// input into the child and calls the child's own `handle_input`. This view
    /// can do neither: both need `AppView`, which it cannot name (ARCH P2).
    pub fn visible_embeds(&self) -> Vec<embed::VisibleEmbed> {
        embed::visible_embeds(&self.display, &self.cam, self.embed_focus.current())
    }

    /// Map a screen point into the focused embed's child pixel space, if the
    /// focused element is still in the culled frame. `FrameHost` calls this to
    /// rewrite pointer coordinates before handing an event to the child.
    pub fn embed_to_child_space(&self, screen: Point) -> Option<Point> {
        let id = self.embed_focus.current()?;
        let elem = self.find_element(id)?;
        Some(embed::to_child_space(elem, &self.cam, screen))
    }

    /// The nested document currently holding input focus
    /// (`toile-embeds`), resolved from the focused board element to its
    /// embedded `DocumentId` -- the key into `FrameHost`'s child views. `None`
    /// when the board itself has focus.
    pub fn focused_child(&self) -> Option<DocumentId> {
        let id = self.embed_focus.current()?;
        match &self.find_element(id)?.geom {
            DisplayGeom::DocEmbed { document_id, .. } => Some(*document_id),
            _ => None,
        }
    }

    /// The child the last `handle_input` routed into, for `FrameHost` to forward
    /// the (coordinate-remapped) event to (`toile-embeds`, "Input
    /// focus"). Cleared at the start of every `handle_input`, so it reflects only
    /// the latest event.
    pub fn child_route(&self) -> Option<DocumentId> {
        self.child_route
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

    /// Decide whether the just-arrived event belongs to the focused embed `id`
    /// and so must be handed to the child instead of the board
    /// (`toile-embeds`, "Input focus"). Keystrokes / text / composition
    /// / wheel go to the child while it holds focus; a pointer event goes to it
    /// only while the cursor is inside its quad, so a click that lands outside
    /// still returns focus to the board. `Escape` is never captured -- the board
    /// needs it to blur (see `handle_key`).
    fn route_to_child(&self, ev: &InputEvent, id: &ElementId) -> bool {
        match ev {
            InputEvent::Key { key, .. } => key != "Escape",
            InputEvent::Text { .. }
            | InputEvent::CompositionUpdate { .. }
            | InputEvent::CompositionEnd { .. }
            | InputEvent::Wheel { .. } => true,
            InputEvent::PointerDown { x, y, .. }
            | InputEvent::PointerUp { x, y, .. }
            | InputEvent::PointerMove { x, y }
            | InputEvent::DoubleClick { x, y } => {
                self.screen_in_focused_embed(Point::new(*x, *y), id)
            }
        }
    }

    /// A press landed at `world` / `screen`: arm a possible focus grant if it is
    /// over an unfocused `doc_embed`. Consumes nothing -- the board's own gesture
    /// proceeds regardless, and the grant is settled on release.
    fn arm_embed_press(&mut self, world: Point, screen: Point) {
        let hit = embed::embed_at(&self.display, world).map(|e| &e.id);
        self.embed_focus.on_press(hit, screen);
    }

    /// A release landed at `screen`: settle the armed press (a click focuses the
    /// embed, a release elsewhere returns focus to the board). The click that
    /// focuses does not reach the child (`toile-embeds`, "Input
    /// focus"), so entering a document never lands a stray edit on the first cell
    /// it happens to hit.
    fn settle_embed_press(&mut self, screen: Point) {
        if self.embed_focus.on_release(screen) {
            self.needs_full = true;
        }
    }

    /// Return focus to the board (`Escape`, a document swap). Reports whether
    /// anything was focused.
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
    fn drive_embed_lifecycle(&mut self, now_ms: f64) {
        for cmd in self.embed_sessions.sync(&self.display, now_ms) {
            self.send_open(cmd);
        }
        // Focus cannot outlive the element that held it: an embed deleted by a
        // peer, or scrolled far enough away to be closed, leaves the board with
        // nothing to route input into.
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

    /// Send a document-session control command (child open / close,
    /// `toile-embeds`). Unlike `send_edit` this is not addressed to a
    /// document via `LogicCommand::App`: it manages the child's session itself.
    fn send_open(&self, cmd: LogicCommand) {
        if self.fatal {
            return;
        }
        match self.to_logic.try_send(cmd) {
            Ok(()) => {}
            Err(flume::TrySendError::Full(_)) => {
                // Dropping an open would leave a blank embed and dropping a close
                // would leak a session, so both are worth a warning; neither is
                // worth blocking the UI thread. The next frame retries the open
                // (the embed is still in the cull set); a lost close is reclaimed
                // when the board closes.
                tracing::warn!("Toile: UI->Logic channel full, embed session command dropped");
            }
            Err(flume::TrySendError::Disconnected(_)) => {
                tracing::error!("Toile: UI->Logic channel disconnected");
            }
        }
    }

    /// Close every child session this view opened (`toile-embeds`:
    /// "Closing the Toile closes every child immediately, no debounce").
    fn close_all_embeds(&mut self) {
        for cmd in self.embed_sessions.close_all() {
            self.send_open(cmd);
        }
        self.embed_focus.blur();
    }

    fn handle_pointer_down(&mut self, x: f64, y: f64, button: i16) -> bool {
        let screen = Point::new(x, y);
        self.last_pointer = screen;
        // A press in the canvas returns keyboard focus to the transparent text
        // pipe (Trame parity, `trame-cell-editing`). After interacting with the DOM chrome
        // (e.g. picking a sticky colour in the palette), focus sits on a chrome
        // button; without this, the board would feel one click behind -- the first
        // canvas click only refocusing, the second acting. The gesture still runs
        // in this same call, so a single click both refocuses and places.
        self.text_input.focus();
        // Right button: select under the cursor and publish the menu scope
        // (`toile-menus`). The DOM `contextmenu` listener opens the menu;
        // pointerdown fires first, so the scope signal is fresh when the entry
        // `Memo` recomputes.
        if button == 2 {
            self.handle_right_click(screen);
            return true;
        }
        // Space-drag or Hand tool or middle button: begin a pan.
        if self.space_down || self.machine.tool() == Tool::Hand || button == 1 {
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
        // Arm a possible embed focus (`toile-embeds`). Consumes
        // nothing: the board's own gesture runs below either way, and the grant is
        // settled on release -- a drag from here moves the embed instead of
        // entering it.
        self.arm_embed_press(world, screen);
        // Capture the selection before the click settles it, to detect a press on
        // an already-selected element (the click-to-edit trigger,
        // `toile-text-editing`).
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
        // A first click only selects (the element was not selected before, so
        // nothing is armed); a second click enters edit on release, unless a drag
        // turns it into a move.
        self.click_edit_candidate = (button == 0 && self.machine.tool() == Tool::Select)
            .then(|| match self.selection.as_slice() {
                [id] if *prev_selection == *self.selection && self.is_text_editable(id) => {
                    Some((id.clone(), screen))
                }
                _ => None,
            })
            .flatten();
        self.publish_cursor(screen);
        self.needs_full = true;
        true
    }

    /// Whether an element id resolves to an inline-editable Sticky / Text element.
    fn is_text_editable(&self, id: &ElementId) -> bool {
        self.find_element(id)
            .is_some_and(|e| matches!(e.kind, ElementKind::Sticky | ElementKind::Text))
    }

    // -- Context menu bridge (`toile-menus`) -------------------------

    /// React to a right-click: resolve the topmost hit to its group-selection
    /// target (`toile-menus`) and, unless it is already part of the current
    /// selection, select just it; an empty hit clears the selection (empty-canvas
    /// menu). Then publish the selection scope for the DOM menu. Emits no command
    /// and starts no gesture.
    fn handle_right_click(&mut self, screen: Point) {
        let world = self.cam.screen_to_world() * screen;
        match hit_topmost(&self.display.nodes, world) {
            Some(hit) => {
                let target = resolve_group_target(hit, &self.display, self.group_scope.as_ref());
                if !self.selection.contains(&target) {
                    self.selection = vec![target];
                }
            }
            None => self.selection.clear(),
        }
        self.needs_full = true;
        self.publish_ctx_scope();
    }

    /// Publish the current selection scope to the DOM context menu
    /// (`toile-menus`). Wasm-only (leptos signal); a host no-op keeps
    /// the input path `cfg`-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn publish_ctx_scope(&self) {
        let scope =
            super::menu::ctx_scope_for(&self.selection, self.group_scope.as_ref(), &self.display);
        self.ctx_scope.set(scope);
        // The style inspector shares the same publish point
        // (`toile-menus`): both are pure functions of the
        // selection x the current display.
        self.style_model.set(super::style::style_scope_for(
            &self.selection,
            &self.display,
        ));
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_ctx_scope(&self) {}

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
        if self.selection.is_empty() {
            return;
        }
        self.send_edit(ToileCommand::RestyleElements {
            ids: self.selection.clone(),
            style,
        });
        self.needs_full = true;
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_restyle(&mut self) {}

    /// Republish the floating selection toolbar's placement + flags
    /// (`toile-styles`). Transform-synced: recomputed every frame
    /// from the live selection x camera so the bar tracks pan/zoom. Hidden
    /// (`None`) on an empty selection, an active gesture (move/resize/rotate), a
    /// pan, or an open inline editor. Written only on change to avoid needless
    /// reactive churn while idle. Wasm-only; a host no-op keeps `update`
    /// cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn publish_float_bar(&self) {
        let next = if self.selection.is_empty()
            || self.machine.is_active()
            || self.panning.is_some()
            || self.editor.is_some()
        {
            None
        } else {
            super::floatbar::build_floatbar(
                &self.selection,
                &self.display,
                &self.cam,
                device_pixel_ratio(),
                canvas_client_offset(),
            )
        };
        if self.float_bar.get_untracked() != next {
            self.float_bar.set(next);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_float_bar(&self) {}

    /// Drain a single-field style edit the floating bar emitted
    /// (`toile-styles`) and apply it to the live selection as one
    /// whole-register `RestyleElements` (`toile-styles`). The base is
    /// the selection's freshly-derived common style so the untouched attributes are
    /// preserved; only the emitted field is overwritten, so back-to-back bar edits
    /// never clobber one another. The view attaches the ids the DOM bar cannot
    /// name. Wasm-only; a host no-op keeps `update` cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn drain_quick_style(&mut self) {
        let Some(edit) = self.quick_style.get_untracked() else {
            return;
        };
        self.quick_style.set(None);
        if self.selection.is_empty() {
            return;
        }
        let mut style = super::style::style_scope_for(&self.selection, &self.display)
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
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_quick_style(&mut self) {}

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
            live_stroke: self
                .machine
                .live_stroke()
                .map(|pts| pts[pts.len().saturating_sub(LIVE_STROKE_TAIL)..].to_vec()),
            laser: self.machine.laser(),
            // The presentation broadcast rides its own frame, emitted by the
            // Logic thread on a slide change (`toile-presentation`).
            present_slide: None,
        }
    }

    /// Send this peer's sample if it changed or the heartbeat is due
    /// (`TOILE-UI-073`). Lossy: a dropped sample is corrected by the next one,
    /// and presence must never displace an edit on a full channel.
    fn broadcast_presence(&mut self, now_ms: f64) {
        if let Some(payload) = self.presence_out.due(self.presence_sample(), now_ms) {
            self.send_lossy(ToileCommand::Presence {
                payload,
                now_ms: now_ms as i64,
            });
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
        if !self.comments.panel.get_untracked() {
            return;
        }
        self.send_edit(ToileCommand::ReadComments { thread: None });
        let showing = self.comments.open.get_untracked().map(|t| t.id);
        if showing.as_ref() == Some(thread) {
            self.send_edit(ToileCommand::ReadComments {
                thread: Some(thread.clone()),
            });
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn reread_comments(&mut self, _thread: &ThreadId) {}

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
    /// names no camera and holds no command channel, so the view carries each
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
        if self.deck.panel.get_untracked() {
            self.send_edit(ToileCommand::ReadDeck);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn reread_deck(&mut self) {}

    /// Publish the frames the panel renders.
    #[cfg(target_arch = "wasm32")]
    fn publish_deck(&mut self, view: ambre_bus::app::toile::DeckView) {
        self.deck.view.set(view);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_deck(&mut self, _view: ambre_bus::app::toile::DeckView) {}

    /// Drain the intent the deck panel emitted this frame. Each edit answers
    /// with the whole new order, which is the only shape `SetSlideOrder` takes;
    /// an edit the model refuses (a frame already on the deck, a rank past its
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
        };
        if let Some(order) = next {
            self.send_edit(ToileCommand::SetSlideOrder { order });
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_deck_intent(&mut self) {}

    /// Bring the camera to a thread's anchor (`TOILE-UI-309`). A point anchor is
    /// its own target; an element anchor may sit outside the culled set, so the
    /// view asks Logic for its bounds and recentres when the answer lands, the
    /// same round trip as the find focus.
    #[cfg(target_arch = "wasm32")]
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
                });
            }
        }
    }

    /// A physical-pixel screen point in the CSS pixels a `position: fixed` DOM
    /// overlay is placed in, the same mapping as the floating selection toolbar.
    #[cfg(target_arch = "wasm32")]
    fn css_point(&self, screen: Point) -> Point {
        let dpr = device_pixel_ratio();
        let (ox, oy) = canvas_client_offset();
        Point::new(ox + screen.x / dpr, oy + screen.y / dpr)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn css_point(&self, screen: Point) -> Point {
        screen
    }

    /// Apply the tool the DOM palette / keyboard selected
    /// (`toile-tools`): if the shared signal differs from the
    /// machine's current tool, switch (which cancels any in-flight gesture and
    /// marks a full repaint). Wasm-only; a host no-op keeps `update` cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn apply_active_tool(&mut self) {
        let tool = self.active_tool.get_untracked();
        if tool != self.machine.tool() {
            self.set_tool(tool);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn apply_active_tool(&mut self) {}

    /// Publish the camera's current zoom to the shared `zoom` signal so the DOM
    /// zoom control / status bar readout tracks pan/zoom
    /// (`toile-tools`). Written only on change to avoid needless
    /// reactive churn. Wasm-only; a host no-op keeps `update` cfg-agnostic.
    #[cfg(target_arch = "wasm32")]
    fn publish_zoom(&self) {
        if (self.zoom.get_untracked() - self.cam.zoom).abs() > f64::EPSILON {
            self.zoom.set(self.cam.zoom);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn publish_zoom(&self) {}

    /// Turn a menu `CommandId` into board mutation(s). `Paste` and `Select all`
    /// are UI-only intents serviced here (the same-session clipboard / local
    /// selection); every other id is resolved against the selection by
    /// `menu::resolve_menu_command` and emitted. Wasm-only (reached only from
    /// `drain_menu_cmd`).
    #[cfg(target_arch = "wasm32")]
    fn apply_menu_command(&mut self, id: CommandId) {
        use super::menu_bar as mb;

        // Top menu bar / zoom control ids (`toile-menus`). Undo
        // / redo and the presentation entry point resolve to a direct board
        // command via the host-tested `menu_bar_edit`; the zoom actions below are
        // local-transform intents that never reach Logic (only the resulting
        // `SetViewport` cull request does).
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
                self.send_edit(ToileCommand::RequestSelectionStats { ids: Vec::new() });
                return;
            }
            mb::ZOOM_TO_SELECTION => {
                if !self.selection.is_empty() {
                    self.pending_fit = true;
                    self.send_edit(ToileCommand::RequestSelectionStats {
                        ids: self.selection.clone(),
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

        if id == super::menu::SELECT_ALL {
            self.select_all();
            return;
        }
        if id == super::menu::PASTE {
            // Menu paste uses the in-memory same-session blob; a cross-tab paste
            // still rides the native Ctrl+V `paste` DOM event
            // (`toile-viewport`).
            if let Some(blob) = self.last_clip.clone() {
                self.paste_clipboard(blob, true);
            }
            return;
        }
        let cmds = super::menu::resolve_menu_command(
            id,
            &self.selection,
            self.group_scope.as_ref(),
            &self.display,
        );
        for cmd in cmds {
            self.send_edit(cmd);
        }
        self.needs_full = true;
    }

    /// Select every displayed element (local, never synced). Shared by `Ctrl+A`
    /// and the context-menu "Select all" entry (`toile-selection`).
    fn select_all(&mut self) {
        self.selection = self
            .display
            .nodes
            .iter()
            .filter_map(|n| match n {
                DisplayNode::Element(e) => Some(e.id.clone()),
                DisplayNode::Connector(_) => None,
            })
            .collect();
        self.needs_full = true;
    }

    fn handle_pointer_move(&mut self, x: f64, y: f64) -> bool {
        let screen = Point::new(x, y);
        self.last_pointer = screen;
        if let Some(prev) = self.panning {
            self.cam.pan_by_screen(screen - prev);
            self.panning = Some(screen);
            self.queue_viewport();
            return true;
        }
        if self.machine.is_active() {
            let world = self.cam.screen_to_world() * screen;
            let out = self.machine.pointer_move(world);
            self.apply_outcome(out);
            self.publish_cursor(screen);
            self.needs_full = true;
            return true;
        }
        // At rest: refresh the directional resize cursor from the hovered edge /
        // corner (`toile-tools`). No repaint -- only the DOM
        // cursor changes.
        self.publish_cursor(screen);
        false
    }

    fn handle_pointer_up(&mut self, x: f64, y: f64) -> bool {
        let screen = Point::new(x, y);
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
        let mut consumed = false;
        if self.machine.is_active() {
            let world = self.cam.screen_to_world() * screen;
            let out = self.machine.pointer_up(world, &self.display);
            self.apply_outcome(out);
            self.publish_cursor(screen);
            self.needs_full = true;
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

    fn handle_double_click(&mut self, x: f64, y: f64) -> bool {
        let screen = Point::new(x, y);
        self.last_pointer = screen;
        let world = self.cam.screen_to_world() * screen;
        // Select tool: a double-click on a Sticky / Text element opens the inline
        // editor on it (`toile-text-editing`), taking precedence over group
        // entering.
        if self.machine.tool() == Tool::Select {
            let editable = hit_topmost(&self.display.nodes, world).filter(|hit| {
                self.find_element(hit)
                    .is_some_and(|e| matches!(e.kind, ElementKind::Sticky | ElementKind::Text))
            });
            if let Some(hit) = editable {
                self.open_editor(hit);
                return true;
            }
        }
        let out = self.machine.double_click(world);
        // Select tool: a double-click on a grouped element enters its scope,
        // peeling one nesting level so the next click selects a member
        // (`toile-groups` / `toile-selection`). Only when the
        // tool produced nothing of its own (e.g. pen finish).
        if matches!(out, ToolOutcome::None)
            && self.machine.tool() == Tool::Select
            && let Some(hit) = hit_topmost(&self.display.nodes, world)
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

    /// Switch the active tool (driven by the chrome tool palette,
    /// `toile-tools`, or by the view itself, e.g. Escape reverting to
    /// Select, `toile-tools`). Publishes the shared active-tool signal so
    /// the palette's pressed state follows a view-initiated switch; without this,
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

    /// Publish the position-dependent canvas cursor
    /// (`toile-tools`). An in-flight resize / rotate / move shows
    /// its stable directional / grab cursor; at rest, a hover over a single
    /// selection's edge / corner shows the matching resize cursor; otherwise `None`
    /// lets the chrome fall back to the tool's base cursor. Wasm-only (leptos
    /// signal); a host no-op keeps callers cfg-agnostic.
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
    fn cursor_at(&self, screen: Point) -> Option<std::borrow::Cow<'static, str>> {
        // A single selected element rotates its handles, so the directional resize
        // cursor rotates with them (`toile-tools`).
        let theta = self
            .selection
            .first()
            .map(|id| super::tool::element_rotation(&self.display, id))
            .unwrap_or(0.0);
        if let Some(g) = self.machine.gesture_cursor() {
            return Some(match g {
                GestureCursor::Handle(h) => super::palette::resize_cursor(h, theta),
                GestureCursor::Move => std::borrow::Cow::Borrowed("grabbing"),
            });
        }
        self.machine
            .hover_handle(&self.selection, &self.display, &self.cam, screen)
            .map(|h| super::palette::resize_cursor(h, theta))
    }

    /// Keyboard shortcuts (`toile-tools`). Modifiers ride the event:
    /// `ctrl || meta` is the command chord (Ctrl on Windows/Linux, Cmd on macOS).
    /// Ctrl+C/X/V are intentionally NOT decoded here -- they ride the native
    /// clipboard events (Phase A). Returns `true` when the key was consumed (host
    /// `preventDefault`).
    fn handle_key(&mut self, key: &str, ctrl: bool, _alt: bool, shift: bool, meta: bool) -> bool {
        let cmd = ctrl || meta;
        // Command chords first, so a bare-key fallthrough never shadows them.
        if cmd {
            match key {
                // Undo / redo (`toile-undo`). Ctrl+Shift+Z and
                // Ctrl+Y both redo.
                "z" | "Z" if shift => self.send_edit(ToileCommand::Redo),
                "z" | "Z" => self.send_edit(ToileCommand::Undo),
                "y" | "Y" => self.send_edit(ToileCommand::Redo),
                // Duplicate the selection with a small diagonal nudge
                // (`toile-undo`).
                "d" | "D" if !self.selection.is_empty() => {
                    self.send_edit(ToileCommand::DuplicateElements {
                        ids: self.selection.clone(),
                        delta: Vec2::new(DUPLICATE_NUDGE, DUPLICATE_NUDGE),
                    });
                }
                // Group / ungroup the selection (`toile-groups`).
                "g" | "G" if shift => {
                    for id in self.selection.clone() {
                        self.send_edit(ToileCommand::UngroupElements { group: id });
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
                // selection survives, so Escape steps out one level at a time
                // rather than also clearing what was selected
                // (`toile-embeds`).
                //
                // This arm is reachable while an embed holds focus because
                // `FrameHost` routes every input into the focused child EXCEPT
                // `Escape`, which it hands to this view -- otherwise the key that
                // exits the child would be swallowed by it.
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

    /// Find a displayed element by id (linear over the culled set; the set is
    /// small and this runs only on edit open / paint of the edited element).
    fn find_element(&self, id: &ElementId) -> Option<&DisplayElement> {
        self.display.nodes.iter().find_map(|n| match n {
            DisplayNode::Element(e) if &e.id == id => Some(e),
            _ => None,
        })
    }

    /// Optimistically set an element's body text in the local display, so the
    /// static render matches a just-committed value before the model round-trip
    /// delivers a fresh viewport (avoids a one-frame flash of the old body,
    /// `toile-text-editing`).
    fn patch_element_text(&mut self, id: &ElementId, text: &str) {
        if let Some(DisplayNode::Element(e)) = self
            .display
            .nodes
            .iter_mut()
            .find(|n| matches!(n, DisplayNode::Element(e) if &e.id == id))
        {
            match &mut e.geom {
                DisplayGeom::Sticky(t) | DisplayGeom::Text(t) => *t = text.to_owned(),
                _ => {}
            }
        }
    }

    /// The edited element's text origin (top-left screen corner + padding) and
    /// wrap width in screen pixels, plus its full screen rect (for hit tests).
    fn edit_geometry(&self, e: &DisplayElement) -> (Point, f32, Rect) {
        let sb = world_rect_to_screen(e.bounds, &self.cam);
        let pad = InlineEditor::pad(self.cam.zoom);
        let origin = Point::new(sb.x0 + pad, sb.y0 + pad);
        let wrap_w = (sb.width() - 2.0 * pad).max(1.0) as f32;
        (origin, wrap_w, sb)
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
            let is_sticky = matches!(e.kind, ElementKind::Sticky);
            let text = match &e.geom {
                DisplayGeom::Sticky(t) | DisplayGeom::Text(t) => t.clone(),
                _ => String::new(),
            };
            InlineEditor::open(e, &text, self.theme.palette(), is_sticky)
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

    /// Request a deferred commit + close: the buffer is written back as `SetText`
    /// on the next paint (after a final intent flush).
    fn request_close_editor(&mut self) {
        if self.editor.is_some() {
            self.edit_closing = true;
            self.edit_dirty = true;
        }
    }

    /// Map a screen point into the edited element's local editor frame, or `None`
    /// when the point falls outside the element (a click there closes the edit).
    fn edit_local_point(&self, screen: Point) -> Option<Point> {
        let id = self.editor.as_ref()?.id().clone();
        let e = self.find_element(&id)?;
        let (origin, _w, sb) = self.edit_geometry(e);
        // Unrotate the pointer about the element center into its axis-aligned
        // frame, so clicks land on the rotated in-edit text (mirror of the overlay
        // rotation in `drive_editor`, `toile-text-editing`).
        let rot = super::tool::element_rotation(&self.display, &id);
        let p = if rot != 0.0 {
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
            } => Some(self.edit_key(key, *ctrl, *alt, *shift, *meta)),
            InputEvent::PointerDown { x, y, .. } => {
                self.last_pointer = Point::new(*x, *y);
                match self.edit_local_point(Point::new(*x, *y)) {
                    Some(local) => {
                        self.push_intent(EditIntent::PointerDown {
                            x: local.x as f32,
                            y: local.y as f32,
                        });
                        self.edit_pointer_down = true;
                    }
                    None => self.request_close_editor(),
                }
                Some(true)
            }
            InputEvent::PointerMove { x, y } => {
                self.last_pointer = Point::new(*x, *y);
                if self.edit_pointer_down
                    && let Some(local) = self.edit_local_point(Point::new(*x, *y))
                {
                    self.push_intent(EditIntent::PointerDrag {
                        x: local.x as f32,
                        y: local.y as f32,
                    });
                    return Some(true);
                }
                Some(self.edit_pointer_down)
            }
            InputEvent::PointerUp { .. } => {
                let was = self.edit_pointer_down;
                self.edit_pointer_down = false;
                Some(was)
            }
            InputEvent::DoubleClick { x, y } => {
                match self.edit_local_point(Point::new(*x, *y)) {
                    Some(local) => self.push_intent(EditIntent::WordAt {
                        x: local.x as f32,
                        y: local.y as f32,
                    }),
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
    fn edit_key(&mut self, key: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> bool {
        let cmd = ctrl || meta;
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
        let word = alt; // Alt+Arrow = word motion (Ctrl is reserved for chords).
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
        if self.editor.is_none() {
            return;
        }
        let id = self.editor.as_ref().unwrap().id().clone();
        // Resolve the edited element's screen geometry (may have scrolled out).
        let geom = self.find_element(&id).map(|e| self.edit_geometry(e));

        match geom {
            Some((origin, wrap_w, sb)) => {
                let zoom = self.cam.zoom;
                let palette = self.theme.palette();
                // A rotated element keeps its rotation while editing: rotate the
                // whole overlay about the element's screen-space center, matching
                // the static text pass (`render::text_rotation`,
                // `toile-text-editing`).
                let rot = super::tool::element_rotation(&self.display, &id);
                // Blink the caret on a ~1s cycle so it visibly signals "you can
                // type here" (`toile-text-editing`). `editor.is_some()` keeps the
                // overlay redrawing every frame, so the toggle animates.
                let caret_on = (self.frame_now_ms / CARET_BLINK_MS) as i64 % 2 == 0;
                let ed = self.editor.as_mut().unwrap();
                ed.flush(font_cx, wrap_w, zoom);
                // Always paint the body -- including the closing frame. The
                // content pass already ran this frame with the element's static
                // text suppressed (the editor was still live), so skipping the
                // overlay here would leave one blank frame before the static text
                // returns next frame. Painting the committed text bridges that
                // gap. The caret + IME anchor are for active editing only, so they
                // are gated off while closing.
                let rotated = rot != 0.0;
                if rotated {
                    surface.push_transform(render::rotate_about(sb.center(), rot));
                }
                ed.render(surface, origin, palette, caret_on && !self.edit_closing);
                if rotated {
                    surface.pop_transform();
                }
                if !self.edit_closing
                    && let Some(caret) = ed.caret_local()
                {
                    set_caret_pipe(&self.text_input, origin, caret);
                }
            }
            // Element gone/culled: commit whatever the last flush produced.
            None => self.edit_closing = true,
        }

        if self.edit_closing {
            if let Some(text) = self.editor.as_ref().unwrap().committed_text() {
                // Optimistic local update: reflect the committed text in the
                // static pass immediately, so dropping the editor does not flash
                // the pre-edit body for the frames before the CRDT round-trip
                // returns a fresh viewport (`toile-viewport`). The incoming
                // viewport carries the same text.
                self.patch_element_text(&id, &text);
                self.send_edit(ToileCommand::SetText { id, text });
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
                Err(flume::TryRecvError::Empty) => break,
                Err(flume::TryRecvError::Disconnected) => {
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

        // A freshly-created Sticky / Text element: open its editor now that its
        // geometry has landed in `display` (`toile-text-editing`).
        if let Some(id) = self.edit_pending_open.take() {
            self.open_editor(id);
        }

        // First frame: request the initial culled viewport for the boot camera.
        if !self.initial_viewport_sent {
            self.initial_viewport_sent = true;
            self.needs_full = true;
            self.pending_viewport = Some(self.cam.to_bus_viewport());
        }

        // Flush the coalesced viewport request (at most one per frame).
        if let Some(vp) = self.pending_viewport.take() {
            self.send_lossy(ToileCommand::SetViewport(vp));
        }

        // Pick up a theme switch from the chrome toggle.
        let theme = self.theme_ctl.get();
        if theme != self.theme {
            self.theme = theme;
            self.needs_full = true;
        }
    }

    pub fn plan_repaint(&mut self, now_ms: f64, hud_tick: bool) -> RepaintPlan {
        // Stash the frame clock for paint-time use (inline-edit caret blink,
        // `toile-text-editing`).
        self.frame_now_ms = now_ms;
        // Open / close the nested documents behind the visible `doc_embed`s
        // (`toile-embeds`). Driven here rather than in `update` because
        // this is the one per-frame hook carrying the frame clock the close
        // debounce needs.
        self.drive_embed_lifecycle(now_ms);

        // Advance an in-flight presentation transition
        // (`toile-presentation`): glide the camera and
        // rebuild the Smart Animate overrides from the frame clock, until it
        // completes. Each active frame is a full repaint (the camera and content
        // both change) and re-culls (`queue_viewport`) so elements entering the
        // fitted frame are painted.
        if self.transition.is_some() {
            let frame = self.transition.as_mut().unwrap().advance(now_ms);
            self.cam = frame.cam;
            self.anim_overrides = frame
                .overrides
                .into_iter()
                .map(|a| (a.id.clone(), a))
                .collect();
            self.needs_full = true;
            self.queue_viewport();
            if frame.done {
                self.transition = None;
                self.anim_overrides.clear();
            }
        }

        // Keep the status-bar aggregates (`toile-status-bar`) in step with the
        // selection: on a change, ask Logic for the fresh count/bounds (the reply
        // is mirrored into the band's signal by the shell; this view ignores it
        // unless a Fit / find-focus is pending). Cheap -- fires only when the
        // selection differs.
        if self.selection != self.last_stats_sel {
            self.last_stats_sel = self.selection.clone();
            if self.selection.is_empty() {
                // Empty selection: clear the band directly. An empty-ids request
                // means "whole board" to Logic (used by Fit), never "nothing".
                self.clear_sel_stats();
            } else {
                self.send_edit(ToileCommand::RequestSelectionStats {
                    ids: self.selection.clone(),
                });
            }
        }

        // Broadcast this peer's gesture (`TOILE-UI-073`). Once per frame, so a
        // pointer stream that outruns the display costs one sample; and on a
        // heartbeat while nothing changes, which is what keeps this client in
        // the others' live set and advances the Logic thread's only clock.
        self.broadcast_presence(now_ms);

        let any_dirty = self.needs_full
            || self.viewport_changed
            || self.data_dirty
            || self.machine.is_active()
            || self.edit_dirty
            || self.editor.is_some()
            || hud_tick;

        // Phase 0 has no blit-scroll optimization: any content change is a full
        // repaint (the board is transformed on the GPU for pan/zoom, so this is
        // cheap for the common idle/gesture case where nothing repaints at all).
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
        self.viewport_changed = false;
        self.data_dirty = false;
        self.edit_dirty = false;

        RepaintPlan {
            content,
            content_strip: None,
            overlay,
        }
    }

    pub fn paint_content(
        &mut self,
        surface: &mut WgpuSurface<'_>,
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
            self.editor.as_ref().map(|e| e.id()),
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

    pub fn paint_overlay(
        &mut self,
        surface: &mut WgpuSurface<'_>,
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
        );

        // Remote presence overlay (`toile-presence`): cursors, selection
        // ghosts, live strokes and laser trails, above content, in each peer's
        // cross-app colour.
        if !self.presence.is_empty() {
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
        // so no content and no other overlay hides one (`TOILE-UI-306`), and in
        // screen space, so a pin keeps one size through the zoom
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
        // the board consumes early (e.g. an open inline editor below): `FrameHost`
        // reads `child_route()` after every `handle_input`.
        self.child_route = None;
        // While an inline editor is open it gets first refusal on input
        // (`toile-tools`): `Some(consumed)` settles the event, `None` falls
        // through to the board handlers below (chords, wheel pan/zoom keep working
        // mid-edit).
        if self.editor.is_some()
            && let Some(consumed) = self.edit_input(ev)
        {
            return consumed;
        }
        // Nested-document capture (`toile-embeds`, "Input focus"):
        // while an embed holds focus, input aimed at it belongs to the child, not
        // the board. We record the route and consume the event without running any
        // board handler; `FrameHost` reads `child_route()` and forwards the
        // remapped event into the child's own view. The focusing click itself is
        // NOT captured (focus is still on the board at that point) and `Escape`
        // never is.
        if let Some(focused) = self.embed_focus.current().cloned()
            && self.route_to_child(ev, &focused)
        {
            if let InputEvent::PointerDown { x, y, .. }
            | InputEvent::PointerUp { x, y, .. }
            | InputEvent::PointerMove { x, y }
            | InputEvent::DoubleClick { x, y } = ev
            {
                self.last_pointer = Point::new(*x, *y);
            }
            self.child_route = self.focused_child();
            return true;
        }
        match ev {
            InputEvent::Wheel {
                dx,
                dy,
                delta_mode,
                ctrl_key,
            } => self.handle_wheel(*dx, *dy, *delta_mode, *ctrl_key),
            InputEvent::PointerDown { x, y, button, .. } => {
                self.handle_pointer_down(*x, *y, *button)
            }
            InputEvent::PointerMove { x, y } => self.handle_pointer_move(*x, *y),
            InputEvent::PointerUp { x, y, .. } => self.handle_pointer_up(*x, *y),
            InputEvent::Key {
                key,
                ctrl,
                alt,
                shift,
                meta,
            } => self.handle_key(key, *ctrl, *alt, *shift, *meta),
            InputEvent::DoubleClick { x, y } => self.handle_double_click(*x, *y),
            InputEvent::Text { .. }
            | InputEvent::CompositionUpdate { .. }
            | InputEvent::CompositionEnd { .. } => false,
        }
    }

    // -- Interop bridge (`toile-import-export`) ----------------------------

    /// Drain the file the shell read for us this frame (`TOILE-IO-020`). The
    /// shell owns the file dialog and the drop listener; the view owns the
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
                mime: m.to_owned(),
                at,
            },
            // The shell asks `ui::interop::accepts` before it reads a file, so a
            // type reaching here at all is a bug on that side, not a user error:
            // drop it rather than send the board something no converter reads.
            other => {
                tracing::warn!("Toile: no converter reads {other}");
                return;
            }
        };
        self.send_edit(cmd);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_dropped_file(&mut self) {}

    /// Resolve a copy/cut synchronously inside the native clipboard event
    /// (`toile-clipboard`).
    /// The clipboard is authoritative: with a non-empty selection we emit
    /// [`ToileCommand::CopyElements`] (Logic builds the id-free payload from the
    /// whole board and replies with `ToileEvent::Clipboard`) and return
    /// [`CopyOutcome::Deferred`]; the payload is mirrored to the OS clipboard next
    /// frame by [`poll_clipboard`](Self::poll_clipboard). A cut appends
    /// [`ToileCommand::DeleteElements`] on the same selection immediately after --
    /// channel order guarantees Logic captured the payload before the delete.
    /// An empty selection returns [`CopyOutcome::Nothing`].
    pub fn clipboard_write(&mut self, cut: bool) -> ambre_bus::CopyOutcome {
        // An open inline edit takes precedence over the board selection: copy /
        // cut act on the text selection, not the elements (`toile-text-editing`).
        // `selected_text` is font-free, so it resolves synchronously here; a cut
        // queues a font-bound `DeleteSelection` intent applied at the next paint
        // flush.
        if let Some(ed) = self.editor.as_mut() {
            let Some(text) = ed.selected_text() else {
                return ambre_bus::CopyOutcome::Nothing;
            };
            if cut {
                ed.push(EditIntent::DeleteSelection);
                self.edit_dirty = true;
            }
            return ambre_bus::CopyOutcome::Text(text);
        }
        if self.selection.is_empty() {
            return ambre_bus::CopyOutcome::Nothing;
        }
        let ids = self.selection.clone();
        self.send_edit(ToileCommand::CopyElements { ids: ids.clone() });
        if cut {
            self.send_edit(ToileCommand::DeleteElements { ids });
            self.selection.clear();
            self.needs_full = true;
        }
        ambre_bus::CopyOutcome::Deferred
    }

    /// Drain the clipboard blob built from the last `ToileEvent::Clipboard` reply,
    /// so the host can mirror it to the OS clipboard within the transient user
    /// activation window (`toile-clipboard`). Yields the blob at most once
    /// per copy.
    pub fn poll_clipboard(&mut self) -> Option<ambre_bus::ClipboardBlob> {
        self.pending_clipboard.take()
    }

    /// Service a paste from the shared [`ClipboardBlob`](ambre_bus::ClipboardBlob)
    /// transport (`toile-clipboard`). When `own_copy` (the synchronous
    /// ownership token was still on the clipboard) it prefers the in-memory
    /// `last_clip` at full fidelity -- immune to the async OS write having failed
    /// on Firefox; else it decodes the blob's Ambre bytes. Either way,
    /// `PasteElements` lands the id-free payload at the cursor's world position. A
    /// blob with no decodable Ambre payload is ignored (plain-text paste into a
    /// board is out of MVP).
    pub fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, own_copy: bool) {
        // An open inline edit consumes the paste as text at the caret
        // (`toile-clipboard`): an Ambre element payload is meaningless inside a
        // text field, so route the plain-text mirror through the same insertion
        // path as typing.
        if self.editor.is_some() {
            if !blob.text.is_empty() {
                self.push_intent(EditIntent::Insert(blob.text));
            }
            return;
        }
        let source = match &self.last_clip {
            Some(clip) if own_copy => clip,
            _ => &blob,
        };
        let Some(payload) = source
            .ambre
            .as_ref()
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
        // (`toile-embeds`): the embeds belong to the board we are
        // leaving, and holding their sessions open for 5 s past a route change
        // would leak them.
        self.close_all_embeds();
        self.document_id = document_id;
        self.display = DisplayViewport {
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
        self.edit_open_on_create = false;
        self.edit_closing = false;
        self.edit_dirty = false;
        self.edit_pointer_down = false;
        self.machine.cancel();
        self.presence.clear();
        self.presence_out.reset();
        self.present = false;
        self.transition = None;
        self.anim_overrides.clear();
        self.gesture_overrides.clear();
        self.gesture_commit_pending = false;
        self.initial_viewport_sent = false;
        self.pending_viewport = None;
        self.needs_full = true;
        // Keep the transparent text pipe focused so the keyboard works on the
        // freshly-routed board without a click.
        self.text_input.focus();
    }
}

/// Position the transparent `<textarea>` pipe at the editor caret so the IME
/// candidate box anchors there (`toile-text-editing`). `origin` is the editor's
/// text origin in physical screen pixels; `caret` is the caret box in the
/// editor's local frame.
/// The pipe wants CSS pixels, so both are divided by the device pixel ratio.
fn set_caret_pipe(pipe: &ambre_canvas::TextInput, origin: ambre_draw::Point, caret: BoundingBox) {
    let dpr = device_pixel_ratio();
    let css_x = (origin.x + caret.x0) / dpr;
    let css_y = (origin.y + caret.y0) / dpr;
    let line_h = ((caret.y1 - caret.y0) / dpr).max(1.0);
    pipe.set_caret(css_x, css_y, line_h);
}

/// The browser device pixel ratio (physical px per CSS px), or 1.0 off the web.
/// The author's wall clock in milliseconds, stamped on a comment command
/// (`toile-comments` section 5). The Logic thread holds no clock, and nothing
/// orders by this one (`TOILE-FEAT-012`): the reader sees it, and that is all.
#[cfg(target_arch = "wasm32")]
fn wall_clock_ms() -> i64 {
    js_sys::Date::now() as i64
}

#[cfg(target_arch = "wasm32")]
fn device_pixel_ratio() -> f64 {
    web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .filter(|dpr| *dpr > 0.0)
        .unwrap_or(1.0)
}

#[cfg(not(target_arch = "wasm32"))]
fn device_pixel_ratio() -> f64 {
    1.0
}

/// The canvas element's top-left client offset (CSS px), so a canvas-local anchor
/// maps into viewport coordinates for a `position: fixed` overlay
/// (`toile-board-rendering`).
/// The canvas sits below the header / toolbar chrome, so this offset is non-zero.
/// Off the web, or before the canvas mounts, it is `(0, 0)`.
#[cfg(target_arch = "wasm32")]
fn canvas_client_offset() -> (f64, f64) {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".ambre-canvas").ok().flatten())
        .map(|el| {
            let r = el.get_bounding_client_rect();
            (r.left(), r.top())
        })
        .unwrap_or((0.0, 0.0))
}

#[cfg(test)]
mod wheel_tests {
    use super::{WHEEL_MOUSE_NOTCH_PX, wheel_device};

    #[test]
    fn line_or_page_delta_mode_is_a_mouse() {
        // A classic wheel reports line (1) or page (2) units.
        assert_eq!(wheel_device(0.0, 3.0, 1), Some(true));
        assert_eq!(wheel_device(0.0, 1.0, 2), Some(true));
    }

    #[test]
    fn horizontal_component_is_a_trackpad() {
        // Mice have no horizontal wheel axis; a non-zero dx means two fingers.
        assert_eq!(wheel_device(4.0, 12.0, 0), Some(false));
    }

    #[test]
    fn large_vertical_notch_is_a_mouse() {
        assert_eq!(wheel_device(0.0, WHEEL_MOUSE_NOTCH_PX, 0), Some(true));
    }

    #[test]
    fn fine_vertical_glide_is_ambiguous() {
        // A small pixel-mode dy with no dx could be either; keep the latch.
        assert_eq!(wheel_device(0.0, 6.0, 0), None);
    }
}
