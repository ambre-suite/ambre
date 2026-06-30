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
    COL_WIDTH, CellEditor, CellRect, CursorMove, HeaderHit, HitKind, PendingHit, ROW_HEIGHT,
    SelKind, Selection, TrameUiState,
};
use super::renderer::{self, DrawStats};
use crate::TOTAL_COLS;
#[cfg(target_arch = "wasm32")]
use crate::render::{CommitDirection, FormulaAction};

/// Caret blink half-period: the standard ~530 ms on/off toggle (period ~1.06 s).
const BLINK_MS: f64 = 530.0;

/// Compute the autofill destination rectangle from the source selection `src`
/// and the cell `(row, col)` under the dragged handle (`-ui.md` sec.5.F). The
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
enum ScrollOutcome {
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
enum SendError {
    /// The channel buffer is full; the caller may retry or alert the user.
    Full,
    /// The Logic thread is gone; the channel is permanently disconnected.
    Disconnected,
}

/// Spreadsheet column label for a 0-based index (0 -> "A", 26 -> "AA"). Used for
/// the screen-reader cell announcement.
fn col_label(mut col: u32) -> String {
    let mut label = String::new();
    loop {
        label.insert(0, (b'A' + (col % 26) as u8) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    label
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
/// `ambre-spec-trame-interop.md` §3/§4). Wraps the bytes in a `Blob`, mints an
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
    /// bar can render it -- the HUD migrated off the wgpu overlay (sec.13bis).
    perf_pub: PerfPublisher,
    /// One-shot canvas->launcher channel raised when an in-canvas autofilter
    /// button is clicked (`-ui.md` sec.5.D). The launcher consumes it to open the
    /// DOM `FilterPopover` over the active cell -- the same path as the
    /// context-menu "Filter by column" entry, so there is one filter code path.
    filter_open: FilterOpenPublisher,
    /// Status-bar selection aggregates signal (`-ui.md` sec.13bis). The view
    /// clears it to `None` on a single-cell selection; the Logic round-trip
    /// (`RequestSelectionStats` -> `SelectionStats`) sets it from the launcher's
    /// event loop on a multi-cell one. Browser-only, like the formula signals.
    #[cfg(target_arch = "wasm32")]
    sel_stats: RwSignal<Option<crate::chrome::SelStats>>,
    /// Which surface the last right-click landed on (cell vs header), published
    /// so the launcher swaps the context-menu layout (`-ui.md` sec.5.C). Set in
    /// the right-click `PointerDown` handler, which runs synchronously before the
    /// DOM `contextmenu` listener opens the menu. Browser-only.
    #[cfg(target_arch = "wasm32")]
    ctx_scope: RwSignal<crate::ui::grid::CtxScope>,
    /// Current selection rectangle `(r1, c1, r2, c2)`, published on every
    /// selection change so the launcher can open the multi-column `SortDialog`
    /// over the selection's columns (`-ui.md` sec.5.C). Browser-only.
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
    /// (`-ui.md` sec.13bis): the menu writes it, the view polls it each frame and
    /// applies it via [`set_zoom`](Self::set_zoom). Browser-only, like the formula
    /// signals; pure local view state, never synced.
    #[cfg(target_arch = "wasm32")]
    zoom_sig: RwSignal<f64>,
    /// Active in-cell edit session, or `None` in navigation mode.
    editor: Option<CellEditor>,
    /// True while the pointer is dragging out a text selection inside the open
    /// editor (between a press in the cell and its release).
    editor_text_drag: bool,
    /// True while the pointer is dragging out a grid cell-range selection in
    /// navigation mode (between a press on a cell and its release). Pointer
    /// moves while set extend the live selection rectangle.
    selecting_drag: bool,
    /// In-progress whole-axis (column/row) selection drag from a header body
    /// (`-ui.md` sec.4bis.A), or `None`. Holds which axis is being swept; pointer
    /// moves extend the band to the column/row under the cursor.
    axis_drag: Option<crate::ui::grid::SelKind>,
    /// In-progress header-boundary resize drag (`-ui.md` sec.4bis.C), or `None`.
    /// Holds the [`HeaderHit::ColBorder`] / [`HeaderHit::RowBorder`] being dragged
    /// so the release knows which column/row to resize; pointer moves update the
    /// live guide line ([`TrameUiState::resize_preview`]).
    resize_drag: Option<HeaderHit>,
    /// A pending column/row auto-fit (`-ui.md` sec.4bis.C) awaiting the parley
    /// font context: set on a border double-click, resolved and cleared by the
    /// next [`paint_content`](Self::paint_content) which owns the measurement.
    pending_autofit: Option<HeaderHit>,
    /// In-progress autofill drag from the fill handle, or `None` (`-ui.md`
    /// sec.5.F). Holds the source rectangle the drag started from and the source
    /// block granularity `(rows, cols)` -- `(1, 1)` for a plain cell, the merge
    /// span for a merged-cell source so the drag snaps block by block. Ctrl is
    /// read at *release* time (forces Copy on a multi-cell source and a +1 numeric
    /// Series on a single-cell one, Excel-style), so toggling it mid-drag counts.
    /// Pointer moves grow [`TrameUiState::fill_preview`] and the release emits a
    /// `FillRange`.
    fill_drag: Option<(CellRect, (u32, u32))>,
    /// The CSS cursor currently set on `#ambre-canvas` from bare hover feedback
    /// (sec.4bis.B / sec.5.F): `"col-resize"` / `"row-resize"` / `"pointer"` /
    /// `"crosshair"` / `""`. Tracked to avoid redundant DOM writes every move.
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
    /// latency probe (ambre-spec-chrome 7, Phase 3 profiling). `None` when idle.
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
    /// (`-ui.md` sec.5.E). UI-only view state.
    cut_pending: bool,
}

#[allow(clippy::too_many_arguments)]
impl TrameView {
    pub fn new(
        document_id: DocumentId,
        to_logic: flume::Sender<LogicCommand>,
        from_logic: flume::Receiver<UiEvent>,
        canvas_width: f64,
        canvas_height: f64,
        text_input: TextInput,
        anchor: AnchorPublisher,
        perf_pub: PerfPublisher,
        filter_open: FilterOpenPublisher,
        theme_ctl: ThemeControl,
        #[cfg(target_arch = "wasm32")] formula_ref: RwSignal<String>,
        #[cfg(target_arch = "wasm32")] formula_text: RwSignal<String>,
        #[cfg(target_arch = "wasm32")] formula_action: RwSignal<Option<FormulaAction>>,
        #[cfg(target_arch = "wasm32")] sel_stats: RwSignal<Option<crate::chrome::SelStats>>,
        #[cfg(target_arch = "wasm32")] ctx_scope: RwSignal<crate::ui::grid::CtxScope>,
        #[cfg(target_arch = "wasm32")] sel_bounds: RwSignal<Option<(u32, u32, u32, u32)>>,
        #[cfg(target_arch = "wasm32")] zoom_sig: RwSignal<f64>,
    ) -> Self {
        let window = web_sys::window().expect("no window");
        let perf = window.performance().expect("no Performance API");
        let dpr = window.device_pixel_ratio();
        let theme = theme_ctl.get();

        // Focus the pipe up front so the keyboard works without a click; park it
        // on the active cell so a first keystroke or IME opens there.
        text_input.focus();

        Self {
            document_id,
            to_logic,
            from_logic,
            state: TrameUiState::new(canvas_width, canvas_height),
            text_input,
            dpr,
            anchor,
            last_anchor: None,
            perf_pub,
            filter_open,
            #[cfg(target_arch = "wasm32")]
            sel_stats,
            #[cfg(target_arch = "wasm32")]
            ctx_scope,
            #[cfg(target_arch = "wasm32")]
            sel_bounds,
            theme_ctl,
            theme,
            #[cfg(target_arch = "wasm32")]
            formula_ref,
            #[cfg(target_arch = "wasm32")]
            formula_text,
            #[cfg(target_arch = "wasm32")]
            formula_action,
            #[cfg(target_arch = "wasm32")]
            zoom_sig,
            editor: None,
            editor_text_drag: false,
            selecting_drag: false,
            axis_drag: None,
            resize_drag: None,
            pending_autofit: None,
            fill_drag: None,
            hover_cursor: String::new(),
            edit_dirty: false,
            caret_on: true,
            last_blink_ms: 0.0,
            pending_input_mark: None,
            perf,
            text_enabled: true,
            draw_stats: DrawStats::default(),
            initial_viewport_sent: false,
            pending_viewport: None,
            fatal: false,
            needs_full: true,
            viewport_changed: false,
            data_dirty: false,
            prev_top_row: 0,
            prev_left_col: 0,
            scroll_accum_x: 0.0,
            scroll_accum_y: 0.0,
            fling_vx: 0.0,
            fling_vy: 0.0,
            last_wheel_ms: 0.0,
            coasting: false,
            last_coast_ms: 0.0,
            cut_pending: false,
        }
    }

    /// Send a lossy `TrameCommand` to the Logic thread (viewport requests).
    ///
    /// Lossy: drops the command when the channel is full. Viewport requests are
    /// idempotent and coalesced (only the latest `pending_viewport` is kept), so
    /// a dropped frame is naturally superseded by the next one.
    ///
    /// Runs on the page's main thread, so it must never block.
    fn send_command_lossy(&self, cmd: TrameCommand) {
        if self.fatal {
            return;
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: AppCommand::Trame(cmd),
        };
        match self.to_logic.try_send(msg) {
            Ok(()) => {}
            Err(flume::TrySendError::Full(_)) => {
                tracing::warn!("Trame: UI→Logic channel full, dropping lossy command");
            }
            Err(flume::TrySendError::Disconnected(_)) => {
                tracing::error!("Trame: UI→Logic channel disconnected");
            }
        }
    }

    /// Send a lossless `TrameCommand` to the Logic thread (edits, cell
    /// selection).
    ///
    /// Lossless: returns the `TrySendError` on `Full` so the caller can decide
    /// whether to retry or alert the user. The UI thread never blocks; backpressure
    /// is surfaced synchronously at the call-site.
    fn send_command_lossless(&self, cmd: TrameCommand) -> Result<(), SendError> {
        if self.fatal {
            return Ok(());
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: AppCommand::Trame(cmd),
        };
        self.to_logic.try_send(msg).map_err(|err| match err {
            flume::TrySendError::Full(_) => SendError::Full,
            flume::TrySendError::Disconnected(_) => SendError::Disconnected,
        })
    }

    /// Enter the inert fatal state: a worker thread is gone, so no command can
    /// be answered. Logged exactly once; the page may surface a fatal screen
    /// (see the `ambre-spec-architecture` "thread failure" section).
    fn signal_fatal(&mut self, reason: &str) {
        if self.fatal {
            return;
        }
        self.fatal = true;
        tracing::error!("Trame: fatal — {reason}; the UI is now inert");
        // Latch the canvas terminal screen. A Sync-worker death is a SEPARATE
        // wasm instance, so it reaches us only as this channel disconnect; the
        // panic-hook latch never fires for it. Forward it to the host here.
        ambre_canvas::signal_fatal(reason);
    }

    // -- Edit session ----------------------------------------------------------

    /// Announce the active cell to screen readers via the live region (content
    /// a11y, 2.F): the cell reference plus its value, e.g. `"B2, 42"`. Called on
    /// every selection change so navigating the grid is audible, like a native
    /// spreadsheet.
    fn announce_active(&self) {
        let (row, col) = (self.state.active_row(), self.state.active_col());
        let value = self.cell_seed(row, col);
        let desc = if value.is_empty() {
            format!("{}{}, empty", col_label(col), row + 1)
        } else {
            format!("{}{}, {}", col_label(col), row + 1, value)
        };
        self.text_input.announce(&desc);
        self.refresh_selection_stats();
    }

    /// Refresh the status-bar selection aggregates (`-ui.md` sec.13bis) after a
    /// selection change. A lone cell shows nothing, so we clear the signal
    /// locally; any multi-cell selection asks the Logic thread (which holds every
    /// cell, not just the viewport) for the aggregates, replied as
    /// [`TrameEvent::SelectionStats`] and mirrored into the signal by the
    /// launcher's event loop.
    fn refresh_selection_stats(&self) {
        if self.state.selection.is_multi() {
            self.send_command_lossy(TrameCommand::RequestSelectionStats);
        } else {
            #[cfg(target_arch = "wasm32")]
            self.sel_stats.set(None);
        }
        // Publish the current selection rectangle so the launcher can open the
        // multi-column sort dialog over the selection's columns (sec.5.C).
        #[cfg(target_arch = "wasm32")]
        self.sel_bounds.set(Some(self.state.selection.bounds()));
    }

    /// The active cell's spreadsheet reference (e.g. `"B2"`), shown in the
    /// formula bar's name box (spec sec.6).
    #[cfg(target_arch = "wasm32")]
    fn active_cell_ref(&self) -> String {
        format!(
            "{}{}",
            col_label(self.state.active_col()),
            self.state.active_row() + 1
        )
    }

    /// Recompute the active cell's on-screen anchor and publish it for the
    /// floating selection toolbar -- the centre of the cell's top edge in CSS
    /// px, or `None` when the cell is scrolled out of the grid area. Cheap;
    /// called every frame so the toolbar tracks pans, but only forwarded to the
    /// chrome on change.
    fn publish_anchor(&mut self) {
        let next = self
            .state
            .cell_pixel_rect(self.state.active_row(), self.state.active_col())
            .map(|(x, y, w, _h)| Anchor {
                x: (x + w / 2.0) / self.dpr,
                y: y / self.dpr,
            });
        if next != self.last_anchor {
            self.last_anchor = next;
            self.anchor.publish(next);
        }
    }

    /// Show the caret solid and restart its blink clock. Called on every edit
    /// interaction (typing, IME, caret motion) -- the one chokepoint where user
    /// input mutates the editor, so it also arms the input->caret latency probe.
    fn reset_blink(&mut self) {
        self.caret_on = true;
        self.last_blink_ms = self.perf.now();
        self.edit_dirty = true;
        // Arm the latency probe: timestamp now (~keydown), and drop a
        // `performance.mark` so the measure shows in the browser Performance panel.
        self.pending_input_mark = Some(self.perf.now());
        let _ = self.perf.mark("ambre:input");
    }

    /// Seed text for an editor opened on `(row, col)`: the cell's current text,
    /// or empty if not cached.
    fn cell_seed(&self, row: u32, col: u32) -> String {
        self.state
            .cell_cache
            .get(&(row, col))
            .map(|c| c.raw.clone())
            .unwrap_or_default()
    }

    /// Slide the textarea pipe onto a cell (CSS px) so the IME popup anchors
    /// there. The grid works in physical px; the DOM positions in CSS px.
    fn park_input_on(&self, row: u32, col: u32) {
        if let Some((x, y, _, h)) = self.state.cell_pixel_rect(row, col) {
            self.text_input
                .set_caret(x / self.dpr, y / self.dpr, h / self.dpr);
        }
    }

    /// Open an edit session on the active cell, seeded with `seed`. Focuses the
    /// pipe (clearing its buffer) and positions it on the cell.
    fn begin_edit(&mut self, seed: String) {
        let (row, col) = (self.state.active_row(), self.state.active_col());
        self.editor = Some(CellEditor::new(row, col, seed.clone()));
        self.editor_text_drag = false;
        self.text_input.clear();
        self.text_input.focus();
        self.park_input_on(row, col);
        self.reset_blink();
        #[cfg(target_arch = "wasm32")]
        self.formula_text.set(seed);
    }

    /// Open an edit session on the active cell WITHOUT touching focus or the
    /// formula bar text. Used when an edit originates from the HTML formula bar
    /// input (which keeps DOM focus and owns the buffer): the in-cell editor is
    /// only the rendering/commit vehicle, so stealing focus to the textarea pipe
    /// (as [`begin_edit`](Self::begin_edit) does) would abort the user's typing.
    #[cfg(target_arch = "wasm32")]
    fn begin_edit_external(&mut self) {
        let (row, col) = (self.state.active_row(), self.state.active_col());
        let seed = self.cell_seed(row, col);
        self.editor = Some(CellEditor::new(row, col, seed));
        self.editor_text_drag = false;
        self.reset_blink();
    }

    /// Commit the current edit into the CRDT (via Logic) and update the local
    /// cache optimistically, then leave edit mode. No-op if not editing.
    fn commit_edit(&mut self) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        tracing::info!(
            "[UI] commit_edit → sending SetCellText row={} col={} text='{:.80}'",
            editor.row,
            editor.col,
            editor.buffer,
        );
        if let Err(SendError::Full) = self.send_command_lossless(TrameCommand::SetCellText {
            row: editor.row,
            col: editor.col,
            text: editor.buffer.clone(),
        }) {
            tracing::error!("[UI] channel full, edit dropped — retry next frame");
        }
        // Close the undo group on cell exit so the whole edit is one undo item
        // (§6.C). Lossless: it must not be dropped, or the group would stay open
        // until the next command and could absorb a later edit.
        let _ = self.send_command_lossless(TrameCommand::CommitEdit);
        self.state
            .set_cell_text(editor.row, editor.col, editor.buffer);
        // Content layer must repaint to show the committed text; overlay too, to
        // drop the edit field.
        self.needs_full = true;
        self.edit_dirty = true;
        self.text_input.clear();
        // Announce the committed value to assistive tech.
        self.announce_active();
    }

    /// Commit the current edit's value into *every* selected cell (Ctrl+Enter),
    /// then leave edit mode keeping the selection and active cell put. Each cell
    /// gets the same raw text; the whole fill is one undo group (a single
    /// `CommitEdit` seals it). For a lone single-cell selection this is identical
    /// to [`commit_edit`](Self::commit_edit) but without the cursor advance.
    ///
    /// Note: the raw text is copied verbatim, so a filled *formula* keeps its
    /// references as typed (no per-cell relative reindexing -- that lives in the
    /// Logic thread's paste/fill path, not the UI).
    fn commit_edit_filled(&mut self) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        let text = editor.buffer;
        let cells = self.state.selection.selected_cells();
        tracing::info!(
            "[UI] commit_edit_filled -> {} cells, text='{:.80}'",
            cells.len(),
            text
        );
        // One batch command so the whole fill is a single undo item (a stream of
        // SetCellText would be one undo item per cell).
        let batch = cells.iter().map(|&(r, c)| (r, c, text.clone())).collect();
        if let Err(SendError::Full) =
            self.send_command_lossless(TrameCommand::SetCellsBatch { cells: batch })
        {
            tracing::error!("[UI] channel full, fill batch dropped");
        }
        // Optimistic local update, mirroring the batch.
        for &(row, col) in &cells {
            self.state.set_cell_text(row, col, text.clone());
        }
        self.needs_full = true;
        self.edit_dirty = true;
        self.text_input.clear();
        self.announce_active();
    }

    /// Abandon the current edit without writing anything, then leave edit mode.
    fn cancel_edit(&mut self) {
        if self.editor.take().is_some() {
            // Content is untouched; only the overlay field must be cleared. The
            // formula bar reverts to the (unchanged) active cell's stored value,
            // not blank -- blanking it would read as "user typed empty" next
            // frame and could re-open an edit that clears the cell.
            self.edit_dirty = true;
            self.text_input.clear();
            #[cfg(target_arch = "wasm32")]
            self.formula_text
                .set(self.cell_seed(self.state.active_row(), self.state.active_col()));
        }
    }
}

/// The per-frame driver surface: the inherent methods the launcher's `AppView`
/// registry dispatches to (call order each frame: `update` -> `plan_repaint` ->
/// `paint_content` / `paint_overlay`; `resize` / `handle_input` / `set_document`
/// are out-of-band). Inherent methods: the launcher owns the closed `AppView`
/// enum (no upstream trait, no dependency cycle -- ambre-spec-architecture.md 6).
impl TrameView {
    /// Swap the active document in place (SPA route change). Drop every
    /// document-scoped field back to its `new()` value while preserving the
    /// shared, route-independent plumbing (channels, text pipe, anchor, theme)
    /// and the current canvas size, then force a full repaint and a fresh
    /// initial viewport fetch for the new document. No-op if already current.
    pub fn set_document(&mut self, document_id: DocumentId) {
        if self.document_id == document_id {
            return;
        }
        self.document_id = document_id;
        self.state = TrameUiState::new(self.state.canvas_width, self.state.canvas_height);
        self.editor = None;
        self.edit_dirty = false;
        self.initial_viewport_sent = false;
        self.pending_viewport = None;
        self.needs_full = true;
        self.viewport_changed = false;
        self.data_dirty = false;
        self.prev_top_row = 0;
        self.prev_left_col = 0;
        self.cut_pending = false;
        // Retract any floating-toolbar anchor from the previous document.
        self.last_anchor = None;
        self.anchor.publish(None);
    }

