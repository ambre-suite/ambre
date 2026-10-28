// ambre-trame/src/ui/view/input.rs
// TrameView: keyboard/pointer navigation, wheel/momentum scrolling, cursor moves, resize arming.

use super::*;
use ambre_canvas::PointerButton;
use ambre_commands::{FunctionKey, Key, KeyChar, Modifiers, SelectionMode};
use ambre_core::{ViewPoint, ViewPx};
use ambre_draw::kinetic::Release;

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
                at,
                button: PointerButton::Primary,
                ..
            } if self.fill_drag.is_none() && self.state.filter_button_at(*at).is_some() => {
                if let Some(at) = self.state.filter_button_at(*at) {
                    self.text_input.focus();
                    self.select_cell(at);
                    self.filter_open.publish();
                }
                true
            }
            InputEvent::PointerDown {
                at,
                button: PointerButton::Primary,
                ..
            } if self.fill_drag.is_none() && self.state.fill_handle_at(*at) => {
                self.text_input.focus();
                // A merged-cell source fills block by block (Excel behaviour), so
                // capture the merge-expanded source and its block granularity.
                let bounds = self.state.selection.bounds();
                let src = self.state.expand_to_merges(bounds);
                let block = self.state.fill_source_block(bounds);
                self.fill_drag = Some((src, block));
                self.set_cursor(GridCursor::Crosshair);
                true
            }
            // Dragging the handle grows the destination rectangle, locked to the
            // dominant axis (a fill handle never fills two directions at once).
            InputEvent::PointerMove { at } if self.fill_drag.is_some() => {
                self.drag_point = Some(*at);
                self.follow_drag();
                true
            }
            // Release: emit the fill, grow the selection over the filled range,
            // and clear the drag/preview.
            InputEvent::PointerUp { mods, .. } if self.fill_drag.is_some() => {
                self.drag_point = None;
                self.end_fill_drag(*mods);
                true
            }
            // A press on a header strip (`trame-headers`) acts by zone. The body does
            // a whole-column/row select (Shift extends, drag sweeps); the corner does
            // select-all; a border band arms a resize drag (`trame-resize`). Checked
            // before the grid-cell press since headers sit outside the cell area
            // anyway.
            InputEvent::PointerDown {
                at,
                button: PointerButton::Primary,
                mods,
            } if self.fill_drag.is_none() && self.state.header_hit(*at).is_some() => {
                let Some(hit) = self.state.header_hit(*at) else {
                    return false;
                };
                self.text_input.focus();
                match hit {
                    // Shift extends a band of the same axis.
                    HeaderHit::ColBody(c)
                        if mods.shift() && self.state.selection.kind == SelKind::Cols =>
                    {
                        self.extend_axis(AxisPos::Col(c))
                    }
                    HeaderHit::RowBody(r)
                        if mods.shift() && self.state.selection.kind == SelKind::Rows =>
                    {
                        self.extend_axis(AxisPos::Row(r))
                    }
                    HeaderHit::ColBody(c) => self.press_header(AxisPos::Col(c)),
                    HeaderHit::RowBody(r) => self.press_header(AxisPos::Row(r)),
                    HeaderHit::Corner => self.select_all(),
                    // Resize bands / inline-insert affordance: handled in the
                    // resize-drag wiring (`trame-resize`, `trame-headers`).
                    HeaderHit::ColBorder(c) => self.arm_col_resize(c, at.x),
                    HeaderHit::RowBorder(r) => self.arm_row_resize(r, at.y),
                    HeaderHit::ColInsert(c) => self.insert_col_at(c),
                    HeaderHit::RowInsert(r) => self.insert_row_at(r),
                }
                true
            }
            // A header drag sweeps the whole-axis band to the column/row under the
            // cursor (even below/right of the header strip).
            InputEvent::PointerMove { at } if self.axis_drag.is_some() => {
                match self.axis_drag {
                    Some(SelKind::Cols) => {
                        if let Some(c) = self.state.col_at_x(at.x) {
                            self.extend_axis(AxisPos::Col(c));
                        }
                    }
                    Some(SelKind::Rows) => {
                        if let Some(r) = self.state.row_at_y(at.y) {
                            self.extend_axis(AxisPos::Row(r));
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
            InputEvent::PointerMove { at } if self.reorder_drag.is_some() => {
                let Some(grabbed) = self.reorder_drag else {
                    return false;
                };
                let slot = match grabbed {
                    AxisPos::Col(_) => self.state.col_drop_slot(at.x).map(AxisPos::Col),
                    AxisPos::Row(_) => self.state.row_drop_slot(at.y).map(AxisPos::Row),
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
            InputEvent::PointerMove { at } if self.resize_drag.is_some() => {
                self.update_resize_preview(*at);
                true
            }
            InputEvent::PointerUp { .. } if self.resize_drag.is_some() => {
                self.end_resize_drag();
                true
            }
            // Double-click on a header boundary auto-fits the column/row to its
            // cached content (`trame-resize`).
            InputEvent::DoubleClick { at }
                if matches!(
                    self.state.header_hit(*at),
                    Some(HeaderHit::ColBorder(_) | HeaderHit::RowBorder(_))
                ) =>
            {
                match self.state.header_hit(*at) {
                    Some(HeaderHit::ColBorder(c)) => self.auto_fit_col(c),
                    Some(HeaderHit::RowBorder(r)) => self.auto_fit_row(r),
                    _ => {}
                }
                true
            }
            InputEvent::PointerDown {
                at,
                button: PointerButton::Primary,
                mods,
            } => {
                self.text_input.focus();
                if let Some(at) = self.state.cell_at(*at) {
                    if mods.shift() {
                        self.extend_cell(at);
                    } else if mods.primary() {
                        self.disjoin_cell(at);
                    } else {
                        self.select_cell(at);
                        // Arm drag-to-select: pointer moves now grow the range.
                        self.selecting_drag = true;
                    }
                }
                true
            }
            // Drag with the button held grows the live selection rectangle to the
            // cell under the cursor (mouse range selection).
            InputEvent::PointerMove { at } if self.selecting_drag => {
                self.drag_point = Some(*at);
                self.follow_drag();
                true
            }
            InputEvent::PointerUp { .. } if self.selecting_drag => {
                self.selecting_drag = false;
                self.drag_point = None;
                true
            }
            // Bare hover (no button held) advertises the affordance under the cursor
            // (`trame-resize`, `trame-autofill`). A header boundary shows the
            // col/row-resize cursor, a header body the select pointer, the fill handle
            // the crosshair, everything else the default. Pure feedback.
            InputEvent::PointerMove { at } => {
                let hit = self.state.header_hit(*at);
                let cursor = match hit {
                    Some(HeaderHit::ColBorder(_)) => GridCursor::ColResize,
                    Some(HeaderHit::RowBorder(_)) => GridCursor::RowResize,
                    Some(_) => GridCursor::Pointer,
                    None if self.state.filter_button_at(*at).is_some() => GridCursor::Pointer,
                    None if self.state.fill_handle_at(*at) => GridCursor::Crosshair,
                    None => GridCursor::Unset,
                };
                if cursor != self.hover_cursor {
                    self.hover_cursor = cursor;
                    self.set_cursor(cursor);
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
                at,
                button: PointerButton::Secondary,
                ..
            } if self.state.header_hit(*at).is_some() => {
                let Some(hit) = self.state.header_hit(*at) else {
                    return false;
                };
                self.text_input.focus();
                use crate::ui::grid::CtxScope;
                let scope = match hit {
                    HeaderHit::ColBody(c) => {
                        self.select_axis(AxisPos::Col(c));
                        CtxScope::Column
                    }
                    HeaderHit::RowBody(r) => {
                        self.select_axis(AxisPos::Row(r));
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
                at,
                button: PointerButton::Secondary,
                ..
            } => {
                self.text_input.focus();
                // Right-click inside the current multi-cell selection keeps it, so
                // a context menu can act on the range. Otherwise it collapses to
                // the clicked cell, like a left click.
                if let Some(at) = self.state.cell_at(*at)
                    && !self.state.selection.contains(at)
                {
                    self.select_cell(at);
                }
                self.set_ctx_scope(crate::ui::grid::CtxScope::Cell);
                false
            }
            // Double-click opens an editor on the clicked cell's current text.
            InputEvent::DoubleClick { at } => {
                self.text_input.focus();
                if let Some(at) = self.state.cell_at(*at) {
                    self.select_cell(at);
                    let seed = self.cell_seed(at);
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
            InputEvent::Key { key, mods } => {
                // `primary` is Ctrl, or Cmd on macOS: the jump modifier.
                // Shift extends the selection range across navigation keys
                // (Shift+arrows, Shift+Page, Shift+Home/End, Shift+Ctrl+End).
                let ext = if mods.shift() {
                    SelectionMode::Extend
                } else {
                    SelectionMode::Move
                };
                match *key {
                    Key::ArrowUp => self.nav_move(RowDelta::new(-1), ColDelta::ZERO, ext),
                    Key::ArrowDown => self.nav_move(RowDelta::new(1), ColDelta::ZERO, ext),
                    Key::ArrowLeft => self.nav_move(RowDelta::ZERO, ColDelta::new(-1), ext),
                    Key::ArrowRight => self.nav_move(RowDelta::ZERO, ColDelta::new(1), ext),
                    // Tab / Shift+Tab walk columns; Enter / Shift+Enter walk rows
                    // (convention, `trame-selection`). These always collapse the
                    // selection: Shift is the reverse-direction modifier here, not
                    // a range extender. preventDefault keeps the browser from
                    // moving focus off the canvas.
                    Key::Tab if !mods.primary() => {
                        let step = ColDelta::new(if mods.shift() { -1 } else { 1 });
                        self.nav_move(RowDelta::ZERO, step, SelectionMode::Move)
                    }
                    Key::Enter if !mods.primary() => {
                        let step = RowDelta::new(if mods.shift() { -1 } else { 1 });
                        self.nav_move(step, ColDelta::ZERO, SelectionMode::Move)
                    }
                    // Page Down / Up move by a full viewport height.
                    Key::PageDown | Key::PageUp => {
                        let page =
                            RowDelta::from_count(self.state.viewport_rows.max(RowCount::ONE));
                        let step = if *key == Key::PageUp {
                            page.reversed()
                        } else {
                            page
                        };
                        self.nav_move(step, ColDelta::ZERO, ext)
                    }
                    // Home: column 0 of the row; Ctrl+Home: top-left of the grid.
                    Key::Home if mods.primary() => self.nav_goto(CellPos::ORIGIN, ext),
                    Key::Home => {
                        self.nav_goto(CellPos::new(self.state.active_row(), Col::ZERO), ext)
                    }
                    // End: last cell with content in the row; Ctrl+End: bottom-right
                    // of the used (cached) range. With Shift the range grows to that
                    // target (Shift+Ctrl+End = select to the last non-empty cell).
                    // Best-effort -- the UI thread has no used-range model, so it
                    // scans its local cache (`trame-grid-viewport`).
                    Key::End if mods.primary() => {
                        let at = self
                            .state
                            .last_content_cell()
                            .unwrap_or(self.state.active_cell());
                        self.nav_goto(at, ext)
                    }
                    Key::End => {
                        let row = self.state.active_row();
                        let col = self
                            .state
                            .last_content_col(row)
                            .unwrap_or(self.state.active_col());
                        self.nav_goto(CellPos::new(row, col), ext)
                    }
                    // Escape clears the marching-ants clipboard marquee (`trame-clipboard`),
                    // if any; otherwise it is not ours.
                    Key::Escape if self.state.clipboard_anchor.take().is_some() => {
                        self.cut_pending = false;
                        self.edit_dirty = true;
                        true
                    }
                    // Shift+Delete is Cut: don't consume it, so the browser fires
                    // the native `cut` clipboard event the host services (`trame-clipboard`).
                    Key::Delete if mods.shift() => false,
                    // Delete / Backspace clear the active cell.
                    Key::Delete | Key::Backspace => self.clear_active_cell(),
                    // F2 opens an editor on the cell's current text.
                    Key::F(FunctionKey::F2) => {
                        let seed = self.cell_seed(self.state.active_cell());
                        self.begin_edit(seed);
                        true
                    }
                    // Debug HUD text toggle, moved behind Alt so plain letters type.
                    Key::Char(KeyChar::T) if mods.alt() => {
                        self.text_pass = match self.text_pass {
                            TextPass::Shapes => TextPass::WithText,
                            TextPass::WithText => TextPass::Shapes,
                        };
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
    /// and page deltas are read as pixels first (`KINETIC-015`).
    /// Returns `true` when consumed (so the page itself doesn't scroll).
    pub(crate) fn handle_wheel(
        &mut self,
        delta: ambre_canvas::WheelDelta,
        mods: Modifiers,
    ) -> bool {
        // Leave Ctrl+wheel to the browser (pinch-zoom gesture); not ours yet.
        if mods.primary() {
            return false;
        }
        let outcome = self.scroll.wheel(&mut self.state, delta, self.clock.now());
        self.note_scroll(outcome);
        true
    }

    /// Queue the viewport request a pan owes. Returns the outcome unchanged.
    fn note_scroll(&mut self, outcome: ScrollOutcome) -> ScrollOutcome {
        if matches!(outcome, ScrollOutcome::Moved) {
            self.viewport_changed = true;
            self.pending_viewport
                .coalesce(self.state.scroll_viewport_cmd());
        }
        outcome
    }

    /// Advance the kinetic motion by one frame, from `update`.
    pub(crate) fn step_momentum(&mut self) {
        let outcome = self.scroll.tick(&mut self.state, self.clock.now());
        self.note_scroll(outcome);
    }

    /// Grow the held selection or fill preview to the cell the drag points
    /// at, the nearest edge cell once it leaves the cell area.
    fn follow_drag(&mut self) {
        let Some(point) = self.drag_point else {
            return;
        };
        let Some(at) = self.state.drag_cell_at(point) else {
            return;
        };
        if let Some((src, block)) = self.fill_drag {
            self.state.fill_preview = Some(fill_dest(src, at, block));
            self.edit_dirty = true; // repaint the preview outline.
        } else if self.selecting_drag {
            self.extend_cell(at);
        }
    }

    /// Scroll a selection or fill drag held at an edge, once a frame
    /// (`KINETIC-007`). The first tick only starts the clock.
    pub(crate) fn step_edge_scroll(&mut self) {
        let held = self.selecting_drag || self.fill_drag.is_some();
        let speed = self
            .drag_point
            .filter(|_| held)
            .map(|at| self.state.edge_scroll_speed(at));
        let outcome = self.scroll.edge(&mut self.state, speed, self.clock.now());
        if matches!(self.note_scroll(outcome), ScrollOutcome::Moved) {
            self.follow_drag();
        }
    }

    /// A finger went down on the grid. It stops any motion left
    /// (`KINETIC-006`).
    pub fn touch_down(&mut self, at: ViewPoint, now: ambre_core::PageTime) {
        self.scroll.press(at, now);
    }

    /// A finger moved: past the tap slop the grid follows it, cell by cell.
    pub fn touch_move(&mut self, at: ViewPoint, now: ambre_core::PageTime) {
        let outcome = self.scroll.pan(&mut self.state, at, now);
        self.note_scroll(outcome);
    }

    /// A finger lifted. A pan released with speed keeps moving; a tap is a
    /// click where it landed.
    pub fn touch_up(&mut self, now: ambre_core::PageTime) {
        if let Release::Tap { at } = self.scroll.release(now) {
            for ev in [
                InputEvent::PointerDown {
                    at,
                    button: PointerButton::Primary,
                    mods: Modifiers::NONE,
                },
                InputEvent::PointerUp {
                    at,
                    button: PointerButton::Primary,
                    mods: Modifiers::NONE,
                },
            ] {
                self.handle_input(&ev);
            }
        }
    }

    /// The browser cancelled the finger: no click and no flick
    /// (`KINETIC-010`).
    pub fn touch_cancel(&mut self) {
        self.scroll.cancel();
    }

    /// Move the active cell by `dr` rows and `dc` columns; returns `true`
    /// (consumed). With `extend` (Shift held) the selection range grows
    /// instead of collapsing.
    pub(crate) fn nav_move(&mut self, dr: RowDelta, dc: ColDelta, extend: SelectionMode) -> bool {
        let mv = self.state.move_cursor(dr, dc, extend);
        self.apply_cursor_move(mv)
    }

    /// Jump the active cell to absolute `at` (Home / End / Ctrl+Home /
    /// Ctrl+End); returns `true` (consumed). With `extend` the range grows to
    /// the target (Shift+End, Shift+Ctrl+End...).
    pub(crate) fn nav_goto(&mut self, at: CellPos, extend: SelectionMode) -> bool {
        let mv = self.state.move_to(at, extend);
        self.apply_cursor_move(mv)
    }

    /// Build the active-cell report from the current selection, carrying the
    /// whole selection (`trame-selection`): the live rectangle's corners, the
    /// committed disjoint rectangles and the band kind. The Logic thread needs
    /// all of it to resolve the parameterless intents against every rectangle.
    pub(crate) fn set_active_cmd(&self) -> TrameCommand {
        self.state.selection.report()
    }

    /// Extend the live selection rectangle to `at` (Shift+click / drag),
    /// keeping the anchor. Moves the active cell, so Logic and the pipe stay in
    /// sync via the same path as a plain selection.
    pub(crate) fn extend_cell(&mut self, at: CellPos) {
        if at == self.state.active_cell() {
            return;
        }
        self.state.select_extend(at);
        self.viewport_changed = true;
        if let Err(SendError::Refused) = self.send_command_lossless(self.set_active_cmd()) {
            tracing::error!("Trame: the outbox is full, the selection extend was dropped");
        }
        self.park_input_on(at);
        self.announce_active();
    }

    /// Add `at` as a fresh disjoint range (Ctrl+click), committing the
    /// previous live rectangle. The new cell becomes active.
    pub(crate) fn disjoin_cell(&mut self, at: CellPos) {
        self.state.select_disjoint(at);
        self.viewport_changed = true;
        if let Err(SendError::Refused) = self.send_command_lossless(self.set_active_cmd()) {
            tracing::error!("Trame: the outbox is full, the disjoint select was dropped");
        }
        self.park_input_on(at);
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
                self.pending_viewport.coalesce(scroll);
            }
            self.park_input_on(self.state.active_cell());
            self.announce_active();
        }
        true
    }

    /// Clear every selected cell's content (Delete / Backspace in navigation
    /// mode). Forwards one `SetCellsBatch` of empty texts, then empties the
    /// local cache. A refused send leaves the cache as it was (`BUS-018`).
    /// Returns `true` (consumed).
    pub(crate) fn clear_active_cell(&mut self) -> bool {
        let cells = self.state.selection.selected_cells();
        // One batch command so the whole clear is a single undo item.
        let batch = cells.iter().map(|&at| (at, String::new())).collect();
        if self
            .send_command_lossless(TrameCommand::SetCellsBatch { cells: batch })
            .is_err()
        {
            tracing::error!("Trame: the cell clear was dropped, the cells keep their text");
            return true;
        }
        for &at in &cells {
            self.state.set_cell_text(at, String::new());
        }
        self.needs_full = true;
        self.announce_active();
        true
    }

    /// Set `cursor` on `#ambre-canvas` (`trame-resize`, `trame-autofill`);
    /// [`GridCursor::Unset`] resets it. No-op off-wasm.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn set_cursor(&self, cursor: GridCursor) {
        use wasm_bindgen::JsCast;
        if let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("ambre-canvas"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = el.style().set_property("cursor", cursor.css());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn set_cursor(&self, _cursor: GridCursor) {}

    /// Publish the surface the latest right-click landed on so the launcher picks
    /// the cell vs header menu layout (`trame-context-menu`). Only re-writes
    /// the signal on a change (cheap). Off-wasm only the field is written.
    pub(crate) fn set_ctx_scope(&mut self, scope: crate::ui::grid::CtxScope) {
        self.menu_scope = scope;
        #[cfg(target_arch = "wasm32")]
        if self.ctx_scope.get_untracked() != scope {
            self.ctx_scope.set(scope);
        }
    }

    /// Arm a column resize from a press on column `c`'s right border. Shows the
    /// live vertical guide at the pointer x.
    pub(crate) fn arm_col_resize(&mut self, c: Col, x: ViewPx) {
        self.resize_drag = Some(HeaderHit::ColBorder(c));
        self.state.resize_preview = Some(crate::ui::grid::ResizePreview::Col(
            crate::ui::grid::on_grid(x),
        ));
        self.set_cursor(GridCursor::ColResize);
        self.edit_dirty = true;
    }

    /// Arm a row resize from a press on row `r`'s bottom border.
    pub(crate) fn arm_row_resize(&mut self, r: Row, y: ViewPx) {
        self.resize_drag = Some(HeaderHit::RowBorder(r));
        self.state.resize_preview = Some(crate::ui::grid::ResizePreview::Row(
            crate::ui::grid::on_grid(y),
        ));
        self.set_cursor(GridCursor::RowResize);
        self.edit_dirty = true;
    }

    /// Move the live resize guide to the pointer, clamped so the dragged
    /// column/row never shrinks below its minimum.
    pub(crate) fn update_resize_preview(&mut self, at: ViewPoint) {
        let (x, y) = (
            crate::ui::grid::on_grid(at.x),
            crate::ui::grid::on_grid(at.y),
        );
        match self.resize_drag {
            Some(HeaderHit::ColBorder(c)) => {
                // The guide is in logical px, so the minimum width is the base
                // clamp scaled by the current zoom.
                let min_x = self.state.col_x(c) + self.state.scaled(crate::ui::grid::MIN_COL_WIDTH);
                self.state.resize_preview = Some(crate::ui::grid::ResizePreview::Col(x.max(min_x)));
            }
            Some(HeaderHit::RowBorder(r)) => {
                let min_y =
                    self.state.row_y(r) + self.state.scaled(crate::ui::grid::MIN_ROW_HEIGHT);
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
        self.set_cursor(GridCursor::Unset);
        self.edit_dirty = true;
        let (Some(drag), Some(preview)) = (drag, preview) else {
            return;
        };
        match (drag, preview) {
            (HeaderHit::ColBorder(c), crate::ui::grid::ResizePreview::Col(px)) => {
                // The guide is in logical (zoomed) px; the stored width is BASE
                // px (zoom-independent content). Divide the on-screen extent
                // back out by the zoom before persisting.
                let width = self
                    .state
                    .unscaled(px - self.state.col_x(c))
                    .max(crate::ui::grid::MIN_COL_WIDTH);
                for col in self.resize_target_cols(c) {
                    self.send_resize_col(col, width);
                }
            }
            (HeaderHit::RowBorder(r), crate::ui::grid::ResizePreview::Row(py)) => {
                let height = self
                    .state
                    .unscaled(py - self.state.row_y(r))
                    .max(crate::ui::grid::MIN_ROW_HEIGHT);
                for row in self.resize_target_rows(r) {
                    self.send_resize_row(row, height);
                }
            }
            _ => {}
        }
    }
}
