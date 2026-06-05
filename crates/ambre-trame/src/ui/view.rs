// ambre-trame/src/ui/view.rs
// ============================================================================
// TrameView -- the Trame implementation of `ambre_canvas::DocView`.
//
// Owns everything that used to live in ambre-app's `UiState`: the grid
// navigation state, the inbound/outbound logic-thread channels, the dirty-rect
// bookkeeping, and the per-frame repaint decision. The generic FrameHost in
// ambre-canvas drives it (update → plan_repaint → paint_*), knowing nothing about
// spreadsheets.
// ============================================================================

use ambre_bus::{AppCommand, AppEvent, IPC_TEST_DOC_ID, LogicCommand, TrameCommand, TrameEvent, UiEvent};
use ambre_draw::{FontContext, Rect};
use ambre_canvas::{
    ContentRepaint, DocView, FrameStats, InputEvent, OverlayRepaint, RepaintPlan, TextCache,
    TextInput, WgpuSurface,
};

use super::grid::{CellEditor, COL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH, ROW_HEIGHT, TrameUiState};
use super::renderer::{self, DrawStats};

/// Caret blink half-period: the standard ~530 ms on/off toggle (period ~1.06 s).
const BLINK_MS: f64 = 530.0;

/// Trame's `DocView`: a virtual-scroll spreadsheet grid.
pub struct TrameView {
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
    /// Active in-cell edit session, or `None` in navigation mode.
    editor: Option<CellEditor>,
    /// The edit overlay changed this frame (keystroke, caret move, IME pass).
    edit_dirty: bool,
    /// Caret blink phase: `true` = caret drawn. Toggled every `BLINK_MS`, reset
    /// to `true` (solid) on any edit interaction, as platforms do.
    caret_on: bool,
    /// `performance.now()` of the last blink toggle (or blink reset).
    last_blink_ms: f64,

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
}

impl TrameView {
    pub fn new(
        to_logic: flume::Sender<LogicCommand>,
        from_logic: flume::Receiver<UiEvent>,
        canvas_width: f64,
        canvas_height: f64,
        text_input: TextInput,
    ) -> Self {
        let window = web_sys::window().expect("no window");
        let perf = window.performance().expect("no Performance API");
        let dpr = window.device_pixel_ratio();

        // Focus the pipe up front so the keyboard works without a click; park it
        // on the active cell so a first keystroke or IME opens there.
        text_input.focus();

        Self {
            to_logic,
            from_logic,
            state: TrameUiState::new(canvas_width, canvas_height),
            text_input,
            dpr,
            editor: None,
            edit_dirty: false,
            caret_on: true,
            last_blink_ms: 0.0,
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
        }
    }

    /// Send a typed `TrameCommand` to the Logic thread.
    ///
    /// Runs on the page's main thread, so it must never block: a full channel
    /// drops the command (a viewport request is superseded by the next one via
    /// `version`; nothing here is a non-idempotent P3 mutation). A disconnect
    /// is surfaced by `update`'s drain, which sets `fatal`.
    fn send_command(&self, cmd: TrameCommand) {
        if self.fatal {
            return;
        }
        let msg = LogicCommand::App {
            document_id: IPC_TEST_DOC_ID,
            command: AppCommand::Trame(cmd),
        };
        match self.to_logic.try_send(msg) {
            Ok(()) => {}
            Err(flume::TrySendError::Full(_)) => {
                tracing::warn!("Trame: UI→Logic channel full, dropping viewport command");
            }
            Err(flume::TrySendError::Disconnected(_)) => {
                tracing::error!("Trame: UI→Logic channel disconnected");
            }
        }
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
    }

    // -- Edit session ----------------------------------------------------------

    /// Show the caret solid and restart its blink clock. Called on every edit
    /// interaction so the caret never blinks mid-keystroke.
    fn reset_blink(&mut self) {
        self.caret_on = true;
        self.last_blink_ms = self.perf.now();
        self.edit_dirty = true;
    }

