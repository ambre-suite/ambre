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

use ambre_bus::{AppCommand, AppEvent, DocumentId, LogicCommand, TrameCommand, TrameEvent, UiEvent};
use ambre_draw::{Clock, FontContext, Rect};
use ambre_canvas::{
    Anchor, AnchorPublisher, ContentRepaint, FrameStats, InputEvent, OverlayRepaint, RepaintPlan,
    TextCache, TextInput, Theme, ThemeControl, WgpuSurface,
};
#[cfg(target_arch = "wasm32")]
use leptos::prelude::RwSignal;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::{Get, Set};

use super::grid::{
    CellEditor, COL_WIDTH, CursorMove, HEADER_HEIGHT, HEADER_WIDTH, HitKind,
    PendingHit, ROW_HEIGHT, TabDrag, TabMenu, TabMenuItem, TabRename, TrameUiState,
};
use super::renderer::{self, DrawStats};
#[cfg(target_arch = "wasm32")]
use crate::render::{CommitDirection, FormulaAction};

/// Caret blink half-period: the standard ~530 ms on/off toggle (period ~1.06 s).
const BLINK_MS: f64 = 530.0;

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
    /// Active in-cell edit session, or `None` in navigation mode.
    editor: Option<CellEditor>,
    /// True while the pointer is dragging out a text selection inside the open
    /// editor (between a press in the cell and its release).
    editor_text_drag: bool,
    /// True while the pointer is dragging out a grid cell-range selection in
    /// navigation mode (between a press on a cell and its release). Pointer
    /// moves while set extend the live selection rectangle.
    selecting_drag: bool,
    /// Active in-place sheet-tab rename, or `None` (spec sec.13).
    tab_rename: Option<TabRename>,
    /// Open per-tab context menu (`{ Rename, Delete }`), or `None`.
    tab_menu: Option<TabMenu>,
    /// In-progress tab drag-reorder, or `None`.
    tab_drag: Option<TabDrag>,
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
        theme_ctl: ThemeControl,
        #[cfg(target_arch = "wasm32")] formula_ref: RwSignal<String>,
        #[cfg(target_arch = "wasm32")] formula_text: RwSignal<String>,
        #[cfg(target_arch = "wasm32")] formula_action: RwSignal<Option<FormulaAction>>,
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
            theme_ctl,
            theme,
            #[cfg(target_arch = "wasm32")]
            formula_ref,
            #[cfg(target_arch = "wasm32")]
            formula_text,
            #[cfg(target_arch = "wasm32")]
            formula_action,
            editor: None,
            editor_text_drag: false,
            selecting_drag: false,
            tab_rename: None,
            tab_menu: None,
            tab_drag: None,
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
    }

    /// The active cell's spreadsheet reference (e.g. `"B2"`), shown in the
    /// formula bar's name box (spec sec.6).
    #[cfg(target_arch = "wasm32")]
    fn active_cell_ref(&self) -> String {
        format!("{}{}", col_label(self.state.active_col()), self.state.active_row() + 1)
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
        if let Some((x, y, _, _)) = self.state.cell_pixel_rect(row, col) {
            self.text_input
                .set_caret(x / self.dpr, y / self.dpr, ROW_HEIGHT / self.dpr);
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
        let Some(editor) = self.editor.take() else { return };
        tracing::info!(
            "[UI] commit_edit → sending SetCellText row={} col={} text='{:.80}'",
            editor.row, editor.col,
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
        self.state.set_cell_text(editor.row, editor.col, editor.buffer);
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
        let Some(editor) = self.editor.take() else { return };
        let text = editor.buffer;
        let cells = self.state.selection.selected_cells();
        tracing::info!("[UI] commit_edit_filled -> {} cells, text='{:.80}'", cells.len(), text);
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
        self.tab_rename = None;
        self.tab_menu = None;
        self.tab_drag = None;
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

    pub fn update(&mut self) {
        // Drain every queued UiEvent without blocking. A `Disconnected` error
        // means the Logic thread is gone (panic=abort or shutdown) — there is
        // no recovery, so flip to the fatal state instead of silently looping.
        loop {
            match self.from_logic.try_recv() {
                // StyleApplied targets the chrome formatting toolbar; the UI-thread
                // dispatcher consumes it. The canvas ignores it -- it renders
                // per-cell formatting from the cache (refreshed by the accompanying
                // DataReset), not from the active-cell style event.
                Ok(UiEvent::Viewport { event: AppEvent::Trame(TrameEvent::StyleApplied { .. }), .. }) => {}
                Ok(UiEvent::Viewport { event: AppEvent::Trame(TrameEvent::SheetList { sheets, active }), .. }) => {
                    // The active sheet may have changed (switch, or delete
                    // fallback decided by the Logic thread); adopt it and refresh
                    // the strip. A separate DataReset re-fetches the viewport.
                    self.state.on_sheet_list(sheets, active);
                    self.needs_full = true;
                }
                Ok(UiEvent::Viewport { event: AppEvent::Trame(TrameEvent::Error { message }), .. }) => {
                    // A rejected lifecycle op (e.g. deleting the last sheet).
                    // No toast surface yet -- log it and announce for a11y.
                    tracing::warn!("[UI] Trame rejected: {message}");
                    self.text_input.announce(&message);
                }
                // Shared filter (sec.5.D): track the filtered column for the funnel
                // badge. The accompanying DataReset re-fetches the collapsed rows.
                Ok(UiEvent::Viewport { event: AppEvent::Trame(TrameEvent::FilterApplied { col, .. }), .. }) => {
                    self.state.filtered_col = Some(col);
                    self.needs_full = true;
                }
                Ok(UiEvent::Viewport { event: AppEvent::Trame(TrameEvent::FilterCleared), .. }) => {
                    self.state.filtered_col = None;
                    self.needs_full = true;
                }
                Ok(UiEvent::Viewport { event: AppEvent::Trame(TrameEvent::DataReset), .. }) => {
                    // Local snapshot restored: re-fetch the visible region so the
                    // restored cells render, then full-repaint with the result.
                    self.needs_full = true;
                    let cmd = self.state.scroll_viewport_cmd();
                    self.send_command_lossy(cmd);
                }
                Ok(UiEvent::Viewport { event: AppEvent::Trame(update), .. }) => {
                    self.state.on_viewport_data(update);
                    // Just flag data dirty. If a scroll happens the same frame the
                    // strip path reads the refreshed cache; otherwise plan_repaint
                    // promotes data_dirty to a Full repaint.
                    self.data_dirty = true;
                }
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
        if self.editor.is_some() || self.tab_rename.is_some() {
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
                            CommitDirection::Down => { self.nav_move(1, 0, false); }
                            CommitDirection::Right => { self.nav_move(0, 1, false); }
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
            } else {
                let dy_px = -(dr as i32) * ROW_HEIGHT as i32;
                let dx_px = -(dc as i32) * COL_WIDTH as i32;
                let w = self.state.canvas_width;
                // Bottom edge of the *cell area*, not the full canvas: the
                // sheet-tab strip (sec.13) occupies [grid_bottom, canvas_height]
                // and is repainted via the overlay. Computing the scroll strip
                // with canvas_height would place the entering-row band inside the
                // tab strip and leave the real bottom cell row (height
                // TAB_STRIP_HEIGHT) stale after the shift -- the source of the
                // edge flicker on Arrow/Page navigation.
                let h = self.state.grid_bottom();
                // Top of the scrollable cell area: below the formula bar band
                // (sec.6) and the column-header strip.
                let hh = HEADER_HEIGHT;
                // If the scroll exceeds the viewport, nothing useful to shift.
                let abs_dx = dx_px.unsigned_abs();
                let abs_dy = dy_px.unsigned_abs();
                if abs_dx as f64 >= w || abs_dy as f64 >= h {
                    (ContentRepaint::Full, None)
                } else {
                    // Strip = the area of the destination offscreen exposed by
                    // the shift, restricted to the cell area.
                    let hw = HEADER_WIDTH;
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

        RepaintPlan { content, content_strip, overlay }
    }

    pub fn paint_content(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        strip: Option<Rect>,
    ) {
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
            stats.fps,
            stats.frame_ms,
            stats.drain_ms,
            stats.draw_ms,
            stats.gpu_ms,
            &PerfClock(&self.perf),
            &mut self.draw_stats,
        );

        // Bottom sheet-tab strip (sec.13), above the grid/HUD.
        renderer::draw_sheet_tabs(
            surface,
            &self.state,
            self.theme.palette(),
            font_cx,
            text_cache,
            self.text_enabled,
            self.tab_rename.as_ref(),
            self.tab_drag.as_ref(),
            self.caret_on,
        );

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

        // The tab context menu floats above all else.
        if let Some(menu) = &self.tab_menu {
            renderer::draw_tab_menu(surface, self.theme.palette(), font_cx, text_cache, menu);
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.state.resize(width as f64, height as f64);
        self.needs_full = true;
    }

    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        // Wheel / two-finger touchpad scroll pans the viewport in any mode and
        // never disturbs the cursor or an open editor.
        if let InputEvent::Wheel { dx, dy, delta_mode, ctrl_key } = ev {
            return self.handle_wheel(*dx, *dy, *delta_mode, *ctrl_key);
        }
        // An open tab rename owns text, IME and the editing keys until it ends.
        if self.tab_rename.is_some()
            && let Some(consumed) = self.handle_tab_rename_input(ev)
        {
            return consumed;
        }
        // An open tab context menu intercepts clicks (select item or dismiss).
        if self.tab_menu.is_some()
            && let Some(consumed) = self.handle_tab_menu_input(ev)
        {
            return consumed;
        }
        // Tab strip interactions (activate / create / rename / drag-reorder).
        if let Some(consumed) = self.handle_tab_input(ev) {
            return consumed;
        }
        // An open edit session owns text, IME and the editing keys.
        if self.editor.is_some() {
            return self.handle_input_editing(ev);
        }
        self.handle_input_nav(ev)
    }
}

// -- Sheet tabs (spec sec.13) -------------------------------------------------
impl TrameView {
    /// Pointer interactions on the bottom tab strip, plus an in-flight drag.
    /// Returns `None` for events that are not ours, so they fall through to the
    /// grid (e.g. a pointer-down above the strip).
    fn handle_tab_input(&mut self, ev: &InputEvent) -> Option<bool> {
        match ev {
            InputEvent::PointerDown { x, y, button: 0, .. } if self.state.in_tab_strip(*y) => {
                // Any open cell edit is committed before leaving the grid.
                self.commit_edit();
                if self.state.add_button_at(*x, *y) {
                    self.create_tab();
                } else if let Some(i) = self.state.tab_at(*x, *y) {
                    let (id, _) = self.state.sheets[i].clone();
                    self.activate_tab(id.clone());
                    // Arm a potential drag-reorder from this grab point.
                    self.tab_drag = Some(TabDrag { sheet_id: id, grab_x: *x, cur_x: *x, moved: false });
                }
                Some(true)
            }
            // Right-click a tab opens its context menu; consume so the browser
            // menu does not appear over the canvas.
            InputEvent::PointerDown { x, y, button: 2, .. } if self.state.in_tab_strip(*y) => {
                if let Some(i) = self.state.tab_at(*x, *y) {
                    let (id, _) = self.state.sheets[i].clone();
                    let menu_h = 2.0 * super::grid::TAB_MENU_ITEM_HEIGHT;
                    self.tab_menu = Some(TabMenu { sheet_id: id, x: *x, y: self.state.grid_bottom() - menu_h });
                    self.edit_dirty = true;
                    return Some(true);
                }
                Some(false)
            }
            // Double-click a tab starts an in-place rename.
            InputEvent::DoubleClick { x, y } if self.state.in_tab_strip(*y) => {
                self.commit_edit();
                if let Some(i) = self.state.tab_at(*x, *y) {
                    self.begin_tab_rename(i);
                }
                Some(true)
            }
            InputEvent::PointerMove { x, .. } if self.tab_drag.is_some() => {
                if let Some(d) = self.tab_drag.as_mut() {
                    d.cur_x = *x;
                    if (d.cur_x - d.grab_x).abs() > 4.0 {
                        d.moved = true;
                    }
                }
                self.edit_dirty = true;
                Some(true)
            }
            InputEvent::PointerUp { .. } if self.tab_drag.is_some() => {
                if let Some(d) = self.tab_drag.take()
                    && d.moved
                {
                    let to_index = self.state.drop_index_at(d.cur_x);
                    if let Err(SendError::Full) =
                        self.send_command_lossless(TrameCommand::MoveSheet { sheet_id: d.sheet_id, to_index })
                    {
                        tracing::error!("Trame: UI->Logic channel full, MoveSheet dropped");
                    }
                }
                self.edit_dirty = true;
                Some(true)
            }
            _ => None,
        }
    }

    /// Clicks while the tab context menu is open: select an item, or dismiss on
    /// an outside click (which then falls through to normal routing).
    fn handle_tab_menu_input(&mut self, ev: &InputEvent) -> Option<bool> {
        let InputEvent::PointerDown { x, y, .. } = ev else { return None };
        let menu = self.tab_menu.as_ref()?;
        let item = menu.item_at(*x, *y);
        let sheet_id = menu.sheet_id.clone();
        self.tab_menu = None;
        self.edit_dirty = true;
        match item {
            Some(TabMenuItem::Rename) => {
                if let Some(i) = self.state.sheets.iter().position(|(id, _)| *id == sheet_id) {
                    self.begin_tab_rename(i);
                }
                Some(true)
            }
            Some(TabMenuItem::Delete) => {
                if let Err(SendError::Full) =
                    self.send_command_lossless(TrameCommand::DeleteSheet { sheet_id })
                {
                    tracing::error!("Trame: UI->Logic channel full, DeleteSheet dropped");
                }
                Some(true)
            }
            // Outside click: just dismiss, let the click proceed.
            None => None,
        }
    }

    /// Text/keyboard routing while an in-place tab rename is active.
    fn handle_tab_rename_input(&mut self, ev: &InputEvent) -> Option<bool> {
        match ev {
            InputEvent::Text { text } => {
                if let Some(r) = self.tab_rename.as_mut() {
                    r.editor.insert(text);
                }
                self.reset_blink();
                Some(true)
            }
            InputEvent::CompositionUpdate { text } => {
                if let Some(r) = self.tab_rename.as_mut() {
                    r.editor.preview = text.clone();
                }
                self.reset_blink();
                Some(true)
            }
            InputEvent::CompositionEnd { text } => {
                if let Some(r) = self.tab_rename.as_mut() {
                    r.editor.preview.clear();
                    r.editor.insert(text);
                }
                self.reset_blink();
                Some(true)
            }
            InputEvent::Key { key, ctrl, meta, .. } => {
                if *ctrl || *meta {
                    return Some(false);
                }
                match key.as_str() {
                    "Enter" => {
                        self.commit_tab_rename();
                        Some(true)
                    }
                    "Escape" => {
                        self.cancel_tab_rename();
                        Some(true)
                    }
                    "Backspace" => {
                        if let Some(r) = self.tab_rename.as_mut() {
                            r.editor.backspace();
                        }
                        self.reset_blink();
                        Some(true)
                    }
                    "ArrowLeft" => {
                        if let Some(r) = self.tab_rename.as_mut() {
                            r.editor.move_left();
                        }
                        self.reset_blink();
                        Some(true)
                    }
                    "ArrowRight" => {
                        if let Some(r) = self.tab_rename.as_mut() {
                            r.editor.move_right();
                        }
                        self.reset_blink();
                        Some(true)
                    }
                    // Printable keys fall through to the textarea -> `Text`.
                    _ => Some(false),
                }
            }
            // A pointer-down elsewhere commits the rename, then proceeds.
            InputEvent::PointerDown { .. } => {
                self.commit_tab_rename();
                None
            }
            _ => None,
        }
    }

    /// Switch the active sheet to `sheet_id` (tab click). Resets the scroll and
    /// selection; the Logic thread replies with `DataReset` + `SheetList`.
    fn activate_tab(&mut self, sheet_id: String) {
        if sheet_id == self.state.active_sheet {
            return;
        }
        if let Err(SendError::Full) =
            self.send_command_lossless(TrameCommand::ActivateSheet { sheet_id: sheet_id.clone() })
        {
            tracing::error!("Trame: UI->Logic channel full, ActivateSheet dropped");
        }
        self.state.active_sheet = sheet_id;
        self.state.top_row = 0;
        self.state.left_col = 0;
        self.state.select(0, 0);
        self.needs_full = true;
        // Re-fetch immediately; the forthcoming DataReset will refresh again.
        self.pending_viewport = Some(self.state.scroll_viewport_cmd());
    }

    /// Create a new sheet via the "+" button. Does not switch the active sheet
    /// (the Logic thread mints the id and replies `SheetList`).
    fn create_tab(&mut self) {
        let name = format!("Sheet{}", self.state.sheets.len() + 1);
        if let Err(SendError::Full) =
            self.send_command_lossless(TrameCommand::CreateSheet { name })
        {
            tracing::error!("Trame: UI->Logic channel full, CreateSheet dropped");
        }
    }

    /// Open the in-place rename editor on tab `index`, seeded with its name.
    fn begin_tab_rename(&mut self, index: usize) {
        let Some((id, name)) = self.state.sheets.get(index).cloned() else { return };
        self.tab_rename = Some(TabRename { sheet_id: id, editor: CellEditor::new(0, 0, name) });
        self.text_input.clear();
        self.text_input.focus();
        // Park the IME pipe on the tab (CSS px).
        let (x0, _) = self.state.tab_slot_x(index);
        self.text_input
            .set_caret(x0 / self.dpr, self.state.grid_bottom() / self.dpr, ROW_HEIGHT / self.dpr);
        self.reset_blink();
    }

    /// Commit the in-place rename (sends `RenameSheet` unless the name is blank).
    fn commit_tab_rename(&mut self) {
        let Some(r) = self.tab_rename.take() else { return };
        let name = r.editor.buffer.trim().to_owned();
        if !name.is_empty()
            && let Err(SendError::Full) =
                self.send_command_lossless(TrameCommand::RenameSheet { sheet_id: r.sheet_id, name })
        {
            tracing::error!("Trame: UI->Logic channel full, RenameSheet dropped");
        }
        self.text_input.clear();
        self.edit_dirty = true;
    }

    /// Abandon the in-place rename without writing anything.
    fn cancel_tab_rename(&mut self) {
        if self.tab_rename.take().is_some() {
            self.text_input.clear();
            self.edit_dirty = true;
        }
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
            InputEvent::PointerDown { x, y, button: 0, shift, ctrl } => {
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
            // A right-click selects the cell under the cursor before the DOM
            // context menu opens on it (standard spreadsheet behaviour). We don't
            // preventDefault here -- the `contextmenu` listener owns the menu.
            InputEvent::PointerDown { x, y, button: 2, .. } => {
                self.text_input.focus();
                // Right-click inside the current multi-cell selection keeps it
                // (so a context menu can act on the range); otherwise it collapses
                // to the clicked cell, like a left click.
                if let Some((row, col)) = self.state.cell_at(*x, *y)
                    && !self.state.selection.contains(row, col)
                {
                    self.select_cell(row, col);
                }
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
            InputEvent::Key { key, ctrl, meta, alt, shift } => {
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
                        let col = self.state.last_content_col(row).unwrap_or(self.state.active_col());
                        self.nav_goto(row, col, ext)
                    }
                    // Escape clears the marching-ants clipboard marquee (sec.5.E),
                    // if any; otherwise it is not ours.
                    "Escape" => {
                        if self.state.clipboard_anchor.take().is_some() {
                            self.cut_pending = false;
                            self.edit_dirty = true;
                            true
                        } else {
                            false
                        }
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
        // Normalise the unit to physical pixels. Deltas already include dpr.
        let (sx, sy) = match delta_mode {
            1 => (COL_WIDTH, ROW_HEIGHT),                              // DOM_DELTA_LINE
            2 => (self.state.canvas_width, self.state.canvas_height),  // DOM_DELTA_PAGE
            _ => (1.0, 1.0),                                           // DOM_DELTA_PIXEL
        };
        let (px, py) = (dx * sx, dy * sy);

        // The fingers are driving: any active momentum is overridden by the
        // direct input, so cancel the coast.
        self.coasting = false;

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
        let dc = (self.scroll_accum_x / COL_WIDTH) as i32;
        let dr = (self.scroll_accum_y / ROW_HEIGHT) as i32;
        if dr == 0 && dc == 0 {
            return ScrollOutcome::Idle;
        }
        self.scroll_accum_x -= dc as f64 * COL_WIDTH;
        self.scroll_accum_y -= dr as f64 * ROW_HEIGHT;
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
        TrameCommand::SetActiveCell { row, col, anchor_row, anchor_col }
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
            let text = if cut { self.cut_selection() } else { self.copy_selection() }?;
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
        let payload =
            ambre_bus::ClipboardPayload { src_row: row, src_col: col, rows, cols, cells };
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
        let Some(ed) = self.editor.as_ref() else { return false };
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
            InputEvent::PointerDown { x, y, button: 0, .. } => {
                if self.pointer_in_editor(*x, *y) {
                    if let Some(ed) = self.editor.as_mut() {
                        ed.pending = Some(PendingHit { x: *x, kind: HitKind::Place });
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
                    ed.pending = Some(PendingHit { x: *x, kind: HitKind::Extend });
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
                    ed.pending = Some(PendingHit { x: *x, kind: HitKind::Word });
                }
                self.reset_blink();
                true
            }
            // A right-click commits and selects the cell under the cursor, so the
            // DOM context menu opens on it. We don't preventDefault (the
            // `contextmenu` listener owns the menu).
            InputEvent::PointerDown { x, y, button: 2, .. } => {
                self.commit_edit();
                self.text_input.focus();
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
                    self.select_cell(row, col);
                }
                false
            }
            InputEvent::Key { key, ctrl, meta, shift, .. } => {
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
                            if *shift { ed.extend_left() } else { ed.move_left() }
                        }
                        self.reset_blink();
                        true
                    }
                    "ArrowRight" => {
                        if let Some(ed) = self.editor.as_mut() {
                            if *shift { ed.extend_right() } else { ed.move_right() }
                        }
                        self.reset_blink();
                        true
                    }
                    "Home" => {
                        if let Some(ed) = self.editor.as_mut() {
                            if *shift { ed.extend_home() } else { ed.move_home() }
                        }
                        self.reset_blink();
                        true
                    }
                    "End" => {
                        if let Some(ed) = self.editor.as_mut() {
                            if *shift { ed.extend_end() } else { ed.move_end() }
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
