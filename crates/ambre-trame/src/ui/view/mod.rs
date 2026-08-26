// ambre-trame/src/ui/view.rs
// ============================================================================
// TrameView -- the Trame per-frame view, a variant of the launcher's closed
// `AppView` registry.
//
// Owns everything that used to live in ambre-app's `UiState`: the grid
// navigation state, the inbound/outbound logic-thread channels, the dirty-rect
// bookkeeping, and the per-frame repaint decision. The launcher's FrameHost
// drives it through `AppView` (update -> plan_repaint -> paint_*), knowing
// nothing about spreadsheets. The driver-facing methods are inherent `pub`
// methods (see the dedicated impl block).
// ============================================================================

use ambre_bus::{
    AppCommand, AppEvent, DocumentId, FillMode, GridRange, LogicCommand, TrameCommand, TrameEvent,
    UiEvent,
};
use ambre_canvas::{
    Anchor, AnchorPublisher, ContentRepaint, FilterOpenPublisher, FrameStats, InputEvent,
    OverlayRepaint, PerfPublisher, PerfSnapshot, RepaintPlan, TextCache, TextInput, Theme,
    ThemeControl, WgpuSurface,
};
use ambre_draw::{Clock, FontContext, Rect};
#[cfg(target_arch = "wasm32")]
use leptos::prelude::RwSignal;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::{Get, GetUntracked, Set};

use super::grid::{
    COL_WIDTH, CellEditor, CellRect, CursorMove, HeaderHit, ROW_HEIGHT, SelKind, Selection,
    TrameUiState,
};
use super::renderer::{self, DrawStats};
use crate::TOTAL_COLS;
#[cfg(target_arch = "wasm32")]
use crate::render::{CommitDirection, FormulaAction};

/// Caret blink half-period: the standard ~530 ms on/off toggle (period ~1.06 s).
const BLINK_MS: f64 = 530.0;

/// Compute the autofill destination rectangle from the source selection `src`
/// and the cell `(row, col)` under the dragged handle (`trame-autofill`). The
/// fill handle locks to a single axis: whichever direction the pointer extends
/// farthest from the source wins, and the destination always contains the
/// source. Returns the normalized rectangle `(r1, c1, r2, c2)`.
///
/// `block` is the source's fill granularity `(rows, cols)`: a plain cell source
/// is `(1, 1)` and the destination tracks the pointer cell exactly, while a
/// merged-cell source carries its block size so the extension snaps to whole
/// blocks (Excel behaviour -- a 2x1 merge fills two rows at a time).
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

/// `Clock` adapter over the browser `performance.now()`. The renderer takes a
/// `&dyn Clock` for its sub-phase timings so it can also run headless (tests
/// pass `ambre_draw::testing::FixedClock`); on screen this wraps `Performance`.
struct PerfClock<'a>(&'a web_sys::Performance);

impl Clock for PerfClock<'_> {
    fn now_ms(&self) -> f64 {
        self.0.now()
    }
}

