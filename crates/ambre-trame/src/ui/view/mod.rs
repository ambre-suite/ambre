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

use crate::message::{FillMode, TrameCommand, TrameEvent};
use ambre_bus::{
    Feed, LogicCommand, LossyOutcome, Outbox, PasteSource, Port, SendOutcome, UiEvent,
};
use ambre_canvas::{
    AnchorPublisher, BusyPublisher, ContentRepaint, FilterOpenPublisher, FrameStats, InputEvent,
    LayerRepaint, PerfPublisher, PerfSnapshot, RepaintPlan, TextCache, TextInput, Theme,
    ThemeControl,
};
use ambre_commands::ClipboardWrite;
use ambre_core::id::{Document, Id};
use ambre_core::index::{Delta, Len, Pos};
use ambre_draw::{Clock, DrawSurface, FontContext, Rect};
#[cfg(target_arch = "wasm32")]
use leptos::prelude::RwSignal;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::{GetUntracked, Set};

use super::grid::{
    AxisPos, COL_WIDTH, CellEditor, CursorMove, HeaderHit, ROW_HEIGHT, SelKind, Selection,
    TrameUiState,
};
use super::renderer::{self, DrawStats};
use crate::render::{CommitDirection, FormulaAction, TextPass, TextPen};
use crate::{CellPos, CellRect, Col, MergeSpan, Row};
use ambre_core::{FracMs, Instant, PageLoad, TimeSpan};

/// The grid's logical `point` as a canvas point. The grid walks its
/// geometry in f64; the canvas reads the pointer in f32.
fn view_point(point: ambre_draw::Point) -> ambre_core::Point<ambre_core::Px<ambre_core::View>> {
    ambre_core::Point::new(
        ambre_core::Px::<ambre_core::View>::new(point.x as f32),
        ambre_core::Px::<ambre_core::View>::new(point.y as f32),
    )
}

/// Caret blink half-period: the standard ~530 ms on/off toggle (period ~1.06 s).
const BLINK_PERIOD: TimeSpan<FracMs> = TimeSpan::<FracMs>::new(530.0);

/// The one-slot viewport queue (`TRAME-UI-025`). Every request source writes
/// through [`coalesce`](Self::coalesce) or
/// [`coalesce_cells`](Self::coalesce_cells), so the slot holds at most one
/// request. The frame flush drains it once.
#[derive(Default)]
struct ViewportQueue {
    pending: Option<PendingViewport>,
}

/// The request the slot holds, and what it covers.
#[derive(Debug, Clone)]
enum PendingViewport {
    /// The whole window.
    Window(TrameCommand),
    /// The cells of a ranged change alone (`TRAME-UI-184`).
    Cells(TrameCommand),
}

impl PendingViewport {
    /// The command to send.
    fn command(&self) -> &TrameCommand {
        match self {
            Self::Window(cmd) | Self::Cells(cmd) => cmd,
        }
    }
}

impl ViewportQueue {
    /// Replace the pending request with `cmd`, a request for the whole
    /// window; the newest request wins.
    fn coalesce(&mut self, cmd: TrameCommand) {
        self.pending = Some(PendingViewport::Window(cmd));
    }

    /// Hold `cmd`, a request for the cells of a ranged change. A pending
    /// request for the whole window covers it, and two ranged requests become
    /// one over both, under the newer version.
    fn coalesce_cells(&mut self, cmd: TrameCommand) {
        self.pending = match self.pending.take() {
            None => Some(PendingViewport::Cells(cmd)),
            Some(window @ PendingViewport::Window(_)) => Some(window),
            Some(PendingViewport::Cells(held)) => {
                Some(PendingViewport::Cells(spanning(&held, cmd)))
            }
        };
    }

    /// Take the one pending request, if any, leaving the slot empty.
    fn take(&mut self) -> Option<PendingViewport> {
        self.pending.take()
    }

    /// Restore a request the queue just dropped (`BUS-024`).
    fn re_offer(&mut self, pending: PendingViewport) {
        self.pending = Some(pending);
    }
}

