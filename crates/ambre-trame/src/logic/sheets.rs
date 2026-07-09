// ambre-trame/src/logic/sheets.rs.rs
// Sheet lifecycle, named ranges, engine/format reseed
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;

impl TrameState {
    /// Reset the positional formula engine and re-feed it from the active
    /// sheet's cells at their current grid positions (resolved through the
    /// freshly rebuilt [`Self::index`]). Required after any structural op or
    /// import, where a cell's grid position may have changed under stable keys.
    pub(super) fn reseed_engine(&mut self) {
        self.engine.reset();
        // Sheet maps and named ranges must be current before feeding cells, so
        // that `Sheet2!A1` and named references resolve while compiling.
        self.rebuild_sheet_maps();
        self.rebuild_named();

        // Feed every sheet's cells under a uid-anchored [`CellId`], tagged with
        // the sheet's local index. The active sheet is index 0 (see
        // `sheets_by_idx`), so its cells address the same keys the single-sheet
        // read paths use. Each cell's stable key already carries its
        // `(row_uid, col_uid)`, so feeding is a direct `CellId::bound`; a cell
        // whose row/column uid is absent from the sheet index (a deleted line) is
        // skipped.
        let indices = self.engine_indices();
        let sheets: Vec<(String, u16)> = self
            .sheets_by_idx
            .iter()
            .enumerate()
            .map(|(idx, id)| (id.clone(), idx as u16))
            .collect();
        for (sheet_id, idx) in sheets {
            let Some(index) = indices.get(&idx) else {
                continue;
            };
            let LoroValue::Map(cells) = self.doc.get_map(cells_map_name(&sheet_id)).get_value()
            else {
                continue;
            };
            // Masking is derived per sheet from that sheet's `merges_{sheet}` map,
            // so a masked conflicted cell is excluded from the DAG on *every*
            // sheet -- a cross-sheet reference to it converges regardless of the
            // active tab (§6). For the active sheet this equals `self.masked_cells`.
            let masked = self.masked_cells_for(&sheet_id, index);
            for (key, value) in cells.iter() {
                // Masked cells (covered by a merge, conflicting value) stay out of
                // the DAG.
                if masked.contains(key) {
                    continue;
                }
                let (Some((rl, cl)), LoroValue::String(text)) = (parse_cell_key(key), value) else {
                    continue;
                };
                if index.contains_row(&rl) && index.contains_col(&cl) {
                    let ctx = TrameCtx {
                        sheet_name_to_u16: &self.sheet_name_to_u16,
                        sheet_id_to_u16: &self.sheet_id_to_u16,
                        named: &self.named,
                    };
                    self.engine.on_cell_changed(
                        CellId::bound(idx, rl, cl),
                        text.as_str(),
                        &indices,
                        &ctx,
                    );
                }
            }
        }
        self.engine.compute_all_dirty(&indices);
    }

    /// Rebuild [`Self::sheets_by_idx`] and [`Self::sheet_name_to_u16`] from the
    /// `sheets` CRDT map. The active sheet is pinned to local index 0; remaining
    /// sheets follow in sorted-id order. The mapping is local-only (computed
    /// values never leave this client), so the exact ordering need not match
    /// peers -- only stability within this session matters.
    pub(super) fn rebuild_sheet_maps(&mut self) {
        // Collect {id -> name} from the CRDT.
        let mut entries: Vec<(String, String)> = Vec::new();
        if let LoroValue::Map(sheets) = self.doc.get_map(SHEETS_MAP).get_value() {
            for (id, name) in sheets.iter() {
                let name = match name {
                    LoroValue::String(s) => s.to_string(),
                    _ => id.clone(),
                };
                entries.push((id.clone(), name));
            }
        }
        // Active sheet first (index 0), then the rest by id for determinism.
        entries.sort_by(|a, b| {
            let a_active = a.0 == self.active_sheet;
            let b_active = b.0 == self.active_sheet;
            b_active.cmp(&a_active).then_with(|| a.0.cmp(&b.0))
        });

        self.sheets_by_idx.clear();
        self.sheet_name_to_u16.clear();
        self.sheet_id_to_u16.clear();
        self.sheet_id_to_name.clear();
        for (idx, (id, name)) in entries.into_iter().enumerate() {
            self.sheet_name_to_u16.insert(name.clone(), idx as u16);
            self.sheet_id_to_u16.insert(id.clone(), idx as u16);
            self.sheet_id_to_name.insert(id.clone(), name);
            self.sheets_by_idx.push(id);
        }
        // A document always has at least the active sheet at index 0.
        if self.sheets_by_idx.is_empty() {
            self.sheets_by_idx.push(self.active_sheet.clone());
        }
    }

