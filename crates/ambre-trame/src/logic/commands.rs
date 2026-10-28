// ambre-trame/src/logic/commands.rs.rs
// Command dispatch, undo/redo history
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::find::Replacement;
use super::*;
use ambre_core::HistoryStep;

impl TrameState {
    /// Handle a UI command and return a typed outcome.
    ///
    /// Must complete in < 50 ms (P2); scroll requests in < 10 ms (P1).
    pub fn handle_command(&mut self, cmd: TrameCommand) -> TrameOutcome {
        let mut outcome = self.dispatch_command(cmd);
        // Any command that produced a CRDT delta also moved the `UndoManager`
        // stacks, so the chrome's Undo/Redo affordances must be refreshed. Undo
        // and Redo already append their own `UndoState`; skip the duplicate.
        let already_reported = matches!(outcome.display.last(), Some(TrameEvent::UndoState { .. }));
        if outcome.update.is_some() && !already_reported {
            outcome.display.push(self.undo_state_event());
        }
        outcome
    }

    /// Execute one command and return its raw outcome, before the central
    /// [`UndoState`](TrameEvent::UndoState) refresh applied by [`handle_command`].
    pub(super) fn dispatch_command(&mut self, cmd: TrameCommand) -> TrameOutcome {
        // Seal any still-open interactive edit group before handling the next command.
        // An edit is grouped across all its commits: the cell write plus derived
        // side-effects such as a filtered-column recompute. It is normally closed by
        // `CommitEdit` on cell exit. This is the fallback boundary when the UI omits it
        // (for example a non-edit command intervenes). One cell edit is then always
        // exactly one undo item (`trame-cell-editing`, `trame-undo`). `group_end` is a
        // no-op when no group is open.
        self.undo_manager.group_end();
        match cmd {
            TrameCommand::SetActiveCell {
                row,
                col,
                anchor_row,
                anchor_col,
                extra,
                kind,
            } => {
                self.active = CellPos::new(row, col);
                // Store the whole selection: the anchor corner, every committed
                // disjoint rectangle, and the band kind. The readers resolve the
                // live rectangle and the used range from these.
                self.selection = TrackedSelection {
                    anchor: CellPos::new(anchor_row, anchor_col),
                    extra,
                    kind,
                };
                // Report the new active cell's formatting so the chrome toggle
                // buttons reflect the selection.
                // Resolved style (incl. bold/italic) drives the formatting toolbar.
                TrameOutcome::display_only(vec![self.style_state_event()])
            }
            TrameCommand::CommitEdit => {
                // Cell exit: the fallback seal at the top of `dispatch_command`
                // has already closed the edit group, so the whole edit is now one
                // undo item. No CRDT delta of its own (`trame-cell-editing`).
                TrameOutcome::display_only(vec![])
            }
            TrameCommand::SetCellText { row, col, text } => {
                tracing::info!(
                    "[Logic] SetCellText row={:?} col={:?} text='{:.80}'",
                    row,
                    col,
                    text,
                );
                // Open the edit group: the cell write below and any derived
                // commit (filter recompute) merge into a single undo item. The
                // next `CommitEdit` seals it, or the fallback seal above.
                let _ = self.undo_manager.group_start();
                self.write_cell(CellPos::new(row, col), &FormulaSource::new(text));
                self.finish_cell_writes()
            }
            TrameCommand::BeginCellEdit { session, row, col } => {
                self.begin_cell_edit(session, CellPos::new(row, col))
            }
            TrameCommand::CommitCellEdit { session, text } => {
                tracing::info!("[Logic] CommitCellEdit session={session} text='{text:.80}'");
                self.commit_cell_edit(session, &FormulaSource::new(text))
            }
            TrameCommand::EndCellEdit { session } => self.end_cell_edit(session),
            TrameCommand::SetCellsBatch { cells } => {
                tracing::info!("[Logic] SetCellsBatch {} cells", cells.len());
                // A multi-cell fill / clear (Ctrl+Enter, Delete over a range).
                // All writes share ONE undo group, so the whole batch is a single
                // undo item. The group spans the loop and is sealed by the next
                // command's fallback `group_end` (or an explicit `CommitEdit`).
                let _ = self.undo_manager.group_start();
                // Pre-materialize every written cell so all the excluded-origin
                // `materialize` commits happen up front. Interleaving them between
                // the value commits would split the batch into one undo item per
                // cell (see `write_cell`). Doing them first leaves the value writes
                // in a single trailing commit -> one undo item for the batch.
                for (at, text) in &cells {
                    if !text.is_empty() {
                        let _ = self.materialize(*at);
                    }
                }
                for (at, text) in cells {
                    self.write_cell(at, &FormulaSource::new(text));
                }
                self.finish_cell_writes()
            }
            TrameCommand::SetCellStyle { patch } => self.set_cell_style(patch),
            TrameCommand::ScrollViewport {
                version,
                row_start,
                row_end,
                col_start,
                col_end,
            } => {
                // Clamp to grid bounds.
                let row_end = row_end.min(crate::LAST_ROW);
                let col_end = col_end.min(crate::LAST_COL);

                if row_start > row_end || col_start > col_end {
                    return TrameOutcome::display_only(vec![]);
                }

                // Synchronous recomputation of any dirty cells before serving
                // the viewport. Ensures the UI displays up-to-date formula results.
                let indices = self.engine_indices();
                self.engine.compute_all_dirty(&indices);

                let window = CellRect::spanning(
                    CellPos::new(row_start, col_start),
                    CellPos::new(row_end, col_end),
                );
                let cells = self.pack_cells(window);

                // Per-axis metric slices for the requested window: the UI feeds
                // them straight into its variable-metric GridLayout
                // (`trame-resize`).
                let col_widths = window
                    .cols()
                    .into_iter()
                    .map(|c| self.col_width(c))
                    .collect();
                let row_heights = window
                    .rows()
                    .into_iter()
                    .map(|r| self.row_height(r))
                    .collect();

                let mut display = vec![TrameEvent::ViewportData {
                    version,
                    row_start,
                    col_start,
                    cells,
                    col_widths,
                    row_heights,
                }];
                // Hydrate the tab strip on the first viewport request of the
                // session (the UI has no separate sheet-list fetch command).
                if !self.sheet_list_sent {
                    self.sheet_list_sent = true;
                    display.push(self.sheet_list_event());
                }
                TrameOutcome::display_only(display)
            }
            TrameCommand::InsertRow { at } => self.insert_axis(at),
            TrameCommand::DeleteRow { at } => self.delete_axis(at),
            TrameCommand::InsertColumn { at } => self.insert_axis(at),
            TrameCommand::DeleteColumn { at } => self.delete_axis(at),
            TrameCommand::MoveRow { from, to } => self.move_axis(from, to),
            TrameCommand::MoveColumn { from, to } => self.move_axis(from, to),
            TrameCommand::MergeCells { rect } => self.merge_cells(rect),
            TrameCommand::UnmergeCells { row, col } => self.unmerge_cells(CellPos::new(row, col)),
            TrameCommand::ResizeCol {
                version: _,
                sheet_id,
                col,
                width,
            } => self.resize_col(&sheet_id, col, width),
            TrameCommand::ResizeRow {
                version: _,
                sheet_id,
                row,
                height,
            } => self.resize_row(&sheet_id, row, height),
            // -- Parameterless structural intents (grid context menu). Geometry
            // comes from the tracked active cell / selection (`SetActiveCell`).
            TrameCommand::InsertRowAbove => self.insert_axis(self.active.row),
            TrameCommand::InsertRowBelow => self.insert_axis(self.active.row.next()),
            TrameCommand::InsertColLeft => self.insert_axis(self.active.col),
            TrameCommand::InsertColRight => self.insert_axis(self.active.col.next()),
            TrameCommand::DeleteSelectedRows => self.delete_selected_axis::<Row>(),
            TrameCommand::DeleteSelectedCols => self.delete_selected_axis::<Col>(),
            TrameCommand::MergeSelection => self.merge_cells(self.live_selection_rect()),
            TrameCommand::UnmergeActive => self.unmerge_selection(),
            // Single-column shared sort over the tracked selection
            // (`trame-sort`). Reference column = the range's first column
            // (`sort_col = 0`), no header row. The active sheet is the only
            // sortable one (see
            // `sort_range`).
            TrameCommand::SortSelectionAsc => self.sort_selection(SortOrder::Ascending),
            TrameCommand::SortSelectionDesc => self.sort_selection(SortOrder::Descending),
            // Whole-sheet sort by the active column (whole-column selection,
            // `trame-sort`): sorts the used range, ref column = active column.
            TrameCommand::SortSheetAsc => self.sort_sheet(SortOrder::Ascending),
            TrameCommand::SortSheetDesc => self.sort_sheet(SortOrder::Descending),
            // Multi-column sort over the tracked selection (the "Sort range..."
            // dialog, `trame-sort`): geometry + active sheet resolved by Logic.
            TrameCommand::SortSelectionMulti { sort_keys, header } => {
                self.sort_selection_multi(&sort_keys, header)
            }
            TrameCommand::Undo => self.apply_history(HistoryStep::Undo),
            TrameCommand::Redo => self.apply_history(HistoryStep::Redo),
            TrameCommand::DefineName { name, reference } => self.define_name(&name, &reference),
            TrameCommand::DeleteName { name } => self.delete_name(&name),
            TrameCommand::ActivateSheet { sheet_id } => self.activate_sheet(&sheet_id),
            TrameCommand::CreateSheet { name } => self.create_sheet(&name),
            TrameCommand::RenameSheet { sheet_id, name } => self.rename_sheet(&sheet_id, &name),
            TrameCommand::MoveSheet { sheet_id, to_index } => self.move_sheet(&sheet_id, to_index),
            TrameCommand::DeleteSheet { sheet_id } => self.delete_sheet(&sheet_id),
            TrameCommand::SortRange {
                sheet_id,
                rect,
                sort_col,
                order,
                header,
            } => self.sort_range(&sheet_id, rect, sort_col, order, header),
            TrameCommand::SortRangeMulti {
                sheet_id,
                rect,
                sort_keys,
                header,
            } => self.sort_range_multi(&sheet_id, rect, &sort_keys, header),
            TrameCommand::ApplyFilter {
                sheet_id,
                col,
                kind,
            } => self.apply_filter(&sheet_id, col, kind),
            TrameCommand::ClearFilter { sheet_id } => self.clear_filter(&sheet_id),
            // Parameterless siblings: geometry resolved against the tracked active
            // cell / active sheet (`trame-shared-filter`), like the sorts above.
            TrameCommand::RequestActiveColumnValues => self.active_column_values(),
            TrameCommand::RequestSortRange => self.sort_range_query(),
            TrameCommand::RequestColumnContent { col } => self.column_content(col),
            TrameCommand::RequestSelectionStats => self.selection_stats(),
            // Authoritative copy: serialize the tracked selection from the
            // document (never viewport-culled), replied via `Clipboard`.
            TrameCommand::CopyRange => self.copy_range(),
            TrameCommand::ApplyFilterToActiveColumn { kind } => {
                let sheet = self.active_sheet.clone();
                self.apply_filter(&sheet, self.active.col, kind)
            }
            TrameCommand::ClearActiveFilter => {
                let sheet = self.active_sheet.clone();
                self.clear_filter(&sheet)
            }
            TrameCommand::PasteRange {
                target_row,
                target_col,
                payload,
            } => self.paste_range(CellPos::new(target_row, target_col), payload),
            TrameCommand::PasteText { text } => self.paste_text(&text),
            TrameCommand::PasteClipboardAtActive { payload } => {
                self.paste_range(self.active, payload)
            }
            TrameCommand::FillRange {
                sheet_id,
                src,
                dest,
                mode,
            } => self.fill_range(&sheet_id, src, dest, mode),

            TrameCommand::FindNext { query } => match query {
                Some(query) => self.find_next(&query.text, query.options),
                None => Self::find_nothing(),
            },
            TrameCommand::ReplaceAll { query, replacement } => match query {
                Some(query) => {
                    self.replace_all(&query.text, &Replacement::new(replacement), query.options)
                }
                None => Self::replace_nothing(),
            },
            TrameCommand::ImportCsv {
                data,
                delimiter,
                header,
                target_sheet_id,
                start_row,
                start_col,
            } => self.import_csv(
                data,
                delimiter,
                header,
                target_sheet_id,
                CellPos::new(start_row, start_col),
            ),
            TrameCommand::ExportCsv {
                sheet_id,
                delimiter,
                range,
            } => self.export_csv(&sheet_id, delimiter, range),
            TrameCommand::ExportActiveSheetCsv => {
                let sheet = self.active_sheet.clone();
                self.export_csv(&sheet, None, None)
            }
            // Served by the Logic thread: the container's header carries the
            // document's identity, which a workbook does not know
            // (`ambre-spec-docfile.md`). Its dispatcher intercepts the command
            // before the document lookup, so this arm is reached only by a
            // caller driving `TrameState` directly. It answers rather than
            // falling silent.
            TrameCommand::ExportAmbreFile | TrameCommand::ExportAmbreFileSealed { .. } => {
                TrameOutcome::display_only(vec![TrameEvent::Error {
                    message: "the file is written by the Logic thread, not by the workbook"
                        .to_owned(),
                }])
            }

            TrameCommand::ReplaceOne { query, replacement } => match query {
                Some(query) => {
                    self.replace_one(&query.text, &Replacement::new(replacement), query.options)
                }
                None => Self::replace_nothing(),
            },

            // Deferred feature surface (`trame-engine`): the command vocabulary
            // is complete and typed. These handlers land in later phases
            // (personal views / multi-sort -> Phase 2, conditional formatting &
            // validation -> Phase 2/3, protection -> Post-MVP). Until then each
            // is an explicit no-op: no CRDT delta, no display event, so an early
            // UI build can issue them harmlessly.
            TrameCommand::SaveFilterView { .. }
            | TrameCommand::ActivateFilterView { .. }
            | TrameCommand::AddCondFormat { .. }
            | TrameCommand::UpdateCondFormat { .. }
            | TrameCommand::RemoveCondFormat { .. }
            | TrameCommand::AddValidation { .. }
            | TrameCommand::RemoveValidation { .. }
            | TrameCommand::SetProtection { .. }
            | TrameCommand::RemoveProtection { .. } => TrameOutcome::display_only(vec![]),
        }
    }

