// ambre-trame/src/logic/sort.rs.rs
// Single-column range/sheet sort
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;

impl TrameState {
    /// Single-column sort of a row range (`ambre-spec-trame-features.md` §3).
    ///
    /// Reorders whole rows of the active sheet by the typed value in column
    /// `col_start + sort_col`. Cells of a row outside `[col_start..=col_end]`
    /// travel with it (Google Sheets "sort range").
    ///
    /// **Realized as a pure row-order permutation of `rows_{sheet}`**: each
    /// sortable row is keyed by its own move-invariant `Uid` (the list element
    /// *value*, not a Loro slot id), and the desired order is replayed with
    /// `LoroMovableList::mov`. Cells keyed by `(row_uid, col_uid)` travel with
    /// their row for free -- no cell content is read or rewritten (§2.A, §12).
    ///
    /// References then **follow the data**: the formula engine's bytecode is
    /// uid-anchored, so the sort preserves it (no reseed/re-parse) and merely
    /// recomputes -- every `A1` resolves to its row/column's new position, the
    /// section 12 guarantee. The `mov` ops produce a CRDT delta like any edit;
    /// LWW on the row order resolves concurrent sorts deterministically.
    ///
    /// Sorting only ever targets the **active sheet** (the UI sends an
    /// [`ActivateSheet`](TrameCommand::ActivateSheet) before any range op): the
    /// formula cache the sort keys read from is pinned to index 0. A command for
    /// any other sheet, or an empty/virtual range, is a no-op.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sort_range(
        &mut self,
        sheet_id: &str,
        row_start: u32,
        row_end: u32,
        col_start: u32,
        col_end: u32,
        sort_col: u32,
        ascending: bool,
        has_header: bool,
    ) -> TrameOutcome {
        // Only the active sheet has its values positioned in the engine cache.
        if sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        // Clamp the range to the materialized rows; virtual rows carry no cell
        // data to reorder and no value to sort on.
        let last = self.index.row_len().saturating_sub(1) as u32;
        let row_end = row_end.min(last);
        if col_start + sort_col > col_end || row_start > row_end {
            return TrameOutcome::display_only(vec![]);
        }
        let sort_col_abs = col_start + sort_col;
        // First sortable position: the header row (if any) stays pinned in place.
        let sort_start = if has_header { row_start + 1 } else { row_start };
        if sort_start >= row_end {
            // Zero or one sortable row: nothing to reorder.
            return TrameOutcome::display_only(vec![]);
        }

        // Key each sortable row by its uid and the evaluated value of its sort
        // column. `evaluated_value` returns the formula result or parsed literal;
        // an empty cell sorts as `Empty` (last). Whole rows travel by uid, so no
        // cell contents are read or rewritten -- the sort is a pure permutation.
        let mut keyed: Vec<(Uid, CellValue)> = (sort_start..=row_end)
            .filter_map(|pos| {
                let row_uid = self.index.row_id(pos as usize)?;
                let value = self
                    .engine
                    .evaluated_value(self.active_cell_id(pos, sort_col_abs))
                    .unwrap_or(CellValue::Empty);
                Some((row_uid, value))
            })
            .collect();

        // Current uid order of the sortable region, to detect an already-sorted
        // no-op after the stable sort below.
        let before: Vec<Uid> = keyed.iter().map(|(uid, _)| *uid).collect();
        keyed.sort_by(|(_, a), (_, b)| {
            let ord = compare_cell_values(a, b);
            if ascending { ord } else { ord.reverse() }
        });
        let target: Vec<Uid> = keyed.into_iter().map(|(uid, _)| uid).collect();
        self.realize_sort(sort_start, &before, target)
    }

    /// Multi-column sort of a row range (`ambre-spec-trame-features.md` §2).
    ///
    /// Same pure row-order permutation as [`sort_range`](Self::sort_range), only
    /// the comparator changes: rows are ordered lexicographically by `sort_keys`
    /// in decreasing priority (`[primary, secondary, ...]`), each key applying its
    /// own ascending/descending direction. Each `SortKey::col` is an index within
    /// the range (`0` = `col_start`). Keys whose column falls outside the range are
    /// skipped. Cells follow their move-invariant row `Uid`, references follow the
    /// data (§12); LWW resolves concurrent sorts.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sort_range_multi(
        &mut self,
        sheet_id: &str,
        row_start: u32,
        row_end: u32,
        col_start: u32,
        col_end: u32,
        sort_keys: &[SortKey],
        has_header: bool,
    ) -> TrameOutcome {
        // Only the active sheet has its values positioned in the engine cache.
        if sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        // Resolve the keys to absolute columns inside the range; drop out-of-range
        // ones. With no usable key there is nothing to order by.
        let keys: Vec<(u32, bool)> = sort_keys
            .iter()
            .filter(|k| col_start + k.col <= col_end)
            .map(|k| (col_start + k.col, k.ascending))
            .collect();
        if keys.is_empty() {
            return TrameOutcome::display_only(vec![]);
        }
        // Clamp the range to the materialized rows (virtual rows carry no data).
        let last = self.index.row_len().saturating_sub(1) as u32;
        let row_end = row_end.min(last);
        if row_start > row_end {
            return TrameOutcome::display_only(vec![]);
        }
        // First sortable position: the header row (if any) stays pinned in place.
        let sort_start = if has_header { row_start + 1 } else { row_start };
        if sort_start >= row_end {
            // Zero or one sortable row: nothing to reorder.
            return TrameOutcome::display_only(vec![]);
        }

        // Key each sortable row by its uid and the evaluated value in every sort
        // column. Whole rows travel by uid, so no cell contents are rewritten.
        let mut keyed: Vec<(Uid, Vec<CellValue>)> = (sort_start..=row_end)
            .filter_map(|pos| {
                let row_uid = self.index.row_id(pos as usize)?;
                let values = keys
                    .iter()
                    .map(|&(col, _)| {
                        self.engine
                            .evaluated_value(self.active_cell_id(pos, col))
                            .unwrap_or(CellValue::Empty)
                    })
                    .collect();
                Some((row_uid, values))
            })
            .collect();

        let before: Vec<Uid> = keyed.iter().map(|(uid, _)| *uid).collect();
        keyed.sort_by(|(_, a), (_, b)| {
            // Lexicographic over the keys: first non-equal column decides, each
            // key applying its own direction.
            for (i, (_, ascending)) in keys.iter().enumerate() {
                let ord = compare_cell_values(&a[i], &b[i]);
                let ord = if *ascending { ord } else { ord.reverse() };
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            std::cmp::Ordering::Equal
        });
        let target: Vec<Uid> = keyed.into_iter().map(|(uid, _)| uid).collect();
        self.realize_sort(sort_start, &before, target)
    }

    /// Realize a computed row permutation as `mov` ops on `rows_{sheet}` and
    /// refresh the derived caches. Shared by the single- and multi-column sorts:
    /// `before` is the current uid order of the sortable region, `target` the
    /// desired one (same multiset), and `sort_start` the first sortable position.
    /// Returns [`TrameEvent::SortComplete`] alone (no CRDT delta) when the order
    /// is already correct.
    fn realize_sort(&mut self, sort_start: u32, before: &[Uid], target: Vec<Uid>) -> TrameOutcome {
        if before == target {
            // Stable sort left the order untouched: acknowledge, no CRDT delta.
            return TrameOutcome::display_only(vec![TrameEvent::SortComplete]);
        }

        // Realize the permutation as `mov` ops on `rows_{sheet}`: cells keyed by
        // the move-invariant uid follow for free (§2.A, §12). Move each desired
        // uid into place left-to-right over the sortable region; `cur` mirrors the
        // list's evolving order so the `mov` indices stay correct.
        let list = self
            .doc
            .get_movable_list(rows_list_name(&self.active_sheet));
        let mut cur: Vec<Uid> = self.index.row_ids().to_vec();
        let base = sort_start as usize;
        for (offset, want) in target.into_iter().enumerate() {
            let dst = base + offset;
            if cur[dst] == want {
                continue;
            }
            // The wanted uid is always at or after `dst` (earlier slots are fixed).
            let src = cur[dst..].iter().position(|u| *u == want).map(|p| p + dst);
            let Some(src) = src else { continue };
            let _ = list.mov(src, dst);
            let u = cur.remove(src);
            cur.insert(dst, u);
        }
        self.doc.commit();

        let mut outcome = self.after_reorder();
        outcome.display.push(TrameEvent::SortComplete);
        outcome
    }

    /// Shared tail of any whole-line reorder (sort, drag move): refresh the
    /// position<->uid cache and the position-keyed derived caches (merges,
    /// formatting, filter) for the new order. Crucially, do NOT reseed (re-parse)
    /// the formula engine: the compiled bytecode is uid-anchored, so preserving it
    /// and merely recomputing lets every reference resolve to its row/column's NEW
    /// position -- references follow the moved data, the section 12 guarantee. A
    /// full reseed would re-bind each `A1` against the post-move index and collapse
    /// the engine back to positional semantics.
    pub(super) fn after_reorder(&mut self) -> TrameOutcome {
        self.rebuild_index();
        let mut display = self.rebuild_merges();
        self.reseed_formatting();
        self.reseed_styles();
        // The hidden set is keyed by row uid, so it follows the rows across the
        // permutation -- remap positions, do not recompute.
        self.rebuild_filter();
        let indices = self.engine_indices();
        self.engine.mark_all_dirty();
        self.engine.compute_all_dirty(&indices);
        let update = self.flush_delta();
        display.push(TrameEvent::DataReset);
        TrameOutcome {
            display,
            update,
            ephemeral: None,
        }
    }
}
