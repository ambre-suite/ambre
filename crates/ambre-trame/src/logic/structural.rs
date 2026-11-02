// ambre-trame/src/logic/structural.rs.rs
// Row/column insert/delete, axis purge, structural reseed
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;
use crate::formula::SheetQualifier;
use crate::formula::identity::{self, AxisEdit, LineEdit};
use crate::model::{delete_line, insert_line, move_line};
use ambre_core::index::{Count, Pos};

impl TrameState {
    /// Carry every formula that reads the active sheet across one structural
    /// edit of it. The lists already hold the new layout and `self.index`
    /// still the old one. Ranges keep their rectangle through a reorder
    /// (`TRAME-227`), widen on an insert inside them (`TRAME-171`) and shrink
    /// on a delete (`TRAME-230`). Runs before the commit, so the rewrite and
    /// the edit are one undo step. Only a formula whose text changes is
    /// written. Returns whether any was.
    pub(super) fn reanchor_formulas(&self, edit: AxisEdit) -> bool {
        let active = &self.active_sheet;
        let new = self.build_sheet_index(active);
        let mut rewritten = false;
        for sheet in &self.sheets_by_idx {
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, sheet).as_str());
            let LoroValue::Map(cells) = map.get_value() else {
                continue;
            };
            let edited = |q: Option<&SheetQualifier>| match q {
                None => sheet == active,
                Some(SheetQualifier::Id(id)) => id == active,
                Some(SheetQualifier::Name(name)) => {
                    self.engine_sheet_by_name
                        .get(&name.key())
                        .and_then(|idx| self.sheets_by_idx.get(idx.position()))
                        == Some(active)
                }
            };
            for (key, value) in cells.iter() {
                let LoroValue::String(text) = value else {
                    continue;
                };
                let Some(body) = text.strip_prefix('=') else {
                    continue;
                };
                // Another sheet's formula reads this one only through a qualifier.
                if sheet != active && !body.contains('#') {
                    continue;
                }
                let Ok(ast) = parse_formula(body) else {
                    continue;
                };
                let before = render::render(&ast);
                let after = identity::reanchor(ast, &edited, &self.index, &new, edit);
                let after = render::render(&after);
                if after != before {
                    let _ = map.insert(key, format!("={after}").as_str());
                    rewritten = true;
                }
            }
        }
        rewritten
    }

    /// Insert a blank row/column at grid position `at`, shifting later elements
    /// down/right. One `LoroMovableList` op; cells keyed by stable id pairs
    /// follow their element automatically. A no-op past the materialized used
    /// range (those rows/columns are already virtual blanks, nothing to shift).
    pub(super) fn insert_axis<L: SheetLine>(&mut self, at: Pos<L>) -> TrameOutcome {
        if at > L::len(&self.index).as_end() {
            return TrameOutcome::display_only(vec![]);
        }
        // The new line carries a freshly minted, move-invariant uid value
        // (`trame-rows-and-columns`).
        let Some(uid) = self.mint_uid() else {
            return TrameOutcome::display_only(vec![super::cells::ids_exhausted()]);
        };
        if insert_line(&self.line_list::<L>(), at, &uid).is_none() {
            return TrameOutcome::display_only(vec![]);
        }
        self.reanchor_formulas(L::edit(LineEdit::Insert { at, n: Count::ONE }));
        self.doc.commit();
        self.after_structural()
    }

    /// Delete the row/column at grid position `at`, shifting later elements
    /// up/left and purging the removed element's cells and formatting. A
    /// reference names its line, so one to a deleted line reads `#REF!`
    /// (`TRAME-008`). One below it follows its cell up, and a range shrinks
    /// ([`reanchor_formulas`](Self::reanchor_formulas)). A no-op in virtual
    /// space.
    pub(super) fn delete_axis<L: SheetLine>(&mut self, at: Pos<L>) -> TrameOutcome {
        let Some(filter_dropped) = self.remove_line(at) else {
            return TrameOutcome::display_only(vec![]);
        };
        self.reanchor_formulas(L::edit(LineEdit::Delete { at, n: Count::ONE }));
        self.doc.commit();
        self.after_line_removal(filter_dropped)
    }

    /// The outcome of a committed removal: the structural refresh, and the
    /// notice that the filter went with its column when it did.
    fn after_line_removal(&mut self, filter_dropped: bool) -> TrameOutcome {
        let mut outcome = self.after_structural();
        if filter_dropped {
            outcome.display.push(TrameEvent::FilterCleared);
        }
        outcome
    }

    /// The list that orders the lines of axis `P` on the active sheet.
    fn line_list<L: SheetLine>(&self) -> loro::LoroMovableList {
        let root = match L::AXIS {
            Axis::Row => SheetRoot::Rows,
            Axis::Col => SheetRoot::Cols,
        };
        self.doc
            .get_movable_list(SheetContainer::of(root, &self.active_sheet).as_str())
    }

    /// Remove the row/column at grid position `at` and purge what it carried,
    /// with no commit, so several removals land as one. `None` when the line is
    /// virtual and nothing was removed, else whether the active filter went
    /// with it.
    fn remove_line<L: SheetLine>(&mut self, at: Pos<L>) -> Option<bool> {
        // A virtual line has nothing materialized to remove.
        let removed = L::uid(&self.index, at)?;
        delete_line(&self.line_list::<L>(), at)?;
        Some(self.purge_axis(removed, L::AXIS))
    }

    /// Move the row/column at grid position `from` to grid position `to`
    /// (Excel-style drag reorder), realized as a single `LoroMovableList::mov` on
    /// `rows_/cols_{sheet}`. Cells keyed by the move-invariant `(row_uid, col_uid)`
    /// pair travel with their line for free. So does every reference to a
    /// cell, while a range keeps its rectangle (`after_reorder`, `TRAME-227`). A no-op
    /// if `from == to` or either index is outside the materialized used range
    /// (virtual lines carry nothing to move).
    pub(super) fn move_axis<L: SheetLine>(&mut self, from: Pos<L>, to: Pos<L>) -> TrameOutcome {
        let end = L::len(&self.index).as_end();
        if from == to || from >= end || to >= end {
            return TrameOutcome::display_only(vec![]);
        }
        if move_line(&self.line_list::<L>(), from, to).is_none() {
            return TrameOutcome::display_only(vec![]);
        }
        let rewritten = self.reanchor_formulas(L::edit(LineEdit::Reorder));
        self.doc.commit();
        self.after_reorder(rewritten)
    }

    /// Delete every row/column spanned by the current selection rectangle,
    /// iterating from the far edge inward so earlier positions stay valid as the
    /// list shifts. The removals share one commit: one undo step (`TRAME-088`)
    /// and one delta carrying every line to the peers (`TRAME-175`). The
    /// selection is left untouched -- the UI owns it and re-reports it on the
    /// next cursor move.
    pub(super) fn delete_selected_axis<L: SheetLine>(&mut self) -> TrameOutcome {
        // The span of every selected rectangle across the axis (`TRAME-UI-129`).
        let Some((start, end)) = self
            .selection_rects()
            .into_iter()
            .map(L::bounds)
            .reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b)))
        else {
            return TrameOutcome::display_only(vec![]);
        };
        let mut removals: Vec<bool> = Vec::new();
        let mut at = Some(end);
        while let Some(line) = at.filter(|line| *line >= start) {
            removals.extend(self.remove_line(line));
            at = line.prev();
        }
        if removals.is_empty() {
            return TrameOutcome::display_only(vec![]);
        }
        // The removed lines are the materialized run from `start`.
        let n = Count::from_len(removals.len());
        self.reanchor_formulas(L::edit(LineEdit::Delete { at: start, n }));
        self.doc.commit();
        self.after_line_removal(removals.contains(&true))
    }

    /// The range [`sort_selection`](Self::sort_selection) and the multi-key sort
    /// would order, for the sort dialog (`TRAME-UI-101`). It reads the same
    /// [`selection_bounds`](Self::selection_bounds), so the columns the dialog
    /// offers are the columns the sort orders.
    pub(super) fn sort_range_query(&self) -> TrameOutcome {
        let rect = self.selection_bounds();
        TrameOutcome::display_only(vec![TrameEvent::SortRange {
            r1: rect.top_left.row,
            c1: rect.top_left.col,
            r2: rect.bottom_right.row,
            c2: rect.bottom_right.col,
        }])
    }

    /// Single-column shared sort resolved against the tracked selection rectangle
    /// (`trame-sort`). Sorts on the range's first column with no header row,
    /// exactly the parameterless context-menu intent. Whole rows are permuted, so
    /// cells outside the selection move with their row (Google Sheets "sort
    /// range"). Delegates to [`sort_range`](Self::sort_range) on the active
    /// sheet, which guards the empty/single-row cases and replies with
    /// [`TrameEvent::SortComplete`].
    pub(super) fn sort_selection(&mut self, order: SortOrder) -> TrameOutcome {
        let rect = self.selection_bounds();
        let sheet = self.active_sheet.clone();
        self.sort_range(&sheet, rect, Count::<Col>::ZERO, order, HeaderRow::Absent)
    }

    /// Whole-sheet single-column sort by the active cell's column (`trame-sort`,
    /// the "sort by column" affordance for a whole-column selection).
    /// Sorts every materialized row of the sheet across the full used-column span,
    /// so a row's cells outside the active column travel with it. The active
    /// column is passed as `sort_col` over a range anchored at column 0, with no
    /// header row. Delegates to [`sort_range`](Self::sort_range), which guards the
    /// empty range and replies with [`TrameEvent::SortComplete`].
    pub(super) fn sort_sheet(&mut self, order: SortOrder) -> TrameOutcome {
        let rect = CellRect::spanning(
            CellPos::ORIGIN,
            CellPos::new(
                Row::len(&self.index).last().unwrap_or(Pos::<Row>::ZERO),
                Col::len(&self.index).last().unwrap_or(Pos::<Col>::ZERO),
            ),
        );
        let sort_col = self.active.col.count_from(Pos::<Col>::ZERO);
        let sheet = self.active_sheet.clone();
        self.sort_range(&sheet, rect, sort_col, order, HeaderRow::Absent)
    }

    /// Multi-column sort over the tracked selection rectangle (`trame-sort`,
    /// the "Sort range..." dialog). Parameterless sibling of
    /// [`sort_selection`](Self::sort_selection): resolves the range against the
    /// tracked selection and delegates to
    /// [`sort_range_multi`](Self::sort_range_multi) on the active sheet. Keys are
    /// in decreasing priority; each `col` is range-relative (`0` = the selection's
    /// first column).
    pub(super) fn sort_selection_multi(
        &mut self,
        sort_keys: &[SortKey],
        header: HeaderRow,
    ) -> TrameOutcome {
        let rect = self.selection_bounds();
        let sheet = self.active_sheet.clone();
        self.sort_range_multi(&sheet, rect, sort_keys, header)
    }

    /// Drop every cell and formatting entry whose key references the removed
    /// row/column id, so a deleted line leaves no orphan entries behind.
    /// `true` when the removed column carried the active filter, which went
    /// with it.
    pub(super) fn purge_axis(&mut self, removed: Uid, axis: Axis) -> bool {
        for root in [SheetRoot::Cells, SheetRoot::CellStyle] {
            let map = self
                .doc
                .get_map(SheetContainer::of(root, &self.active_sheet).as_str());
            let LoroValue::Map(entries) = map.get_value() else {
                continue;
            };
            let stale: Vec<String> = entries
                .keys()
                .filter(|k| {
                    parse_cell_key(k).is_some_and(|(rl, cl)| match axis {
                        Axis::Row => rl == removed,
                        Axis::Col => cl == removed,
                    })
                })
                .cloned()
                .collect();
            for key in stale {
                let _ = map.delete(&key);
            }
        }
        // A filter on the removed column goes with it (`TRAME-FEAT-030`).
        let filter_dropped = matches!(axis, Axis::Col)
            && self
                .load_active_filter()
                .is_some_and(|(col, _)| col == removed);
        if filter_dropped {
            self.drop_filter();
        }
        // The line's own formatting is keyed by its uid alone: its size, and
        // for a row its filter mark (`TRAME-082`).
        let line_maps = match axis {
            Axis::Row => vec![SheetRoot::RowHeights, SheetRoot::FilterHidden],
            Axis::Col => vec![SheetRoot::ColWidths],
        };
        let key = removed.to_key_part();
        for root in line_maps {
            let map = self
                .doc
                .get_map(SheetContainer::of(root, &self.active_sheet).as_str());
            if map.get(&key).is_some() {
                let _ = map.delete(&key);
            }
        }
        filter_dropped
    }

    /// Shared tail of every structural op. It rebuilds the position <-> id
    /// cache, since positions shifted. It reseeds the formula engine, which
    /// reads each stored line at its new position, and the formatting sets
    /// from the new layout. It then flushes
    /// the CRDT delta and asks the UI to re-fetch its viewport.
    pub(super) fn after_structural(&mut self) -> TrameOutcome {
        self.rebuild_index();
        // Merges resolve through the freshly rebuilt index; do this before the
        // engine reseed so masked cells are excluded from the DAG.
        let mut display = self.rebuild_merges();
        self.reseed_engine();
        self.reseed_styles();
        // The hidden set is keyed by stable row uid, so a structural shift just
        // remaps it to the new positions (no recompute).
        self.rebuild_filter();
        // So are the sizes: each follows its own line (`TRAME-170`,
        // `TRAME-UI-037`).
        self.reseed_metrics();
        let update = self.flush_delta();
        display.push(TrameEvent::DataReset);
        TrameOutcome {
            display,
            update,
            ephemeral: None,
        }
    }
}
