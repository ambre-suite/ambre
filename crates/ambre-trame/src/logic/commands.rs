// ambre-trame/src/logic/commands.rs.rs
// Command dispatch, undo/redo history
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;

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
        // Seal any still-open interactive edit group before handling the next
        // command. An edit is grouped across all its commits (cell write plus
        // derived side-effects such as a filtered-column recompute) and normally
        // closed by `CommitEdit` on cell exit; this is the fallback boundary when
        // the UI omits it (e.g. a non-edit command intervenes), so one cell edit
        // is always exactly one undo item (`-ui.md` §6.C, `-logic.md` §8).
        // `group_end` is a no-op when no group is open.
        self.undo_manager.group_end();
        match cmd {
            TrameCommand::SetActiveCell {
                row,
                col,
                anchor_row,
                anchor_col,
            } => {
                self.active = (row, col);
                // Normalize the active cell and the anchor corner into the
                // selection rectangle the structural intents resolve against.
                self.selection = (
                    row.min(anchor_row),
                    col.min(anchor_col),
                    row.max(anchor_row),
                    col.max(anchor_col),
                );
                // Report the new active cell's formatting so the chrome toggle
                // buttons reflect the selection.
                // Resolved style (incl. bold/italic) drives the formatting toolbar.
                TrameOutcome::display_only(vec![self.style_state_event()])
            }
            TrameCommand::CommitEdit => {
                // Cell exit: the fallback seal at the top of `dispatch_command`
                // has already closed the edit group, so the whole edit is now one
                // undo item. No CRDT delta of its own (`-ui.md` §6.C).
                TrameOutcome::display_only(vec![])
            }
            TrameCommand::SetCellText { row, col, text } => {
                tracing::info!(
                    "[Logic] SetCellText row={} col={} text='{:.80}'",
                    row,
                    col,
                    text,
                );
                // Open the edit group: the cell write below and any derived
                // commit (filter recompute) merge into a single undo item, sealed
                // by the next `CommitEdit` (or the fallback seal above).
                let _ = self.undo_manager.group_start();
                self.write_cell(row, col, &text);
                self.finish_cell_writes()
            }
            TrameCommand::SetCellsBatch { cells } => {
                tracing::info!("[Logic] SetCellsBatch {} cells", cells.len());
                // A multi-cell fill / clear (Ctrl+Enter, Delete over a range).
                // All writes share ONE undo group so the whole batch is a single
                // undo item -- the group spans the loop and is sealed by the next
                // command's fallback `group_end` (or an explicit `CommitEdit`).
                let _ = self.undo_manager.group_start();
                // Pre-materialize every written cell so all the excluded-origin
                // `materialize` commits happen up front. Interleaving them between
                // the value commits would split the batch into one undo item per
                // cell (see `write_cell`); doing them first leaves the value writes
                // in a single trailing commit -> one undo item for the batch.
                for (row, col, text) in &cells {
                    if !text.is_empty() {
                        let _ = self.materialize(*row, *col);
                    }
                }
                for (row, col, text) in cells {
                    self.write_cell(row, col, &text);
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
                let row_end = row_end.min(TOTAL_ROWS.saturating_sub(1));
                let col_end = col_end.min(TOTAL_COLS.saturating_sub(1));

                if row_start > row_end || col_start > col_end {
                    return TrameOutcome::display_only(vec![]);
                }

                // Synchronous recomputation of any dirty cells before serving
                // the viewport. Ensures the UI displays up-to-date formula results.
                let indices = self.engine_indices();
                self.engine.compute_all_dirty(&indices);

                let mut cells = Vec::with_capacity(
                    ((row_end - row_start + 1) * (col_end - col_start + 1)) as usize,
                );

                for row in row_start..=row_end {
                    for col in col_start..=col_end {
                        // A merge master carries its span; a covered cell renders
                        // blank (swallowed by the master). Both flag `is_merged`.
                        let merge_span = self
                            .merges
                            .get(&(row, col))
                            .map(|&(end_row, end_col)| (end_col - col + 1, end_row - row + 1));
                        let is_covered = self.covered.contains(&(row, col));
                        let style = self.styles.get(&(row, col)).cloned().unwrap_or_default();
                        let text = if is_covered {
                            String::new()
                        } else {
                            // Apply the cell's number format to its computed value
                            // (sec.5bis): a plain `General` cell renders verbatim.
                            self.format_display(row, col, self.cell_text(row, col), &style)
                        };
                        cells.push(DisplayCell {
                            row,
                            col,
                            text,
                            raw: if is_covered {
                                String::new()
                            } else {
                                // Reconstruct cross-sheet qualifiers from the
                                // stable id to the current display name (§2.D).
                                self.formula_display(&self.cell_raw(row, col))
                            },
                            bold: self.bold.contains(&(row, col)),
                            italic: self.italic.contains(&(row, col)),
                            underline: style.underline,
                            strikethrough: style.strikethrough,
                            h_align: style.h_align,
                            v_align: style.v_align,
                            wrap: style.wrap,
                            fg: style.fg,
                            bg: style.bg,
                            font: style.font.clone(),
                            size: style.size,
                            borders: style.borders,
                            is_merged: merge_span.is_some() || is_covered,
                            merge_span,
                            is_filtered: self.hidden_rows.contains(&row),
                        });
                    }
                }

                // Per-axis metric slices for the requested window: the UI feeds
                // them straight into its variable-metric GridLayout (sec.4bis.C).
                let col_widths = (col_start..=col_end).map(|c| self.col_width(c)).collect();
                let row_heights = (row_start..=row_end).map(|r| self.row_height(r)).collect();

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
            TrameCommand::InsertRow { at } => self.insert_axis(at, Axis::Row),
            TrameCommand::DeleteRow { at } => self.delete_axis(at, Axis::Row),
            TrameCommand::InsertColumn { at } => self.insert_axis(at, Axis::Col),
            TrameCommand::DeleteColumn { at } => self.delete_axis(at, Axis::Col),
            TrameCommand::MergeCells {
                row,
                col,
                row_end,
                col_end,
            } => self.merge_cells(row, col, row_end, col_end),
            TrameCommand::UnmergeCells { row, col } => self.unmerge_cells(row, col),
            TrameCommand::ResizeCol {
                version: _,
                sheet_id,
                col,
                width_px,
            } => self.resize_col(&sheet_id, col, width_px),
            TrameCommand::ResizeRow {
                version: _,
                sheet_id,
                row,
                height_px,
            } => self.resize_row(&sheet_id, row, height_px),
            // -- Parameterless structural intents (grid context menu). Geometry
            // comes from the tracked active cell / selection (`SetActiveCell`).
            TrameCommand::InsertRowAbove => self.insert_axis(self.active.0, Axis::Row),
            TrameCommand::InsertRowBelow => self.insert_axis(self.active.0 + 1, Axis::Row),
            TrameCommand::InsertColLeft => self.insert_axis(self.active.1, Axis::Col),
            TrameCommand::InsertColRight => self.insert_axis(self.active.1 + 1, Axis::Col),
            TrameCommand::DeleteSelectedRows => self.delete_selected_axis(Axis::Row),
            TrameCommand::DeleteSelectedCols => self.delete_selected_axis(Axis::Col),
            TrameCommand::MergeSelection => {
                let (r1, c1, r2, c2) = self.selection;
                self.merge_cells(r1, c1, r2, c2)
            }
            TrameCommand::UnmergeActive => self.unmerge_selection(),
            // Single-column shared sort over the tracked selection (`-ui.md`
            // sec.5.C). Reference column = the range's first column (`sort_col = 0`),
            // no header row. The active sheet is the only sortable one (see
            // `sort_range`).
            TrameCommand::SortSelectionAsc => self.sort_selection(true),
            TrameCommand::SortSelectionDesc => self.sort_selection(false),
            // Whole-sheet sort by the active column (whole-column selection,
            // sec.5.C): sorts the full used range, ref column = active column.
            TrameCommand::SortSheetAsc => self.sort_sheet(true),
            TrameCommand::SortSheetDesc => self.sort_sheet(false),
            TrameCommand::Undo => self.apply_history(true),
            TrameCommand::Redo => self.apply_history(false),
            TrameCommand::DefineName { name, reference } => self.define_name(&name, &reference),
            TrameCommand::DeleteName { name } => self.delete_name(&name),
            TrameCommand::ActivateSheet { sheet_id } => self.activate_sheet(&sheet_id),
            TrameCommand::CreateSheet { name } => self.create_sheet(&name),
            TrameCommand::RenameSheet { sheet_id, name } => self.rename_sheet(&sheet_id, &name),
            TrameCommand::MoveSheet { sheet_id, to_index } => self.move_sheet(&sheet_id, to_index),
            TrameCommand::DeleteSheet { sheet_id } => self.delete_sheet(&sheet_id),
            TrameCommand::SortRange {
                sheet_id,
                row_start,
                row_end,
                col_start,
                col_end,
                sort_col,
                ascending,
                has_header,
            } => self.sort_range(
                &sheet_id, row_start, row_end, col_start, col_end, sort_col, ascending, has_header,
            ),
            TrameCommand::ApplyFilter {
                sheet_id,
                col,
                kind,
            } => self.apply_filter(&sheet_id, col, kind),
            TrameCommand::ClearFilter { sheet_id } => self.clear_filter(&sheet_id),
            // Parameterless siblings: geometry resolved against the tracked active
            // cell / active sheet (`-ui.md` sec.5.D), like the sort intents above.
            TrameCommand::RequestActiveColumnValues => self.active_column_values(),
            TrameCommand::RequestSelectionStats => self.selection_stats(),
            TrameCommand::ApplyFilterToActiveColumn { kind } => {
                let sheet = self.active_sheet.clone();
                self.apply_filter(&sheet, self.active.1, kind)
            }
            TrameCommand::ClearActiveFilter => {
                let sheet = self.active_sheet.clone();
                self.clear_filter(&sheet)
            }
            TrameCommand::PasteRange {
                target_row,
                target_col,
                payload,
            } => self.paste_range(target_row, target_col, payload),
            TrameCommand::PasteText { text } => self.paste_text(&text),
            TrameCommand::PasteClipboardAtActive { payload } => {
                let (row, col) = self.active;
                self.paste_range(row, col, payload)
            }
            TrameCommand::FillRange {
                sheet_id,
                src,
                dest,
                mode,
            } => self.fill_range(&sheet_id, src, dest, mode),

            TrameCommand::FindNext {
                query,
                match_case,
                whole_cell,
            } => self.find_next(query, match_case, whole_cell),
            TrameCommand::ReplaceAll {
                query,
                replacement,
                match_case,
                whole_cell,
                in_formulas,
                all_sheets,
            } => self.replace_all(
                query,
                replacement,
                match_case,
                whole_cell,
                in_formulas,
                all_sheets,
            ),
            TrameCommand::ImportCsv {
                data,
                delimiter,
                has_header,
                target_sheet_id,
                start_row,
                start_col,
            } => self.import_csv(
                data,
                delimiter,
                has_header,
                target_sheet_id,
                start_row,
                start_col,
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

            TrameCommand::ReplaceOne {
                query,
                replacement,
                match_case,
                whole_cell,
                in_formulas,
            } => self.replace_one(query, replacement, match_case, whole_cell, in_formulas),

            // Deferred feature surface (`ambre-spec-trame-features.md §10`): the
            // command vocabulary is complete and typed, but these handlers land
            // in later phases (personal views / multi-sort -> Phase 2,
            // conditional formatting & validation -> Phase 2/3, protection ->
            // Post-MVP). Until then each is an explicit no-op: no CRDT delta, no
            // display event, so an early UI build can issue them harmlessly.
            TrameCommand::SaveFilterView { .. }
            | TrameCommand::ActivateFilterView { .. }
            | TrameCommand::SortRangeMulti { .. }
            | TrameCommand::AddCondFormat { .. }
            | TrameCommand::UpdateCondFormat { .. }
            | TrameCommand::RemoveCondFormat { .. }
            | TrameCommand::AddValidation { .. }
            | TrameCommand::RemoveValidation { .. }
            | TrameCommand::SetProtection { .. }
            | TrameCommand::RemoveProtection { .. } => TrameOutcome::display_only(vec![]),
        }
    }

    /// Apply one undo (`undo = true`) or redo step from Loro's `UndoManager`,
    /// then rebuild every derived cache from the resulting CRDT state and
    /// broadcast the inverse ops it produced (§8).
    ///
    /// `undo()` / `redo()` commit the inverse of a past local commit as fresh
    /// ops, so the rebuild path is identical to a structural op or import: the
    /// index, merge cache, formula engine and formatting are all reseeded from
    /// the new document state. Returns the same `DataReset` + active-cell state
    /// the UI needs to repaint, plus an [`UndoState`] reporting the new stack
    /// availability. When the requested stack is empty the call is a no-op and
    /// only the (unchanged) availability is reported.
    ///
    /// [`UndoState`]: ambre_bus::TrameEvent::UndoState
    pub(super) fn apply_history(&mut self, undo: bool) -> TrameOutcome {
        let changed = if undo {
            self.undo_manager.undo()
        } else {
            self.undo_manager.redo()
        }
        .unwrap_or(false);

        if !changed {
            return TrameOutcome::display_only(vec![self.undo_state_event()]);
        }

        // The inverse ops are already committed by undo()/redo(); reseed the
        // derived caches and broadcast the delta exactly as `after_structural`.
        let mut outcome = self.after_structural();
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