    /// Rebuild [`Self::named`] from the `named_ranges` CRDT map: parse each stored
    /// A1-syntax reference into a concrete cell/range target, resolving any
    /// sheet qualifier against [`Self::sheet_name_to_u16`]. A reference that
    /// fails to parse is dropped (the formula using it surfaces `#PARSE!`).
    pub(super) fn rebuild_named(&mut self) {
        self.named.clear();
        let LoroValue::Map(names) = self.doc.get_map(NAMED_RANGES_MAP).get_value() else {
            return;
        };
        // Names cannot reference other names: resolve sheet qualifiers only.
        let no_names: HashMap<String, NameTarget> = HashMap::new();
        for (name, reference) in names.iter() {
            let LoroValue::String(reference) = reference else {
                continue;
            };
            let ctx = TrameCtx {
                sheet_name_to_u16: &self.sheet_name_to_u16,
                sheet_id_to_u16: &self.sheet_id_to_u16,
                named: &no_names,
            };
            // Unqualified references in a name definition default to the active
            // sheet (index 0).
            let target = parse_formula(reference)
                .and_then(|ast| resolve_refs(ast, 0, &ctx))
                .ok()
                .and_then(expr_to_name_target);
            if let Some(target) = target {
                self.named.insert(name.clone(), target);
            }
        }
    }

    /// Build a fresh [`SheetIndex`] for an arbitrary sheet id (the cross-sheet
    /// reseed needs each sheet's position<->id map, not just the active one).
    pub(super) fn build_sheet_index(&self, sheet_id: &str) -> SheetIndex {
        let rows = self.doc.get_movable_list(rows_list_name(sheet_id));
        let cols = self.doc.get_movable_list(cols_list_name(sheet_id));
        SheetIndex::build(&rows, &cols)
    }

