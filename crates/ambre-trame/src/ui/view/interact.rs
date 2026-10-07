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

    /// Emit a `ResizeCol`, then update the local metric cache so the grid
    /// reflows before the echoing `DataReset` confirms it (`TRAME-UI-041`).
    ///
    /// A size is a document write, so it goes out lossless: a full queue
    /// holds it (`BUS-011`). The local map takes the width only once the outbox
    /// carries the command: a width the document never took hides the loss
    /// (`TRAME-UI-127`).
    pub(crate) fn send_resize_col(&mut self, col: u32, width: f32) {
        let version = self.state.bump_version();
        let cmd = TrameCommand::ResizeCol {
            version,
            sheet_id: self.state.active_sheet.clone(),
            col,
            width_px: width,
        };
        if self.send_command_lossless(cmd).is_err() {
            tracing::error!("Trame: the resize was dropped, the column keeps its width");
            return;
        }
        self.state.col_widths.insert(col, width);
        self.viewport_changed = true;
        self.needs_full = true;
    }

    /// Emit a `ResizeRow` (row analogue of [`send_resize_col`]).
    pub(crate) fn send_resize_row(&mut self, row: u32, height: f32) {
        let version = self.state.bump_version();
        let cmd = TrameCommand::ResizeRow {
            version,
            sheet_id: self.state.active_sheet.clone(),
            row,
            height_px: height,
        };
        if self.send_command_lossless(cmd).is_err() {
            tracing::error!("Trame: the resize was dropped, the row keeps its height");
            return;
        }
        self.state.row_heights.insert(row, height);
        self.viewport_changed = true;
        self.needs_full = true;
    }

    /// Request a column auto-fit (double-click on the border, `trame-resize`).
    /// The viewport cache holds the loaded rows only. So the column's whole
    /// content is asked of the Logic thread, and the paint after the answer
    /// measures it (`TRAME-UI-128`). A column is never resized from a part of
    /// its content.
    pub(crate) fn auto_fit_col(&mut self, c: u32) {
        if self
            .send_command_lossless(TrameCommand::RequestColumnContent { col: c })
            .is_err()
        {
            tracing::error!("Trame: the auto-fit request was refused, the column keeps its width");
        }
    }

    pub(crate) fn auto_fit_row(&mut self, r: u32) {
        self.pending_autofit = Some(HeaderHit::RowBorder(r));
        self.edit_dirty = true;
    }

    /// Measure a row's cached cells and emit a resize sized to its content
    /// (`trame-resize`). UI-side over the viewport cache: the parley layouts of every
    /// cached cell of the row are measured. The max natural extent, plus padding and
    /// clamped to the minimum, becomes the new height.
    pub(crate) fn resolve_autofit(
        &mut self,
        hit: HeaderHit,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
    ) {
        use ambre_draw::Alignment;
        match hit {
            // A column auto-fit measures the whole column the Logic thread
            // returns (`auto_fit_col`), never this cache.
            HeaderHit::ColBorder(_) => {}
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

    /// Inline "+" insert at a column boundary (`trame-headers`): move the tracked
    /// active cell onto column `c`. Then fire the existing `InsertColLeft` intent
    /// so the Logic thread resolves the position exactly as from the menu.
    ///
    /// The intent is a document mutation, so it goes out lossless (`BUS-023`).
    /// A command the outbox took nowhere selects no band: a whole-column
    /// selection would read as an insertion that never happened (`BUS-018`).
    pub(crate) fn insert_col_at(&mut self, c: u32) {
        self.select_cell(self.state.active_row(), c);
        if self
            .send_command_lossless(TrameCommand::InsertColLeft)
            .is_err()
        {
            tracing::error!("Trame: the insert was dropped, no column was added");
            return;
        }
        // Select the freshly created column (it lands at `c`) so the insertion
        // reads as a clear whole-column band rather than a silent shift.
        self.select_axis(crate::ui::grid::SelKind::Cols, c);
    }

    pub(crate) fn insert_row_at(&mut self, r: u32) {
        self.select_cell(r, self.state.active_col());
        if self
            .send_command_lossless(TrameCommand::InsertRowAbove)
            .is_err()
        {
            tracing::error!("Trame: the insert was dropped, no row was added");
            return;
        }
        // Select the freshly created row (it lands at `r`) for the same feedback.
        self.select_axis(crate::ui::grid::SelKind::Rows, r);
    }

    /// Finish an autofill drag (`trame-autofill`): emit `FillRange` over the source +
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
        // Ctrl inverts the default Excel/Sheets behaviour. But the single- vs
        // multi-cell test is per fill line, that is, on the source extent *along the
        // drag axis*. A vertical drag fills column lines, so it cares about the
        // source row count; a horizontal drag cares about the source col count. A
        // single source value along that axis forces a +1 numeric Series; a
        // multi-value source forces a verbatim Copy. Without Ctrl, Auto lets the
        // Logic thread pick Series vs Copy.
        //
        // A merged source counts blocks, not raw cells, along the drag axis. One
        // block is a single value (Ctrl -> +1 Series), and several blocks are
        // several values (Ctrl -> Copy). `block` is the merge span; divide the
        // source extent by it to get the block count along the axis.
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
        if self.send_command_lossless(cmd).is_err() {
            // The fill never left, so the selection must not grow over cells the
            // document never wrote (`BUS-018`, `TRAME-UI-133`).
            tracing::error!("Trame: the fill was refused, the selection stays on the source");
            self.announce_active();
            return;
        }
        // Grow the selection to the whole filled rectangle (anchor top-left,
        // active bottom-right), matching Sheets after a fill-handle drag.
        self.state.selection.anchor = (dr1, dc1);
        self.state.selection.active = (dr2, dc2);
        self.state.selection.extra.clear();
        self.viewport_changed = true;
        if let Err(SendError::Refused) = self.send_command_lossless(self.set_active_cmd()) {
            tracing::error!("Trame: the outbox is full, the fill selection was dropped");
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
        if let Err(SendError::Refused) = self.send_command_lossless(self.set_active_cmd()) {
            tracing::error!("Trame: the outbox is full, the cell selection was dropped");
        }
        self.park_input_on(row, col);
        self.announce_active();
    }

    /// Build the active-cell report for a whole-axis selection (`trame-selection`).
    /// The band carries no extent across the other axis. The Logic thread resolves
    /// that extent from the used range (`TRAME-UI-033`).
    pub(crate) fn axis_active_cmd(&self) -> TrameCommand {
        self.set_active_cmd()
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
    /// the previewed insertion slot. It emits `MoveColumn` / `MoveRow` unless it
    /// lands back on its origin, then reselects the moved band and clears the
    /// drag. The slot is an "insert before" index. `LoroMovableList::mov` takes
    /// the destination index *after* the source is removed, so a slot past the
    /// origin shifts down by one.
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
        // A move is a document mutation, so it goes out lossless (`BUS-023`). A
        // command the outbox took nowhere reselects nothing: a band drawn at the
        // destination would read as a move that never happened (`BUS-018`).
        if self.send_command_lossless(cmd).is_err() {
            tracing::error!("Trame: the move was dropped, the line stays where it was");
            return;
        }
        // Keep the moved column/row selected at its new position.
        self.select_axis(kind, to);
    }

    /// Select a whole column/row band from a header click (`trame-selection`) and report
    /// it to Logic as a used-range rectangle.
    pub(crate) fn select_axis(&mut self, kind: crate::ui::grid::SelKind, idx: u32) {
        self.state.selection = Selection::axis(kind, idx, idx);
        self.viewport_changed = true;
        if let Err(SendError::Refused) = self.send_command_lossless(self.axis_active_cmd()) {
            tracing::error!("Trame: the outbox is full, the axis select was dropped");
        }
        self.announce_active();
    }

    /// Extend the active whole-axis band to column/row `idx` (Shift+click or a
    /// header drag), keeping the anchor. A band extension changes the selection,
    /// so it refreshes the aggregates like every other selection path
    /// (`TRAME-UI-109`).
    pub(crate) fn extend_axis(&mut self, idx: u32) {
        let kind = self.state.selection.kind;
        if kind == crate::ui::grid::SelKind::Cells {
            return;
        }
        self.state.selection.extend_axis(kind, idx);
        self.viewport_changed = true;
        if let Err(SendError::Refused) = self.send_command_lossless(self.axis_active_cmd()) {
            tracing::error!("Trame: the outbox is full, the axis extend was dropped");
        }
        self.announce_active();
    }

    /// Select the whole sheet from the header corner (`trame-selection`): a column band
    /// spanning every column.
    pub(crate) fn select_all(&mut self) {
        self.state.selection = Selection::axis(crate::ui::grid::SelKind::Cols, 0, TOTAL_COLS - 1);
        self.viewport_changed = true;
        if let Err(SendError::Refused) = self.send_command_lossless(self.axis_active_cmd()) {
            tracing::error!("Trame: the outbox is full, the select-all was dropped");
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

    /// Resolve a copy/cut synchronously inside the native clipboard event
    /// (`trame-clipboard`). An open in-cell text selection wins and returns its
    /// text directly ([`CopyOutcome::Text`]). Otherwise the grid copy is
    /// authoritative: the whole selection is built Logic-side, with no viewport
    /// limit. So we only emit [`TrameCommand::CopyRange`], arm the marching-ants
    /// anchor and the cut-pending flag synchronously, and return
    /// [`CopyOutcome::Deferred`]. The payload arrives one frame later via
    /// `TrameEvent::Clipboard` and is mirrored to the OS clipboard by
    /// [`poll_clipboard`](Self::poll_clipboard). [`CopyOutcome::Nothing`] means
    /// "let the browser default proceed".
    pub fn clipboard_write(&mut self, cut: bool) -> ambre_bus::CopyOutcome {
        use ambre_bus::CopyOutcome;
        // In-cell text selection takes precedence over the grid range.
        if self.editor.is_some() {
            let text = if cut {
                self.cut_selection()
            } else {
                self.copy_selection()
            };
            return match text {
                Some(t) => CopyOutcome::Text(t),
                None => CopyOutcome::Nothing,
            };
        }
        // Copy is authoritative (Logic reads the full doc), so we cannot decide
        // emptiness from the viewport cache without reintroducing the limit. Only
        // the trivial case, a single active cell that is empty in cache, lets the
        // browser default proceed. Any multi-cell selection defers to Logic.
        let (r1, c1, r2, c2) = self.state.selection.bounds();
        if (r1, c1) == (r2, c2)
            && self
                .state
                .cell_cache
                .get(&(r1, c1))
                .is_none_or(|d| d.raw.is_empty())
        {
            return CopyOutcome::Nothing;
        }
        // Authoritative build lives on the Logic thread; the UI holds only the
        // marching-ants anchor and the cut flag (both UI state), set now.
        if let Err(SendError::Refused) = self.send_command_lossless(TrameCommand::CopyRange) {
            tracing::error!("Trame: the outbox is full, the copy was dropped");
            return CopyOutcome::Nothing;
        }
        self.state.clipboard_anchor = Some(self.state.selection.bounds());
        self.cut_pending = cut;
        self.edit_dirty = true; // repaint the marching-ants overlay
        CopyOutcome::Deferred
    }

    /// Drain the clipboard blob built from the last `TrameEvent::Clipboard` reply.
    /// The host can then mirror it to the OS clipboard within the transient user
    /// activation window (`trame-clipboard`). Yields the blob at most once per copy.
    pub fn poll_clipboard(&mut self) -> Option<ambre_bus::ClipboardBlob> {
        self.pending_clipboard.take()
    }

    /// Service a paste from the shared [`ClipboardBlob`](ambre_bus::ClipboardBlob)
    /// transport (`trame-clipboard`). Fidelity order: an open in-cell edit takes
    /// the plain text to the caret. Else, when `own_copy` says the synchronous
    /// ownership token was still on the clipboard, the in-memory `last_clip` at
    /// full fidelity. That one is immune to the async OS write having failed. Else
    /// the blob's decoded Ambre payload (cross-tab/app internal round-trip), else
    /// the `text/plain` mirror as literal cells.
    pub fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, own_copy: bool) {
        if self.is_editing() {
            // In-cell edit: a typed payload is meaningless inside a text field;
            // route the text through the same insertion path as typing.
            self.handle_input(&InputEvent::Text { text: blob.text });
            return;
        }
        // The kept copy serves on this tab's token, whatever the asynchronous
        // OS write did; otherwise the OS blob does (`UI-THREAD-053`).
        let source = ambre_bus::ClipboardBlob::source(self.last_clip.as_ref(), &blob, own_copy);
        let payload = source
            .ambre
            .as_ref()
            .filter(|_| !source.plain)
            .and_then(|bytes| postcard::from_bytes::<ambre_bus::ClipboardPayload>(bytes).ok());
        match payload {
            Some(payload) => self.grid_paste_payload(payload),
            None => self.grid_paste_text(&blob.text),
        }
    }

    /// Paste a typed Ambre payload at the active cell (`trame-clipboard`). The
    /// Logic thread reindexes relative formulas and applies styles; a pending cut
    /// then clears the source range.
    pub fn grid_paste_payload(&mut self, payload: ambre_bus::ClipboardPayload) {
        let (row, col) = (self.state.active_row(), self.state.active_col());
        let (rows, cols) = (payload.rows, payload.cols);
        let sent = self.send_command_lossless(TrameCommand::PasteRange {
            target_row: row,
            target_col: col,
            payload,
        });
        if sent.is_err() {
            // The paste never left, so neither the selection nor the pending cut
            // may act as though it had (`BUS-018`). Clearing the source here is
            // how a cut destroys a range it never moved (`TRAME-UI-125`).
            tracing::error!("Trame: the paste was refused, the source range is left alone");
            self.needs_full = true;
            return;
        }
        // Select the whole destination rectangle, not just the landing cell, so
        // the pasted block reads back as a single selection (`trame-clipboard`).
        self.select_pasted_region(row, col, rows, cols);
        self.finish_paste();
        self.needs_full = true;
    }

    /// Paste external tab-separated text at the active cell (`trame-clipboard`):
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
        let sent = self.send_command_lossless(TrameCommand::PasteRange {
            target_row: row,
            target_col: col,
            payload,
        });
        if sent.is_err() {
            tracing::error!("Trame: the paste was refused, the selection stays where it was");
            self.needs_full = true;
            return;
        }
        // Select the whole destination rectangle (see `grid_paste_payload`).
        self.select_pasted_region(row, col, rows, cols);
        self.needs_full = true;
    }

    /// Select the rectangle a paste just landed on: top-left at the target cell,
    /// extending `rows` x `cols` (the payload's dimensions), clamped to the grid.
    /// A single-cell paste collapses to a plain single selection. The active
    /// corner stays on the top-left so subsequent typing overwrites there, while
    /// the whole pasted block is highlighted (`trame-clipboard`).
    fn select_pasted_region(&mut self, row: u32, col: u32, rows: u32, cols: u32) {
        let r2 = (row + rows.saturating_sub(1)).min(crate::TOTAL_ROWS - 1);
        let c2 = (col + cols.saturating_sub(1)).min(TOTAL_COLS - 1);
        self.state.select_rect(row, col, r2, c2);
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
            // The source clear is a document mutation like any other. The grid
            // may only forget the range once the command is on its way
            // (`BUS-018`). A refused clear would otherwise show an empty range
            // the document still holds (`TRAME-UI-142`).
            if self
                .send_command_lossless(TrameCommand::SetCellsBatch { cells: batch })
                .is_ok()
            {
                for r in r1..=r2 {
                    for c in c1..=c2 {
                        self.state.set_cell_text(r, c, String::new());
                    }
                }
            } else {
                tracing::error!("Trame: the source clear was refused, the cut range is kept");
            }
        }
        self.cut_pending = false;
        self.state.clipboard_anchor = None;
    }

    /// Map a logical pointer position into the open editor's local (text-relative)
    /// frame, for a pointer intent the shared core resolves against its layout.
    /// Single-line: the local y is 0 (line 0), so only x carries information.
    fn editor_local_point(&self, x: f64, _y: f64) -> Option<(f32, f32)> {
        let ed = self.editor.as_ref()?;
        let (cx, _cy, _cw, _ch) = self.state.cell_pixel_rect(ed.row, ed.col)?;
        let pad = crate::render::CELL_PAD_X * self.state.zoom;
        Some(((x - (cx + pad)).max(0.0) as f32, 0.0))
    }

    /// True when logical pixel `(x, y)` falls on the cell currently being
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
                // Queue the insertion; the shared editor applies it at the paint
                // flush and the formula bar mirrors `ed.text()` there (frame.rs).
                if let Some(ed) = self.editor.as_mut() {
                    ed.insert(text);
                }
                self.reset_blink();
                true
            }
            InputEvent::CompositionUpdate { text } => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.set_compose(text);
                }
                self.reset_blink();
                true
            }
            InputEvent::CompositionEnd { text } => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.commit_compose(text);
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
                    if let Some((lx, ly)) = self.editor_local_point(*x, *y)
                        && let Some(ed) = self.editor.as_mut()
                    {
                        ed.point_down(lx, ly);
                    }
                    self.editor_text_drag = true;
                    self.reset_blink();
                    return true;
                }
                // Press outside: commit (deferred to the paint flush) and let the
                // click select the cell under it.
                self.pending_commit = Some(PendingCommit::Stay);
                self.edit_dirty = true;
                self.text_input.focus();
                if let Some((row, col)) = self.state.cell_at(*x, *y) {
                    self.select_cell(row, col);
                }
                true
            }
            // Drag inside the edited cell extends the text selection.
            InputEvent::PointerMove { x, y } if self.editor_text_drag => {
                if let Some((lx, ly)) = self.editor_local_point(*x, *y)
                    && let Some(ed) = self.editor.as_mut()
                {
                    ed.point_drag(lx, ly);
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
                if let Some((lx, ly)) = self.editor_local_point(*x, *y)
                    && let Some(ed) = self.editor.as_mut()
                {
                    ed.word_at(lx, ly);
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
                self.pending_commit = Some(PendingCommit::Stay);
                self.edit_dirty = true;
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
                alt,
                ..
            } => {
                // Ctrl/Cmd+Enter fills the whole selection with the typed value;
                // Ctrl/Cmd+A selects the whole buffer; other command chords
                // (Ctrl/Cmd+X) fall through to the chrome keymap.
                if *ctrl || *meta {
                    if key == "Enter" {
                        // Deferred: the fill reads the buffer after the paint flush.
                        // `edit_dirty` schedules that paint (else it would wait for
                        // the blink tick).
                        self.pending_commit = Some(PendingCommit::Fill);
                        self.edit_dirty = true;
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
                    // Alt+Enter inserts a newline (multi-line cell); a bare Enter
                    // commits and moves down (spreadsheet convention).
                    "Enter" if *alt => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.newline();
                        }
                        self.reset_blink();
                        true
                    }
                    // Enter / Tab commit and move (spreadsheet). The commit is
                    // deferred to the paint flush so the queued edits are applied
                    // before the buffer is read (`trame-cell-editing`).
                    "Enter" => {
                        self.pending_commit = Some(PendingCommit::Nav(1, 0));
                        self.edit_dirty = true;
                        true
                    }
                    "Tab" => {
                        self.pending_commit = Some(PendingCommit::Nav(0, 1));
                        self.edit_dirty = true;
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
                    // Delete removes the selection, else the char after the caret.
                    // Shift+Delete is "cut": let it fall through so the browser
                    // fires the `cut` clipboard event the host listens for.
                    "Delete" if !*shift => {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.delete_forward();
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
                    // Up/Down move the caret between lines inside the edit (a
                    // multi-line cell); Shift extends the selection. Cell nav by
                    // arrows only applies when NOT editing.
                    "ArrowUp" => {
                        if let Some(ed) = self.editor.as_mut() {
                            if *shift { ed.extend_up() } else { ed.move_up() }
                        }
                        self.reset_blink();
                        true
                    }
                    "ArrowDown" => {
                        if let Some(ed) = self.editor.as_mut() {
                            if *shift {
                                ed.extend_down()
                            } else {
                                ed.move_down()
                            }
                        }
                        self.reset_blink();
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

/// The width a column auto-fit gives the column holding `cells`: the widest
/// natural text, plus the cell padding, never under the minimum. A column with
/// no content takes the default width (`trame-resize`). `cells` is the whole
/// column (`TRAME-UI-128`).
pub(crate) fn fit_column_width(
    cells: &[crate::DisplayCell],
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
) -> f32 {
    let pad = (crate::render::CELL_PAD_X * 2.0) as f32;
    let widest = cells
        .iter()
        .filter(|cell| !cell.text.is_empty())
        .map(|cell| {
            let size = cell.size.map(f32::from).unwrap_or(crate::render::BODY_SIZE);
            text_cache
                .get_or_build_font(
                    font_cx,
                    &cell.text,
                    size,
                    f32::MAX,
                    ambre_draw::Alignment::Start,
                    cell.bold,
                    cell.italic,
                    cell.font.as_deref(),
                    false,
                )
                .width()
        })
        .fold(0.0f32, f32::max);
    if cells.iter().any(|cell| !cell.text.is_empty()) {
        (widest + pad).max(crate::ui::grid::MIN_COL_WIDTH as f32)
    } else {
        crate::ui::grid::COL_WIDTH as f32
    }
}

#[cfg(test)]
mod tests {
    use super::fit_column_width;
    use ambre_canvas::TextCache;
    use ambre_draw::FontContext;

    /// A column with content is sized from it; an empty one takes the default
    /// width (`trame-resize`).
    #[test]
    fn a_column_fit_sizes_from_content_or_defaults_when_empty() {
        let (mut font_cx, mut cache) = (FontContext::new(), TextCache::new(1.0));
        let empty = fit_column_width(&[], &mut font_cx, &mut cache);
        assert_eq!(empty, crate::ui::grid::COL_WIDTH as f32);
        let blank = crate::DisplayCell::default();
        assert_eq!(
            fit_column_width(&[blank], &mut font_cx, &mut cache),
            empty,
            "a blank measures nothing"
        );
        let cell = crate::DisplayCell {
            text: "content".to_owned(),
            ..Default::default()
        };
        let sized = fit_column_width(&[cell], &mut font_cx, &mut cache);
        assert!(sized >= crate::ui::grid::MIN_COL_WIDTH as f32);
        assert_ne!(sized, empty, "content is measured, not defaulted");
    }
}