    /// Canvas-side reaction to a single [`TrameEvent`] (`-ui.md` sec.18 "full
    /// TrameEvent dispatch"). Exhaustive on purpose: a new variant must fail the
    /// build here rather than be dropped silently. Variants that only affect the
    /// DOM chrome (toasts, formatting toolbar, undo/redo enablement) are explicit
    /// no-ops -- they are handled in the per-frame relay (`ambre-app`), not here.
    fn dispatch_trame(&mut self, te: TrameEvent) {
        match te {
            // Cell content batch: merge into the cache and flag data dirty. If a
            // scroll happens the same frame the strip path reads the refreshed
            // cache; otherwise plan_repaint promotes data_dirty to a Full repaint.
            TrameEvent::ViewportData { .. } => {
                self.state.on_viewport_data(te);
                self.data_dirty = true;
                // A batch may carry a newly-created merge whose span now covers
                // the active cell (e.g. the drag-end corner after a merge). Re-home
                // the cursor on the master so typing targets the top-left, not a
                // covered cell (which would reject the write as a conflict). Skip
                // while dragging/editing so an in-progress gesture is not disturbed.
                if !self.selecting_drag
                    && self.editor.is_none()
                    && self.state.snap_active_to_master()
                {
                    self.send_command_lossy(self.set_active_cmd());
                    self.needs_full = true;
                }
            }
            // The active sheet may have changed (switch, or delete fallback
            // decided by the Logic thread); adopt it so edits target the right
            // sheet. The DOM `SheetTabs` strip mirrors the same event for the UI;
            // a separate DataReset re-fetches the viewport.
            TrameEvent::SheetList { active, .. } => {
                self.state.active_sheet = active;
                self.needs_full = true;
            }
            // A rejected lifecycle op (e.g. deleting the last sheet). The chrome
            // toasts it; here we log it and announce it for a11y.
            TrameEvent::Error { message } => {
                tracing::warn!("[UI] Trame rejected: {message}");
                self.text_input.announce(&message);
            }
            // Shared filter (sec.5.D): track the filtered column for the funnel
            // badge. The accompanying DataReset re-fetches the collapsed rows.
            TrameEvent::FilterApplied { col, .. } => {
                self.state.filtered_col = Some(col);
                self.needs_full = true;
            }
            TrameEvent::FilterCleared => {
                self.state.filtered_col = None;
                self.needs_full = true;
            }
            // Find & Replace (sec.5.G): highlight the match set and jump the
            // active cell onto the focused match (reusing the navigation path, so
            // an off-screen match scrolls into view). An empty set clears the
            // highlight without moving the cursor.
            TrameEvent::FindResult { matches, active } => {
                self.state.find_matches = matches.clone();
                self.needs_full = true;
                if let Some(i) = active
                    && let Some(&(row, col)) = matches.get(i)
                {
                    self.nav_goto(row, col, false);
                }
            }
            // Wholesale content change: a local snapshot restore (DataReset) or a
            // conditional-formatting rule change (CondFormatsUpdated, §3). Both
            // invalidate the cache -- re-fetch the visible region, then full-repaint
            // with the result.
            TrameEvent::DataReset | TrameEvent::CondFormatsUpdated { .. } => {
                self.needs_full = true;
                let cmd = self.state.scroll_viewport_cmd();
                self.send_command_lossy(cmd);
            }
            // Logic asks the canvas to adopt a selection it cannot derive locally
            // (e.g. after an unmerge, the freed cells stay selected though the
            // span-defining merge is gone). Active cell collapses to the top-left
            // corner; the anchor sits at the bottom-right so the rectangle renders
            // whole. Mirror the active cell back to Logic and the input pipe.
            TrameEvent::SelectRange { r1, c1, r2, c2 } => {
                self.state.select_rect(r1, c1, r2, c2);
                self.needs_full = true;
                self.viewport_changed = true;
                if let Err(SendError::Full) = self.send_command_lossless(self.set_active_cmd()) {
                    tracing::error!("Trame: UI->Logic channel full, SelectRange dropped");
                }
                self.park_input_on(r1, c1);
                self.announce_active();
            }
            // Interop export (sec.18, `-interop.md` §3/§4): the Logic thread
            // serialized the file bytes; the UI Thread owns the DOM, so the
            // download is triggered here. No canvas state changes.
            TrameEvent::ExportComplete { filename, data } => {
                trigger_download(&filename, &data);
            }
            // Interop import finished (`-interop.md` §4.A): the accompanying
            // DataReset already re-fetches the populated region, so this is purely
            // informational here (the chrome relay may toast it).
            TrameEvent::ImportComplete { .. } => {}
            // Chrome-only variants: surfaced as toasts / toolbar state / undo-redo
            // enablement by the per-frame relay (`ambre-app`). No canvas state
            // changes -- the cache is refreshed by the DataReset that accompanies
            // any of these that mutate content.
            TrameEvent::StyleApplied { .. }
            | TrameEvent::MergeRejected { .. }
            | TrameEvent::MergeConflictDetected { .. }
            | TrameEvent::UndoState { .. }
            | TrameEvent::SortComplete
            | TrameEvent::ColumnValues { .. }
            | TrameEvent::SelectionStats { .. }
            | TrameEvent::ReplaceDone { .. }
            | TrameEvent::PasteRejected { .. }
            | TrameEvent::FilterViewSaved { .. }
            | TrameEvent::ValidationError { .. }
            | TrameEvent::ValidationWarning { .. }
            | TrameEvent::ProtectionDenied { .. }
            | TrameEvent::ProtectionViolationWarning { .. } => {}
        }
    }

