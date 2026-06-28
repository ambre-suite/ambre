// ambre-trame/src/logic/structural.rs.rs
// Row/column insert/delete, axis purge, structural reseed
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;

impl TrameState {
    /// Insert a blank row/column at grid position `at`, shifting later elements
    /// down/right. One `LoroMovableList` op; cells keyed by stable id pairs
    /// follow their element automatically. A no-op past the materialized used
    /// range (those rows/columns are already virtual blanks, nothing to shift).
    pub(super) fn insert_axis(&mut self, at: u32, axis: Axis) -> TrameOutcome {
        let at = at as usize;
        let used = match axis {
            Axis::Row => self.index.row_len(),
            Axis::Col => self.index.col_len(),
        };
        if at > used {
            return TrameOutcome::display_only(vec![]);
        }
        let list_name = match axis {
            Axis::Row => rows_list_name(&self.active_sheet),
            Axis::Col => cols_list_name(&self.active_sheet),
        };
        // The new line carries a freshly minted, move-invariant uid value (§2.A).
        let uid = self.mint_uid();
        if self
            .doc
            .get_movable_list(list_name)
            .insert(at, uid.to_key_part().as_str())
            .is_err()
        {
            return TrameOutcome::display_only(vec![]);
        }
        self.doc.commit();
        self.after_structural()
    }

    /// Delete the row/column at grid position `at`, shifting later elements
    /// up/left and purging the removed element's cells and formatting. Any
    /// formula whose referenced cell was below the deletion now reads the
    /// shifted (or emptied) position -- references are not rewritten
    /// (`ambre-spec-trame-logic.md` §12). Full id-based `#REF!` on delete is a
    /// separate broken-references story. A no-op in virtual space.
    pub(super) fn delete_axis(&mut self, at: u32, axis: Axis) -> TrameOutcome {
        let at = at as usize;
        let removed = match axis {
            Axis::Row => self.index.row_id(at),
            Axis::Col => self.index.col_id(at),
        };
        let Some(removed) = removed else {
            return TrameOutcome::display_only(vec![]); // virtual: nothing materialized
        };
        let list_name = match axis {
            Axis::Row => rows_list_name(&self.active_sheet),
            Axis::Col => cols_list_name(&self.active_sheet),
        };
        if self.doc.get_movable_list(list_name).delete(at, 1).is_err() {
            return TrameOutcome::display_only(vec![]);
        }
        self.purge_axis(removed, axis);
        self.doc.commit();
        self.after_structural()
    }

    /// Delete every row/column spanned by the current selection rectangle,
    /// iterating from the far edge inward so earlier positions stay valid as the
    /// list shifts. Each line is its own [`delete_axis`](Self::delete_axis) commit
    /// (hence its own undo item, matching `-ui.md` sec.5.C); the final outcome
    /// carries the refreshed display. The selection is left untouched -- the UI
    /// owns it and re-reports it on the next cursor move.
    pub(super) fn delete_selected_axis(&mut self, axis: Axis) -> TrameOutcome {
        let (r1, c1, r2, c2) = self.selection;
        let (start, end) = match axis {
            Axis::Row => (r1, r2),
            Axis::Col => (c1, c2),
        };
        let mut outcome = TrameOutcome::display_only(vec![]);
        for at in (start..=end).rev() {
            outcome = self.delete_axis(at, axis);
        }
        outcome
    }

    /// Single-column shared sort resolved against the tracked selection
    /// rectangle (`-ui.md` sec.5.C). Sorts on the range's first column with no
    /// header row, exactly the parameterless context-menu intent: whole rows are
    /// permuted so cells outside the selection move with their row (Google Sheets
    /// "sort range"). Delegates to [`sort_range`](Self::sort_range) on the active
    /// sheet, which guards the empty/single-row cases and replies with
    /// [`TrameEvent::SortComplete`].
    pub(super) fn sort_selection(&mut self, ascending: bool) -> TrameOutcome {
        let (r1, c1, r2, c2) = self.selection;
        let sheet = self.active_sheet.clone();
        self.sort_range(&sheet, r1, r2, c1, c2, 0, ascending, false)
    }

    /// Whole-sheet single-column sort by the active cell's column (`-ui.md`
    /// sec.5.C, the "sort by column" affordance for a whole-column selection).
    /// Sorts every materialized row of the sheet across the full used-column span,
    /// so a row's cells outside the active column travel with it. The active
    /// column is passed as `sort_col` over a range anchored at column 0, with no
    /// header row. Delegates to [`sort_range`](Self::sort_range), which guards the
    /// empty range and replies with [`TrameEvent::SortComplete`].
    pub(super) fn sort_sheet(&mut self, ascending: bool) -> TrameOutcome {
        let row_end = self.index.row_len().saturating_sub(1) as u32;
        let col_end = self.index.col_len().saturating_sub(1) as u32;
        let sort_col = self.active.1;
        let sheet = self.active_sheet.clone();
        self.sort_range(&sheet, 0, row_end, 0, col_end, sort_col, ascending, false)
    }

    /// Multi-column sort over the tracked selection rectangle (`-ui.md` sec.5.C,
    /// the "Sort range..." dialog). Parameterless sibling of
    /// [`sort_selection`](Self::sort_selection): resolves the range against the
    /// tracked selection and delegates to
    /// [`sort_range_multi`](Self::sort_range_multi) on the active sheet. Keys are
    /// in decreasing priority; each `col` is range-relative (`0` = the selection's
    /// first column).
    pub(super) fn sort_selection_multi(
        &mut self,
        sort_keys: &[SortKey],
        has_header: bool,
    ) -> TrameOutcome {
        let (r1, c1, r2, c2) = self.selection;
        let sheet = self.active_sheet.clone();
        self.sort_range_multi(&sheet, r1, r2, c1, c2, sort_keys, has_header)
    }

    /// Drop every cell and formatting entry whose key references the removed
    /// row/column id, so a deleted line leaves no orphan entries behind.
    pub(super) fn purge_axis(&mut self, removed: Uid, axis: Axis) {
        for name in [
            cells_map_name(&self.active_sheet),
            format_map_name(&self.active_sheet),
            style_map_name(&self.active_sheet),
            fgcolor_map_name(&self.active_sheet),
            bgcolor_map_name(&self.active_sheet),
            font_map_name(&self.active_sheet),
            numfmt_map_name(&self.active_sheet),
        ]
        .into_iter()
        .chain((0..BORDER_EDGE_ORDER).map(|e| bordcolor_map_name(&self.active_sheet, e)))
        {
            let map = self.doc.get_map(name);
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
    }

    /// Shared tail of every structural op: rebuild the position <-> id cache
    /// (positions shifted), reseed the positional formula engine and the
    /// formatting sets from the new layout, then flush the CRDT delta and ask
    /// the UI to re-fetch its viewport.
    pub(super) fn after_structural(&mut self) -> TrameOutcome {
        self.rebuild_index();
        // Merges resolve through the freshly rebuilt index; do this before the
        // engine reseed so masked cells are excluded from the DAG.
        let mut display = self.rebuild_merges();
        self.reseed_engine();
        self.reseed_formatting();
        self.reseed_styles();
        // The hidden set is keyed by stable row uid, so a structural shift just
        // remaps it to the new positions (no recompute).
        self.rebuild_filter();
        let update = self.flush_delta();
        display.push(TrameEvent::DataReset);
        TrameOutcome {
            display,
            update,
            ephemeral: None,
        }
    }
}
