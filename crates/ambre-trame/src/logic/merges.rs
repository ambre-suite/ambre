// ambre-trame/src/logic/merges.rs.rs
// Merge / unmerge and merge rebuild
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;

impl TrameState {
    /// Merge the range with corners `(r0,c0)`-`(r1,c1)` (given in any order) into
    /// one cell. Rejected locally if a covered cell is non-empty (§6); otherwise
    /// records the master->extent entry in `merges_{sheet}` keyed by stable ids.
    pub(super) fn merge_cells(&mut self, r0: u32, c0: u32, r1: u32, c1: u32) -> TrameOutcome {
        let (top, bottom) = (r0.min(r1), r0.max(r1));
        let (left, right) = (c0.min(c1), c0.max(c1));
        // A single cell is not a merge.
        if top == bottom && left == right {
            return TrameOutcome::display_only(vec![]);
        }
        // Client-side rule: every covered cell (excluding the master) must be
        // empty locally, matching Excel/Sheets. Cannot prevent a later
        // concurrent conflict -- that path masks instead (rebuild_merges).
        for row in top..=bottom {
            for col in left..=right {
                if (row, col) == (top, left) {
                    continue;
                }
                if self.cell_has_raw(row, col) {
                    return TrameOutcome::display_only(vec![TrameEvent::MergeRejected {
                        reason: format!(
                            "cell {}{} is not empty",
                            col_to_alpha(col),
                            row + 1
                        ),
                    }]);
                }
            }
        }
        // Materialize the bottom-right corner so the whole rectangle has ids.
        if self.materialize(bottom, right).is_none() {
            return TrameOutcome::display_only(vec![]);
        }
        let (Some(master), Some(extent)) = (
            self.index.resolve(top as usize, left as usize),
            self.index.resolve(bottom as usize, right as usize),
        ) else {
            return TrameOutcome::display_only(vec![]);
        };
        let _ = self.doc.get_map(merges_map_name(&self.active_sheet)).insert(
            &cell_key(master.0, master.1),
            cell_key(extent.0, extent.1).as_str(),
        );
        self.doc.commit();
        self.after_merge_change()
    }

    /// Split the merge whose rectangle contains `(row, col)`, restoring covered
    /// cells (and any masked value). A no-op when the cell is not in a merge.
    pub(super) fn unmerge_cells(&mut self, row: u32, col: u32) -> TrameOutcome {
        let master = self.merges.iter().find_map(|(&(top, left), &(bottom, right))| {
            (row >= top && row <= bottom && col >= left && col <= right).then_some((top, left))
        });
        let Some((top, left)) = master else {
            return TrameOutcome::display_only(vec![]);
        };
        let Some((rl, cl)) = self.index.resolve(top as usize, left as usize) else {
            return TrameOutcome::display_only(vec![]);
        };
        let _ = self.doc.get_map(merges_map_name(&self.active_sheet)).delete(&cell_key(rl, cl));
        self.doc.commit();
        self.after_merge_change()
    }

    /// Shared tail of merge/unmerge: rebuild the derived merge cache (which also
    /// re-evaluates masking), reseed the formula engine so unmasked cells rejoin
    /// the DAG, flush the delta and ask the UI to re-fetch. Conflict events from
    /// the rebuild ride along with the `DataReset`.
    pub(super) fn after_merge_change(&mut self) -> TrameOutcome {
        let mut display = self.rebuild_merges();
        self.reseed_engine();
        display.push(TrameEvent::DataReset);
        let update = self.flush_delta();
        TrameOutcome { display, update, ephemeral: None }
    }

    /// Whether grid `(row, col)` holds a non-empty string value in the CRDT.
    /// Resolves the position to its stable id pair; a virtual cell has no value.
    pub(super) fn cell_has_raw(&self, row: u32, col: u32) -> bool {
        self.index
            .resolve(row as usize, col as usize)
            .and_then(|(rl, cl)| {
                self.doc
                    .get_map(cells_map_name(&self.active_sheet))
                    .get(&cell_key(rl, cl))
                    .map(|v| v.get_deep_value())
            })
            .is_some_and(|v| matches!(v, LoroValue::String(s) if !s.is_empty()))
    }

    /// Rebuild the derived merge caches ([`Self::merges`], [`Self::covered`],
    /// [`Self::masked_cells`]) from the active sheet's `merges_{sheet}` map.
    /// Requires [`Self::index`] to be current (keys are stable id pairs mapped
    /// back to grid positions). Returns one [`TrameEvent::MergeConflictDetected`]
    /// per masked cell.
    ///
    /// Overlapping merges are resolved deterministically: rectangles are placed
    /// in ascending master-key order and any that intersects an already-placed
    /// one is skipped. The order is identical on every peer (it derives only
    /// from the stable keys), so all clients converge on the same layout without
    /// mutating the CRDT (no corrective op, no conflict loop -- §6).
    pub(super) fn rebuild_merges(&mut self) -> Vec<TrameEvent> {
        self.merges.clear();
        self.covered.clear();
        self.masked_cells.clear();
        let LoroValue::Map(entries) =
            self.doc.get_map(merges_map_name(&self.active_sheet)).get_value()
        else {
            return Vec::new();
        };
        // Collect resolvable rectangles tagged with their master key for a
        // deterministic, peer-independent overlap precedence.
        let mut rects: Vec<MergeRect> = entries
            .iter()
            .filter_map(|(master_key, value)| {
                let LoroValue::String(extent_key) = value else { return None };
                let (mr, mc) = parse_cell_key(master_key)?;
                let (er, ec) = parse_cell_key(extent_key)?;
                let top = self.index.row_pos(&mr)? as u32;
                let left = self.index.col_pos(&mc)? as u32;
                let bottom = self.index.row_pos(&er)? as u32;
                let right = self.index.col_pos(&ec)? as u32;
                Some((master_key.to_string(), (top.min(bottom), left.min(right)), (top.max(bottom), right.max(left))))
            })
            .collect();
        rects.sort_by(|a, b| a.0.cmp(&b.0));

        let mut events = Vec::new();
        let mut occupied: HashSet<(u32, u32)> = HashSet::new();
        for (_key, (top, left), (bottom, right)) in rects {
            let overlaps = (top..=bottom)
                .flat_map(|r| (left..=right).map(move |c| (r, c)))
                .any(|cell| occupied.contains(&cell));
            if overlaps {
                continue;
            }
            self.merges.insert((top, left), (bottom, right));
            for row in top..=bottom {
                for col in left..=right {
                    occupied.insert((row, col));
                    if (row, col) == (top, left) {
                        continue;
                    }
                    self.covered.insert((row, col));
                    // Mask a covered cell that still holds a value (concurrent
                    // conflict): keep the data, exclude it from the engine.
                    if self.cell_has_raw(row, col) {
                        if let Some((rl, cl)) = self.index.resolve(row as usize, col as usize) {
                            self.masked_cells.insert(cell_key(rl, cl));
                        }
                        events.push(TrameEvent::MergeConflictDetected {
                            master_row: top,
                            master_col: left,
                            covered_row: row,
                            covered_col: col,
                        });
                    }
                }
            }
        }
        events
    }

}
