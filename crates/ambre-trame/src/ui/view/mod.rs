// ambre-trame/src/ui/view.rs
// ============================================================================
// TrameView -- the Trame per-frame view, a variant of the launcher's closed
// `AppView` registry.
//
// Owns everything that used to live in ambre-app's `UiState`: the grid
// navigation state, the inbound/outbound logic-thread queues, the dirty-rect
// bookkeeping, and the per-frame repaint decision. The launcher's FrameHost
// drives it through `AppView` (update -> plan_repaint -> paint_*), knowing
// nothing about spreadsheets. The driver-facing methods are inherent `pub`
// methods (see the dedicated impl block).
// ============================================================================

use ambre_bus::{
    AppCommand, AppEvent, DocumentId, FillMode, GridRange, LogicCommand, LossyOutcome, Outbox,
    SendOutcome, TrameCommand, TrameEvent, UiEvent,
};
use ambre_canvas::{
    Anchor, AnchorPublisher, BusyPublisher, ContentRepaint, FilterOpenPublisher, FrameStats,
    InputEvent, OverlayRepaint, PerfPublisher, PerfSnapshot, RepaintPlan, TextCache, TextInput,
    Theme, ThemeControl,
};
use ambre_draw::{Clock, DrawSurface, FontContext, Rect};
#[cfg(target_arch = "wasm32")]
use leptos::prelude::RwSignal;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::{GetUntracked, Set};

use super::grid::{
    COL_WIDTH, CellEditor, CellRect, CursorMove, HeaderHit, ROW_HEIGHT, SelKind, Selection,
    TrameUiState,
};
use super::renderer::{self, DrawStats};
use crate::TOTAL_COLS;
use crate::render::{CommitDirection, FormulaAction};

/// Caret blink half-period: the standard ~530 ms on/off toggle (period ~1.06 s).
const BLINK_MS: f64 = 530.0;

/// The one-slot viewport queue (`TRAME-UI-025`). Every request source writes
/// through [`coalesce`](Self::coalesce), so the slot holds at most one request
/// and the newest wins. The frame flush drains it once.
#[derive(Default)]
struct ViewportQueue {
    pending: Option<TrameCommand>,
}

impl ViewportQueue {
    /// Replace the pending request with `cmd`; the newest request wins.
    fn coalesce(&mut self, cmd: TrameCommand) {
        self.pending = Some(cmd);
    }

    /// Take the one pending request, if any, leaving the slot empty.
    fn take(&mut self) -> Option<TrameCommand> {
        self.pending.take()
    }

    /// Restore a request the queue just dropped (`BUS-024`).
    fn re_offer(&mut self, cmd: TrameCommand) {
        self.pending = Some(cmd);
    }
}

/// Compute the autofill destination rectangle from the source selection `src`
/// and the cell `(row, col)` under the dragged handle (`trame-autofill`). The
/// fill handle locks to a single axis: whichever direction the pointer extends
/// farthest from the source wins, and the destination always contains the
/// source. Returns the normalized rectangle `(r1, c1, r2, c2)`.
///
/// `block` is the source's fill granularity `(rows, cols)`. A plain cell source
/// is `(1, 1)`, and the destination tracks the pointer cell exactly. A
/// merged-cell source carries its block size so the extension snaps to whole
/// blocks (Excel behaviour). A 2x1 merge fills two rows at a time.
fn fill_dest(src: CellRect, row: u32, col: u32, block: (u32, u32)) -> CellRect {
    let (sr1, sc1, sr2, sc2) = src;
    let (bh, bw) = block;
    // Extent the pointer reaches beyond the source on each axis.
    let drow = row.saturating_sub(sr2).max(sr1.saturating_sub(row));
    let dcol = col.saturating_sub(sc2).max(sc1.saturating_sub(col));
    if drow >= dcol {
        // Vertical fill: extend rows toward the pointer, snapped to whole blocks.
        if row > sr2 {
            let nb = (row - sr2).div_ceil(bh);
            (sr1, sc1, sr2 + nb * bh, sc2)
        } else if row < sr1 {
            let nb = (sr1 - row).div_ceil(bh);
            (sr1.saturating_sub(nb * bh), sc1, sr2, sc2)
        } else {
            src
        }
    } else {
        // Horizontal fill: extend columns, snapped to whole blocks.
        if col > sc2 {
            let nb = (col - sc2).div_ceil(bw);
            (sr1, sc1, sr2, sc2 + nb * bw)
        } else if col < sc1 {
            let nb = (sc1 - col).div_ceil(bw);
            (sr1, sc1.saturating_sub(nb * bw), sr2, sc2)
        } else {
            src
        }
    }
}