    /// Seed text for an editor opened on `(row, col)`: the cell's current text,
    /// or empty if not cached.
    fn cell_seed(&self, row: u32, col: u32) -> String {
        self.state
            .cell_cache
            .get(&(row, col))
            .map(|c| c.text.clone())
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
        let (row, col) = (self.state.active_row, self.state.active_col);
        self.editor = Some(CellEditor::new(row, col, seed));
        self.text_input.clear();
        self.text_input.focus();
        self.park_input_on(row, col);
        self.reset_blink();
    }

    /// Commit the current edit into the CRDT (via Logic) and update the local
    /// cache optimistically, then leave edit mode. No-op if not editing.
    fn commit_edit(&mut self) {
        let Some(editor) = self.editor.take() else { return };
        self.send_command(TrameCommand::SetCellText {
            row: editor.row,
            col: editor.col,
            text: editor.buffer.clone(),
        });
        self.state.set_cell_text(editor.row, editor.col, editor.buffer);
        // Content layer must repaint to show the committed text; overlay too, to
        // drop the edit field.
        self.needs_full = true;
        self.edit_dirty = true;
        self.text_input.clear();
    }

    /// Abandon the current edit without writing anything, then leave edit mode.
    fn cancel_edit(&mut self) {
        if self.editor.take().is_some() {
            // Content is untouched; only the overlay field must be cleared.
            self.edit_dirty = true;
            self.text_input.clear();
        }
    }
}

