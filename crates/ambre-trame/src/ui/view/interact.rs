// ambre-trame/src/ui/view/interact.rs
// TrameView: resize application, fill/reorder drags, axis selection, clipboard and paste.

use super::*;

impl TrameView {
    /// Columns a column resize applies to: the whole-column selection's band when
    /// the dragged column is part of it, else just the dragged column.
    pub(crate) fn resize_target_cols(&self, c: u32) -> Vec<u32> {
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
    pub(crate) fn resize_target_rows(&self, r: u32) -> Vec<u32> {
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
    pub(crate) fn send_resize_col(&mut self, col: u32, width: f32) {
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
    pub(crate) fn send_resize_row(&mut self, row: u32, height: f32) {
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
    pub(crate) fn auto_fit_col(&mut self, c: u32) {
        self.pending_autofit = Some(HeaderHit::ColBorder(c));
        self.edit_dirty = true;
    }

    pub(crate) fn auto_fit_row(&mut self, r: u32) {
        self.pending_autofit = Some(HeaderHit::RowBorder(r));
        self.edit_dirty = true;
    }

    /// Measure a column/row's cached cells and emit a resize sized to its content
    /// (sec.4bis.C). UI-side over the viewport cache: the parley layouts of every
    /// cached cell of the axis are measured and the max natural extent (plus
    /// padding, clamped to the minimum) becomes the new width/height. A column
    /// with no cached content snaps to the default.
    pub(crate) fn resolve_autofit(
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
    pub(crate) fn insert_col_at(&mut self, c: u32) {
        self.select_cell(self.state.active_row(), c);
        self.send_command_lossy(TrameCommand::InsertColLeft);
        // Select the freshly created column (it lands at `c`) so the insertion
        // reads as a clear whole-column band rather than a silent shift.
        self.select_axis(crate::ui::grid::SelKind::Cols, c);
    }

    pub(crate) fn insert_row_at(&mut self, r: u32) {
        self.select_cell(r, self.state.active_col());
        self.send_command_lossy(TrameCommand::InsertRowAbove);
        // Select the freshly created row (it lands at `r`) for the same feedback.
        self.select_axis(crate::ui::grid::SelKind::Rows, r);
    }

    /// Finish an autofill drag (sec.5.F): emit `FillRange` over the source +
    /// dragged extent, grow the selection to cover the filled range, then clear
    /// the drag/preview. A drag that never left the source (no extension) is a
    /// no-op beyond clearing the state.
    pub(crate) fn end_fill_drag(&mut self, ctrl: bool) {
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
    pub(crate) fn select_cell(&mut self, row: u32, col: u32) {
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
    pub(crate) fn axis_active_cmd(&self) -> TrameCommand {
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

    /// Whether the current whole-axis selection is of `kind` and its band covers
    /// column/row `idx`. Gates the reorder-drag grab: a press on an already
    /// selected header starts a move, any other press selects.
    pub(crate) fn axis_sel_contains(&self, kind: crate::ui::grid::SelKind, idx: u32) -> bool {
        let sel = &self.state.selection;
        if sel.kind != kind {
            return false;
        }
        let (a, b) = match kind {
            crate::ui::grid::SelKind::Cols => (sel.anchor.1, sel.active.1),
            crate::ui::grid::SelKind::Rows => (sel.anchor.0, sel.active.0),
            crate::ui::grid::SelKind::Cells => return false,
        };
        (a.min(b)..=a.max(b)).contains(&idx)
    }

    /// Finish a header reorder drag (Excel-style): drop the grabbed column/row at
    /// the previewed insertion slot, emitting `MoveColumn` / `MoveRow` unless it
    /// lands back on its origin, then reselect the moved band and clear the drag.
    /// The slot is an "insert before" index; `LoroMovableList::mov` takes the
    /// destination index *after* the source is removed, so a slot past the origin
    /// shifts down by one.
    pub(crate) fn end_reorder_drag(&mut self) {
        let Some((kind, from)) = self.reorder_drag.take() else {
            return;
        };
        let slot = self.state.reorder_preview.take().map(|(_, s)| s);
        self.edit_dirty = true;
        let Some(slot) = slot else { return };
        // Dropping on the origin's own slot or the one just past it is a no-op.
        if slot == from || slot == from + 1 {
            return;
        }
        let to = if slot > from { slot - 1 } else { slot };
        let cmd = match kind {
            crate::ui::grid::SelKind::Cols => TrameCommand::MoveColumn { from, to },
            crate::ui::grid::SelKind::Rows => TrameCommand::MoveRow { from, to },
            crate::ui::grid::SelKind::Cells => return,
        };
        self.send_command_lossy(cmd);
        // Keep the moved column/row selected at its new position.
        self.select_axis(kind, to);
    }

    /// Select a whole column/row band from a header click (sec.4bis.A) and report
    /// it to Logic as a used-range rectangle.
    pub(crate) fn select_axis(&mut self, kind: crate::ui::grid::SelKind, idx: u32) {
        self.state.selection = Selection::axis(kind, idx, idx);
        self.viewport_changed = true;
        if let Err(SendError::Full) = self.send_command_lossless(self.axis_active_cmd()) {
            tracing::error!("Trame: UI->Logic channel full, axis select dropped");
        }
        self.announce_active();
    }

    /// Extend the active whole-axis band to column/row `idx` (Shift+click or a
    /// header drag), keeping the anchor.
    pub(crate) fn extend_axis(&mut self, idx: u32) {
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
    pub(crate) fn select_all(&mut self) {
        self.state.selection = Selection::axis(crate::ui::grid::SelKind::Cols, 0, TOTAL_COLS - 1);
        self.viewport_changed = true;
        if let Err(SendError::Full) = self.send_command_lossless(self.axis_active_cmd()) {
            tracing::error!("Trame: UI->Logic channel full, select-all dropped");
        }
        self.announce_active();
    }

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
    pub(crate) fn finish_paste(&mut self) {
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
    pub(crate) fn pointer_in_editor(&self, x: f64, y: f64) -> bool {
        let Some(ed) = self.editor.as_ref() else {
            return false;
        };
        self.state.cell_at(x, y) == Some((ed.row, ed.col))
    }

    /// Input routing while editing a cell.
    pub(crate) fn handle_input_editing(&mut self, ev: &InputEvent) -> bool {
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