/// One request over the cells `held` and `newer` ask for, under the version
/// of `newer`. Either one that is no viewport request is kept as it is.
fn spanning(held: &TrameCommand, newer: TrameCommand) -> TrameCommand {
    let (
        TrameCommand::ScrollViewport {
            row_start: r0,
            row_end: r1,
            col_start: c0,
            col_end: c1,
            ..
        },
        TrameCommand::ScrollViewport {
            version,
            row_start,
            row_end,
            col_start,
            col_end,
        },
    ) = (held, &newer)
    else {
        return newer;
    };
    TrameCommand::ScrollViewport {
        version: *version,
        row_start: (*row_start).min(*r0),
        row_end: (*row_end).max(*r1),
        col_start: (*col_start).min(*c0),
        col_end: (*col_end).max(*c1),
    }
}

/// Compute the autofill destination rectangle from the source selection `src`
/// and the cell `at` under the dragged handle (`trame-autofill`). The fill
/// handle locks to a single axis: whichever direction the pointer extends
/// farthest from the source wins, and the destination always contains the
/// source.
///
/// `block` is the source's fill granularity. A plain cell source is
/// [`MergeSpan::CELL`], and the destination tracks the pointer cell exactly. A
/// merged-cell source carries its block size so the extension snaps to whole
/// blocks (Excel behaviour). A 2x1 merge fills two rows at a time.
fn fill_dest(src: CellRect, at: CellPos, block: MergeSpan) -> CellRect {
    let (first, last) = (src.top_left, src.bottom_right);
    let (bh, bw) = (block.rows.get().max(1), block.cols.get().max(1));
    // Extent the pointer reaches beyond the source on each axis.
    let drow = at.row.len_from(last.row).max(first.row.len_from(at.row));
    let dcol = at.col.len_from(last.col).max(first.col.len_from(at.col));
    // Whole blocks of `size` lines covering `reach` lines.
    let snap = |reach: u32, size: u32| reach.div_ceil(size).saturating_mul(size);
    if drow.get() >= dcol.get() {
        // Vertical fill: extend rows toward the pointer, snapped to whole blocks.
        if at.row > last.row {
            let grow = Len::<Row>::new(snap(at.row.len_from(last.row).get(), bh));
            CellRect {
                bottom_right: CellPos::new(last.row + grow, last.col),
                ..src
            }
        } else if at.row < first.row {
            let grow = Len::<Row>::new(snap(first.row.len_from(at.row).get(), bh));
            CellRect {
                top_left: CellPos::new(first.row - grow, first.col),
                ..src
            }
        } else {
            src
        }
    } else {
        // Horizontal fill: extend columns, snapped to whole blocks.
        if at.col > last.col {
            let grow = Len::<Col>::new(snap(at.col.len_from(last.col).get(), bw));
            CellRect {
                bottom_right: CellPos::new(last.row, last.col + grow),
                ..src
            }
        } else if at.col < first.col {
            let grow = Len::<Col>::new(snap(first.col.len_from(at.col).get(), bw));
            CellRect {
                top_left: CellPos::new(first.row, first.col - grow),
                ..src
            }
        } else {
            src
        }
    }
}

/// A mark the view drops on the browser's Performance panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PerfMark {
    /// A key or a text input reached the view.
    Input,
}

// Only the browser reads the names: a native build has no panel.
#[cfg(target_arch = "wasm32")]
impl PerfMark {
    /// The name the Performance panel shows.
    fn name(self) -> &'static str {
        match self {
            Self::Input => "ambre:input",
        }
    }
}

/// A measure the view draws on the browser's Performance panel, from the
/// mark it starts at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PerfMeasure {
    /// From an input to the frame that drew its caret.
    InputToCaret,
}

#[cfg(target_arch = "wasm32")]
impl PerfMeasure {
    /// The name the Performance panel shows.
    fn name(self) -> &'static str {
        match self {
            Self::InputToCaret => "ambre:input-to-caret",
        }
    }

    /// The mark the measure starts at.
    fn start(self) -> PerfMark {
        match self {
            Self::InputToCaret => PerfMark::Input,
        }
    }
}