    /// Apply one undo (`undo = true`) or redo step from Loro's `UndoManager`.
    /// Then rebuild every derived cache from the resulting CRDT state and
    /// broadcast the inverse ops it produced (`trame-undo`).
    ///
    /// `undo()` / `redo()` commit the inverse of a past local commit as fresh
    /// ops. The rebuild path is therefore identical to a structural op or
    /// import. The index, merge cache, formula engine and formatting are all
    /// reseeded from the new document state. Returns the same `DataReset` +
    /// active-cell state the UI needs to repaint, plus an [`UndoState`]
    /// reporting the new stack availability. When the requested stack is empty
    /// the call is a no-op and reports nothing: the availability the chrome
    /// holds is still true (`UNDO-009`).
    ///
    /// [`UndoState`]: ambre_bus::TrameEvent::UndoState
    pub(super) fn apply_history(&mut self, step: HistoryStep) -> TrameOutcome {
        // An undo can take back the sheet the user is on, an import's above
        // all. The reader then lands where a deletion leaves them (`TRAME-075`).
        let order = self.sheet_order_ids();
        let changed = match step {
            HistoryStep::Undo => self.undo_manager.undo(),
            HistoryStep::Redo => self.undo_manager.redo(),
        }
        .unwrap_or(false);

        if !changed {
            return TrameOutcome::display_only(Vec::new());
        }

        // The inverse ops are already committed by undo()/redo(); reseed the
        // derived caches and broadcast the delta exactly as `after_structural`.
        self.leave_a_deleted_sheet(&order);
        let mut outcome = self.after_structural();
        outcome.display.push(self.sheet_list_event());
        outcome.display.push(self.style_state_event());
        outcome.display.push(self.undo_state_event());
        outcome
    }

