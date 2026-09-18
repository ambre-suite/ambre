// ambre-trame/src/ui/view/input.rs
// TrameView: keyboard/pointer navigation, wheel/momentum scrolling, cursor moves, resize arming.

use super::*;

impl TrameView {
    /// Input routing while NOT editing a cell.
    pub(crate) fn handle_input_nav(&mut self, ev: &InputEvent) -> bool {
        match ev {
            // A content click focuses the pipe and (re)selects the clicked cell,
            // so typing or IME can start there (spec: canvas click ->
            // textarea.focus). Shift extends the range to the clicked cell;
            // Ctrl adds it as a disjoint range; a plain click collapses to it and
            // arms a drag-to-select. A press on the fill handle (bottom-right of
            // the selection) arms an autofill drag instead of a new selection
            // (`trame-autofill`). Checked before the plain selection press so the
            // ~7px handle wins the hit.
            //
            // A click on an in-canvas autofilter button (`trame-shared-filter`)
            // selects the button's column on the region's header row. It then
            // raises the one-shot intent so the launcher opens the FilterPopover
            // for that column. That is the same path as the context-menu entry,
            // so there is one filter code path. Tested before the fill handle /
            // cells so the small button wins.
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
            // A press on a header strip (`trame-headers`) acts by zone. The body does
            // a whole-column/row select (Shift extends, drag sweeps); the corner does
            // select-all; a border band arms a resize drag (`trame-resize`). Checked
            // before the grid-cell press since headers sit outside the cell area
            // anyway.
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
                        } else if self.axis_sel_contains(SelKind::Cols, c) {
                            // Press on an already-selected column: grab it for an
                            // Excel-style drag reorder rather than re-selecting.
                            self.reorder_drag = Some((SelKind::Cols, c));
                            self.state.reorder_preview = Some((SelKind::Cols, c));
                        } else {
                            self.select_axis(SelKind::Cols, c);
                            self.axis_drag = Some(SelKind::Cols);
                        }
                    }
                    HeaderHit::RowBody(r) => {
                        if *shift && self.state.selection.kind == SelKind::Rows {
                            self.extend_axis(r);
                        } else if self.axis_sel_contains(SelKind::Rows, r) {
                            self.reorder_drag = Some((SelKind::Rows, r));
                            self.state.reorder_preview = Some((SelKind::Rows, r));
                        } else {
                            self.select_axis(SelKind::Rows, r);
                            self.axis_drag = Some(SelKind::Rows);
                        }
                    }
                    HeaderHit::Corner => self.select_all(),
                    // Resize bands / inline-insert affordance: handled in the
                    // resize-drag wiring (`trame-resize`, `trame-headers`).
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
            // Dragging a grabbed header moves the live insertion guide to the
            // boundary under the cursor (Excel-style reorder preview).
            InputEvent::PointerMove { x, y } if self.reorder_drag.is_some() => {
                let (kind, _) = self.reorder_drag.expect("guarded");
                let slot = match kind {
                    SelKind::Cols => self.state.col_drop_slot(*x).map(|s| (SelKind::Cols, s)),
                    SelKind::Rows => self.state.row_drop_slot(*y).map(|s| (SelKind::Rows, s)),
                    SelKind::Cells => None,
                };
                if let Some(slot) = slot
                    && self.state.reorder_preview != Some(slot)
                {
                    self.state.reorder_preview = Some(slot);
                    self.edit_dirty = true;
                }
                true
            }
            // Release drops the grabbed column/row at the previewed slot, emitting
            // a `MoveColumn` / `MoveRow` unless it lands back on its origin.
            InputEvent::PointerUp { .. } if self.reorder_drag.is_some() => {
                self.end_reorder_drag();
                true
            }
            // Dragging a header boundary moves the live guide line (`trame-resize`);
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
            // cached content (`trame-resize`).
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
            // Bare hover (no button held) advertises the affordance under the cursor
            // (`trame-resize`, `trame-autofill`). A header boundary shows the
            // col/row-resize cursor, a header body the select pointer, the fill handle
            // the crosshair, everything else the default. Pure feedback.
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
            // the corner) before the DOM context menu opens. It also flags the
            // menu as header-scoped (`trame-context-menu`). A border/insert band
            // keeps the existing selection -- the header menu still acts on it. We
            // don't preventDefault here; the `contextmenu` listener owns the menu.
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
                // Right-click inside the current multi-cell selection keeps it, so
                // a context menu can act on the range. Otherwise it collapses to
                // the clicked cell, like a left click.
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
                    ed.set_compose(text);
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
                    // (convention, `trame-selection`). These always collapse the
                    // selection: Shift is the reverse-direction modifier here, not
                    // a range extender. preventDefault keeps the browser from
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
                    // scans its local cache (`trame-grid-viewport`).
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
                    // Escape clears the marching-ants clipboard marquee (`trame-clipboard`),
                    // if any; otherwise it is not ours.
                    "Escape" if self.state.clipboard_anchor.take().is_some() => {
                        self.cut_pending = false;
                        self.edit_dirty = true;
                        true
                    }
                    // Shift+Delete is Cut: don't consume it, so the browser fires
                    // the native `cut` clipboard event the host services (`trame-clipboard`).
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
    pub(crate) fn handle_wheel(
        &mut self,
        dx: f64,
        dy: f64,
        delta_mode: u32,
        ctrl_key: bool,
    ) -> bool {
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
        // velocity. Clamp dt so a near-zero interval can't blow it up.
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
    pub(crate) fn apply_scroll_accum(&mut self) -> ScrollOutcome {
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
    /// then integrates the decaying velocity into the scroll accumulator.
    /// Stops once the speed dies down or the viewport hits an edge.
    pub(crate) fn step_momentum(&mut self) {
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
        // that same dt. The coast is then identical at 60 Hz or 240 Hz; it
        // just runs in finer steps at higher refresh rates.
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
    pub(crate) fn stop_fling(&mut self) {
        self.coasting = false;
        self.fling_vx = 0.0;
        self.fling_vy = 0.0;
        self.scroll_accum_x = 0.0;
        self.scroll_accum_y = 0.0;
    }

    /// Move the active cell by `(dr, dc)`; returns `true` (consumed). With
    /// `extend` (Shift held) the selection range grows instead of collapsing.
    pub(crate) fn nav_move(&mut self, dr: i32, dc: i32, extend: bool) -> bool {
        let mv = self.state.move_cursor(dr, dc, extend);
        self.apply_cursor_move(mv)
    }

    /// Jump the active cell to absolute `(row, col)` (Home / End / Ctrl+Home /
    /// Ctrl+End); returns `true` (consumed). With `extend` the range grows to
    /// the target (Shift+End, Shift+Ctrl+End...).
    pub(crate) fn nav_goto(&mut self, row: u32, col: u32, extend: bool) -> bool {
        let mv = self.state.move_to(row, col, extend);
        self.apply_cursor_move(mv)
    }

    /// Build the active-cell report from the current selection, carrying the
    /// anchor corner. The Logic thread needs that corner to track the whole
    /// selection rectangle, as the structural intents do (merge / delete
    /// rows-cols).
    pub(crate) fn set_active_cmd(&self) -> TrameCommand {
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
    pub(crate) fn extend_cell(&mut self, row: u32, col: u32) {
        if (row, col) == (self.state.active_row(), self.state.active_col()) {
            return;
        }
        self.state.select_extend(row, col);
        self.viewport_changed = true;
        if let Err(SendError::Refused) = self.send_command_lossless(self.set_active_cmd()) {
            tracing::error!("Trame: the outbox is full, the selection extend was dropped");
        }
        self.park_input_on(row, col);
        self.announce_active();
    }

    /// Add `(row, col)` as a fresh disjoint range (Ctrl+click), committing the
    /// previous live rectangle. The new cell becomes active.
    pub(crate) fn disjoin_cell(&mut self, row: u32, col: u32) {
        self.state.select_disjoint(row, col);
        self.viewport_changed = true;
        if let Err(SendError::Refused) = self.send_command_lossless(self.set_active_cmd()) {
            tracing::error!("Trame: the outbox is full, the disjoint select was dropped");
        }
        self.park_input_on(row, col);
        self.announce_active();
    }

    /// Forward a cursor move to Logic and keep the text pipe / a11y in sync.
    /// Shared by relative ([`nav_move`](Self::nav_move)) and absolute
    /// ([`nav_goto`](Self::nav_goto)) navigation. A `None` move (clamped at an
    /// edge / no change) is a no-op but still reports the key as consumed.
    pub(crate) fn apply_cursor_move(&mut self, mv: Option<CursorMove>) -> bool {
        self.viewport_changed = true;
        if let Some(mv) = mv {
            if let Err(SendError::Refused) = self.send_command_lossless(mv.active) {
                tracing::error!("Trame: the outbox is full, the cursor move was dropped");
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
    /// mode). Optimistically empties the local cache, then forwards an empty
    /// `SetCellText` per cell -- the same path a committed empty edit takes. A
    /// trailing `CommitEdit` seals them as one undo group. Returns `true`
    /// (consumed).
    pub(crate) fn clear_active_cell(&mut self) -> bool {
        let cells = self.state.selection.selected_cells();
        // One batch command so the whole clear is a single undo item.
        let batch = cells.iter().map(|&(r, c)| (r, c, String::new())).collect();
        if let Err(SendError::Refused) =
            self.send_command_lossless(TrameCommand::SetCellsBatch { cells: batch })
        {
            tracing::error!("Trame: the outbox is full, the cell clear was dropped");
        }
        for &(row, col) in &cells {
            self.state.set_cell_text(row, col, String::new());
        }
        self.needs_full = true;
        self.announce_active();
        true
    }

    /// Set the canvas cursor to the autofill crosshair (`on == true`) or back to
    /// the default (`trame-autofill`): the thin "+" Excel/Sheets show over the
    /// fill handle. A DOM write on the `#ambre-canvas` element; a no-op off-wasm.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn set_fill_cursor(&self, on: bool) {
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
    pub(crate) fn set_fill_cursor(&self, _on: bool) {}

    /// Set an arbitrary CSS cursor on `#ambre-canvas` (`trame-resize`):
    /// `"col-resize"` / `"row-resize"` over a header boundary, `"pointer"` over a
    /// header body, `""` to reset. Generalises [`set_fill_cursor`]. No-op off-wasm.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn set_cursor(&self, css: &str) {
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
    pub(crate) fn set_cursor(&self, _css: &str) {}

    /// Publish the surface the latest right-click landed on so the launcher picks
    /// the cell vs header menu layout (`trame-context-menu`). Only re-writes
    /// the signal on a change (cheap). No-op off-wasm (headless tests).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn set_ctx_scope(&self, scope: crate::ui::grid::CtxScope) {
        if self.ctx_scope.get_untracked() != scope {
            self.ctx_scope.set(scope);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn set_ctx_scope(&self, _scope: crate::ui::grid::CtxScope) {}

    /// Arm a column resize from a press on column `c`'s right border. Shows the
    /// live vertical guide at the pointer x.
    pub(crate) fn arm_col_resize(&mut self, c: u32, x: f64) {
        self.resize_drag = Some(HeaderHit::ColBorder(c));
        self.state.resize_preview = Some(crate::ui::grid::ResizePreview::Col(x));
        self.set_cursor("col-resize");
        self.edit_dirty = true;
    }

    /// Arm a row resize from a press on row `r`'s bottom border.
    pub(crate) fn arm_row_resize(&mut self, r: u32, y: f64) {
        self.resize_drag = Some(HeaderHit::RowBorder(r));
        self.state.resize_preview = Some(crate::ui::grid::ResizePreview::Row(y));
        self.set_cursor("row-resize");
        self.edit_dirty = true;
    }

    /// Move the live resize guide to the pointer, clamped so the dragged
    /// column/row never shrinks below its minimum.
    pub(crate) fn update_resize_preview(&mut self, x: f64, y: f64) {
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
    /// selection, the size is applied to every band in the selection (`trame-resize`).
    pub(crate) fn end_resize_drag(&mut self) {
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
                // px (zoom-independent content). Divide the on-screen extent
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
}