/// Drop `mark` on the browser's Performance panel. A native build has no
/// panel.
fn performance_mark(mark: PerfMark) {
    #[cfg(target_arch = "wasm32")]
    if let Some(perf) = web_sys::window().and_then(|w| w.performance()) {
        let _ = perf.mark(mark.name());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = mark;
}

/// Draw `measure` from its start mark on the browser's Performance panel. A
/// native build has no panel.
fn performance_measure(measure: PerfMeasure) {
    #[cfg(target_arch = "wasm32")]
    if let Some(perf) = web_sys::window().and_then(|w| w.performance()) {
        let _ = perf.measure_with_start_mark(measure.name(), measure.start().name());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = measure;
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
/// reference, then its value or "empty".
pub(crate) fn cell_announcement(at: CellPos, value: &str) -> String {
    let at = format!(
        "{}{}",
        crate::columns::col_to_alpha(at.col),
        crate::columns::row_label(at.row)
    );
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
fn trigger_download(
    filename: &ambre_client_core::ExportFilename,
    data: &ambre_core::Payload<ambre_client_core::FileBytes>,
) {
    use wasm_bindgen::JsCast;
    let array = js_sys::Uint8Array::from(data.as_bytes());
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
        anchor.set_download(filename.as_str());
        anchor.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

/// Host-build stub: there is no DOM to download into outside the browser. Kept
/// so the exhaustive `dispatch_trame` compiles in headless tests.
#[cfg(not(target_arch = "wasm32"))]
fn trigger_download(
    filename: &ambre_client_core::ExportFilename,
    data: &ambre_core::Payload<ambre_client_core::FileBytes>,
) {
    #[cfg(test)]
    DOWNLOADS.with(|d| {
        d.borrow_mut()
            .push((filename.to_string(), data.as_bytes().to_vec()))
    });
    #[cfg(not(test))]
    let _ = (filename, data);
}

#[cfg(all(test, not(target_arch = "wasm32")))]
thread_local! {
    /// Each file the host stub was handed, in order: its name and its bytes
    /// (`PDF-003`).
    pub(crate) static DOWNLOADS: std::cell::RefCell<Vec<(String, Vec<u8>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Trame's view: a virtual-scroll spreadsheet grid (an `AppView::Trame`).
/// A deferred cell-edit commit (applied at the next paint, after the editor
/// flushes its queued intents). Set by grid keys / clicks that finish an edit.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PendingCommit {
    /// Commit and move the cursor by rows and columns (Enter, Tab, Up/Down).
    Nav(Delta<Row>, Delta<Col>),
    /// Commit in place, cursor unchanged (a click / right-click elsewhere; the
    /// click's own selection has already moved the active cell).
    Stay,
    /// Commit the value into every selected cell (Ctrl/Cmd+Enter).
    Fill,
}

/// The chrome signals a sheet view shares with the shell. The shell reads
/// what the view publishes and writes back what the user asks for. ambre-app
/// creates each signal once and hands the same set to the shell and the view.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub struct TrameSignals {
    /// The reference of the active cell, shown in the formula bar.
    pub formula_ref: RwSignal<String>,
    /// The reference of the whole selection, such as `B2:D5`, which the
    /// header of the context panel shows.
    pub selection_ref: RwSignal<crate::ui::panel::PanelSelection>,
    /// The page setup and the frozen pane of the active sheet, which the
    /// context panel shows. `None` before the first sheet list.
    pub sheet_setup: RwSignal<Option<crate::ui::panel::ActiveSheet>>,
    /// The text of the formula bar.
    pub formula_text: RwSignal<String>,
    /// What the formula bar asks the view to do.
    pub formula_action: RwSignal<Option<FormulaAction>>,
    /// The aggregates of the selection.
    pub sel_stats: RwSignal<Option<crate::chrome::SelStats>>,
    /// The scope of a right-click, read by the context menu.
    pub ctx_scope: RwSignal<super::grid::MenuScope>,
    /// The grid zoom the status bar selects.
    pub zoom: RwSignal<ambre_core::Scale<ambre_core::Doc, ambre_core::View>>,
    /// The active sheet.
    pub active_sheet: RwSignal<Id<ambre_core::id::Sheet>>,
    /// Whether a filter is active on the sheet.
    pub filter_active: RwSignal<crate::chrome::SheetFilter>,
    /// The find counters.
    pub find_result: RwSignal<Option<crate::chrome::FindPosition>>,
    /// A command the view performs itself, set by the command handler.
    pub view_cmd: RwSignal<Option<ambre_core::StaticKey<ambre_core::Commands>>>,
    /// What the shell's floating surfaces cover, as each one measures
    /// itself. A zoom to the selection reads it (`ZOOM-011`).
    pub cover: RwSignal<ambre_client_core::cover::Cover>,
}

/// The wiring of a sheet view: its channels to the Logic thread, its text
/// pipe, its publishers, its clock and its chrome signals. ambre-app builds
/// the parts and fills one per view.
pub struct TrameWiring {
    /// The queue to the Logic thread. The view moves it into its outbox,
    /// its one way to write (`BUS-011`).
    pub to_logic: Port<TrameCommand>,
    /// The queue from the Logic thread.
    pub from_logic: Feed<TrameEvent>,
    /// The shared text pipe.
    pub text_input: TextInput,
    /// When embedded as a `doc_embed` child (`toile-embeds`), the child view
    /// must not grab the shared text pipe on construction. The top-level
    /// document owns the caret until the user clicks into the embed. A
    /// top-level view passes `Standalone` and focuses immediately.
    pub embedding: ambre_canvas::Embedding,
    /// The screen position of the active cell.
    pub anchor: AnchorPublisher,
    /// The per-frame perf snapshot.
    pub perf_pub: PerfPublisher,
    /// The level a full outbox raises.
    pub busy_pub: BusyPublisher,
    /// The intent an in-canvas filter button raises.
    pub filter_open: FilterOpenPublisher,
    /// The active theme.
    pub theme_ctl: ThemeControl,
    /// The clock the view reads time from: the page's on screen, a test's
    /// natively.
    pub clock: Box<dyn Clock>,
    /// The chrome signals the view shares with the shell.
    #[cfg(target_arch = "wasm32")]
    pub signals: TrameSignals,
}

pub struct TrameView {
    /// The document this view renders (from the active route).
    document_id: Id<Document>,
    /// The one send path to the Logic thread. It keeps a lossless command a
    /// full queue could not take, and offers it again on a later frame
    /// (`BUS-011`). It drops a lossy one (`BUS-023`).
    outbox: Outbox<TrameCommand>,
    /// The busy level the chrome reads while the outbox holds anything
    /// (`BUS-016`).
    busy_pub: BusyPublisher,
    /// Inbound queue from the Logic thread, drained each frame in `update`.
    from_logic: Feed<TrameEvent>,

    /// Grid navigation + cell cache.
    state: TrameUiState,

    /// Transparent textarea pipe (keyboard + IME + clipboard). The view focuses
    /// it on a content click and slides it to the caret during an edit session.
    text_input: TextInput,
    /// Publishes the active cell's on-screen anchor (CSS px) so the chrome can
    /// transform-sync a floating selection toolbar onto it.
    anchor: AnchorPublisher,
    /// Last anchor published, so it is only re-published on change.
    last_anchor: Option<ambre_core::Point<ambre_core::Px<ambre_core::View>>>,
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
    ctx_scope: RwSignal<crate::ui::grid::MenuScope>,
    /// The scope the last right-click resolved, kept on every target so a
    /// host test reads what the browser would publish (`TRAME-UI-117`).
    menu_scope: crate::ui::grid::CtxScope,
    /// The point the last menu key named, kept on every target so a host test
    /// reads where the shell would open the menu (`TRAME-UI-189`).
    menu_key_at: Option<ambre_core::Point<ambre_core::Px<ambre_core::View>>>,

    /// Shared theme switch (chrome toggles it, the canvas reads it).
    theme_ctl: ThemeControl,
    /// Theme used to render the last frame; a change forces a full repaint.
    theme: Theme,
    /// Formula bar DOM signals (HTML `input`, synced with in-cell editor).
    /// Only present in the browser; native headless tests skip them.
    #[cfg(target_arch = "wasm32")]
    formula_ref: RwSignal<String>,
    #[cfg(target_arch = "wasm32")]
    selection_ref: RwSignal<crate::ui::panel::PanelSelection>,
    #[cfg(target_arch = "wasm32")]
    sheet_setup: RwSignal<Option<crate::ui::panel::ActiveSheet>>,
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
    zoom_sig: RwSignal<ambre_core::Scale<ambre_core::Doc, ambre_core::View>>,
    #[cfg(target_arch = "wasm32")]
    view_cmd: RwSignal<Option<ambre_core::StaticKey<ambre_core::Commands>>>,
    /// What the floating surfaces cover, as the chrome last measured it.
    #[cfg(target_arch = "wasm32")]
    cover: RwSignal<ambre_client_core::cover::Cover>,
    /// The active sheet, the filter flag and the find counter, published for
    /// the chrome. The view keeps each of them, so it alone writes these
    /// signals (`UI-THREAD-003`). Browser-only, like the formula signals.
    #[cfg(target_arch = "wasm32")]
    active_sheet: RwSignal<Id<ambre_core::id::Sheet>>,
    #[cfg(target_arch = "wasm32")]
    filter_active: RwSignal<crate::chrome::SheetFilter>,
    #[cfg(target_arch = "wasm32")]
    find_result: RwSignal<Option<crate::chrome::FindPosition>>,
    /// Active in-cell edit session, or `None` in navigation mode.
    editor: Option<CellEditor>,
    /// The token the next edit session opens under. Each session takes it and
    /// moves it on, so an event about a closed session matches no open editor.
    next_edit_session: ambre_bus::Token<crate::EditSession>,
    /// A commit requested from a grid key / click, deferred to the next paint. The
    /// shared editor applies edits only at paint-time flush, since a `FontContext`
    /// is live only then. The buffer is not final when the key is handled, so the
    /// commit runs after `draw_edit` flushes. `None` when no commit is pending.
    pending_commit: Option<PendingCommit>,
    /// The keys and texts typed while a commit waits for the paint flush. They
    /// belong after the commit, so the flush replays them once it ran
    /// (`TRAME-UI-183`).
    deferred_input: Vec<InputEvent>,
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
    pending_column_fit: Option<(Pos<Col>, Vec<crate::DisplayCell>)>,
    /// In-progress autofill drag from the fill handle, or `None`
    /// (`trame-autofill`). Holds the source rectangle the drag started from and
    /// the source block granularity. That is [`MergeSpan::CELL`] for a plain
    /// cell, or the merge span for a merged-cell source so the drag snaps block by
    /// block. Ctrl is read at *release* time, so toggling it mid-drag counts. It
    /// forces Copy on a multi-cell source and a +1 numeric Series on a single-cell
    /// one, Excel-style. Pointer moves grow [`TrameUiState::fill_preview`] and the
    /// release emits a `FillRange`.
    fill_drag: Option<(CellRect, MergeSpan)>,
    /// In-progress Excel-style header reorder drag, or `None`. Holds the grabbed
    /// origin column or row.
    /// Pointer moves update the [`TrameUiState::reorder_preview`] insertion slot,
    /// and the release emits a `MoveColumn` / `MoveRow` (a no-op if it lands back
    /// on the origin). Armed only when the press lands on an already-selected
    /// header body, so a plain click still selects/extends the axis.
    reorder_drag: Option<AxisPos>,
    /// The cursor currently set on `#ambre-canvas` from bare hover feedback
    /// (`trame-resize`, `trame-autofill`). Tracked to avoid redundant DOM
    /// writes every move.
    hover_cursor: GridCursor,
    /// The edit overlay changed this frame (keystroke, caret move, IME pass).
    edit_dirty: bool,
    /// Caret blink phase: `true` = caret drawn. Toggled every `BLINK_PERIOD`, reset
    /// to `true` (solid) on any edit interaction, as platforms do.
    caret_on: bool,
    /// Where the last paint laid out the in-cell caret, in canvas pixels. The
    /// pipe sits there while a cell is edited (`CHROME-007`).
    caret_drawn: Option<ambre_draw::Rect>,
    /// The clock's time of the last blink toggle (or blink reset).
    last_blink: Instant<PageLoad>,
    /// The clock's time of the input awaiting its caret paint, set by an edit
    /// interaction and consumed once the caret is next drawn. This is the
    /// keydown->caret latency probe, against the keystroke-to-draft budget of
    /// `ambre-spec-budgets.md`. `None` when idle.
    pending_input_mark: Option<Instant<PageLoad>>,

    /// The one clock the view reads: the blink, the scroll, the latency probe
    /// and the renderer's sub-phase timings. The page's on screen, a test's
    /// natively.
    clock: Box<dyn Clock>,

    text_pass: TextPass,
    /// Per-frame sub-phase breakdown produced by the renderer, shown on the HUD.
    draw_stats: DrawStats,
    initial_viewport_sent: bool,
    /// A selection-statistics request the queue refused, still owed to the
    /// band. Each frame offers it again until it leaves (`TRAME-UI-185`).
    stats_owed: bool,

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
    prev_top_row: Pos<Row>,
    prev_left_col: Pos<Col>,

    // -- Wheel / touchpad scroll ------------------------------------------------
    /// The scroll motion, built apart from the browser so a native test drives
    /// it (`scroll.rs`).
    scroll: scroll::GridScroll,
    /// Where a held selection or fill drag last pointed.
    drag_point: Option<ambre_core::Point<ambre_core::Px<ambre_core::View>>>,
    /// The last point the pointer was seen at, the anchor of a wheel zoom
    /// (`ZOOM-003`).
    pointer: Option<ambre_core::Point<ambre_core::Px<ambre_core::View>>>,

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

// The `impl TrameView` block is spread across sibling modules; all reach the
// free helpers, consts and types here via `use super::*`.
mod frame;
mod input;
mod interact;
mod lifecycle;
mod scroll;

/// A cursor the view sets on `#ambre-canvas` (`trame-resize`,
/// `trame-autofill`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum GridCursor {
    /// No cursor of the view's own: the stylesheet's applies.
    #[default]
    Unset,
    /// Over a header body or a filter button.
    Pointer,
    /// Over a column boundary, or dragging one.
    ColResize,
    /// Over a row boundary, or dragging one.
    RowResize,
    /// Over the fill handle.
    Crosshair,
}

impl GridCursor {
    /// The CSS `cursor` value, the empty one for [`Self::Unset`].
    #[cfg(target_arch = "wasm32")]
    pub(crate) const fn css(self) -> &'static str {
        match self {
            Self::Unset => "",
            Self::Pointer => "pointer",
            Self::ColResize => "col-resize",
            Self::RowResize => "row-resize",
            Self::Crosshair => "crosshair",
        }
    }
}

/// What the view does with a focused formula bar and no edit session.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FormulaBarSync {
    /// Nothing moved.
    Idle,
    /// The user typed: open an edit session seeded with what the bar holds.
    OpenSession,
    /// The cell changed under an untouched bar, a collaborator's write: show
    /// the new value, and open nothing.
    Refresh,
}

/// The formula bar as the view sees it for one frame (`trame-cell-editing`).
///
/// The browser copies its signals and its focus in before the sync, and copies
/// the result back out. A host test sets the fields itself.
#[derive(Default)]
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
    pub(crate) focused: BarFocus,
}

ambre_core::define_flag! {
    /// Whether the formula bar holds the keyboard focus.
    pub(crate) enum BarFocus {
        /// The focus is elsewhere: the grid, or outside the page.
        Blurred,
        /// The bar holds the focus, so a key types into it.
        Focused,
    }
}

/// The lengths alone: the reference names a sheet, and the texts are cell
/// content.
impl std::fmt::Debug for FormulaBar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let chars = |text: &str| text.chars().count();
        f.debug_struct("FormulaBar")
            .field("reference_chars", &chars(&self.reference))
            .field("text_chars", &chars(&self.text))
            .field("shown_chars", &chars(&self.shown))
            .field("action", &self.action)
            .field("focused", &self.focused)
            .finish()
    }
}