    /// Current undo/redo availability, for the chrome's Undo/Redo affordances.
    pub(super) fn undo_state_event(&self) -> TrameEvent {
        TrameEvent::UndoState {
            can_undo: self.undo_manager.can_undo(),
            can_redo: self.undo_manager.can_redo(),
        }
    }
}

impl TrameState {
    /// The display cells of `window`, as the viewport serves them: computed
    /// text through its number format, the raw text, and the resolved style.
    /// The formula cache must be current.
    pub(super) fn pack_cells(&self, window: CellRect) -> Vec<DisplayCell> {
        let mut cells = Vec::with_capacity(
            window.row_count().get() as usize * window.col_count().get() as usize,
        );
        // One set of sheet layouts for the whole window: a cross-sheet formula
        // builds its sheet's index once, not once per cell.
        let lines = self.sheet_lines();

        for at in window.cells() {
            let CellPos { row, col } = at;
            // A merge master carries its span; a covered cell renders
            // blank (swallowed by the master). Both flag `is_merged`.
            let merge_span = self
                .merges
                .get(&at)
                .map(|&end| CellRect::spanning(at, end).extent());
            let is_covered = self.covered.contains(&at);
            let style = self.styles.get(&at).cloned().unwrap_or_default();
            let text = if is_covered {
                String::new()
            } else {
                // Apply the cell's number format to its computed value
                // (`trame-number-formats`): a `General` cell is verbatim.
                self.format_display(at, self.cell_text(at), &style)
            };
            cells.push(DisplayCell {
                row,
                col,
                text,
                raw: if is_covered {
                    String::new()
                } else {
                    // Reconstruct positions from the stored lines, and
                    // cross-sheet qualifiers from the stable id to the
                    // display name (`TRAME-228`, `trame-sheets`).
                    self.formula_display(&self.cell_raw(at), &self.active_sheet, &lines)
                        .into_string()
                },
                bold: Weight::from_bool(self.bold.contains(&at)),
                italic: Slant::from_bool(self.italic.contains(&at)),
                underline: style.underline,
                strikethrough: style.strikethrough,
                h_align: style.h_align,
                numeric: CellValueKind::from_bool(!is_covered && self.cell_is_numeric(at)),
                v_align: style.v_align,
                wrap: style.wrap,
                fg: style.fg,
                bg: style.bg,
                font: style.font.clone(),
                size: style.size,
                borders: style.borders,
                is_merged: Merge::from_bool(merge_span.is_some() || is_covered),
                merge_span,
                is_filtered: RowFilter::from_bool(self.hidden_rows.contains(&row)),
            });
        }
        cells
    }

    /// Every non-empty cell of grid column `col`, all rows, for an auto-fit
    /// (`TRAME-UI-128`). The viewport cache holds only the loaded window, so the
    /// canvas cannot measure a column whose content lies outside it.
    pub(super) fn column_content(&mut self, col: Col) -> TrameOutcome {
        let indices = self.engine_indices();
        self.engine.compute_all_dirty(&indices);
        let cells = match Row::last(&self.index) {
            None => Vec::new(),
            Some(last) => self
                .pack_cells(CellRect::spanning(
                    CellPos::new(Row::ZERO, col),
                    CellPos::new(last, col),
                ))
                .into_iter()
                .filter(|cell| !cell.text.is_empty())
                .collect(),
        };
        TrameOutcome::display_only(vec![TrameEvent::ColumnContent { col, cells }])
    }
}
