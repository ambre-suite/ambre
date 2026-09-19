// ambre-trame/src/logic/merges.rs.rs
// Merge / unmerge and merge rebuild
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;

impl TrameState {
    /// Merge the range with corners `(r0,c0)`-`(r1,c1)` (given in any order) into
    /// one cell. Rejected locally if a covered cell is non-empty
    /// (`trame-cell-merges`); otherwise
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
                        reason: format!("cell {}{} is not empty", col_to_alpha(col), row + 1),
                    }]);
                }
            }
        }
        self.insert_merge_entry(top, left, bottom, right);
        self.doc.commit();
        self.after_merge_change()
    }

    /// Record one master->extent merge entry in `merges_{sheet}` for the
    /// rectangle `(top,left)`-`(bottom,right)` (corners already ordered). Shared
    /// by the context-menu merge and the merge-aware autofill
    /// (`trame-autofill`); does not
    /// commit or rebuild the derived caches -- the caller batches those.
    pub(super) fn insert_merge_entry(&mut self, top: u32, left: u32, bottom: u32, right: u32) {
        // Materialize the bottom-right corner so the whole rectangle has ids.
        if self.materialize(bottom, right).is_none() {
            return;
        }
        let (Some(master), Some(extent)) = (
            self.index.resolve(top as usize, left as usize),
            self.index.resolve(bottom as usize, right as usize),
        ) else {
            return;
        };
        let _ = self
            .doc
            .get_map(merges_map_name(&self.active_sheet))
            .insert(
                &cell_key(master.0, master.1),
                cell_key(extent.0, extent.1).as_str(),
            );
    }

    /// Split the merge whose rectangle contains `(row, col)`, restoring covered
    /// cells (and any masked value). A no-op when the cell is not in a merge.
    pub(super) fn unmerge_cells(&mut self, row: u32, col: u32) -> TrameOutcome {
        if !self.delete_merge_at(row, col) {
            return TrameOutcome::display_only(vec![]);
        }
        self.doc.commit();
        self.after_merge_change()
    }

    /// Split every merge intersecting the tracked selection rectangle. The
    /// context-menu intent resolves against the whole selection, not just the
    /// active cell, so one step splits every merge the selection touches.
    pub(super) fn unmerge_selection(&mut self) -> TrameOutcome {
        // Bounding box of every selected rectangle (`TRAME-UI-129`).
        let (mut r1, mut c1, mut r2, mut c2) = self.selection_bounds();
        // Masters whose rectangle intersects the selection. Snapshot first: the
        // deletion mutates the CRDT, but `self.merges` stays put until rebuild.
        let masters: Vec<(u32, u32, u32, u32)> = self
            .merges
            .iter()
            .filter_map(|(&(top, left), &(bottom, right))| {
                let intersects = top <= r2 && bottom >= r1 && left <= c2 && right >= c1;
                intersects.then_some((top, left, bottom, right))
            })
            .collect();
        let mut deleted = false;
        for (top, left, bottom, right) in masters {
            if self.delete_merge_at(top, left) {
                deleted = true;
                // Grow the post-unmerge selection over every freed block. A
                // merge straddling the selection edge extends past it, and the
                // user needs every cell the split exposed, not the drag rect.
                r1 = r1.min(top);
                c1 = c1.min(left);
                r2 = r2.max(bottom);
                c2 = c2.max(right);
            }
        }
        if !deleted {
            return TrameOutcome::display_only(vec![]);
        }
        self.doc.commit();
        let mut outcome = self.after_merge_change();
        outcome
            .display
            .push(TrameEvent::SelectRange { r1, c1, r2, c2 });
        outcome
    }

    /// Delete the merge entry whose rectangle contains `(row, col)` from the
    /// `merges_{sheet}` map. Returns whether an entry was removed; does not
    /// commit or rebuild the derived caches -- the caller batches those.
    fn delete_merge_at(&mut self, row: u32, col: u32) -> bool {
        let Some((top, left)) = self
            .merges
            .iter()
            .find_map(|(&(top, left), &(bottom, right))| {
                (row >= top && row <= bottom && col >= left && col <= right).then_some((top, left))
            })
        else {
            return false;
        };
        let Some((rl, cl)) = self.index.resolve(top as usize, left as usize) else {
            return false;
        };
        self.doc
            .get_map(merges_map_name(&self.active_sheet))
            .delete(&cell_key(rl, cl))
            .is_ok()
    }

    /// Shared tail of merge/unmerge. It rebuilds the derived merge cache (which
    /// also re-evaluates masking) and reseeds the formula engine, so unmasked
    /// cells rejoin the DAG. It then flushes the delta and asks the UI to
    /// re-fetch. Conflict events from the rebuild ride along with the
    /// `DataReset`.
    pub(super) fn after_merge_change(&mut self) -> TrameOutcome {
        let mut display = self.rebuild_merges();
        self.reseed_engine();
        display.push(TrameEvent::DataReset);
        let update = self.flush_delta();
        TrameOutcome {
            display,
            update,
            ephemeral: None,
        }
    }

    /// Whether grid `(row, col)` holds a non-empty string value in the CRDT.
    /// Resolves the position to its stable id pair; a virtual cell has no value.
    pub(super) fn cell_has_raw(&self, row: u32, col: u32) -> bool {
        has_raw_value(
            self.index
                .resolve(row as usize, col as usize)
                .and_then(|(rl, cl)| {
                    self.doc
                        .get_map(cells_map_name(&self.active_sheet))
                        .get(&cell_key(rl, cl))
                        .map(|v| v.get_deep_value())
                }),
        )
    }

    /// Rebuild the derived merge caches ([`Self::merges`], [`Self::covered`],
    /// [`Self::masked_cells`]) from the active sheet's `merges_{sheet}` map.
    /// Requires [`Self::index`] to be current (keys are stable id pairs mapped
    /// back to grid positions). Returns one [`TrameEvent::MergeConflictDetected`]
    /// per masked cell.
    ///
    /// Overlap resolution is deterministic: the walk places rectangles in
    /// ascending master-key order and skips any that intersects one already
    /// placed. That order derives from the stable keys alone, so every peer
    /// reaches one layout and mutates nothing -- no corrective op, no conflict
    /// loop (`trame-cell-merges`).
    pub(super) fn rebuild_merges(&mut self) -> Vec<TrameEvent> {
        self.merges.clear();
        self.covered.clear();
        self.masked_cells.clear();
        let placed = self.resolved_merges(&self.active_sheet, &self.index);

        let mut events = Vec::new();
        for ((top, left), (bottom, right)) in placed {
            self.merges.insert((top, left), (bottom, right));
            for row in top..=bottom {
                for col in left..=right {
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

    /// The merge rectangles of `sheet_id` that survive overlap resolution, in
    /// placement order, as `(master, bottom-right)` grid positions. The walk
    /// takes entries in ascending master-key order and skips any rectangle
    /// meeting one already placed. That key is stable and that order derives
    /// from the document alone. Every peer resolves one layout, and none
    /// mutates the CRDT -- no corrective op, no conflict loop
    /// (`trame-cell-merges`, `TRAME-126`, `TRAME-144`).
    ///
    /// Sheet-parametric and side-effect-free. Both consumers -- the display
    /// caches ([`Self::rebuild_merges`]) and the formula feed
    /// ([`Self::masked_cells_for`]) -- therefore read one answer instead of
    /// each deriving its own. A second copy of this precedence would let a
    /// masked cell differ from the block the grid draws.
    pub(super) fn resolved_merges(
        &self,
        sheet_id: &str,
        index: &SheetIndex,
    ) -> Vec<((u32, u32), (u32, u32))> {
        let LoroValue::Map(entries) = self.doc.get_map(merges_map_name(sheet_id)).get_value()
        else {
            return Vec::new();
        };
        // Collect resolvable rectangles tagged with their master key for a
        // deterministic, peer-independent overlap precedence.
        let mut rects: Vec<MergeRect> = entries
            .iter()
            .filter_map(|(master_key, value)| {
                let LoroValue::String(extent_key) = value else {
                    return None;
                };
                let (mr, mc) = parse_cell_key(master_key)?;
                let (er, ec) = parse_cell_key(extent_key)?;
                let top = index.row_pos(&mr)? as u32;
                let left = index.col_pos(&mc)? as u32;
                let bottom = index.row_pos(&er)? as u32;
                let right = index.col_pos(&ec)? as u32;
                Some((
                    master_key.to_string(),
                    (top.min(bottom), left.min(right)),
                    (top.max(bottom), right.max(left)),
                ))
            })
            .collect();
        rects.sort_by(|a, b| a.0.cmp(&b.0));

        let mut placed = Vec::with_capacity(rects.len());
        let mut occupied: HashSet<(u32, u32)> = HashSet::new();
        for (_key, (top, left), (bottom, right)) in rects {
            let cells = (top..=bottom).flat_map(|r| (left..=right).map(move |c| (r, c)));
            if cells.clone().any(|cell| occupied.contains(&cell)) {
                continue;
            }
            occupied.extend(cells);
            placed.push(((top, left), (bottom, right)));
        }
        placed
    }

    /// Compute the masked-cell key set for an arbitrary sheet, using `index` for
    /// position<->id resolution. The masking half of the merge rebuild, over the
    /// same [`Self::resolved_merges`] the display caches read: it touches no
    /// active-sheet cache and emits no event. The formula engine feed
    /// ([`Self::reseed_engine`]) uses it to exclude masked cells uniformly on
    /// every sheet, not only the displayed one. A cross-sheet reference to a
    /// masked conflicted cell therefore reads the same value regardless of the
    /// active tab (`trame-cell-merges`, `TRAME-020`). For the active sheet (fed
    /// with [`Self::index`]) it returns exactly [`Self::masked_cells`].
    pub(super) fn masked_cells_for(&self, sheet_id: &str, index: &SheetIndex) -> HashSet<String> {
        let cells = self.doc.get_map(cells_map_name(sheet_id));
        self.resolved_merges(sheet_id, index)
            .into_iter()
            .flat_map(|((top, left), (bottom, right))| {
                (top..=bottom)
                    .flat_map(move |row| (left..=right).map(move |col| (row, col)))
                    // The master carries the value; only what it covers is masked.
                    .filter(move |&pos| pos != (top, left))
            })
            .filter_map(|(row, col)| index.resolve(row as usize, col as usize))
            .map(|(rl, cl)| cell_key(rl, cl))
            // A covered cell that still holds a value is a concurrent conflict:
            // mask it (keep the data, exclude it from the DAG).
            .filter(|key| has_raw_value(cells.get(key).map(|v| v.get_deep_value())))
            .collect()
    }
}

/// Whether a stored cell value counts as holding something. The one place that
/// test lives. The display path ([`TrameState::cell_has_raw`]) and the formula
/// feed ([`TrameState::masked_cells_for`]) therefore cannot disagree about
/// which covered cell is a concurrent conflict.
fn has_raw_value(value: Option<LoroValue>) -> bool {
    value.is_some_and(|v| matches!(v, LoroValue::String(s) if !s.is_empty()))
}
