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

use ambre_bus::{AppCommand, AppEvent, DocumentId, LogicCommand, TrameCommand, TrameEvent, UiEvent};
use ambre_draw::{FontContext, Rect};
use ambre_canvas::{
    Anchor, AnchorPublisher, ContentRepaint, DocView, FrameStats, InputEvent, OverlayRepaint,
    RepaintPlan, TextCache, TextInput, Theme, ThemeControl, WgpuSurface,
};

use super::grid::{CellEditor, COL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH, ROW_HEIGHT, TrameUiState};
use super::renderer::{self, DrawStats};

/// Caret blink half-period: the standard ~530 ms on/off toggle (period ~1.06 s).
const BLINK_MS: f64 = 530.0;

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
    /// Publishes the active cell's on-screen anchor (CSS px) so the chrome can
    /// transform-sync a floating selection toolbar onto it.
    anchor: AnchorPublisher,
    /// Last anchor published, so it is only re-published on change.
    last_anchor: Option<Anchor>,

    /// Shared theme switch (chrome toggles it, the canvas reads it).
    theme_ctl: ThemeControl,
    /// Theme used to render the last frame; a change forces a full repaint.
    theme: Theme,
    /// Active in-cell edit session, or `None` in navigation mode.
    editor: Option<CellEditor>,
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
}

impl TrameView {
    pub fn new(
        to_logic: flume::Sender<LogicCommand>,
        from_logic: flume::Receiver<UiEvent>,
        canvas_width: f64,
        canvas_height: f64,
        text_input: TextInput,
        anchor: AnchorPublisher,
        theme_ctl: ThemeControl,
    ) -> Self {
        let window = web_sys::window().expect("no window");
        let perf = window.performance().expect("no Performance API");
        let dpr = window.device_pixel_ratio();
        let theme = theme_ctl.get();

        // Focus the pipe up front so the keyboard works without a click; park it
        // on the active cell so a first keystroke or IME opens there.
        text_input.focus();

        Self {
            to_logic,
            from_logic,
            state: TrameUiState::new(canvas_width, canvas_height),
            text_input,
            dpr,
            anchor,
            last_anchor: None,
            theme_ctl,
            theme,
            editor: None,
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
            document_id: DocumentId::new(),
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

    /// Announce the active cell to screen readers via the live region (content
    /// a11y, 2.F): the cell reference plus its value, e.g. `"B2, 42"`. Called on
    /// every selection change so navigating the grid is audible, like a native
    /// spreadsheet.
    fn announce_active(&self) {
        let (row, col) = (self.state.active_row, self.state.active_col);
        let value = self.cell_seed(row, col);
        let desc = if value.is_empty() {
            format!("{}{}, empty", col_label(col), row + 1)
        } else {
            format!("{}{}, {}", col_label(col), row + 1, value)
        };
        self.text_input.announce(&desc);
    }

    /// Recompute the active cell's on-screen anchor and publish it for the
    /// floating selection toolbar -- the centre of the cell's top edge in CSS
    /// px, or `None` when the cell is scrolled out of the grid area. Cheap;
    /// called every frame so the toolbar tracks pans, but only forwarded to the
    /// chrome on change.
    fn publish_anchor(&mut self) {
        let next = self
            .state
            .cell_pixel_rect(self.state.active_row, self.state.active_col)
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
        // Announce the committed value to assistive tech.
        self.announce_active();
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

        // Advance any touchpad momentum coast before the viewport flush, so the
        // ScrollViewport it produces ships this same frame.
        self.step_momentum();

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
            self.theme.palette(),
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
            self.theme.palette(),
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

    fn resize(&mut self, width: u32, height: u32) {
        self.state.resize(width as f64, height as f64);
        self.needs_full = true;
    }

    fn handle_input(&mut self, ev: &InputEvent) -> bool {
        // Wheel / two-finger touchpad scroll pans the viewport in any mode and
        // never disturbs the cursor or an open editor.
        if let InputEvent::Wheel { dx, dy, delta_mode, ctrl_key } = ev {
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
            // A content click focuses the pipe and selects the clicked cell, so
            // typing or IME can start there (spec: canvas click -> textarea.focus).
            InputEvent::PointerDown { x, y, button: 0 } => {
                self.text_input.focus();
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
                    self.select_cell(row, col);
                }
                true
            }
            // A right-click selects the cell under the cursor before the DOM
            // context menu opens on it (standard spreadsheet behaviour). We don't
            // preventDefault here -- the `contextmenu` listener owns the menu.
            InputEvent::PointerDown { x, y, button: 2 } => {
                self.text_input.focus();
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
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

    /// Move the active cell by `(dr, dc)`; returns `true` (consumed).
    fn nav_move(&mut self, dr: i32, dc: i32) -> bool {
        self.viewport_changed = true;
        if let Some(mv) = self.state.move_cursor(dr, dc) {
            self.send_command(mv.active);
            if let Some(scroll) = mv.scroll {
                self.pending_viewport = Some(scroll);
            }
            self.park_input_on(self.state.active_row, self.state.active_col);
            self.announce_active();
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
        self.announce_active();
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
            // A right-click commits and selects the cell under the cursor, so the
            // DOM context menu opens on it. We don't preventDefault (the
            // `contextmenu` listener owns the menu).
            InputEvent::PointerDown { x, y, button: 2 } => {
                self.commit_edit();
                self.text_input.focus();
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
                    self.select_cell(row, col);
                }
                false
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