    pub fn update(&mut self) {
        // Drain every queued UiEvent without blocking. A `Disconnected` error
        // means the Logic thread is gone (panic=abort or shutdown) — there is
        // no recovery, so flip to the fatal state instead of silently looping.
        loop {
            match self.from_logic.try_recv() {
                // Single entry point for every Trame logic message: an exhaustive
                // dispatcher (`dispatch_trame`) decides the canvas-side reaction.
                // The chrome side (toasts / toolbar / command store) consumes the
                // same event in the per-frame relay; the two never overlap.
                Ok(UiEvent::Viewport {
                    event: AppEvent::Trame(te),
                    ..
                }) => self.dispatch_trame(te),
                Ok(UiEvent::FatalError { message }) => self.signal_fatal(&message),
                // Other UiEvent variants (session, CRUD, channels…) are not yet
                // consumed by the Trame view; ignore them for now.
                Ok(_) => {}
                Err(flume::TryRecvError::Empty) => break,
                Err(flume::TryRecvError::Disconnected) => {
                    self.signal_fatal("Logic thread disconnected");
                    break;
                }
            }
        }

        // Apply the latest grid zoom selected in the status-bar menu (`-ui.md`
        // sec.13bis). `set_zoom` is a no-op when unchanged; on a real change it
        // queues the viewport re-request flushed just below.
        #[cfg(target_arch = "wasm32")]
        {
            let z = self.zoom_sig.get_untracked();
            self.set_zoom(z);
        }

        // Advance any touchpad momentum coast before the viewport flush, so the
        // ScrollViewport it produces ships this same frame.
        self.step_momentum();

        // Flush the coalesced viewport request: at most one ScrollViewport per
        // frame, regardless of how many arrow keys fired since the last frame.
        if let Some(cmd) = self.pending_viewport.take() {
            self.send_command_lossy(cmd);
        }

        if !self.initial_viewport_sent {
            self.initial_viewport_sent = true;
            self.needs_full = true;
            let cmd = self.state.scroll_viewport_cmd();
            self.send_command_lossy(cmd);
        }

        // Pick up a theme switch (chrome toggled it): repaint everything with
        // the new palette.
        let theme = self.theme_ctl.get();
        if theme != self.theme {
            self.theme = theme;
            self.needs_full = true;
        }

        // Keep the floating selection toolbar glued to the active cell as the
        // viewport pans (published on change only).
        self.publish_anchor();

        // Caret blink: while editing, flip the phase every half-period and dirty
        // the overlay so it redraws. Costs one overlay redraw per ~530 ms; idle
        // (non-editing) frames never touch it.
        if self.editor.is_some() {
            let now = self.perf.now();
            if now - self.last_blink_ms >= BLINK_MS {
                self.caret_on = !self.caret_on;
                self.last_blink_ms = now;
                self.edit_dirty = true;
            }
        }

        // While a clipboard marquee is active, dirty the overlay each frame so
        // the marching-ants dashes crawl (sec.5.E). Cleared on Escape / next copy.
        if self.state.clipboard_anchor.is_some() {
            self.edit_dirty = true;
        }

        // Sync the HTML formula bar with the grid (spec sec.6).
        #[cfg(target_arch = "wasm32")]
        {
            self.formula_ref.set(self.active_cell_ref());

            // While the formula input is focused it OWNS the text: the user is
            // typing there, so the view must not overwrite `formula_text` (that
            // would fight the caret). A divergence from the active cell's value
            // means an edit is in progress -- open / drive an edit session from
            // the input's content. While the input is NOT focused, the formula
            // bar is read-only: mirror the in-cell editor buffer (live), or the
            // active cell's stored value when idle.
            if formula_input_focused() {
                let text = self.formula_text.get();
                if let Some(ed) = self.editor.as_mut() {
                    if ed.buffer != text {
                        ed.buffer = text;
                        self.edit_dirty = true;
                    }
                } else {
                    let seed = self.cell_seed(self.state.active_row(), self.state.active_col());
                    if text != seed {
                        // First typed character in the formula bar: open an edit
                        // session on the active cell, seeded with what was typed.
                        self.begin_edit_external();
                        if let Some(ed) = self.editor.as_mut() {
                            ed.buffer = text;
                            self.edit_dirty = true;
                        }
                    }
                }
            } else if let Some(ed) = self.editor.as_ref() {
                let buffer = ed.buffer.clone();
                if self.formula_text.get() != buffer {
                    self.formula_text.set(buffer);
                }
            } else {
                let seed = self.cell_seed(self.state.active_row(), self.state.active_col());
                if self.formula_text.get() != seed {
                    self.formula_text.set(seed);
                }
            }

            // Handle formula bar commit / cancel actions from the DOM input.
            if let Some(action) = self.formula_action.get() {
                match action {
                    FormulaAction::Commit(dir) => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.buffer = self.formula_text.get();
                        }
                        self.commit_edit();
                        match dir {
                            CommitDirection::Down => {
                                self.nav_move(1, 0, false);
                            }
                            CommitDirection::Right => {
                                self.nav_move(0, 1, false);
                            }
                            CommitDirection::Stay => {}
                        }
                        // Return focus to the grid and reset the formula text to
                        // the cell we landed on. Without this reset `formula_text`
                        // still holds the just-committed value: if the input keeps
                        // DOM focus for even one frame, the focused branch above
                        // would see `text != seed` on the NEW cell and re-open an
                        // edit there, writing the value into the cell below too.
                        self.text_input.focus();
                        self.formula_text
                            .set(self.cell_seed(self.state.active_row(), self.state.active_col()));
                    }
                    // Ctrl/Cmd+Enter: fill the value into every selected cell and
                    // keep the selection put (no cursor advance).
                    FormulaAction::CommitFill => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.buffer = self.formula_text.get();
                        }
                        self.commit_edit_filled();
                        self.text_input.focus();
                        self.formula_text
                            .set(self.cell_seed(self.state.active_row(), self.state.active_col()));
                    }
                    FormulaAction::Cancel => {
                        self.cancel_edit();
                        self.text_input.focus();
                    }
                }
                self.formula_action.set(None);
            }
        }
    }

    pub fn plan_repaint(&mut self, hud_tick: bool) -> RepaintPlan {
        // Formatting lives per-cell in the persistent cell layer, so a cursor
        // move needs no special repaint -- the cells keep their runs as they
        // shift. Only a CellFormat event (handled in `update`) dirties content.
        let any_dirty = self.needs_full
            || self.viewport_changed
            || self.data_dirty
            || self.edit_dirty
            || hud_tick;

        let (content, content_strip): (ContentRepaint, Option<Rect>) = if self.needs_full {
            (ContentRepaint::Full, None)
        } else if self.viewport_changed {
            let dr = self.state.top_row as i64 - self.prev_top_row as i64;
            let dc = self.state.left_col as i64 - self.prev_left_col as i64;
            if dr == 0 && dc == 0 {
                // Cursor moved within the viewport without scrolling. If data
                // also arrived the visible content need a refresh; otherwise the
                // content offscreen is still valid (the border is in the overlay).
                if self.data_dirty {
                    (ContentRepaint::Full, None)
                } else {
                    (ContentRepaint::Reuse, None)
                }
            } else if dr != 0 && dc != 0 {
                // Diagonal scroll would need an L-shaped strip; go full.
                (ContentRepaint::Full, None)
            } else if !self.state.col_widths.is_empty() || !self.state.row_heights.is_empty() {
                // The blit-scroll shift below assumes uniform metrics: moving `dr`
                // rows shifts the content by `dr * ROW_HEIGHT`. With any resized
                // axis (sec.4bis.C) that pixel distance is wrong, so repaint fully
                // rather than tear. Resized axes are rare; the cost is bounded.
                (ContentRepaint::Full, None)
            } else {
                // One whole-cell step is the base metric scaled by the grid zoom.
                let dy_px = (-(dr as f64) * ROW_HEIGHT * self.state.zoom).round() as i32;
                let dx_px = (-(dc as f64) * COL_WIDTH * self.state.zoom).round() as i32;
                let w = self.state.canvas_width;
                // Bottom edge of the cell area (now the full canvas: the sheet-tab
                // strip moved to DOM chrome, so it no longer reserves a band here).
                let h = self.state.grid_bottom();
                // Top of the scrollable cell area: below the formula bar band
                // (sec.6) and the column-header strip.
                let hh = self.state.header_h();
                // If the scroll exceeds the viewport, nothing useful to shift.
                let abs_dx = dx_px.unsigned_abs();
                let abs_dy = dy_px.unsigned_abs();
                if abs_dx as f64 >= w || abs_dy as f64 >= h {
                    (ContentRepaint::Full, None)
                } else {
                    // Strip = the area of the destination offscreen exposed by
                    // the shift, restricted to the cell area.
                    let hw = self.state.header_w();
                    let strip = if dy_px > 0 {
                        Rect::new(hw, hh, w, hh + dy_px as f64)
                    } else if dy_px < 0 {
                        Rect::new(hw, h + dy_px as f64, w, h)
                    } else if dx_px > 0 {
                        Rect::new(hw, hh, hw + dx_px as f64, h)
                    } else {
                        Rect::new(w + dx_px as f64, hh, w, h)
                    };
                    (ContentRepaint::Scroll { dx_px, dy_px }, Some(strip))
                }
            }
        } else if self.data_dirty {
            // Data arrived without scroll/cursor movement: repaint so the freshly
            // cached text shows up.
            (ContentRepaint::Full, None)
        } else {
            (ContentRepaint::Reuse, None)
        };

        let overlay = if any_dirty {
            OverlayRepaint::Redraw
        } else {
            OverlayRepaint::Reuse
        };

        // Consume the per-frame flags.
        self.needs_full = false;
        self.viewport_changed = false;
        self.data_dirty = false;
        self.edit_dirty = false;
        self.prev_top_row = self.state.top_row;
        self.prev_left_col = self.state.left_col;

        // When the content layer is reused the renderer won't run; zero its HUD
        // sub-phase timings so they read 0 instead of stale values.
        if matches!(content, ContentRepaint::Reuse) {
            self.draw_stats.bg_ms = 0.0;
            self.draw_stats.cells_ms = 0.0;
            self.draw_stats.cells_drawn = 0;
        }

        RepaintPlan {
            content,
            content_strip,
            overlay,
        }
    }

    pub fn paint_content(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        strip: Option<Rect>,
    ) {
        // Resolve a pending auto-fit now that the font context is in hand
        // (sec.4bis.C): measure the column/row's cached cells and emit a resize.
        if let Some(hit) = self.pending_autofit.take() {
            self.resolve_autofit(hit, font_cx, text_cache);
        }
        renderer::draw_cells(
            surface,
            &self.state,
            self.theme.palette(),
            font_cx,
            text_cache,
            self.text_enabled,
            strip,
            &PerfClock(&self.perf),
            &mut self.draw_stats,
        );
    }

    pub fn paint_overlay(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        stats: &FrameStats,
    ) {
        renderer::draw_overlay(
            surface,
            &self.state,
            self.theme.palette(),
            font_cx,
            text_cache,
            self.text_enabled,
            &PerfClock(&self.perf),
            &mut self.draw_stats,
        );

        // Publish the per-frame perf snapshot to the DOM status bar (the HUD no
        // longer paints on the wgpu overlay). Merge the driver timings (`stats`)
        // with the view's sub-phase breakdown and the glyph-cache misses.
        self.perf_pub.publish(PerfSnapshot {
            fps: stats.fps,
            frame_ms: stats.frame_ms,
            drain_ms: stats.drain_ms,
            draw_ms: stats.draw_ms,
            gpu_ms: stats.gpu_ms,
            bg_ms: self.draw_stats.bg_ms,
            cells_ms: self.draw_stats.cells_ms,
            hdrs_ms: self.draw_stats.hdrs_ms,
            cells_drawn: self.draw_stats.cells_drawn,
            input_lat_ms: self.draw_stats.input_latency_ms,
            glyph_misses: text_cache.frame_misses as u32,
        });

        // The in-cell edit field sits above everything (after the active border).
        if let Some(editor) = &mut self.editor {
            renderer::draw_edit(
                surface,
                &self.state,
                self.theme.palette(),
                editor,
                self.caret_on,
                font_cx,
                text_cache,
            );

            // The caret was just drawn: close the input->caret latency probe.
            // Measured at CPU-paint completion (one GPU submit, ~0.5-1 ms, still
            // ahead). The `performance.measure` lands in the browser Performance
            // panel; the duration also feeds the HUD (`inlat`). A 10 KB paste
            // travels the same Text path, so it is covered too.
            if let Some(t0) = self.pending_input_mark.take() {
                self.draw_stats.input_latency_ms = (self.perf.now() - t0) as f32;
                let _ = self
                    .perf
                    .measure_with_start_mark("ambre:input-to-caret", "ambre:input");
            }
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.state.resize(width as f64, height as f64);
        self.needs_full = true;
    }

    /// Set the grid zoom factor (`1.0` = 100%); scales cells, header strips and
    /// the cell font together (`-ui.md` sec.13bis). A no-op when unchanged. On a
    /// real change the visible range shifts, so we request the new viewport and
    /// force a full repaint. Pure local view state -- never synced.
    pub fn set_zoom(&mut self, zoom: f64) -> bool {
        if !self.state.set_zoom(zoom) {
            return false;
        }
        self.needs_full = true;
        self.viewport_changed = true;
        self.pending_viewport = Some(self.state.scroll_viewport_cmd());
        true
    }

    /// The current grid zoom factor (`1.0` = 100%), for the status-bar readout.
    pub fn zoom(&self) -> f64 {
        self.state.zoom
    }

    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        // Wheel / two-finger touchpad scroll pans the viewport in any mode and
        // never disturbs the cursor or an open editor.
        if let InputEvent::Wheel {
            dx,
            dy,
            delta_mode,
            ctrl_key,
        } = ev
        {
            return self.handle_wheel(*dx, *dy, *delta_mode, *ctrl_key);
        }
        // An open edit session owns text, IME and the editing keys.
        if self.editor.is_some() {
            return self.handle_input_editing(ev);
        }
        self.handle_input_nav(ev)
    }
}