    /// Define or redefine a named range, persist it to the CRDT, and reseed so
    /// formulas using the name pick it up immediately.
    pub(super) fn define_name(&mut self, name: &str, reference: &str) -> TrameOutcome {
        let _ = self.doc.get_map(NAMED_RANGES_MAP).insert(name, reference);
        self.doc.commit();
        self.reseed_engine();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    /// Remove a named range and reseed. A no-op if the name is undefined.
    pub(super) fn delete_name(&mut self, name: &str) -> TrameOutcome {
        let map = self.doc.get_map(NAMED_RANGES_MAP);
        if map.get(name).is_none() {
            return TrameOutcome::display_only(vec![]);
        }
        let _ = map.delete(name);
        self.doc.commit();
        self.reseed_engine();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    // -------------------------------------------------------------------------
    // Sheet lifecycle (create / rename / reorder / delete / activate) -- §2.D
    // -------------------------------------------------------------------------

    /// Every sheet id registered in the `sheets` CRDT map (unordered).
    pub(super) fn all_sheet_ids(&self) -> Vec<String> {
        match self.doc.get_map(SHEETS_MAP).get_value() {
            LoroValue::Map(sheets) => sheets.keys().cloned().collect(),
            _ => Vec::new(),
        }
    }

    /// Canonical tab-strip order, deterministic on every peer: the implicit
    /// default sheet first, then any sheet not yet present in `sheet_order`
    /// (sorted by id -- covers a create whose order op has not merged), then the
    /// explicit `sheet_order` entries that still correspond to a live sheet.
    pub(super) fn sheet_order_ids(&self) -> Vec<String> {
        let listed: Vec<String> = match self.doc.get_movable_list(SHEET_ORDER).get_value() {
            LoroValue::List(items) => items
                .iter()
                .filter_map(|v| match v {
                    LoroValue::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let in_list: HashSet<&String> = listed.iter().collect();
        let mut unlisted: Vec<String> = self
            .all_sheet_ids()
            .into_iter()
            .filter(|id| !in_list.contains(id))
            .collect();
        // Default sheet first, the rest by id for a stable, peer-independent order.
        unlisted.sort_by(|a, b| {
            let a_def = a == DEFAULT_SHEET;
            let b_def = b == DEFAULT_SHEET;
            b_def.cmp(&a_def).then_with(|| a.cmp(b))
        });
        let existing: HashSet<String> = self.all_sheet_ids().into_iter().collect();
        drop(in_list);
        unlisted
            .into_iter()
            .chain(listed.into_iter().filter(|id| existing.contains(id)))
            .collect()
    }

    /// Build the [`TrameEvent::SheetList`] the tab strip consumes, in canonical
    /// order with the active sheet flagged.
    pub(super) fn sheet_list_event(&self) -> TrameEvent {
        let map = self.doc.get_map(SHEETS_MAP);
        let sheets = self
            .sheet_order_ids()
            .into_iter()
            .map(|id| {
                let name = match map.get(&id).map(|v| v.get_deep_value()) {
                    Some(LoroValue::String(s)) => s.to_string(),
                    _ => id.clone(),
                };
                (id, name)
            })
            .collect();
        TrameEvent::SheetList {
            sheets,
            active: self.active_sheet.clone(),
        }
    }

    /// Rebuild every active-sheet derived cache (index, merges, formula engine,
    /// formatting) from the current CRDT state. Used when the active sheet
    /// changes (switch or delete fallback) -- no CRDT mutation, no delta.
    /// Returns merge-conflict events from the rebuild.
    pub(super) fn reseed_active_sheet(&mut self) -> Vec<TrameEvent> {
        self.rebuild_index();
        let conflicts = self.rebuild_merges();
        self.reseed_engine();
        self.reseed_formatting();
        self.reseed_styles();
        self.rebuild_filter();
        self.reseed_metrics();
        conflicts
    }

    /// Switch the active sheet and re-derive its caches (§2.D). A no-op if the id
    /// is unknown or already active.
    pub(super) fn activate_sheet(&mut self, sheet_id: &str) -> TrameOutcome {
        if sheet_id == self.active_sheet || !self.all_sheet_ids().iter().any(|id| id == sheet_id) {
            return TrameOutcome::display_only(vec![]);
        }
        self.active_sheet = sheet_id.to_owned();
        let mut display = self.reseed_active_sheet();
        display.push(TrameEvent::DataReset);
        display.push(self.sheet_list_event());
        display.push(self.style_state_event());
        TrameOutcome::display_only(display)
    }

    /// Add a sheet with a freshly minted peer-scoped id (§2.D). The new sheet is
    /// empty; its containers are created lazily on first write. Does not switch
    /// the active sheet (the UI sends [`TrameCommand::ActivateSheet`] for that).
    pub(super) fn create_sheet(&mut self, name: &str) -> TrameOutcome {
        let id = format!("{:016x}_{:08x}", self.doc.peer_id(), self.sheet_counter);
        self.sheet_counter += 1;
        let _ = self.doc.get_map(SHEETS_MAP).insert(&id, name);
        let _ = self.doc.get_movable_list(SHEET_ORDER).push(id.as_str());
        self.doc.commit();
        // Refresh the name -> index map so the new sheet is addressable.
        self.rebuild_sheet_maps();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![self.sheet_list_event()],
            update,
            ephemeral: None,
        }
    }

    /// Rename a sheet's display label (LWW on the name). The stable id is
    /// untouched. The name -> index map changes, so reseed to recompile any
    /// cross-sheet reference (the display-name binding is the §2.D MVP gap).
    /// A no-op if the id is unknown.
    pub(super) fn rename_sheet(&mut self, sheet_id: &str, name: &str) -> TrameOutcome {
        if !self.all_sheet_ids().iter().any(|id| id == sheet_id) {
            return TrameOutcome::display_only(vec![]);
        }
        let _ = self.doc.get_map(SHEETS_MAP).insert(sheet_id, name);
        self.doc.commit();
        self.reseed_engine();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![self.sheet_list_event(), TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    /// Move a sheet to `to_index` in the tab strip (§2.D). Pure presentation, no
    /// recompute. Materializes the canonical order into `sheet_order` first (the
    /// implicit default and unmerged entries are otherwise absent) so positions
    /// are well-defined, then issues one move op. A no-op if the id is unknown or
    /// already at the target position.
    pub(super) fn move_sheet(&mut self, sheet_id: &str, to_index: u32) -> TrameOutcome {
        let order = self.sheet_order_ids();
        let Some(from) = order.iter().position(|id| id == sheet_id) else {
            return TrameOutcome::display_only(vec![]);
        };
        let to = (to_index as usize).min(order.len().saturating_sub(1));
        if from == to {
            return TrameOutcome::display_only(vec![]);
        }
        let list = self.doc.get_movable_list(SHEET_ORDER);
        // Rewrite the list to the canonical order if it diverges, so `mov`
        // operates on the positions the tab strip actually shows.
        let current: Vec<String> = match list.get_value() {
            LoroValue::List(items) => items
                .iter()
                .filter_map(|v| match v {
                    LoroValue::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        if current != order {
            let _ = list.clear();
            for id in &order {
                let _ = list.push(id.as_str());
            }
        }
        let _ = list.mov(from, to);
        self.doc.commit();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![self.sheet_list_event()],
            update,
            ephemeral: None,
        }
    }

    /// Delete a sheet and its per-sheet containers (§2.D). Rejected locally if it
    /// is the last remaining sheet. If the active sheet is deleted, the active
    /// sheet falls back to the previous tab (else the first remaining). The sheet
    /// set changed, so a full reseed remaps engine indices and lets cross-sheet
    /// formulas targeting the gone id surface `#REF!`.
    pub(super) fn delete_sheet(&mut self, sheet_id: &str) -> TrameOutcome {
        let ids = self.all_sheet_ids();
        if !ids.iter().any(|id| id == sheet_id) {
            return TrameOutcome::display_only(vec![]);
        }
        if ids.len() <= 1 {
            return TrameOutcome::display_only(vec![TrameEvent::Error {
                message: "a workbook must keep at least one sheet".to_owned(),
            }]);
        }
        // Pick the fallback active sheet from the pre-delete order.
        let order = self.sheet_order_ids();
        let list = self.doc.get_movable_list(SHEET_ORDER);
        if let LoroValue::List(items) = list.get_value()
            && let Some(pos) = items
                .iter()
                .position(|v| matches!(v, LoroValue::String(s) if s.as_str() == sheet_id))
        {
            let _ = list.delete(pos, 1);
        }
        let _ = self.doc.get_map(SHEETS_MAP).delete(sheet_id);
        self.drop_sheet_containers(sheet_id);
        self.doc.commit();

        if self.active_sheet == sheet_id {
            let pos = order.iter().position(|id| id == sheet_id).unwrap_or(0);
            self.active_sheet = if pos > 0 {
                order[pos - 1].clone()
            } else {
                order
                    .iter()
                    .find(|id| id.as_str() != sheet_id)
                    .cloned()
                    .unwrap_or_else(|| DEFAULT_SHEET.to_owned())
            };
        }

        let mut display = self.reseed_active_sheet();
        display.push(self.sheet_list_event());
        display.push(TrameEvent::DataReset);
        let update = self.flush_delta();
        TrameOutcome {
            display,
            update,
            ephemeral: None,
        }
    }

    /// Drop every cell, formatting and merge entry of `sheet_id` and clear its
    /// row/column lists, so a deleted sheet leaves no orphan containers behind.
    pub(super) fn drop_sheet_containers(&mut self, sheet_id: &str) {
        for name in [
            cells_map_name(sheet_id),
            format_map_name(sheet_id),
            style_map_name(sheet_id),
            fgcolor_map_name(sheet_id),
            bgcolor_map_name(sheet_id),
            font_map_name(sheet_id),
            numfmt_map_name(sheet_id),
            merges_map_name(sheet_id),
        ]
        .into_iter()
        .chain((0..BORDER_EDGE_ORDER).map(|e| bordcolor_map_name(sheet_id, e)))
        {
            let map = self.doc.get_map(name);
            if let LoroValue::Map(entries) = map.get_value() {
                let keys: Vec<String> = entries.keys().cloned().collect();
                for key in keys {
                    let _ = map.delete(&key);
                }
            }
        }
        for name in [rows_list_name(sheet_id), cols_list_name(sheet_id)] {
            let _ = self.doc.get_movable_list(name).clear();
        }
    }
}