// -- Kinetic (momentum) touchpad scrolling ------------------------------------
// Firefox/Linux does not deliver an OS momentum phase after a two-finger flick,
// so we synthesise it: track the scroll velocity while the fingers drive, then
// coast with friction once the wheel events stop.
//
/// Gap (ms) with no `wheel` event after which the gesture is considered ended
/// and coasting may begin. Continuous touchpad scrolling fires well under this.
const FLING_GAP_MS: f64 = 60.0;
/// Velocity retained per 16.667 ms of coasting (≈ exponential friction). This
/// is a *time constant*, not a per-frame factor: the decay is raised to
/// `dt / 16.667` each frame, so the deceleration curve over wall-clock time is
/// identical at 60 Hz, 240 Hz, or any variable refresh rate.
const FLING_FRICTION: f64 = 0.94;
/// Minimum speed (physical px/ms) at gesture end to bother coasting at all.
const FLING_START_PX_PER_MS: f64 = 0.05;
/// Speed (physical px/ms) below which coasting stops.
const FLING_STOP_PX_PER_MS: f64 = 0.012;
/// Smoothing for the running velocity estimate: weight of the newest sample.
const FLING_VELOCITY_BLEND: f64 = 0.8;
/// A single pixel-mode `wheel` delta at or above this (physical px) is treated
/// as a notched mouse wheel, not a touchpad glide: applied 1:1 with no inertia.
/// Touchpad events stay well below this; a mouse notch (Chrome ~100 px/dpr,
/// Firefox uses line mode anyway) sits comfortably above it.
const DISCRETE_WHEEL_PX: f64 = 40.0;

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
/// `flume::TrySendError` variants without carrying the (large) `LogicCommand`
/// payload, so the `Err` path stays cheap (see `clippy::result_large_err`).
pub(crate) enum SendError {
    /// The channel buffer is full; the caller may retry or alert the user.
    Full,
    /// The Logic thread is gone; the channel is permanently disconnected.
    Disconnected,
}