/// Drop a mark named `name` on the browser's Performance panel. A native build
/// has no panel.
fn performance_mark(name: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(perf) = web_sys::window().and_then(|w| w.performance()) {
        let _ = perf.mark(name);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = name;
}

/// Draw the measure `name` from the mark `start` on the browser's Performance
/// panel. A native build has no panel.
fn performance_measure(name: &str, start: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(perf) = web_sys::window().and_then(|w| w.performance()) {
        let _ = perf.measure_with_start_mark(name, start);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (name, start);
}

/// Result of folding the pixel accumulator into a viewport pan.
pub(crate) enum ScrollOutcome {
    /// Less than a whole row/col accumulated; nothing to do yet.
    Idle,
    /// The viewport moved.
    Moved,
    /// A full row/col was due but the viewport is pinned against a grid border.
    Edge,
}

/// Backpressure outcome of a lossless UI->Logic send. Mirrors the two
/// `ambre_bus::conduit::TrySendError` variants without carrying the (large) `LogicCommand`
/// payload, so the `Err` path stays cheap (see `clippy::result_large_err`).
pub(crate) enum SendError {
    /// The outbox took the command nowhere: the queue was full and the hold
    /// queue already stood at its bound. Nothing carries it further, so the
    /// caller MUST NOT reflect it in the view state (`BUS-018`).
    Refused,
    /// The Logic thread is gone; the queue is permanently disconnected.
    Disconnected,
}

/// What a screen reader hears when the active cell moves (`CHROME-023`): its
/// reference, then its value or "empty". Rows and columns are 0-based here.
pub(crate) fn cell_announcement(row: u32, col: u32, value: &str) -> String {
    let at = format!("{}{}", crate::columns::col_to_alpha(col), row + 1);
    if value.is_empty() {
        format!("{at}, empty")
    } else {
        format!("{at}, {value}")
    }
}

/// Whether the HTML formula bar `<input>` (id `ambre-formula-input`) currently
/// holds DOM focus. The view uses this to decide who owns the formula text: the
/// user (typing in the input) or the grid (mirroring the active cell).
#[cfg(target_arch = "wasm32")]
fn formula_input_focused() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .map(|el| el.id() == "ambre-formula-input")
        .unwrap_or(false)
}

/// Trigger a browser download of `data` named `filename` (interop export,
/// `trame-import-export`). Wraps the bytes in a `Blob`, mints an object URL,
/// and clicks a synthetic `<a download>`. The URL is revoked right after so it
/// does not leak. Best-effort: any missing DOM handle is logged and skipped
/// (the export bytes are already computed, only the download is lost).
#[cfg(target_arch = "wasm32")]
fn trigger_download(filename: &str, data: &[u8]) {
    use wasm_bindgen::JsCast;
    let array = js_sys::Uint8Array::from(data);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let blob = match web_sys::Blob::new_with_u8_array_sequence(&parts) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("[UI] download blob failed: {e:?}");
            return;
        }
    };
    let url = match web_sys::Url::create_object_url_with_blob(&blob) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("[UI] download URL failed: {e:?}");
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

/// Host-build stub: there is no DOM to download into outside the browser. Kept
/// so the exhaustive `dispatch_trame` compiles in headless tests.
#[cfg(not(target_arch = "wasm32"))]
fn trigger_download(_filename: &str, _data: &[u8]) {}