impl TrameView {
    // -- Navigation mode -------------------------------------------------------

    /// Input routing while NOT editing a cell.
    fn handle_input_nav(&mut self, ev: &InputEvent) -> bool {
        match ev {
            // A content click focuses the pipe and (re)selects the clicked cell,
            // so typing or IME can start there (spec: canvas click ->
            // textarea.focus). Shift extends the range to the clicked cell;
            // Ctrl adds it as a disjoint range; a plain click collapses to it and
            // arms a drag-to-select.
            // A press on the fill handle (bottom-right of the selection) arms an
            // autofill drag instead of a new selection (sec.5.F). Checked before
            // the plain selection press so the ~7px handle wins the hit.
            // A click on an in-canvas autofilter button (sec.5.D) selects the
            // button's column on the region's header row, then raises the one-shot
            // intent so the launcher opens the FilterPopover for that column --
            // the same path as the context-menu entry (one filter code path).
            // Tested before the fill handle / cells so the small button wins.
            InputEvent::PointerDown {
                x, y, button: 0, ..
            } if self.fill_drag.is_none() && self.state.filter_button_at(*x, *y).is_some() => {
                if let Some((row, col)) = self.state.filter_button_at(*x, *y) {
                    self.text_input.focus();
                    self.select_cell(row, col);
                    self.filter_open.publish();
                }
                true
            }
            InputEvent::PointerDown {
                x, y, button: 0, ..
            } if self.fill_drag.is_none() && self.state.fill_handle_at(*x, *y) => {
                self.text_input.focus();
                // A merged-cell source fills block by block (Excel behaviour), so
                // capture the merge-expanded source and its block granularity.
                let bounds = self.state.selection.bounds();
                let src = self.state.expand_to_merges(bounds);
                let block = self.state.fill_source_block(bounds);
                self.fill_drag = Some((src, block));
                self.set_fill_cursor(true);
                true
            }
            // Dragging the handle grows the destination rectangle, locked to the
            // dominant axis (a fill handle never fills two directions at once).
            InputEvent::PointerMove { x, y } if self.fill_drag.is_some() => {
                if let Some((row, col)) = self.state.cell_at(*x, *y)
                    && let Some((src, block)) = self.fill_drag
                {
                    self.state.fill_preview = Some(fill_dest(src, row, col, block));
                    self.edit_dirty = true; // repaint the preview outline.
                }
                true
            }
            // Release: emit the fill, grow the selection over the filled range,
            // and clear the drag/preview.
            InputEvent::PointerUp { ctrl, .. } if self.fill_drag.is_some() => {
                self.end_fill_drag(*ctrl);
                true
            }
            // A press on a header strip (sec.4bis): body -> whole-column/row
            // select (Shift extends, drag sweeps); corner -> select-all; a border
            // band -> arm a resize drag (sec.4bis.C). Checked before the grid-cell
            // press since headers sit outside the cell area anyway.
            InputEvent::PointerDown {
                x,
                y,
                button: 0,
                shift,
                ..
            } if self.fill_drag.is_none() && self.state.header_hit(*x, *y).is_some() => {
                self.text_input.focus();
                match self.state.header_hit(*x, *y).expect("guarded") {
                    HeaderHit::ColBody(c) => {
                        if *shift && self.state.selection.kind == SelKind::Cols {
                            self.extend_axis(c);
                        } else {
                            self.select_axis(SelKind::Cols, c);
                            self.axis_drag = Some(SelKind::Cols);
                        }
                    }
                    HeaderHit::RowBody(r) => {
                        if *shift && self.state.selection.kind == SelKind::Rows {
                            self.extend_axis(r);
                        } else {
                            self.select_axis(SelKind::Rows, r);
                            self.axis_drag = Some(SelKind::Rows);
                        }
                    }
                    HeaderHit::Corner => self.select_all(),
                    // Resize bands / inline-insert affordance: handled in the
                    // resize-drag wiring (sec.4bis.C/D).
                    HeaderHit::ColBorder(c) => self.arm_col_resize(c, *x),
                    HeaderHit::RowBorder(r) => self.arm_row_resize(r, *y),
                    HeaderHit::ColInsert(c) => self.insert_col_at(c),
                    HeaderHit::RowInsert(r) => self.insert_row_at(r),
                }
                true
            }
            // A header drag sweeps the whole-axis band to the column/row under the
            // cursor (even below/right of the header strip).
            InputEvent::PointerMove { x, y } if self.axis_drag.is_some() => {
                match self.axis_drag {
                    Some(SelKind::Cols) => {
                        if let Some(c) = self.state.col_at_x(*x) {
                            self.extend_axis(c);
                        }
                    }
                    Some(SelKind::Rows) => {
                        if let Some(r) = self.state.row_at_y(*y) {
                            self.extend_axis(r);
                        }
                    }
                    _ => {}
                }
                true
            }
            InputEvent::PointerUp { .. } if self.axis_drag.is_some() => {
                self.axis_drag = None;
                true
            }
            // Dragging a header boundary moves the live guide line (sec.4bis.C);
            // the release emits the resize and clears the preview.
            InputEvent::PointerMove { x, y } if self.resize_drag.is_some() => {
                self.update_resize_preview(*x, *y);
                true
            }
            InputEvent::PointerUp { .. } if self.resize_drag.is_some() => {
                self.end_resize_drag();
                true
            }
            // Double-click on a header boundary auto-fits the column/row to its
            // cached content (sec.4bis.C).
            InputEvent::DoubleClick { x, y }
                if matches!(
                    self.state.header_hit(*x, *y),
                    Some(HeaderHit::ColBorder(_) | HeaderHit::RowBorder(_))
                ) =>
            {
                match self.state.header_hit(*x, *y) {
                    Some(HeaderHit::ColBorder(c)) => self.auto_fit_col(c),
                    Some(HeaderHit::RowBorder(r)) => self.auto_fit_row(r),
                    _ => {}
                }
                true
            }
            InputEvent::PointerDown {
                x,
                y,
                button: 0,
                shift,
                ctrl,
            } => {
                self.text_input.focus();
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
                    if *shift {
                        self.extend_cell(row, col);
                    } else if *ctrl {
                        self.disjoin_cell(row, col);
                    } else {
                        self.select_cell(row, col);
                        // Arm drag-to-select: pointer moves now grow the range.
                        self.selecting_drag = true;
                    }
                }
                true
            }
            // Drag with the button held grows the live selection rectangle to the
            // cell under the cursor (mouse range selection).
            InputEvent::PointerMove { x, y } if self.selecting_drag => {
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
                    self.extend_cell(row, col);
                }
                true
            }
            InputEvent::PointerUp { .. } if self.selecting_drag => {
                self.selecting_drag = false;
                true
            }
            // Bare hover (no button held): advertise the affordance under the
            // cursor (sec.4bis.B / sec.5.F) -- a header boundary shows the
            // col/row-resize cursor, a header body the select pointer, the fill
            // handle the crosshair, everything else the default. Pure feedback.
            InputEvent::PointerMove { x, y } => {
                let hit = self.state.header_hit(*x, *y);
                let css = match hit {
                    Some(HeaderHit::ColBorder(_)) => "col-resize",
                    Some(HeaderHit::RowBorder(_)) => "row-resize",
                    Some(_) => "pointer",
                    None if self.state.filter_button_at(*x, *y).is_some() => "pointer",
                    None if self.state.fill_handle_at(*x, *y) => "crosshair",
                    None => "",
                };
                if css != self.hover_cursor {
                    self.hover_cursor = css.to_owned();
                    self.set_cursor(css);
                }
                // Show the inline "+" badge while hovering an insert zone.
                let insert = matches!(hit, Some(HeaderHit::ColInsert(_) | HeaderHit::RowInsert(_)))
                    .then_some(hit)
                    .flatten();
                if insert != self.state.insert_hint {
                    self.state.insert_hint = insert;
                    self.edit_dirty = true;
                }
                // Tint the hovered column/row header (chrome-consistent hover
                // feedback). Only the body zones tint; borders/insert bands keep
                // their resize/"+" affordances instead.
                let hover = match hit {
                    Some(
                        h @ (HeaderHit::ColBody(_) | HeaderHit::RowBody(_) | HeaderHit::Corner),
                    ) => Some(h),
                    _ => None,
                };
                if hover != self.state.hover_header {
                    self.state.hover_header = hover;
                    self.edit_dirty = true;
                }
                false
            }
            // A right-click on a header selects the whole column/row (or all, on
            // the corner) before the DOM context menu opens, and flags the menu
            // as header-scoped (sec.5.C). A border/insert band keeps the existing
            // selection -- the header-scoped menu still acts on it. We don't
            // preventDefault here; the `contextmenu` listener owns the menu.
            InputEvent::PointerDown {
                x, y, button: 2, ..
            } if self.state.header_hit(*x, *y).is_some() => {
                self.text_input.focus();
                use crate::ui::grid::CtxScope;
                let scope = match self.state.header_hit(*x, *y).expect("guarded") {
                    HeaderHit::ColBody(c) => {
                        self.select_axis(SelKind::Cols, c);
                        CtxScope::Column
                    }
                    HeaderHit::RowBody(r) => {
                        self.select_axis(SelKind::Rows, r);
                        CtxScope::Row
                    }
                    HeaderHit::Corner => {
                        self.select_all();
                        CtxScope::Corner
                    }
                    // Border / insert bands keep the current selection; scope the
                    // menu to the clicked axis so it still acts on that selection.
                    HeaderHit::ColBorder(_) | HeaderHit::ColInsert(_) => CtxScope::Column,
                    HeaderHit::RowBorder(_) | HeaderHit::RowInsert(_) => CtxScope::Row,
                };
                self.set_ctx_scope(scope);
                false
            }
            // A right-click selects the cell under the cursor before the DOM
            // context menu opens on it (standard spreadsheet behaviour). We don't
            // preventDefault here -- the `contextmenu` listener owns the menu.
            InputEvent::PointerDown {
                x, y, button: 2, ..
            } => {
                self.text_input.focus();
                // Right-click inside the current multi-cell selection keeps it
                // (so a context menu can act on the range); otherwise it collapses
                // to the clicked cell, like a left click.
                if let Some((row, col)) = self.state.cell_at(*x, *y)
                    && !self.state.selection.contains(row, col)
                {
                    self.select_cell(row, col);
                }
                self.set_ctx_scope(crate::ui::grid::CtxScope::Cell);
                false
            }
            // Double-click opens an editor on the clicked cell's current text.
            InputEvent::DoubleClick { x, y } => {
                self.text_input.focus();
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
                    self.select_cell(row, col);
                    let seed = self.cell_seed(row, col);
                    self.begin_edit(seed);
                }
                true
            }
            // First printable text (latin or IME commit) opens an edit replacing
            // the cell, then applies the text.
            InputEvent::Text { text } => {
                self.begin_edit(String::new());
                if let Some(ed) = self.editor.as_mut() {
                    ed.insert(text);
                }
                self.edit_dirty = true;
                true
            }
            InputEvent::CompositionUpdate { text } => {
                self.begin_edit(String::new());
                if let Some(ed) = self.editor.as_mut() {
                    ed.preview = text.clone();
                }
                self.edit_dirty = true;
                true
            }
            InputEvent::CompositionEnd { text } => {
                self.begin_edit(String::new());
                if let Some(ed) = self.editor.as_mut() {
                    ed.insert(text);
                }
                self.edit_dirty = true;
                true
            }
            InputEvent::Key {
                key,
                ctrl,
                meta,
                alt,
                shift,
            } => {
                let chord = *ctrl || *meta; // Ctrl (or Cmd on macOS) jump modifier.
                // Shift extends the selection range across navigation keys
                // (Shift+arrows, Shift+Page, Shift+Home/End, Shift+Ctrl+End).
                let ext = *shift;
                match key.as_str() {
                    "ArrowUp" => self.nav_move(-1, 0, ext),
                    "ArrowDown" => self.nav_move(1, 0, ext),
                    "ArrowLeft" => self.nav_move(0, -1, ext),
                    "ArrowRight" => self.nav_move(0, 1, ext),
                    // Tab / Shift+Tab walk columns; Enter / Shift+Enter walk rows
                    // (spreadsheet convention, sec.5.A) -- these always collapse the
                    // selection (Shift is the reverse-direction modifier here, not
                    // a range extender). preventDefault keeps the browser from
                    // moving focus off the canvas.
                    "Tab" if !chord => self.nav_move(0, if *shift { -1 } else { 1 }, false),
                    "Enter" if !chord => self.nav_move(if *shift { -1 } else { 1 }, 0, false),
                    // Page Down / Up move by a full viewport height.
                    "PageDown" => self.nav_move(self.state.viewport_rows.max(1) as i32, 0, ext),
                    "PageUp" => self.nav_move(-(self.state.viewport_rows.max(1) as i32), 0, ext),
                    // Home: column 0 of the row; Ctrl+Home: top-left of the grid.
                    "Home" if chord => self.nav_goto(0, 0, ext),
                    "Home" => self.nav_goto(self.state.active_row(), 0, ext),
                    // End: last cell with content in the row; Ctrl+End: bottom-right
                    // of the used (cached) range. With Shift the range grows to that
                    // target (Shift+Ctrl+End = select to the last non-empty cell).
                    // Best-effort -- the UI thread has no used-range model, so it
                    // scans its local cache (sec.16).
                    "End" if chord => {
                        let (row, col) = self
                            .state
                            .last_content_cell()
                            .unwrap_or((self.state.active_row(), self.state.active_col()));
                        self.nav_goto(row, col, ext)
                    }
                    "End" => {
                        let row = self.state.active_row();
                        let col = self
                            .state
                            .last_content_col(row)
                            .unwrap_or(self.state.active_col());
                        self.nav_goto(row, col, ext)
                    }
                    // Escape clears the marching-ants clipboard marquee (sec.5.E),
                    // if any; otherwise it is not ours.
                    "Escape" if self.state.clipboard_anchor.take().is_some() => {
                        self.cut_pending = false;
                        self.edit_dirty = true;
                        true
                    }
                    // Shift+Delete is Cut: don't consume it, so the browser fires
                    // the native `cut` clipboard event the host services (sec.5.E).
                    "Delete" if *shift => false,
                    // Delete / Backspace clear the active cell.
                    "Delete" | "Backspace" => self.clear_active_cell(),
                    // F2 opens an editor on the cell's current text.
                    "F2" => {
                        let seed = self.cell_seed(self.state.active_row(), self.state.active_col());
                        self.begin_edit(seed);
                        true
                    }
                    // Debug HUD text toggle, moved behind Alt so plain letters type.
                    "t" | "T" if *alt => {
                        self.text_enabled ^= true;
                        self.needs_full = true;
                        true
                    }
                    // Let plain printable keys fall through (no preventDefault) so
                    // the focused textarea receives them and emits an `input` ->
                    // `Text`. Command chords (Ctrl/Cmd+X) likewise fall through to
                    // the chrome keymap.
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Pan the viewport from a wheel / two-finger touchpad scroll. Pixel deltas
    /// (touchpad, modern mice) accumulate sub-cell residue across events; line
    /// and page deltas (`delta_mode` 1/2) are normalised to pixels first.
    /// Returns `true` when consumed (so the page itself doesn't scroll).
    fn handle_wheel(&mut self, dx: f64, dy: f64, delta_mode: u32, ctrl_key: bool) -> bool {
        // Leave Ctrl+wheel to the browser (pinch-zoom gesture); not ours yet.
        if ctrl_key {
            return false;
        }
        // Normalise the unit to physical pixels. Deltas already include dpr. A
        // line step is one zoomed cell, so it scales with the grid zoom.
        let z = self.state.zoom;
        let (sx, sy) = match delta_mode {
            1 => (COL_WIDTH * z, ROW_HEIGHT * z), // DOM_DELTA_LINE
            2 => (self.state.canvas_width, self.state.canvas_height), // DOM_DELTA_PAGE
            _ => (1.0, 1.0),                      // DOM_DELTA_PIXEL
        };
        let (px, py) = (dx * sx, dy * sy);

        // The fingers are driving: any active momentum is overridden by the
        // direct input, so cancel the coast.
        self.coasting = false;

        // A notched mouse wheel emits discrete, widely-spaced events: line/page
        // units (`delta_mode != 0`) or chunky pixel jumps. Those must NOT feed
        // the velocity estimator, otherwise a single click launches a momentum
        // coast and scrolls far past one row. Only high-resolution pixel input
        // (touchpad two-finger glide) gets kinetic scrolling.
        let discrete =
            delta_mode != 0 || px.abs() >= DISCRETE_WHEEL_PX || py.abs() >= DISCRETE_WHEEL_PX;
        if discrete {
            // No inertia: disarm any pending gesture and apply the step 1:1.
            self.fling_vx = 0.0;
            self.fling_vy = 0.0;
            self.last_wheel_ms = 0.0;
            self.scroll_accum_x += px;
            self.scroll_accum_y += py;
            self.apply_scroll_accum();
            return true;
        }

        // Update the velocity estimate (physical px/ms) from this sample's
        // interval, smoothed to ride out jitter. A long gap means a fresh
        // gesture, so seed from the sample alone rather than blending stale
        // velocity; clamp dt so a near-zero interval can't blow it up.
        let now = self.perf.now();
        let gap = now - self.last_wheel_ms;
        if self.last_wheel_ms == 0.0 || gap > FLING_GAP_MS {
            let dt = gap.clamp(1.0, 16.0);
            self.fling_vx = px / dt;
            self.fling_vy = py / dt;
        } else {
            let dt = gap.clamp(1.0, 100.0);
            let b = FLING_VELOCITY_BLEND;
            self.fling_vx = b * (px / dt) + (1.0 - b) * self.fling_vx;
            self.fling_vy = b * (py / dt) + (1.0 - b) * self.fling_vy;
        }
        self.last_wheel_ms = now;

        // Apply the delta immediately for a 1:1 feel while the fingers move.
        self.scroll_accum_x += px;
        self.scroll_accum_y += py;
        self.apply_scroll_accum();
        true
    }

    /// Peel whole rows/cols out of the pixel accumulator and pan the viewport,
    /// keeping the sub-cell remainder. Shared by direct wheel input and the
    /// momentum coast.
    fn apply_scroll_accum(&mut self) -> ScrollOutcome {
        // A whole-cell step is the base metric scaled by the grid zoom.
        let z = self.state.zoom;
        let step_x = COL_WIDTH * z;
        let step_y = ROW_HEIGHT * z;
        let dc = (self.scroll_accum_x / step_x) as i32;
        let dr = (self.scroll_accum_y / step_y) as i32;
        if dr == 0 && dc == 0 {
            return ScrollOutcome::Idle;
        }
        self.scroll_accum_x -= dc as f64 * step_x;
        self.scroll_accum_y -= dr as f64 * step_y;
        if self.state.scroll_by(dr, dc) {
            self.viewport_changed = true;
            self.pending_viewport = Some(self.state.scroll_viewport_cmd());
            ScrollOutcome::Moved
        } else {
            // Pinned against a grid border: the motion had nowhere to go.
            ScrollOutcome::Edge
        }
    }

    /// Advance the momentum coast by one frame. Called every frame from
    /// `update`. Starts coasting when a flick's velocity outlives the gesture,
    /// integrates the decaying velocity into the scroll accumulator, and stops
    /// once the speed dies down or the viewport hits an edge.
    fn step_momentum(&mut self) {
        let now = self.perf.now();

        // Begin a coast: the gesture ended (gap elapsed) but the fingers left
        // with enough speed. `last_wheel_ms == 0` means no gesture is pending.
        if !self.coasting {
            if self.last_wheel_ms == 0.0 || now - self.last_wheel_ms < FLING_GAP_MS {
                return;
            }
            // Gesture window closed: decide whether to fling or just disarm.
            self.last_wheel_ms = 0.0;
            if self.fling_vx.hypot(self.fling_vy) < FLING_START_PX_PER_MS {
                self.fling_vx = 0.0;
                self.fling_vy = 0.0;
                return;
            }
            self.coasting = true;
            self.last_coast_ms = now;
            // Don't integrate on the launch frame: `dt` would be 0. Start
            // moving next frame, once a real interval has elapsed.
            return;
        }

        let dt = now - self.last_coast_ms;
        self.last_coast_ms = now;
        if dt <= 0.0 {
            return;
        }
        let dt = dt.min(100.0);

        // Integrate over the *measured* frame dt and apply friction scaled to
        // that same dt, so the coast is identical at 60 Hz or 240 Hz (it just
        // runs in finer steps at higher refresh rates).
        self.scroll_accum_x += self.fling_vx * dt;
        self.scroll_accum_y += self.fling_vy * dt;
        let decay = FLING_FRICTION.powf(dt / 16.667);
        self.fling_vx *= decay;
        self.fling_vy *= decay;

        // Stop on a border, or once the speed has bled off.
        if matches!(self.apply_scroll_accum(), ScrollOutcome::Edge)
            || self.fling_vx.hypot(self.fling_vy) < FLING_STOP_PX_PER_MS
        {
            self.stop_fling();
        }
    }

    /// Halt any momentum coast and clear its residue.
    fn stop_fling(&mut self) {
        self.coasting = false;
        self.fling_vx = 0.0;
        self.fling_vy = 0.0;
        self.scroll_accum_x = 0.0;
        self.scroll_accum_y = 0.0;
    }

    /// Move the active cell by `(dr, dc)`; returns `true` (consumed). With
    /// `extend` (Shift held) the selection range grows instead of collapsing.
    fn nav_move(&mut self, dr: i32, dc: i32, extend: bool) -> bool {
        let mv = self.state.move_cursor(dr, dc, extend);
        self.apply_cursor_move(mv)
    }

    /// Jump the active cell to absolute `(row, col)` (Home / End / Ctrl+Home /
    /// Ctrl+End); returns `true` (consumed). With `extend` the range grows to
    /// the target (Shift+End, Shift+Ctrl+End...).
    fn nav_goto(&mut self, row: u32, col: u32, extend: bool) -> bool {
        let mv = self.state.move_to(row, col, extend);
        self.apply_cursor_move(mv)
    }

    /// Build the active-cell report from the current selection, carrying the
    /// anchor corner so the Logic thread can track the whole selection rectangle
    /// (needed by the structural intents -- merge / delete rows-cols).
    fn set_active_cmd(&self) -> TrameCommand {
        let (row, col) = self.state.selection.active;
        let (anchor_row, anchor_col) = self.state.selection.anchor;
        TrameCommand::SetActiveCell {
            row,
            col,
            anchor_row,
            anchor_col,
        }
    }

    /// Extend the live selection rectangle to `(row, col)` (Shift+click / drag),
    /// keeping the anchor. Moves the active cell, so Logic and the pipe stay in
    /// sync via the same path as a plain selection.
    fn extend_cell(&mut self, row: u32, col: u32) {
        if (row, col) == (self.state.active_row(), self.state.active_col()) {
            return;
        }
        self.state.select_extend(row, col);
        self.viewport_changed = true;
        if let Err(SendError::Full) = self.send_command_lossless(self.set_active_cmd()) {
            tracing::error!("Trame: UI->Logic channel full, selection extend dropped");
        }
        self.park_input_on(row, col);
        self.announce_active();
    }

    /// Add `(row, col)` as a fresh disjoint range (Ctrl+click), committing the
    /// previous live rectangle. The new cell becomes active.
    fn disjoin_cell(&mut self, row: u32, col: u32) {
        self.state.select_disjoint(row, col);
        self.viewport_changed = true;
        if let Err(SendError::Full) = self.send_command_lossless(self.set_active_cmd()) {
            tracing::error!("Trame: UI->Logic channel full, disjoint select dropped");
        }
        self.park_input_on(row, col);
        self.announce_active();
    }

    /// Forward a cursor move to Logic and keep the text pipe / a11y in sync.
    /// Shared by relative ([`nav_move`](Self::nav_move)) and absolute
    /// ([`nav_goto`](Self::nav_goto)) navigation. A `None` move (clamped at an
    /// edge / no change) is a no-op but still reports the key as consumed.
    fn apply_cursor_move(&mut self, mv: Option<CursorMove>) -> bool {
        self.viewport_changed = true;
        if let Some(mv) = mv {
            if let Err(SendError::Full) = self.send_command_lossless(mv.active) {
                tracing::error!("Trame: UI→Logic channel full, cursor move dropped");
            }
            if let Some(scroll) = mv.scroll {
                self.pending_viewport = Some(scroll);
            }
            self.park_input_on(self.state.active_row(), self.state.active_col());
            self.announce_active();
        }
        true
    }

    /// Clear every selected cell's content (Delete / Backspace in navigation
    /// mode). Optimistically empties the local cache and forwards an empty
    /// `SetCellText` per cell -- the same path a committed empty edit takes --
    /// sealed as one undo group by a trailing `CommitEdit`. Returns `true`
    /// (consumed).
    fn clear_active_cell(&mut self) -> bool {
        let cells = self.state.selection.selected_cells();
        // One batch command so the whole clear is a single undo item.
        let batch = cells.iter().map(|&(r, c)| (r, c, String::new())).collect();
        if let Err(SendError::Full) =
            self.send_command_lossless(TrameCommand::SetCellsBatch { cells: batch })
        {
            tracing::error!("Trame: UI→Logic channel full, cell clear dropped");
        }
        for &(row, col) in &cells {
            self.state.set_cell_text(row, col, String::new());
        }
        self.needs_full = true;
        self.announce_active();
        true
    }

    /// Set the canvas cursor to the autofill crosshair (`on == true`) or back to
    /// the default (`-ui.md` sec.5.F): the thin "+" Excel/Sheets show over the
    /// fill handle. A DOM write on the `#ambre-canvas` element; a no-op off-wasm.
    #[cfg(target_arch = "wasm32")]
    fn set_fill_cursor(&self, on: bool) {
        use wasm_bindgen::JsCast;
        if let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("ambre-canvas"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = el
                .style()
                .set_property("cursor", if on { "crosshair" } else { "" });
        }
    }

    /// Native/headless build: no DOM canvas to restyle.
    #[cfg(not(target_arch = "wasm32"))]
    fn set_fill_cursor(&self, _on: bool) {}

    /// Set an arbitrary CSS cursor on `#ambre-canvas` (`-ui.md` sec.4bis.B):
    /// `"col-resize"` / `"row-resize"` over a header boundary, `"pointer"` over a
    /// header body, `""` to reset. Generalises [`set_fill_cursor`]. No-op off-wasm.
    #[cfg(target_arch = "wasm32")]
    fn set_cursor(&self, css: &str) {
        use wasm_bindgen::JsCast;
        if let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("ambre-canvas"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = el.style().set_property("cursor", css);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn set_cursor(&self, _css: &str) {}

    /// Publish the surface the latest right-click landed on so the launcher picks
    /// the cell vs header context-menu layout (`-ui.md` sec.5.C). Only re-writes
    /// the signal on a change (cheap). No-op off-wasm (headless tests).
    #[cfg(target_arch = "wasm32")]
    fn set_ctx_scope(&self, scope: crate::ui::grid::CtxScope) {
        if self.ctx_scope.get_untracked() != scope {
            self.ctx_scope.set(scope);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn set_ctx_scope(&self, _scope: crate::ui::grid::CtxScope) {}

    // -- Header resize / insert (sec.4bis.C/D) ---------------------------------

    /// Arm a column resize from a press on column `c`'s right border. Shows the
    /// live vertical guide at the pointer x.
    fn arm_col_resize(&mut self, c: u32, x: f64) {
        self.resize_drag = Some(HeaderHit::ColBorder(c));
        self.state.resize_preview = Some(crate::ui::grid::ResizePreview::Col(x));
        self.set_cursor("col-resize");
        self.edit_dirty = true;
    }

    /// Arm a row resize from a press on row `r`'s bottom border.
    fn arm_row_resize(&mut self, r: u32, y: f64) {
        self.resize_drag = Some(HeaderHit::RowBorder(r));
        self.state.resize_preview = Some(crate::ui::grid::ResizePreview::Row(y));
        self.set_cursor("row-resize");
        self.edit_dirty = true;
    }

    /// Move the live resize guide to the pointer, clamped so the dragged
    /// column/row never shrinks below its minimum.
    fn update_resize_preview(&mut self, x: f64, y: f64) {
        match self.resize_drag {
            Some(HeaderHit::ColBorder(c)) => {
                // The guide is in physical px, so the minimum width is the base
                // clamp scaled by the current zoom.
                let min_x = self.state.col_x(c) + crate::ui::grid::MIN_COL_WIDTH * self.state.zoom;
                self.state.resize_preview = Some(crate::ui::grid::ResizePreview::Col(x.max(min_x)));
            }
            Some(HeaderHit::RowBorder(r)) => {
                let min_y = self.state.row_y(r) + crate::ui::grid::MIN_ROW_HEIGHT * self.state.zoom;
                self.state.resize_preview = Some(crate::ui::grid::ResizePreview::Row(y.max(min_y)));
            }
            _ => {}
        }
        self.edit_dirty = true;
    }

    /// Finish a resize drag: emit `ResizeCol` / `ResizeRow` for the new size and
    /// clear the guide. When the dragged column/row is part of a whole-axis
    /// selection, the size is applied to every band in the selection (sec.4bis.C).
    fn end_resize_drag(&mut self) {
        let drag = self.resize_drag.take();
        let preview = self.state.resize_preview.take();
        self.set_cursor("");
        self.edit_dirty = true;
        let (Some(drag), Some(preview)) = (drag, preview) else {
            return;
        };
        match (drag, preview) {
            (HeaderHit::ColBorder(c), crate::ui::grid::ResizePreview::Col(px)) => {
                // The guide is in physical (zoomed) px; the stored width is BASE
                // px (zoom-independent content), so divide the on-screen extent
                // back out by the zoom before persisting.
                let width = ((px - self.state.col_x(c)) / self.state.zoom)
                    .max(crate::ui::grid::MIN_COL_WIDTH);
                for col in self.resize_target_cols(c) {
                    self.send_resize_col(col, width as f32);
                }
            }
            (HeaderHit::RowBorder(r), crate::ui::grid::ResizePreview::Row(py)) => {
                let height = ((py - self.state.row_y(r)) / self.state.zoom)
                    .max(crate::ui::grid::MIN_ROW_HEIGHT);
                for row in self.resize_target_rows(r) {
                    self.send_resize_row(row, height as f32);
                }
            }
            _ => {}
        }
    }

    /// Columns a column resize applies to: the whole-column selection's band when
    /// the dragged column is part of it, else just the dragged column.
    fn resize_target_cols(&self, c: u32) -> Vec<u32> {
        let sel = &self.state.selection;
        if sel.kind == SelKind::Cols {
            let (c1, c2) = (
                sel.anchor.1.min(sel.active.1),
                sel.anchor.1.max(sel.active.1),
            );
            if c >= c1 && c <= c2 {
                return (c1..=c2).collect();
            }
        }
        vec![c]
    }

    /// Rows a row resize applies to (row analogue of [`resize_target_cols`]).
    fn resize_target_rows(&self, r: u32) -> Vec<u32> {
        let sel = &self.state.selection;
        if sel.kind == SelKind::Rows {
            let (r1, r2) = (
                sel.anchor.0.min(sel.active.0),
                sel.anchor.0.max(sel.active.0),
            );
            if r >= r1 && r <= r2 {
                return (r1..=r2).collect();
            }
        }
        vec![r]
    }

    /// Emit a `ResizeCol`, updating the local metric cache optimistically so the
    /// grid reflows immediately (the echoing `DataReset` confirms it).
    fn send_resize_col(&mut self, col: u32, width: f32) {
        self.state.col_widths.insert(col, width);
        let version = self.state.bump_version();
        self.send_command_lossy(TrameCommand::ResizeCol {
            version,
            sheet_id: self.state.active_sheet.clone(),
            col,
            width_px: width,
        });
        self.viewport_changed = true;
        self.needs_full = true;
    }

    /// Emit a `ResizeRow` (row analogue of [`send_resize_col`]).
    fn send_resize_row(&mut self, row: u32, height: f32) {
        self.state.row_heights.insert(row, height);
        let version = self.state.bump_version();
        self.send_command_lossy(TrameCommand::ResizeRow {
            version,
            sheet_id: self.state.active_sheet.clone(),
            row,
            height_px: height,
        });
        self.viewport_changed = true;
        self.needs_full = true;
    }

    /// Request a column auto-fit (double-click on the border, sec.4bis.C). The
    /// measurement needs the parley font context, so it is deferred to the next
    /// paint, which owns it (mirrors the editor's deferred click resolution).
    fn auto_fit_col(&mut self, c: u32) {
        self.pending_autofit = Some(HeaderHit::ColBorder(c));
        self.edit_dirty = true;
    }
    fn auto_fit_row(&mut self, r: u32) {
        self.pending_autofit = Some(HeaderHit::RowBorder(r));
        self.edit_dirty = true;
    }

    /// Measure a column/row's cached cells and emit a resize sized to its content
    /// (sec.4bis.C). UI-side over the viewport cache: the parley layouts of every
    /// cached cell of the axis are measured and the max natural extent (plus
    /// padding, clamped to the minimum) becomes the new width/height. A column
    /// with no cached content snaps to the default.
    fn resolve_autofit(
        &mut self,
        hit: HeaderHit,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
    ) {
        use ambre_draw::Alignment;
        let pad = (crate::render::CELL_PAD_X * 2.0) as f32;
        match hit {
            HeaderHit::ColBorder(c) => {
                let mut max_w = 0.0f32;
                for ((_, col), data) in self.state.cell_cache.iter() {
                    if *col != c || data.text.is_empty() {
                        continue;
                    }
                    let size = data
                        .size
                        .map(|s| s as f32)
                        .unwrap_or(crate::render::BODY_SIZE);
                    let lyt = text_cache.get_or_build_font(
                        font_cx,
                        &data.text,
                        size,
                        f32::MAX,
                        Alignment::Start,
                        data.bold,
                        data.italic,
                        data.font.as_deref(),
                        false,
                    );
                    max_w = max_w.max(lyt.width());
                }
                let width = if max_w > 0.0 {
                    (max_w + pad).max(crate::ui::grid::MIN_COL_WIDTH as f32)
                } else {
                    crate::ui::grid::COL_WIDTH as f32
                };
                self.send_resize_col(c, width);
            }
            HeaderHit::RowBorder(r) => {
                let mut max_h = 0.0f32;
                for ((row, _), data) in self.state.cell_cache.iter() {
                    if *row != r || data.text.is_empty() {
                        continue;
                    }
                    let size = data
                        .size
                        .map(|s| s as f32)
                        .unwrap_or(crate::render::BODY_SIZE);
                    let lyt = text_cache.get_or_build_font(
                        font_cx,
                        &data.text,
                        size,
                        f32::MAX,
                        Alignment::Start,
                        data.bold,
                        data.italic,
                        data.font.as_deref(),
                        false,
                    );
                    max_h = max_h.max(lyt.height());
                }
                let height = if max_h > 0.0 {
                    (max_h + 4.0).max(crate::ui::grid::MIN_ROW_HEIGHT as f32)
                } else {
                    crate::ui::grid::ROW_HEIGHT as f32
                };
                self.send_resize_row(r, height);
            }
            _ => {}
        }
    }

    /// Inline "+" insert at a column boundary (sec.4bis.D): move the tracked
    /// active cell onto column `c`, then fire the existing `InsertColLeft` intent
    /// so the Logic thread resolves the position exactly as from the menu.
    fn insert_col_at(&mut self, c: u32) {
        self.select_cell(self.state.active_row(), c);
        self.send_command_lossy(TrameCommand::InsertColLeft);
        // Select the freshly created column (it lands at `c`) so the insertion
        // reads as a clear whole-column band rather than a silent shift.
        self.select_axis(crate::ui::grid::SelKind::Cols, c);
    }
    fn insert_row_at(&mut self, r: u32) {
        self.select_cell(r, self.state.active_col());
        self.send_command_lossy(TrameCommand::InsertRowAbove);
        // Select the freshly created row (it lands at `r`) for the same feedback.
        self.select_axis(crate::ui::grid::SelKind::Rows, r);
    }

    /// Finish an autofill drag (sec.5.F): emit `FillRange` over the source +
    /// dragged extent, grow the selection to cover the filled range, then clear
    /// the drag/preview. A drag that never left the source (no extension) is a
    /// no-op beyond clearing the state.
    fn end_fill_drag(&mut self, ctrl: bool) {
        // The drag is over: drop the crosshair (the next hover re-arms it).
        self.set_fill_cursor(false);
        self.hover_cursor = String::new();
        let (Some((src, block)), Some(dest)) =
            (self.fill_drag.take(), self.state.fill_preview.take())
        else {
            self.fill_drag = None;
            self.state.fill_preview = None;
            self.edit_dirty = true;
            return;
        };
        self.edit_dirty = true;
        if dest == src {
            return; // No extension: nothing to fill.
        }
        let (sr1, sc1, sr2, sc2) = src;
        let (dr1, dc1, dr2, dc2) = dest;
        // Ctrl inverts the default Excel/Sheets behaviour, but the single- vs
        // multi-cell test is per fill line, i.e. on the source extent *along the
        // drag axis* (a vertical drag fills column lines, so it cares about the
        // source row count; a horizontal drag cares about the source col count).
        // A single source value along that axis forces a +1 numeric Series; a
        // multi-value source forces a verbatim Copy. Without Ctrl, Auto lets the
        // Logic thread pick Series vs Copy.
        //
        // A merged source counts blocks, not raw cells, along the drag axis: one
        // block is a single value (Ctrl -> +1 Series), several blocks are several
        // values (Ctrl -> Copy). `block` is the merge span; divide the source
        // extent by it to get the block count along the axis.
        let vertical = dr1 < sr1 || dr2 > sr2;
        let single_line = if block != (1, 1) {
            let n = if vertical {
                (sr2 - sr1 + 1) / block.0
            } else {
                (sc2 - sc1 + 1) / block.1
            };
            n == 1
        } else if vertical {
            sr1 == sr2
        } else {
            sc1 == sc2
        };
        let mode = if ctrl {
            if single_line {
                FillMode::Series
            } else {
                FillMode::Copy
            }
        } else {
            FillMode::Auto
        };
        let cmd = TrameCommand::FillRange {
            sheet_id: self.state.active_sheet.clone(),
            src: GridRange {
                row: sr1,
                col: sc1,
                row_end: sr2,
                col_end: sc2,
            },
            dest: GridRange {
                row: dr1,
                col: dc1,
                row_end: dr2,
                col_end: dc2,
            },
            mode,
        };
        if let Err(SendError::Full) = self.send_command_lossless(cmd) {
            tracing::error!("Trame: UI->Logic channel full, FillRange dropped");
        }
        // Grow the selection to the whole filled rectangle (anchor top-left,
        // active bottom-right), matching Sheets after a fill-handle drag.
        self.state.selection.anchor = (dr1, dc1);
        self.state.selection.active = (dr2, dc2);
        self.state.selection.extra.clear();
        self.viewport_changed = true;
        if let Err(SendError::Full) = self.send_command_lossless(self.set_active_cmd()) {
            tracing::error!("Trame: UI->Logic channel full, fill selection dropped");
        }
        self.announce_active();
    }

    /// Select a cell directly (pointer hit), keeping Logic and the pipe in sync.
    fn select_cell(&mut self, row: u32, col: u32) {
        if row == self.state.active_row() && col == self.state.active_col() {
            self.park_input_on(row, col);
            return;
        }
        self.state.select(row, col);
        self.viewport_changed = true;
        if let Err(SendError::Full) = self.send_command_lossless(self.set_active_cmd()) {
            tracing::error!("Trame: UI→Logic channel full, cell selection dropped");
        }
        self.park_input_on(row, col);
        self.announce_active();
    }

    /// Build the active-cell report for a whole-axis selection (`-ui.md`
    /// sec.4bis.A). The band has no on-screen rectangle, so the perpendicular
    /// extent is bounded by the UI's used-range cache (sec.16): a column band
    /// spans rows `0..=last_used_row`, a row band spans cols `0..=last_used_col`.
    /// The Logic thread then resolves sort / filter / delete / style against that
    /// rectangle, and tracks the clicked column/row as the active cell.
    fn axis_active_cmd(&self) -> TrameCommand {
        let sel = &self.state.selection;
        let (last_row, last_col) = self
            .state
            .last_content_cell()
            .unwrap_or((self.state.active_row(), self.state.active_col()));
        match sel.kind {
            crate::ui::grid::SelKind::Cols => TrameCommand::SetActiveCell {
                row: 0,
                col: sel.active.1,
                anchor_row: last_row,
                anchor_col: sel.anchor.1,
            },
            crate::ui::grid::SelKind::Rows => TrameCommand::SetActiveCell {
                row: sel.active.0,
                col: 0,
                anchor_row: sel.anchor.0,
                anchor_col: last_col,
            },
            crate::ui::grid::SelKind::Cells => self.set_active_cmd(),
        }
    }

    /// Select a whole column/row band from a header click (sec.4bis.A) and report
    /// it to Logic as a used-range rectangle.
    fn select_axis(&mut self, kind: crate::ui::grid::SelKind, idx: u32) {
        self.state.selection = Selection::axis(kind, idx, idx);
        self.viewport_changed = true;
        if let Err(SendError::Full) = self.send_command_lossless(self.axis_active_cmd()) {
            tracing::error!("Trame: UI->Logic channel full, axis select dropped");
        }
        self.announce_active();
    }

    /// Extend the active whole-axis band to column/row `idx` (Shift+click or a
    /// header drag), keeping the anchor.
    fn extend_axis(&mut self, idx: u32) {
        let sel = &mut self.state.selection;
        match sel.kind {
            crate::ui::grid::SelKind::Cols => sel.active = (0, idx),
            crate::ui::grid::SelKind::Rows => sel.active = (idx, 0),
            crate::ui::grid::SelKind::Cells => return,
        }
        self.viewport_changed = true;
        if let Err(SendError::Full) = self.send_command_lossless(self.axis_active_cmd()) {
            tracing::error!("Trame: UI->Logic channel full, axis extend dropped");
        }
    }

    /// Select the whole sheet from the header corner (sec.4bis.A): a column band
    /// spanning every column.
    fn select_all(&mut self) {
        self.state.selection = Selection::axis(crate::ui::grid::SelKind::Cols, 0, TOTAL_COLS - 1);
        self.viewport_changed = true;
        if let Err(SendError::Full) = self.send_command_lossless(self.axis_active_cmd()) {
            tracing::error!("Trame: UI->Logic channel full, select-all dropped");
        }
        self.announce_active();
    }

    // -- Edit mode -------------------------------------------------------------

    /// The selected text inside the open editor (copy / Ctrl+Insert), or `None`
    /// when not editing or the selection is empty. Read synchronously by the
    /// host's `copy`/`cut` listeners to feed `ClipboardEvent.clipboardData`.
    pub fn copy_selection(&self) -> Option<String> {
        let ed = self.editor.as_ref()?;
        let s = ed.selected_text();
        (!s.is_empty()).then(|| s.to_owned())
    }

    /// Like [`copy_selection`], but also deletes the selection (cut / Shift+Del).
    ///
    /// [`copy_selection`]: Self::copy_selection
    pub fn cut_selection(&mut self) -> Option<String> {
        let ed = self.editor.as_mut()?;
        let s = ed.selected_text().to_owned();
        if s.is_empty() {
            return None;
        }
        ed.delete_selection();
        self.reset_blink();
        Some(s)
    }

    /// Whether an in-cell edit session is open (the host routes a paste to the
    /// caret rather than the grid when so).
    pub fn is_editing(&self) -> bool {
        self.editor.is_some()
    }

    // -- Grid clipboard (spec sec.5.E) -----------------------------------------

    /// Build the clipboard contents for a copy/cut (`-ui.md` sec.5.E). An open
    /// in-cell text selection wins (returns `text/plain` only, no Ambre payload);
    /// otherwise the grid selection is serialised, returning both the TSV text
    /// (OS interop) and the typed [`ClipboardPayload`](ambre_bus::ClipboardPayload)
    /// (internal round-trip with styles + formula reindexing). Arms the
    /// marching-ants anchor and the cut-pending flag. `None` means "nothing to
    /// copy -- let the browser default proceed".
    pub fn clipboard_write(
        &mut self,
        cut: bool,
    ) -> Option<(String, Option<ambre_bus::ClipboardPayload>)> {
        // In-cell text selection takes precedence over the grid range.
        if self.editor.is_some() {
            let text = if cut {
                self.cut_selection()
            } else {
                self.copy_selection()
            }?;
            return Some((text, None));
        }
        let payload = self.state.build_clipboard_payload();
        if payload.cells.is_empty() {
            return None;
        }
        self.state.clipboard_anchor = Some(self.state.selection.bounds());
        self.cut_pending = cut;
        self.edit_dirty = true; // repaint the marching-ants overlay
        Some((payload.to_tsv(), Some(payload)))
    }

    /// Paste a typed Ambre payload at the active cell (`-ui.md` sec.5.E). The
    /// Logic thread reindexes relative formulas and applies styles; a pending cut
    /// then clears the source range.
    pub fn grid_paste_payload(&mut self, payload: ambre_bus::ClipboardPayload) {
        let (row, col) = (self.state.active_row(), self.state.active_col());
        if let Err(SendError::Full) = self.send_command_lossless(TrameCommand::PasteRange {
            target_row: row,
            target_col: col,
            payload,
        }) {
            tracing::error!("Trame: UI->Logic channel full, paste dropped");
        }
        self.finish_paste();
        self.needs_full = true;
    }

    /// Paste external tab-separated text at the active cell (`-ui.md` sec.5.E):
    /// each field becomes a literal cell (`src == target`, so no relative-formula
    /// shift). Used when the clipboard carries no Ambre payload.
    pub fn grid_paste_text(&mut self, text: &str) {
        let (row, col) = (self.state.active_row(), self.state.active_col());
        let normalized = text.replace('\r', "");
        let mut cells = Vec::new();
        let mut rows = 0u32;
        let mut cols = 0u32;
        for (dr, line) in normalized.trim_end_matches('\n').split('\n').enumerate() {
            rows = rows.max(dr as u32 + 1);
            for (dc, field) in line.split('\t').enumerate() {
                cols = cols.max(dc as u32 + 1);
                if field.is_empty() {
                    continue;
                }
                cells.push(ambre_bus::ClipboardCell {
                    dr: dr as u32,
                    dc: dc as u32,
                    raw: field.to_owned(),
                    ..Default::default()
                });
            }
        }
        if cells.is_empty() {
            return;
        }
        let payload = ambre_bus::ClipboardPayload {
            src_row: row,
            src_col: col,
            rows,
            cols,
            cells,
        };
        if let Err(SendError::Full) = self.send_command_lossless(TrameCommand::PasteRange {
            target_row: row,
            target_col: col,
            payload,
        }) {
            tracing::error!("Trame: UI->Logic channel full, paste dropped");
        }
        self.needs_full = true;
    }

    /// After a paste: clear the source range if the clipboard held a cut, then
    /// drop the marching-ants anchor.
    fn finish_paste(&mut self) {
        if self.cut_pending
            && let Some((r1, c1, r2, c2)) = self.state.clipboard_anchor
        {
            let batch = (r1..=r2)
                .flat_map(|r| (c1..=c2).map(move |c| (r, c, String::new())))
                .collect();
            let _ = self.send_command_lossless(TrameCommand::SetCellsBatch { cells: batch });
            for r in r1..=r2 {
                for c in c1..=c2 {
                    self.state.set_cell_text(r, c, String::new());
                }
            }
        }
        self.cut_pending = false;
        self.state.clipboard_anchor = None;
    }

    /// True when physical pixel `(x, y)` falls on the cell currently being
    /// edited -- the gate for in-cell mouse text selection vs. committing.
    fn pointer_in_editor(&self, x: f64, y: f64) -> bool {
        let Some(ed) = self.editor.as_ref() else {
            return false;
        };
        self.state.cell_at(x, y) == Some((ed.row, ed.col))
    }

    /// Input routing while editing a cell.
    fn handle_input_editing(&mut self, ev: &InputEvent) -> bool {
        match ev {
            InputEvent::Text { text } => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.insert(text);
                    #[cfg(target_arch = "wasm32")]
                    self.formula_text.set(ed.buffer.clone());
                }
                self.reset_blink();
                true
            }
            InputEvent::CompositionUpdate { text } => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.preview = text.clone();
                }
                self.reset_blink();
                true
            }
            InputEvent::CompositionEnd { text } => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.preview.clear();
                    ed.insert(text);
                    #[cfg(target_arch = "wasm32")]
                    self.formula_text.set(ed.buffer.clone());
                }
                self.reset_blink();
                true
            }
            // A press inside the edited cell starts a mouse text selection; a
            // press outside commits and reselects the clicked cell.
            InputEvent::PointerDown {
                x, y, button: 0, ..
            } => {
                if self.pointer_in_editor(*x, *y) {
                    if let Some(ed) = self.editor.as_mut() {
                        ed.pending = Some(PendingHit {
                            x: *x,
                            kind: HitKind::Place,
                        });
                    }
                    self.editor_text_drag = true;
                    self.reset_blink();
                    return true;
                }
                self.commit_edit();
                self.text_input.focus();
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
                    self.select_cell(row, col);
                }
                true
            }
            // Drag inside the edited cell extends the text selection.
            InputEvent::PointerMove { x, .. } if self.editor_text_drag => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.pending = Some(PendingHit {
                        x: *x,
                        kind: HitKind::Extend,
                    });
                }
                self.reset_blink();
                true
            }
            InputEvent::PointerUp { .. } if self.editor_text_drag => {
                self.editor_text_drag = false;
                true
            }
            // Double-click inside the edited cell selects the word under it.
            InputEvent::DoubleClick { x, y } if self.pointer_in_editor(*x, *y) => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.pending = Some(PendingHit {
                        x: *x,
                        kind: HitKind::Word,
                    });
                }
                self.reset_blink();
                true
            }
            // A right-click commits and selects the cell under the cursor, so the
            // DOM context menu opens on it. We don't preventDefault (the
            // `contextmenu` listener owns the menu).
            InputEvent::PointerDown {
                x, y, button: 2, ..
            } => {
                self.commit_edit();
                self.text_input.focus();
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
                    self.select_cell(row, col);
                }
                false
            }
            InputEvent::Key {
                key,
                ctrl,
                meta,
                shift,
                ..
            } => {
                // Ctrl/Cmd+Enter fills the whole selection with the typed value;
                // Ctrl/Cmd+A selects the whole buffer; other command chords
                // (Ctrl/Cmd+X) fall through to the chrome keymap.
                if *ctrl || *meta {
                    if key == "Enter" {
                        self.commit_edit_filled();
                        return true;
                    }
                    if matches!(key.as_str(), "a" | "A") {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.select_all();
                        }
                        self.reset_blink();
                        return true;
                    }
                    return false;
                }
                match key.as_str() {
                    "Enter" => {
                        self.commit_edit();
                        self.nav_move(1, 0, false);
                        true
                    }
                    "Tab" => {
                        self.commit_edit();
                        self.nav_move(0, 1, false);
                        true
                    }
                    "Escape" => {
                        self.cancel_edit();
                        true
                    }
                    "Backspace" => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.backspace();
                            #[cfg(target_arch = "wasm32")]
                            self.formula_text.set(ed.buffer.clone());
                        }
                        self.reset_blink();
                        true
                    }
                    // Delete removes the selection, else the char after the caret.
                    // Shift+Delete is "cut": let it fall through so the browser
                    // fires the `cut` clipboard event the host listens for.
                    "Delete" if !*shift => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.delete_forward();
                            #[cfg(target_arch = "wasm32")]
                            self.formula_text.set(ed.buffer.clone());
                        }
                        self.reset_blink();
                        true
                    }
                    // Shift extends the selection; a bare arrow collapses it.
                    "ArrowLeft" => {
                        if let Some(ed) = self.editor.as_mut() {
                            if *shift {
                                ed.extend_left()
                            } else {
                                ed.move_left()
                            }
                        }
                        self.reset_blink();
                        true
                    }
                    "ArrowRight" => {
                        if let Some(ed) = self.editor.as_mut() {
                            if *shift {
                                ed.extend_right()
                            } else {
                                ed.move_right()
                            }
                        }
                        self.reset_blink();
                        true
                    }
                    "Home" => {
                        if let Some(ed) = self.editor.as_mut() {
                            if *shift {
                                ed.extend_home()
                            } else {
                                ed.move_home()
                            }
                        }
                        self.reset_blink();
                        true
                    }
                    "End" => {
                        if let Some(ed) = self.editor.as_mut() {
                            if *shift {
                                ed.extend_end()
                            } else {
                                ed.move_end()
                            }
                        }
                        self.reset_blink();
                        true
                    }
                    // Up/Down commit and move, as in a spreadsheet.
                    "ArrowUp" => {
                        self.commit_edit();
                        self.nav_move(-1, 0, false);
                        true
                    }
                    "ArrowDown" => {
                        self.commit_edit();
                        self.nav_move(1, 0, false);
                        true
                    }
                    // Printable keys fall through to the textarea -> `Text`.
                    _ => false,
                }
            }
            _ => false,
        }
    }
}