impl FormulaBar {
    /// Decide from the bar's text, the text the view last showed in it, and
    /// the active cell's stored `seed`. Only a text the user typed opens a
    /// session (`TRAME-UI-155`). A stored value that moved under the bar is a
    /// collaborator's. The session it would open is one the user never
    /// started.
    pub(crate) fn sync(&self, seed: &str) -> FormulaBarSync {
        if self.text != self.shown {
            FormulaBarSync::OpenSession
        } else if seed != self.shown {
            FormulaBarSync::Refresh
        } else {
            FormulaBarSync::Idle
        }
    }
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
            row_start: Pos::<Row>::ZERO,
            row_end: Pos::<Row>::new(10),
            col_start: Pos::<Col>::ZERO,
            col_end: Pos::<Col>::new(5),
        };
        let mut queue = ViewportQueue::default();
        queue.coalesce(scroll(ambre_core::Seq::nth(1)));
        queue.coalesce(scroll(ambre_core::Seq::nth(2)));
        assert!(
            matches!(
                queue.take().map(|p| p.command().clone()),
                Some(TrameCommand::ScrollViewport { version, .. })
                    if version == ambre_core::Seq::nth(2)
            ),
            "the newer request wins"
        );
        assert!(queue.take().is_none(), "one send per frame");
    }

    /// `CHROME-023`: the grid announces the active cell by its reference and
    /// its value, and says an empty cell is empty rather than nothing.
    #[test]
    fn the_active_cell_is_announced_by_reference_and_value() {
        let at = |row, col| CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col));
        assert_eq!(cell_announcement(at(1, 1), "42"), "B2, 42");
        assert_eq!(cell_announcement(at(0, 26), ""), "AA1, empty");
    }

    /// Only a text the user typed opens an edit session (`TRAME-UI-155`): a
    /// collaborator's write under an untouched bar refreshes it and opens
    /// nothing.
    #[test]
    fn only_a_typed_text_opens_an_edit_from_the_formula_bar() {
        let bar = |text: &str| FormulaBar {
            text: text.to_owned(),
            shown: "old".to_owned(),
            ..FormulaBar::default()
        };
        // The bar shows "old"; a peer rewrote the cell to "new".
        assert_eq!(bar("old").sync("new"), FormulaBarSync::Refresh);
        // The user typed a character.
        assert_eq!(bar("old5").sync("old"), FormulaBarSync::OpenSession);
        // The user typed while the peer wrote: the typing wins.
        assert_eq!(bar("old5").sync("new"), FormulaBarSync::OpenSession);
        assert_eq!(bar("old").sync("old"), FormulaBarSync::Idle);
    }

    /// The formula bar holds cell content, so its Debug form holds lengths.
    #[test]
    fn the_formula_bar_prints_lengths_alone() {
        let bar = FormulaBar {
            reference: "Payroll!A1".to_owned(),
            text: "=salary".to_owned(),
            shown: "salary".to_owned(),
            ..FormulaBar::default()
        };
        let printed = format!("{bar:?}");
        assert!(!printed.contains("Payroll"), "{printed}");
        assert!(!printed.contains("salary"), "{printed}");
        assert!(printed.contains("text_chars: 7"), "{printed}");
    }

    fn scroll_cmd(version: u64) -> TrameCommand {
        TrameCommand::ScrollViewport {
            version: ambre_core::Seq::nth(version),
            row_start: Pos::<Row>::ZERO,
            row_end: Pos::<Row>::ZERO,
            col_start: Pos::<Col>::ZERO,
            col_end: Pos::<Col>::ZERO,
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
            matches!(
                sent.command(),
                TrameCommand::ScrollViewport { version, .. }
                    if *version == ambre_core::Seq::nth(2)
            ),
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
            again.command(),
            TrameCommand::ScrollViewport { version, .. }
                if *version == ambre_core::Seq::nth(3)
        ));
        assert!(q.take().is_none());
    }

    /// A request over `rows` and `cols` under `version`.
    fn cells_cmd(version: u64, rows: (u32, u32), cols: (u32, u32)) -> TrameCommand {
        TrameCommand::ScrollViewport {
            version: ambre_core::Seq::nth(version),
            row_start: Pos::<Row>::new(rows.0),
            row_end: Pos::<Row>::new(rows.1),
            col_start: Pos::<Col>::new(cols.0),
            col_end: Pos::<Col>::new(cols.1),
        }
    }

    /// A pending request for the whole window covers a ranged one, which the
    /// slot then drops (`TRAME-UI-184`).
    #[test]
    fn a_pending_window_request_covers_a_ranged_one() {
        let mut q = ViewportQueue::default();
        q.coalesce(scroll_cmd(4));
        q.coalesce_cells(cells_cmd(5, (2, 3), (1, 1)));
        let sent = q.take().expect("pending");
        assert!(matches!(sent, PendingViewport::Window(_)), "{sent:?}");
        assert!(matches!(
            sent.command(),
            TrameCommand::ScrollViewport { version, .. } if *version == ambre_core::Seq::nth(4)
        ));
    }

    /// Two ranged requests leave one, over both and under the newer version,
    /// and a request for the whole window replaces them (`TRAME-UI-184`).
    #[test]
    fn two_ranged_requests_flush_as_one_over_both() {
        let mut q = ViewportQueue::default();
        q.coalesce_cells(cells_cmd(5, (2, 3), (1, 1)));
        q.coalesce_cells(cells_cmd(6, (7, 8), (0, 0)));
        assert_eq!(
            q.take().map(|p| format!("{:?}", p.command())),
            Some(format!("{:?}", cells_cmd(6, (2, 8), (0, 1))))
        );
        q.coalesce_cells(cells_cmd(7, (2, 3), (1, 1)));
        q.coalesce(scroll_cmd(8));
        assert!(matches!(q.take(), Some(PendingViewport::Window(_))));
    }
}