/// Trame's view: a virtual-scroll spreadsheet grid (an `AppView::Trame`).
/// A deferred cell-edit commit (applied at the next paint, after the editor
/// flushes its queued intents). Set by grid keys / clicks that finish an edit.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PendingCommit {
    /// Commit and move the cursor by `(drow, dcol)` (Enter, Tab, Up/Down).
    Nav(i32, i32),
    /// Commit in place, cursor unchanged (a click / right-click elsewhere; the
    /// click's own selection has already moved the active cell).
    Stay,
    /// Commit the value into every selected cell (Ctrl/Cmd+Enter).
    Fill,
}

pub struct TrameView {
    /// The document this view renders (from the active route).
    document_id: DocumentId,
    /// Outbound queue to the Logic thread, for the lossy class only.
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
    /// The lossless send path: it keeps a command a full queue could not take,
    /// and offers it again on a later frame (`BUS-011`).
    outbox: Outbox,
    /// The busy level the chrome reads while the outbox holds anything
    /// (`BUS-016`).
    busy_pub: BusyPublisher,
    /// Inbound queue from the Logic thread, drained each frame in `update`.
    from_logic: ambre_bus::conduit::Receiver<UiEvent>,

    /// Grid navigation + cell cache.
    state: TrameUiState,

    /// Transparent textarea pipe (keyboard + IME + clipboard). The view focuses
    /// it on a content click and slides it to the caret during an edit session.
    text_input: TextInput,
    /// Publishes the active cell's on-screen anchor (CSS px) so the chrome can
    /// transform-sync a floating selection toolbar onto it.
    anchor: AnchorPublisher,
    /// Last anchor published, so it is only re-published on change.
    last_anchor: Option<Anchor>,
    /// Publishes the per-frame perf snapshot (fps + timings) so the DOM status
    /// bar can render it -- the HUD left the wgpu overlay (`trame-status-bar`).
    perf_pub: PerfPublisher,
    /// One-shot canvas->launcher signal raised when an in-canvas autofilter button is
    /// clicked (`trame-shared-filter`). The launcher consumes it to open the DOM
    /// `FilterPopover` over the active cell. That is the same path as the context-menu
    /// "Filter by column" entry, so there is one filter code path.
    filter_open: FilterOpenPublisher,
    /// Status-bar selection aggregates signal (`trame-status-bar`). The view
    /// clears it to `None` on a single-cell selection. The Logic round-trip
    /// (`RequestSelectionStats` -> `SelectionStats`) sets it from the launcher's
    /// event loop on a multi-cell one. Browser-only, like the formula signals.
    #[cfg(target_arch = "wasm32")]
    sel_stats: RwSignal<Option<crate::chrome::SelStats>>,
    /// Which surface the last right-click landed on (cell vs header), published
    /// so the launcher swaps the context-menu layout (`trame-context-menu`). Set in
    /// the right-click `PointerDown` handler, which runs synchronously before the
    /// DOM `contextmenu` listener opens the menu. Browser-only.
    #[cfg(target_arch = "wasm32")]
    ctx_scope: RwSignal<crate::ui::grid::CtxScope>,