impl DocView for TrameView {
    fn update(&mut self) {
        // Drain every queued UiEvent without blocking. A `Disconnected` error
        // means the Logic thread is gone (panic=abort or shutdown) — there is
        // no recovery, so flip to the fatal state instead of silently looping.
        loop {
            match self.from_logic.try_recv() {
                Ok(UiEvent::Viewport { event: AppEvent::Trame(TrameEvent::CellFormat { row, col, bold, italic }), .. }) => {
                    // Formatting is pinned to the cell, not the selection.
                    self.state.set_cell_format(row, col, bold, italic);
                    self.needs_full = true;
                }
                // BoldState / ItalicState target the chrome toggle buttons; the
                // UI-thread dispatcher consumes them. The canvas ignores them --
                // it renders per-cell formatting from the cache instead.
                Ok(UiEvent::Viewport { event: AppEvent::Trame(TrameEvent::BoldState { .. } | TrameEvent::ItalicState { .. }), .. }) => {}
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

        // Flush the coalesced viewport request: at most one ScrollViewport per
        // frame, regardless of how many arrow keys fired since the last frame.
        if let Some(cmd) = self.pending_viewport.take() {
            self.send_command(cmd);
        }

        if !self.initial_viewport_sent {
            self.initial_viewport_sent = true;
            self.needs_full = true;
            let cmd = self.state.scroll_viewport_cmd();
            self.send_command(cmd);
        }

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
    }

    fn plan_repaint(&mut self, hud_tick: bool) -> RepaintPlan {
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
                let h = self.state.canvas_height;
                // If the scroll exceeds the viewport, nothing useful to shift.
                let abs_dx = dx_px.unsigned_abs();
                let abs_dy = dy_px.unsigned_abs();
                if abs_dx as f64 >= w || abs_dy as f64 >= h {
                    (ContentRepaint::Full, None)
                } else {
                    // Strip = the area of the destination offscreen exposed by
                    // the shift, restricted to the cell area.
                    let hh = HEADER_HEIGHT;
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

    fn paint_content(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        strip: Option<Rect>,
    ) {
        renderer::draw_cells(
            surface,
            &self.state,
            font_cx,
            text_cache,
            self.text_enabled,
            strip,
            &self.perf,
            &mut self.draw_stats,
        );
    }

    fn paint_overlay(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        stats: &FrameStats,
    ) {
        renderer::draw_overlay(
            surface,
            &self.state,
            font_cx,
            text_cache,
            self.text_enabled,
            stats.fps,
            stats.frame_ms,
            stats.drain_ms,
            stats.draw_ms,
            stats.gpu_ms,
            &self.perf,
            &mut self.draw_stats,
        );

        // The in-cell edit field sits above everything (after the active border).
        if let Some(editor) = &self.editor {
            renderer::draw_edit(surface, &self.state, editor, self.caret_on, font_cx, text_cache);
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.state.resize(width as f64, height as f64);
        self.needs_full = true;
    }

    fn handle_input(&mut self, ev: &InputEvent) -> bool {
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
            // A content click focuses the pipe and selects the clicked cell, so
            // typing or IME can start there (spec: canvas click -> textarea.focus).
            InputEvent::PointerDown { x, y, button: 0 } => {
                self.text_input.focus();
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
                    self.select_cell(row, col);
                }
                true
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
            InputEvent::Key { key, alt, .. } => match key.as_str() {
                "ArrowUp" => self.nav_move(-1, 0),
                "ArrowDown" => self.nav_move(1, 0),
                "ArrowLeft" => self.nav_move(0, -1),
                "ArrowRight" => self.nav_move(0, 1),
                // Enter / F2 open an editor on the cell's current text.
                "Enter" | "F2" => {
                    let seed = self.cell_seed(self.state.active_row, self.state.active_col);
                    self.begin_edit(seed);
                    true
                }
                // Debug HUD text toggle, moved behind Alt so plain letters type.
                "t" | "T" if *alt => {
                    self.text_enabled ^= true;
                    self.needs_full = true;
                    true
                }
                // Let plain printable keys fall through (no preventDefault) so the
                // focused textarea receives them and emits an `input` -> `Text`.
                _ => false,
            },
            _ => false,
        }
    }

    /// Move the active cell by `(dr, dc)`; returns `true` (consumed).
    fn nav_move(&mut self, dr: i32, dc: i32) -> bool {
        self.viewport_changed = true;
        if let Some(mv) = self.state.move_cursor(dr, dc) {
            self.send_command(mv.active);
            if let Some(scroll) = mv.scroll {
                self.pending_viewport = Some(scroll);
            }
            self.park_input_on(self.state.active_row, self.state.active_col);
        }
        true
    }

    /// Select a cell directly (pointer hit), keeping Logic and the pipe in sync.
    fn select_cell(&mut self, row: u32, col: u32) {
        if row == self.state.active_row && col == self.state.active_col {
            self.park_input_on(row, col);
            return;
        }
        self.state.active_row = row;
        self.state.active_col = col;
        self.viewport_changed = true;
        self.send_command(TrameCommand::SetActiveCell { row, col });
        self.park_input_on(row, col);
    }

    // -- Edit mode -------------------------------------------------------------

    /// Input routing while editing a cell.
    fn handle_input_editing(&mut self, ev: &InputEvent) -> bool {
        match ev {
            InputEvent::Text { text } => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.insert(text);
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
                }
                self.reset_blink();
                true
            }
            // A click outside commits and reselects the clicked cell.
            InputEvent::PointerDown { x, y, button: 0 } => {
                self.commit_edit();
                self.text_input.focus();
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
                    self.select_cell(row, col);
                }
                true
            }
            InputEvent::Key { key, ctrl, meta, .. } => {
                // Leave command chords (Ctrl/Cmd+X) to the chrome keymap.
                if *ctrl || *meta {
                    return false;
                }
                match key.as_str() {
                    "Enter" => {
                        self.commit_edit();
                        self.nav_move(1, 0);
                        true
                    }
                    "Tab" => {
                        self.commit_edit();
                        self.nav_move(0, 1);
                        true
                    }
                    "Escape" => {
                        self.cancel_edit();
                        true
                    }
                    "Backspace" => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.backspace();
                        }
                        self.reset_blink();
                        true
                    }
                    "ArrowLeft" => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.move_left();
                        }
                        self.reset_blink();
                        true
                    }
                    "ArrowRight" => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.move_right();
                        }
                        self.reset_blink();
                        true
                    }
                    // Up/Down commit and move, as in a spreadsheet.
                    "ArrowUp" => {
                        self.commit_edit();
                        self.nav_move(-1, 0);
                        true
                    }
                    "ArrowDown" => {
                        self.commit_edit();
                        self.nav_move(1, 0);
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
