// ambre-trame/src/logic/merges.rs.rs
// Merge / unmerge and merge rebuild
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;

impl TrameState {
    /// Merge `rect` into one cell. Rejected locally if a covered cell is
    /// non-empty (`trame-cell-merges`); otherwise records the master->extent
    /// entry in `merges_{sheet}` keyed by stable ids.
    pub(super) fn merge_cells(&mut self, rect: CellRect) -> TrameOutcome {
        // A single cell is not a merge.
        if rect.is_single_cell() {
            return TrameOutcome::display_only(vec![]);
        }
        // Client-side rule: every covered cell (excluding the master) must be
        // empty locally, matching Excel/Sheets. Cannot prevent a later
        // concurrent conflict -- that path masks instead (rebuild_merges).
        let full = rect
            .cells()
            .filter(|&at| at != rect.top_left)
            .find(|&at| self.cell_has_raw(at));
        if let Some(at) = full {
            return TrameOutcome::display_only(vec![TrameEvent::MergeRejected {
                reason: format!(
                    "cell {}{} is not empty",
                    col_to_alpha(at.col),
                    row_label(at.row)
                ),
            }]);
        }
        self.insert_merge_entry(rect);
        self.doc.commit();
        self.after_merge_change()
    }

    /// Record one master->extent merge entry in `merges_{sheet}` for `rect`. Shared
    /// by the context-menu merge and the merge-aware autofill
    /// (`trame-autofill`); does not
    /// commit or rebuild the derived caches -- the caller batches those.
    pub(super) fn insert_merge_entry(&mut self, rect: CellRect) {
        // Materialize the bottom-right corner so the whole rectangle has ids.
        if self.materialize(rect.bottom_right).is_none() {
            return;
        }
        let (Some(master), Some(extent)) = (
            self.index.resolve(rect.top_left),
            self.index.resolve(rect.bottom_right),
        ) else {
            return;
        };
        let _ = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Merges, &self.active_sheet).as_str())
            .insert(
                CellKey::of(master.0, master.1).as_str(),
                CellKey::of(extent.0, extent.1).as_str(),
            );
    }

    /// Split the merge whose rectangle contains `at`, restoring covered
    /// cells (and any masked value). A no-op when the cell is not in a merge.
    pub(super) fn unmerge_cells(&mut self, at: CellPos) -> TrameOutcome {
        if !self.delete_merge_at(at) {
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
        let mut freed = self.selection_bounds();
        // Masters whose rectangle intersects the selection. Snapshot first: the
        // deletion mutates the CRDT, but `self.merges` stays put until rebuild.
        let blocks: Vec<CellRect> = self
            .merges
            .iter()
            .map(|(&master, &end)| CellRect::spanning(master, end))
            .filter(|block| block.overlaps(freed))
            .collect();
        let mut deleted = false;
        for block in blocks {
            if self.delete_merge_at(block.top_left) {
                deleted = true;
                // Grow the post-unmerge selection over every freed block. A
                // merge straddling the selection edge extends past it, and the
                // user needs every cell the split exposed, not the drag rect.
                freed = freed.union(block);
            }
        }
        if !deleted {
            return TrameOutcome::display_only(vec![]);
        }
        self.doc.commit();
        let mut outcome = self.after_merge_change();
        outcome
            .display
            .push(TrameEvent::SelectRange { rect: freed });
        outcome
    }

    /// Delete the merge entry whose rectangle contains `at` from the
    /// `merges_{sheet}` map. Returns whether an entry was removed; does not
    /// commit or rebuild the derived caches -- the caller batches those.
    fn delete_merge_at(&mut self, at: CellPos) -> bool {
        let Some(master) = self.merges.iter().find_map(|(&master, &end)| {
            CellRect::spanning(master, end)
                .contains(at)
                .then_some(master)
        }) else {
            return false;
        };
        let Some((rl, cl)) = self.index.resolve(master) else {
            return false;
        };
        self.doc
            .get_map(SheetContainer::of(SheetRoot::Merges, &self.active_sheet).as_str())
            .delete(CellKey::of(rl, cl).as_str())
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

    /// Whether the grid cell `at` holds a non-empty string value in the CRDT.
    /// Resolves the position to its stable id pair; a virtual cell has no value.
    pub(super) fn cell_has_raw(&self, at: CellPos) -> bool {
        has_raw_value(self.index.resolve(at).and_then(|(rl, cl)| {
            self.doc
                .get_map(SheetContainer::of(SheetRoot::Cells, &self.active_sheet).as_str())
                .get(CellKey::of(rl, cl).as_str())
                .map(|v| v.get_deep_value())
        }))
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
        for block in placed {
            let master = block.top_left;
            self.merges.insert(master, block.bottom_right);
            for at in block.cells().filter(|&at| at != master) {
                self.covered.insert(at);
                // Mask a covered cell that still holds a value (concurrent
                // conflict): keep the data, exclude it from the engine.
                if self.cell_has_raw(at) {
                    if let Some((rl, cl)) = self.index.resolve(at) {
                        self.masked_cells.insert(CellKey::of(rl, cl));
                    }
                    events.push(TrameEvent::MergeConflictDetected {
                        master_row: master.row,
                        master_col: master.col,
                        covered_row: at.row,
                        covered_col: at.col,
                    });
                }
            }
        }
        events
    }

    /// The merge rectangles of `sheet_id` that survive overlap resolution, in
    /// placement order, in grid positions. The walk
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
    pub(super) fn resolved_merges(&self, sheet_id: &SheetId, index: &SheetIndex) -> Vec<CellRect> {
        let LoroValue::Map(entries) = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Merges, sheet_id).as_str())
            .get_value()
        else {
            return Vec::new();
        };
        // Collect resolvable rectangles tagged with their master key for a
        // deterministic, peer-independent overlap precedence.
        let mut rects: Vec<(String, CellRect)> = entries
            .iter()
            .filter_map(|(master_key, value)| {
                let LoroValue::String(extent_key) = value else {
                    return None;
                };
                let (mr, mc) = parse_cell_key(master_key)?;
                let (er, ec) = parse_cell_key(extent_key)?;
                let rect = CellRect::spanning(index.pos_of(&mr, &mc)?, index.pos_of(&er, &ec)?);
                // A peer may write an entry of one cell. It is no merge
                // (`TRAME-128`), so it masks nothing and draws no block.
                (!rect.is_single_cell()).then(|| (master_key.to_string(), rect))
            })
            .collect();
        rects.sort_by(|a, b| a.0.cmp(&b.0));

        let mut placed = Vec::with_capacity(rects.len());
        let mut occupied: HashSet<CellPos> = HashSet::new();
        for (_, rect) in rects {
            if rect.cells().any(|cell| occupied.contains(&cell)) {
                continue;
            }
            occupied.extend(rect.cells());
            placed.push(rect);
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
    pub(super) fn masked_cells_for(
        &self,
        sheet_id: &SheetId,
        index: &SheetIndex,
    ) -> HashSet<CellKey> {
        let cells = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Cells, sheet_id).as_str());
        self.resolved_merges(sheet_id, index)
            .into_iter()
            .flat_map(|block| {
                block
                    .cells()
                    // The master carries the value; only what it covers is masked.
                    .filter(move |&at| at != block.top_left)
            })
            .filter_map(|at| index.resolve(at))
            .map(|(rl, cl)| CellKey::of(rl, cl))
            // A covered cell that still holds a value is a concurrent conflict:
            // mask it (keep the data, exclude it from the DAG).
            .filter(|key| has_raw_value(cells.get(key.as_str()).map(|v| v.get_deep_value())))
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