    /// Shared theme switch (chrome toggles it, the canvas reads it).
    theme_ctl: ThemeControl,
    /// Theme used to render the last frame; a change forces a full repaint.
    theme: Theme,
    /// Formula bar DOM signals (HTML `input`, synced with in-cell editor).
    /// Only present in the browser; native headless tests skip them.
    #[cfg(target_arch = "wasm32")]
    formula_ref: RwSignal<String>,
    #[cfg(target_arch = "wasm32")]
    formula_text: RwSignal<String>,
    /// The formula bar the sync reads and writes. In the browser the frame
    /// copies the signals in before the sync and back out after it.
    formula: FormulaBar,
    #[cfg(target_arch = "wasm32")]
    formula_action: RwSignal<Option<FormulaAction>>,
    /// Grid zoom factor (`1.0` = 100%) shared with the status-bar zoom menu
    /// (`trame-status-bar`). The menu writes it, and the view polls it each frame
    /// and applies it via [`set_zoom`](Self::set_zoom). Browser-only, like the
    /// formula signals; pure local view state, never synced.
    #[cfg(target_arch = "wasm32")]
    zoom_sig: RwSignal<f64>,
    /// Active in-cell edit session, or `None` in navigation mode.
    editor: Option<CellEditor>,
    /// The token of the last edit session opened. Each session takes the next
    /// one, so an event about a closed session matches no open editor.
    edit_session: u64,
    /// A commit requested from a grid key / click, deferred to the next paint. The
    /// shared editor applies edits only at paint-time flush, since a `FontContext`
    /// is live only then. The buffer is not final when the key is handled, so the
    /// commit runs after `draw_edit` flushes. `None` when no commit is pending.
    pending_commit: Option<PendingCommit>,
    /// True while the pointer is dragging out a text selection inside the open
    /// editor (between a press in the cell and its release).
    editor_text_drag: bool,
    /// True while the pointer is dragging out a grid cell-range selection in
    /// navigation mode (between a press on a cell and its release). Pointer
    /// moves while set extend the live selection rectangle.
    selecting_drag: bool,
    /// In-progress whole-axis (column/row) selection drag from a header body
    /// (`trame-selection`), or `None`. Holds which axis is being swept; pointer
    /// moves extend the band to the column/row under the cursor.
    axis_drag: Option<crate::ui::grid::SelKind>,
    /// In-progress header-boundary resize drag (`trame-resize`), or `None`.
    /// Holds the [`HeaderHit::ColBorder`] / [`HeaderHit::RowBorder`] being dragged
    /// so the release knows which column/row to resize; pointer moves update the
    /// live guide line ([`TrameUiState::resize_preview`]).
    resize_drag: Option<HeaderHit>,
    /// A pending column/row auto-fit (`trame-resize`) awaiting the parley font
    /// context. Set on a border double-click, it is resolved and cleared by
    /// the next [`paint_content`](Self::paint_content), which owns the
    /// measurement.
    pending_autofit: Option<HeaderHit>,
    /// A column auto-fit whose whole content the Logic thread returned
    /// (`TRAME-UI-128`), awaiting the font context like
    /// [`pending_autofit`](Self::pending_autofit).
    pending_column_fit: Option<(u32, Vec<crate::DisplayCell>)>,
    /// In-progress autofill drag from the fill handle, or `None`
    /// (`trame-autofill`). Holds the source rectangle the drag started from and
    /// the source block granularity `(rows, cols)`. That is `(1, 1)` for a plain
    /// cell, or the merge span for a merged-cell source so the drag snaps block by
    /// block. Ctrl is read at *release* time, so toggling it mid-drag counts. It
    /// forces Copy on a multi-cell source and a +1 numeric Series on a single-cell
    /// one, Excel-style. Pointer moves grow [`TrameUiState::fill_preview`] and the
    /// release emits a `FillRange`.
    fill_drag: Option<(CellRect, (u32, u32))>,
    /// In-progress Excel-style header reorder drag, or `None`. Holds the grabbed
    /// axis and the origin column/row index (`(Cols, from)` / `(Rows, from)`).
    /// Pointer moves update the [`TrameUiState::reorder_preview`] insertion slot,
    /// and the release emits a `MoveColumn` / `MoveRow` (a no-op if it lands back
    /// on the origin). Armed only when the press lands on an already-selected
    /// header body, so a plain click still selects/extends the axis.
    reorder_drag: Option<(crate::ui::grid::SelKind, u32)>,
    /// The CSS cursor currently set on `#ambre-canvas` from bare hover feedback
    /// (`trame-resize`, `trame-autofill`): `"col-resize"` / `"row-resize"` /
    /// `"pointer"` / `"crosshair"` / `""`. Tracked to avoid redundant DOM writes
    /// every move.
    hover_cursor: String,
    /// The edit overlay changed this frame (keystroke, caret move, IME pass).
    edit_dirty: bool,
    /// Caret blink phase: `true` = caret drawn. Toggled every `BLINK_MS`, reset
    /// to `true` (solid) on any edit interaction, as platforms do.
    caret_on: bool,
    /// Where the last paint laid out the in-cell caret, in canvas pixels. The
    /// pipe sits there while a cell is edited (`CHROME-007`).
    caret_drawn: Option<ambre_draw::Rect>,
    /// The clock's time of the last blink toggle (or blink reset).
    last_blink_ms: f64,
    /// The clock's time of the input awaiting its caret paint, set by an edit
    /// interaction and consumed once the caret is next drawn. This is the
    /// keydown->caret latency probe, against the keystroke-to-draft budget of
    /// `ambre-spec-budgets.md`. `None` when idle.
    pending_input_mark: Option<f64>,