/// Spreadsheet column label for a 0-based index (0 -> "A", 26 -> "AA"). Used for
/// the screen-reader cell announcement.
fn col_label(col: u32) -> String {
    crate::columns::col_to_alpha(col)
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
/// `trame-import-export`). Wraps the bytes in a `Blob`, mints an
/// object URL, and clicks a synthetic `<a download>`; the URL is revoked right
/// after so it does not leak. Best-effort: any missing DOM handle is logged and
/// skipped (the export bytes are already computed, only the download is lost).
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
    /// Outbound channel to the Logic thread.
    to_logic: flume::Sender<LogicCommand>,
    /// Inbound channel from the Logic thread, drained each frame in `update`.
    from_logic: flume::Receiver<UiEvent>,

    /// Grid navigation + cell cache.
    state: TrameUiState,

    /// Transparent textarea pipe (keyboard + IME + clipboard). The view focuses
    /// it on a content click and slides it to the caret during an edit session.
    text_input: TextInput,
    /// devicePixelRatio: the grid works in physical pixels, the textarea
    /// positions in CSS pixels, so caret coords are divided by this.
    dpr: f64,
    /// Publishes the active cell's on-screen anchor (CSS px) so the chrome can
    /// transform-sync a floating selection toolbar onto it.
    anchor: AnchorPublisher,
    /// Last anchor published, so it is only re-published on change.
    last_anchor: Option<Anchor>,
    /// Publishes the per-frame perf snapshot (fps + timings) so the DOM status
    /// bar can render it -- the HUD left the wgpu overlay (`trame-status-bar`).
    perf_pub: PerfPublisher,
    /// One-shot canvas->launcher channel raised when an in-canvas autofilter
    /// button is clicked (`trame-shared-filter`). The launcher consumes it to open the
    /// DOM `FilterPopover` over the active cell -- the same path as the
    /// context-menu "Filter by column" entry, so there is one filter code path.
    filter_open: FilterOpenPublisher,
    /// Status-bar selection aggregates signal (`trame-status-bar`). The view
    /// clears it to `None` on a single-cell selection; the Logic round-trip
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
    /// Current selection rectangle `(r1, c1, r2, c2)`, published on every
    /// selection change so the launcher can open the multi-column `SortDialog`
    /// over the selection's columns (`trame-sort`). Browser-only.
    #[cfg(target_arch = "wasm32")]
    sel_bounds: RwSignal<Option<(u32, u32, u32, u32)>>,

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
    #[cfg(target_arch = "wasm32")]
    formula_action: RwSignal<Option<FormulaAction>>,
    /// Grid zoom factor (`1.0` = 100%) shared with the status-bar zoom menu
    /// (`trame-status-bar`): the menu writes it, the view polls it each frame and
    /// applies it via [`set_zoom`](Self::set_zoom). Browser-only, like the formula
    /// signals; pure local view state, never synced.
    #[cfg(target_arch = "wasm32")]
    zoom_sig: RwSignal<f64>,
    /// Active in-cell edit session, or `None` in navigation mode.
    editor: Option<CellEditor>,
    /// A commit requested from a grid key / click, deferred to the next paint.
    /// The shared editor applies edits only at paint-time flush (a `FontContext`
    /// is live only then), so the buffer is not final when the key is handled;
    /// the commit runs after `draw_edit` flushes. `None` when no commit is pending.
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
    /// A pending column/row auto-fit (`trame-resize`) awaiting the parley
    /// font context: set on a border double-click, resolved and cleared by the
    /// next [`paint_content`](Self::paint_content) which owns the measurement.
    pending_autofit: Option<HeaderHit>,
    /// In-progress autofill drag from the fill handle, or `None`
    /// (`trame-autofill`). Holds the source rectangle the drag started from and
    /// the source block granularity `(rows, cols)` -- `(1, 1)` for a plain cell,
    /// the merge
    /// span for a merged-cell source so the drag snaps block by block. Ctrl is
    /// read at *release* time (forces Copy on a multi-cell source and a +1 numeric
    /// Series on a single-cell one, Excel-style), so toggling it mid-drag counts.
    /// Pointer moves grow [`TrameUiState::fill_preview`] and the release emits a
    /// `FillRange`.
    fill_drag: Option<(CellRect, (u32, u32))>,
    /// In-progress Excel-style header reorder drag, or `None`. Holds the grabbed
    /// axis and the origin column/row index (`(Cols, from)` / `(Rows, from)`);
    /// pointer moves update the [`TrameUiState::reorder_preview`] insertion slot
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
    /// `performance.now()` of the last blink toggle (or blink reset).
    last_blink_ms: f64,
    /// `performance.now()` of the input awaiting its caret paint, set by an edit
    /// interaction and consumed once the caret is next drawn -- the keydown->caret
    /// latency probe, against the keystroke-to-draft budget of
    /// `ambre-spec-budgets.md`. `None` when idle.
    pending_input_mark: Option<f64>,

    /// `performance.now()` handle for the renderer's sub-phase timings.
    perf: web_sys::Performance,

    text_enabled: bool,
    /// Per-frame sub-phase breakdown produced by the renderer, shown on the HUD.
    draw_stats: DrawStats,
    initial_viewport_sent: bool,

    /// Latest viewport request awaiting send, coalesced to ≤1 per frame so a
    /// fast key-repeat can't outrun the Logic thread. Set in `handle_input`,
    /// flushed in `update`. Only the newest is kept (it carries the highest
    /// `version`, which supersedes any it replaces).
    pending_viewport: Option<TrameCommand>,
    /// Set once the inbound Logic channel disconnects (Logic thread gone) or a
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
    /// Sub-cell wheel residue (physical px) carried between events, so slow
    /// touchpad scrolls still accumulate into whole rows/cols rather than being
    /// truncated away each frame.
    scroll_accum_x: f64,
    scroll_accum_y: f64,
    /// Running scroll velocity estimate (physical px/ms) from recent wheel
    /// events; drives the momentum coast once the fingers lift.
    fling_vx: f64,
    fling_vy: f64,
    /// Timestamp (perf ms) of the last wheel event; `>0` means a gesture is in
    /// progress / just ended and we should watch for the fling gap.
    last_wheel_ms: f64,
    /// True while coasting after a flick; cleared by a new wheel or a stop.
    coasting: bool,
    /// Timestamp (perf ms) of the last coast step, for the per-frame integration.
    last_coast_ms: f64,

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
    /// for a full-fidelity same-tab paste that never depends on the OS custom
    /// MIME type surviving the round-trip (`trame-clipboard`).
    last_clip: Option<ambre_bus::ClipboardBlob>,
}

#[allow(clippy::too_many_arguments)]
// The `impl TrameView` block is spread across sibling modules; all reach the
// free helpers, consts and types here via `use super::*`.
mod frame;
mod input;
mod interact;
mod lifecycle;
