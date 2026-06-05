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
    WgpuSurface,
};

use super::grid::{COL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH, ROW_HEIGHT, TrameUiState};
use super::renderer::{self, DrawStats};

/// Trame's `DocView`: a virtual-scroll spreadsheet grid.
pub struct TrameView {
    /// Outbound channel to the Logic thread.
    to_logic: flume::Sender<LogicCommand>,
    /// Inbound channel from the Logic thread, drained each frame in `update`.
    from_logic: flume::Receiver<UiEvent>,

    /// Grid navigation + cell cache.
    state: TrameUiState,

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
    ) -> Self {
        let perf = web_sys::window()
            .expect("no window")
            .performance()
            .expect("no Performance API");

        Self {
            to_logic,
            from_logic,
            state: TrameUiState::new(canvas_width, canvas_height),
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
    }

    fn plan_repaint(&mut self, hud_tick: bool) -> RepaintPlan {
        // Formatting lives per-cell in the persistent cell layer, so a cursor
        // move needs no special repaint -- the cells keep their runs as they
        // shift. Only a CellFormat event (handled in `update`) dirties content.
        let any_dirty =
            self.needs_full || self.viewport_changed || self.data_dirty || hud_tick;

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
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.state.resize(width as f64, height as f64);
        self.needs_full = true;
    }

    fn handle_input(&mut self, ev: &InputEvent) -> bool {
        // Command keybindings (e.g. Ctrl+B) are resolved by the chrome's
        // keymap, not here -- this handler is for content navigation only.
        let InputEvent::Key { key, .. } = ev else { return false };
        let (dr, dc): (i32, i32) = match key.as_str() {
            "ArrowUp" => (-1, 0),
            "ArrowDown" => (1, 0),
            "ArrowLeft" => (0, -1),
            "ArrowRight" => (0, 1),
            "t" | "T" => {
                self.text_enabled ^= true;
                // Text toggle affects every cell -- full repaint, no preventDefault.
                self.needs_full = true;
                return false;
            }
            _ => return false,
        };

        // Cursor or scroll move: viewport_changed; plan_repaint decides Scroll
        // vs Full from the actual delta.
        self.viewport_changed = true;
        if let Some(mv) = self.state.move_cursor(dr, dc) {
            // Inform Logic of the new active cell so formatting toggles retarget
            // and the chrome buttons track the selection. Cheap and idempotent;
            // sent immediately (one per keypress, bounded by key-repeat rate).
            self.send_command(mv.active);
            // Coalesce the viewport fetch: keep only the latest; `update` sends
            // it once this frame. Intermediate requests are superseded (their
            // cache eviction already happened inside `move_cursor`).
            if let Some(scroll) = mv.scroll {
                self.pending_viewport = Some(scroll);
            }
        }
        true
    }
}