    /// The one clock the view reads: the blink, the scroll, the latency probe
    /// and the renderer's sub-phase timings. The page's on screen, a test's
    /// natively.
    clock: Box<dyn Clock>,

    text_enabled: bool,
    /// Per-frame sub-phase breakdown produced by the renderer, shown on the HUD.
    draw_stats: DrawStats,
    initial_viewport_sent: bool,

    /// Latest viewport request awaiting send, coalesced to <=1 per frame so a
    /// fast key-repeat can't outrun the Logic thread. Set in `handle_input`,
    /// flushed in `update`. Only the newest is kept (it carries the highest
    /// `version`, which supersedes any it replaces).
    pending_viewport: ViewportQueue,
    /// Set once the inbound Logic queue disconnects (Logic thread gone) or a
    /// [`UiEvent::FatalError`] arrives. Halts all further sends; logged once.
    fatal: bool,

    // -- Dirty-rect bookkeeping -------------------------------------------------
    /// Cells layer needs a full repaint (resize, text toggle, first frame).
    needs_full: bool,
    /// Scroll/cursor changed this frame; the delta decides Scroll vs Full.
    viewport_changed: bool,
    /// New cell data landed; promotes to Full only when no scroll happened.
    data_dirty: bool,
    /// Last rendered scroll position, used to compute the scroll delta.
    prev_top_row: u32,
    prev_left_col: u32,

    // -- Wheel / touchpad scroll ------------------------------------------------
    /// The scroll motion, built apart from the browser so a native test drives
    /// it (`scroll.rs`).
    scroll: scroll::GridScroll,
    /// Where a held selection or fill drag last pointed.
    drag_point: Option<(f64, f64)>,

    /// The last grid copy/cut was a cut: the source range
    /// ([`TrameUiState::clipboard_anchor`]) is cleared after the next paste
    /// (`trame-clipboard`). UI-only view state.
    cut_pending: bool,

    /// A [`ClipboardBlob`] built from a `TrameEvent::Clipboard` reply, awaiting
    /// the host's per-frame [`poll_clipboard`](TrameView::poll_clipboard) which
    /// mirrors it to the OS clipboard inside the transient-activation window
    /// (`trame-clipboard`). `None` between a drain and the next copy.
    pending_clipboard: Option<ambre_bus::ClipboardBlob>,
    /// The last authoritative clipboard blob copied this session, kept in memory
    /// for a full-fidelity same-tab paste. That paste never depends on the OS
    /// custom MIME type surviving the round-trip (`trame-clipboard`).
    last_clip: Option<ambre_bus::ClipboardBlob>,
}

#[allow(clippy::too_many_arguments)]
// The `impl TrameView` block is spread across sibling modules; all reach the
// free helpers, consts and types here via `use super::*`.
mod frame;
mod input;
mod interact;
mod lifecycle;
mod scroll;

/// What the view does with a focused formula bar and no edit session.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) enum FormulaBarSync {
    /// Nothing moved.
    Idle,
    /// The user typed: open an edit session seeded with what the bar holds.
    OpenSession,
    /// The cell changed under an untouched bar, a collaborator's write: show
    /// the new value, and open nothing.
    Refresh,
}

/// Decide from the bar's `input`, the text the view last `shown` in it, and
/// the active cell's stored `seed`. Only a text the user typed opens a session
/// (`TRAME-UI-155`). A stored value that moved under the bar is a
/// collaborator's. The session it would open is one the user never started.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn formula_bar_sync(input: &str, shown: &str, seed: &str) -> FormulaBarSync {
    if input != shown {
        FormulaBarSync::OpenSession
    } else if seed != shown {
        FormulaBarSync::Refresh
    } else {
        FormulaBarSync::Idle
    }
}

/// The formula bar as the view sees it for one frame (`trame-cell-editing`).
///
/// The browser copies its signals and its focus in before the sync, and copies
/// the result back out. A host test sets the fields itself.
#[derive(Debug, Default)]
pub(crate) struct FormulaBar {
    /// The reference of the cell an edit will reach (`TRAME-UI-091`).
    pub(crate) reference: String,
    /// The text the bar holds.
    pub(crate) text: String,
    /// The text the view last wrote. The user typed in the bar exactly when
    /// `text` reads something else (`TRAME-UI-155`).
    pub(crate) shown: String,
    /// A commit or a cancel the bar raised, not served yet.
    pub(crate) action: Option<FormulaAction>,
    /// Whether the bar holds the keyboard focus.
    pub(crate) focused: bool,
}

// The view as a host test drives it, on a detached pipe and no DOM. The
// browser constructor also takes the chrome signals, so wasm leaves it out.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod view_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// `BUS-012`: two scroll requests inside one frame leave the queue as one,
    /// the newer, and the slot is empty after the flush.
    #[test]
    fn two_scrolls_in_one_frame_flush_as_one_request() {
        let scroll = |version| TrameCommand::ScrollViewport {
            version,
            row_start: 0,
            row_end: 10,
            col_start: 0,
            col_end: 5,
        };
        let mut queue = ViewportQueue::default();
        queue.coalesce(scroll(1));
        queue.coalesce(scroll(2));
        assert!(
            matches!(
                queue.take(),
                Some(TrameCommand::ScrollViewport { version: 2, .. })
            ),
            "the newer request wins"
        );
        assert!(queue.take().is_none(), "one send per frame");
    }

    /// `CHROME-023`: the grid announces the active cell by its reference and
    /// its value, and says an empty cell is empty rather than nothing.
    #[test]
    fn the_active_cell_is_announced_by_reference_and_value() {
        assert_eq!(cell_announcement(1, 1, "42"), "B2, 42");
        assert_eq!(cell_announcement(0, 26, ""), "AA1, empty");
    }

    /// Only a text the user typed opens an edit session (`TRAME-UI-155`): a
    /// collaborator's write under an untouched bar refreshes it and opens
    /// nothing.
    #[test]
    fn only_a_typed_text_opens_an_edit_from_the_formula_bar() {
        // The bar shows "old"; a peer rewrote the cell to "new".
        assert_eq!(
            formula_bar_sync("old", "old", "new"),
            FormulaBarSync::Refresh
        );
        // The user typed a character.
        assert_eq!(
            formula_bar_sync("old5", "old", "old"),
            FormulaBarSync::OpenSession
        );
        // The user typed while the peer wrote: the typing wins.
        assert_eq!(
            formula_bar_sync("old5", "old", "new"),
            FormulaBarSync::OpenSession
        );
        assert_eq!(formula_bar_sync("old", "old", "old"), FormulaBarSync::Idle);
    }

    fn scroll_cmd(version: u64) -> TrameCommand {
        TrameCommand::ScrollViewport {
            version,
            row_start: 0,
            row_end: 0,
            col_start: 0,
            col_end: 0,
        }
    }

    /// Many request sources in one frame leave one request: the newest wins, and
    /// one drain sends exactly one (`TRAME-UI-025`).
    #[test]
    fn the_viewport_queue_coalesces_to_one_request() {
        let mut q = ViewportQueue::default();
        assert!(q.take().is_none(), "an empty queue sends nothing");
        q.coalesce(scroll_cmd(1));
        q.coalesce(scroll_cmd(2));
        let sent = q.take().expect("one pending request");
        assert!(
            matches!(sent, TrameCommand::ScrollViewport { version: 2, .. }),
            "the newest request wins"
        );
        assert!(q.take().is_none(), "one drain sends exactly one request");
    }

    /// A request the queue dropped is re-offered, never doubled (`BUS-024`).
    #[test]
    fn a_dropped_request_is_re_offered_not_doubled() {
        let mut q = ViewportQueue::default();
        q.coalesce(scroll_cmd(3));
        let sent = q.take().expect("pending");
        q.re_offer(sent);
        let again = q.take().expect("re-offered");
        assert!(matches!(
            again,
            TrameCommand::ScrollViewport { version: 3, .. }
        ));
        assert!(q.take().is_none());
    }
}
