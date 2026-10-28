// ambre-trame/src/logic/sheets.rs.rs
// Sheet lifecycle, named ranges, engine/format reseed
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;
use crate::formula::types::SheetQualifier;

/// The target a defined name stores: a cell or range reference qualified by
/// the id of a live sheet (`TRAME-062`). Only
/// [`TrameState::name_target`] builds one, from the text a `DefineName`
/// carries.
pub(super) struct StoredNameTarget(Expr);

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
        // read paths use. Each cell's stable key already carries its `(row_uid,
        // col_uid)`, so feeding is a direct `CellId::bound`. A cell whose
        // row/column uid is absent from the sheet index (a deleted line) is
        // skipped.
        let indices = self.engine_indices();
        let sheets: Vec<(SheetId, EngineSheet)> = self
            .sheets_by_idx
            .iter()
            .enumerate()
            .map(|(idx, id)| (id.clone(), EngineSheet::new(idx as u16)))
            .collect();
        for (sheet_id, idx) in sheets {
            let Some(index) = indices.get(&idx) else {
                continue;
            };
            let LoroValue::Map(cells) = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, &sheet_id).as_str())
                .get_value()
            else {
                continue;
            };
            // Masking is derived per sheet from that sheet's `merges_{sheet}` map.
            // A masked conflicted cell is therefore excluded from the DAG on
            // *every* sheet. A cross-sheet reference to it converges regardless of
            // the active tab (`trame-cell-merges`). For the active sheet this
            // equals `self.masked_cells`.
            let masked = self.masked_cells_for(&sheet_id, index);
            // Feed the cells in stable key order. The map iterates in hash order,
            // and the feed order decides which cell of a cycle gets rejected. A
            // document where `A1=B1` and `B1=A1` met through a CRDT merge would
            // otherwise flag a different culprit on each reseed.
            let mut entries: Vec<_> = cells.iter().collect();
            entries.sort_unstable_by_key(|(key, _)| *key);
            for (key, value) in entries {
                // Masked cells (covered by a merge, conflicting value) stay out of
                // the DAG.
                let (Some((rl, cl)), LoroValue::String(text)) = (parse_cell_key(key), value) else {
                    continue;
                };
                if masked.contains(&CellKey::of(rl, cl)) {
                    continue;
                }
                if index.contains_row(&rl) && index.contains_col(&cl) {
                    let ctx = TrameCtx {
                        engine_sheet_by_name: &self.engine_sheet_by_name,
                        engine_sheet_by_id: &self.engine_sheet_by_id,
                        named: &self.named,
                    };
                    self.engine.on_cell_changed(
                        CellId::bound(idx, rl, cl),
                        StoredFormula::new(text.as_str().to_owned()),
                        &indices,
                        &ctx,
                    );
                }
            }
        }
        self.engine.compute_all_dirty(&indices);
    }

    /// Rebuild [`Self::sheets_by_idx`] and [`Self::engine_sheet_by_name`] from the
    /// `sheets` CRDT map. The active sheet is pinned to local index 0; remaining
    /// sheets follow in sorted-id order. The mapping is local-only (computed
    /// values never leave this client), so the exact ordering need not match
    /// peers -- only stability within this session matters.
    pub(super) fn rebuild_sheet_maps(&mut self) {
        // Collect {id -> name}, each name unique (`TRAME-078`).
        let mut entries: Vec<(SheetId, SheetName)> = self.sheet_names().into_iter().collect();
        // Active sheet first (index 0), then the rest by id for determinism.
        entries.sort_by(|a, b| {
            let a_active = a.0 == self.active_sheet;
            let b_active = b.0 == self.active_sheet;
            b_active.cmp(&a_active).then_with(|| a.0.cmp(&b.0))
        });

        self.sheets_by_idx.clear();
        self.engine_sheet_by_name.clear();
        self.engine_sheet_by_id.clear();
        self.sheet_id_to_name.clear();
        for (idx, (id, name)) in entries.into_iter().enumerate() {
            let sheet = EngineSheet::new(idx as u16);
            self.engine_sheet_by_name.insert(name.key(), sheet);
            self.engine_sheet_by_id.insert(id.clone(), sheet);
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
    /// sheet qualifier against [`Self::engine_sheet_by_name`]. A reference that
    /// fails to parse is dropped (the formula using it surfaces `#PARSE!`).
    pub(super) fn rebuild_named(&mut self) {
        self.named.clear();
        let LoroValue::Map(names) = self.doc.get_map(Root::NamedRanges.as_str()).get_value() else {
            return;
        };
        // Names cannot reference other names: resolve sheet qualifiers only.
        let no_names: HashMap<DefinedName, NameTarget> = HashMap::new();
        // Resolved on first need. Reading the order list registers its root
        // container in the snapshot, and a workbook with no legacy definition
        // must not pay for it.
        let mut legacy_sheet: Option<EngineSheet> = None;
        for (name, reference) in names.iter() {
            // A key without a name's shape is one no formula can write.
            let (Ok(name), LoroValue::String(reference)) = (DefinedName::new(name), reference)
            else {
                continue;
            };
            let ctx = TrameCtx {
                engine_sheet_by_name: &self.engine_sheet_by_name,
                engine_sheet_by_id: &self.engine_sheet_by_id,
                named: &no_names,
            };
            // `define_name` stores a qualified reference. An older build stored
            // some unqualified. Such a definition binds to the first sheet of the
            // strip, which every reader agrees on. It never binds to the sheet a
            // reader is looking at (`TRAME-062`).
            let target = parse_formula(reference)
                .and_then(|ast| {
                    let default_sheet = match (&ast, legacy_sheet) {
                        (Expr::SheetRef { .. }, _) => EngineSheet::ACTIVE,
                        (_, Some(sheet)) => sheet,
                        (_, None) => {
                            let sheet = self
                                .sheet_order_ids()
                                .first()
                                .and_then(|id| self.engine_sheet_by_id.get(id))
                                .copied()
                                .unwrap_or(EngineSheet::ACTIVE);
                            legacy_sheet = Some(sheet);
                            sheet
                        }
                    };
                    resolve_refs(ast, default_sheet, &ctx)
                })
                .ok()
                .and_then(expr_to_name_target);
            if let Some(target) = target {
                self.named.insert(name, target);
            }
        }
    }

    /// Build a fresh [`SheetIndex`] for an arbitrary sheet id (the cross-sheet
    /// reseed needs each sheet's position<->id map, not just the active one).
    pub(super) fn build_sheet_index(&self, sheet_id: &SheetId) -> SheetIndex {
        let rows = self
            .doc
            .get_movable_list(SheetContainer::of(SheetRoot::Rows, sheet_id).as_str());
        let cols = self
            .doc
            .get_movable_list(SheetContainer::of(SheetRoot::Cols, sheet_id).as_str());
        SheetIndex::build(&rows, &cols)
    }

    /// The target `reference` names for the name `name`, both checked, or
    /// the refusal said.
    ///
    /// A name no formula can read is refused (`TRAME-155`). So is a definition
    /// that is not a cell or range reference (`TRAME-069`). The name is
    /// checked first.
    pub(super) fn name_target(
        &self,
        name: &DefinedName,
        reference: &str,
    ) -> Result<StoredNameTarget, TrameOutcome> {
        // The grammar tries a bareword as a name last, after a function call, a
        // reference and a boolean. `A1` or `TRUE` have a name's shape and parse
        // as something else. The parse is the one rule that decides.
        let readable =
            matches!(parse_formula(name.as_str()), Ok(Expr::Name(parsed)) if parsed == *name);
        if !readable {
            return Err(TrameOutcome::display_only(vec![TrameEvent::Error {
                message: format!(
                    "\"{name}\" cannot be used as a name: a formula would not read it"
                ),
            }]));
        }
        self.canonical_name_reference(reference).ok_or_else(|| {
            TrameOutcome::display_only(vec![TrameEvent::Error {
                message: format!("\"{reference}\" is not a cell or range reference"),
            }])
        })
    }

    /// Define or redefine a named range, persist it to the CRDT, and reseed so
    /// formulas using the name pick it up immediately. The target is stored in
    /// canonical form: see
    /// [`canonical_name_reference`](Self::canonical_name_reference).
    pub(super) fn define_name(
        &mut self,
        name: &DefinedName,
        target: &StoredNameTarget,
    ) -> TrameOutcome {
        let stored = &target.0;
        let _ = self
            .doc
            .get_map(Root::NamedRanges.as_str())
            .insert(name.as_str(), render::render(stored).as_str());
        self.doc.commit();
        self.reseed_engine();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    /// The reference a name's definition stores, written out as its text. It
    /// is `None` when `reference` is not a cell or range reference on a live
    /// sheet. The qualifier is always a
    /// stable sheet id. An unqualified reference takes the sheet the user is on
    /// when defining it, so every reader resolves one target (`TRAME-062`). A
    /// display name becomes its id, so a rename leaves the name resolving
    /// (`TRAME-067`).
    fn canonical_name_reference(&self, reference: &str) -> Option<StoredNameTarget> {
        let body = reference.trim().trim_start_matches('=');
        let qualified = match parse_formula(body).ok()? {
            bare @ (Expr::CellRef(_) | Expr::Range(_)) => Expr::SheetRef {
                sheet: SheetQualifier::Id(self.active_sheet.clone()),
                inner: Box::new(bare),
            },
            Expr::SheetRef { sheet, inner } => {
                let sheet = match sheet {
                    SheetQualifier::Name(name) => SheetQualifier::Id(
                        self.engine_sheet_by_name
                            .get(&name.key())
                            .and_then(|idx| self.sheets_by_idx.get(idx.position()))
                            .cloned()?,
                    ),
                    SheetQualifier::Id(id) => SheetQualifier::Id(id),
                };
                Expr::SheetRef { sheet, inner }
            }
            _ => return None,
        };
        // The target must resolve now: the qualifier names a live sheet, and the
        // inner node is a reference and not, say, a nested name.
        let no_names: HashMap<DefinedName, NameTarget> = HashMap::new();
        let ctx = TrameCtx {
            engine_sheet_by_name: &self.engine_sheet_by_name,
            engine_sheet_by_id: &self.engine_sheet_by_id,
            named: &no_names,
        };
        resolve_refs(qualified.clone(), EngineSheet::ACTIVE, &ctx)
            .ok()
            .and_then(expr_to_name_target)?;
        Some(StoredNameTarget(qualified))
    }

    /// Remove a named range and reseed. A no-op if the name is undefined.
    pub(super) fn delete_name(&mut self, name: &DefinedName) -> TrameOutcome {
        let map = self.doc.get_map(Root::NamedRanges.as_str());
        if map.get(name.as_str()).is_none() {
            return TrameOutcome::display_only(vec![]);
        }
        let _ = map.delete(name.as_str());
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
    // Sheet lifecycle (create / rename / reorder / delete / activate) --
    // `trame-sheets`
    // -------------------------------------------------------------------------

    /// Every sheet's name as the user reads it, keyed by id, no two alike
    /// whatever their case (`TRAME-078`). A create and a rename never store a
    /// taken name, but two peers can store one concurrently. The sheet with
    /// the smaller id then keeps the stored name, and the other reads it with
    /// the first free ` (n)` suffix. The result depends on the document alone,
    /// so every peer reads the same.
    ///
    /// A stored name the rule of a rename refuses reads as `Sheet<n>`, `n` the
    /// position of its tab. The document keeps what it holds until a rename
    /// (`TRAME-235`). The stored names claim first, then the fallbacks,
    /// then the clashes. A value that is not text reads as the sheet id.
    pub(super) fn sheet_names(&self) -> HashMap<SheetId, SheetName> {
        let mut stored: Vec<(SheetId, SheetName)> =
            match self.doc.get_map(Root::Sheets.as_str()).get_value() {
                LoroValue::Map(sheets) => sheets
                    .iter()
                    .map(|(id, name)| {
                        let name = match name {
                            LoroValue::String(s) => s.to_string(),
                            _ => id.clone(),
                        };
                        (SheetId(id.clone()), SheetName::stored(name))
                    })
                    .collect(),
                _ => Vec::new(),
            };
        // A workbook with no stored sheet reads the default one (`TRAME-022`).
        if stored.is_empty() {
            stored.push((default_sheet(), SheetName::default()));
        }
        stored.sort();
        let (stored, refused): (Vec<_>, Vec<_>) =
            stored.into_iter().partition(|(_, name)| name.obeyed());
        let mut taken: HashSet<SheetNameKey> = HashSet::new();
        let mut names = HashMap::with_capacity(stored.len() + refused.len());
        // Stored names claim first, so neither a fallback nor a suffixed name
        // takes one a later sheet stores as is.
        let mut clashing = Vec::new();
        for (id, name) in stored {
            if taken.insert(name.key()) {
                names.insert(id, name);
            } else {
                clashing.push((id, name));
            }
        }
        if !refused.is_empty() {
            let order = self.sheet_order_ids();
            for (id, _) in refused {
                let position = order.iter().position(|o| *o == id).unwrap_or(order.len());
                let fallback = SheetName::numbered(position + 1);
                if taken.insert(fallback.key()) {
                    names.insert(id, fallback);
                } else {
                    clashing.push((id, fallback));
                }
            }
        }
        for (id, name) in clashing {
            let unique = name.free_variant(|key| taken.contains(key));
            taken.insert(unique.key());
            names.insert(id, unique);
        }
        names
    }

    /// `wanted`, or the first ` (n)` variant of it no sheet carries, for a create
    /// whose name is taken, case ignored (`TRAME-078`).
    pub(super) fn free_sheet_name(&self, wanted: &SheetName) -> SheetName {
        let taken: HashSet<SheetNameKey> =
            self.sheet_names().values().map(SheetName::key).collect();
        if taken.contains(&wanted.key()) {
            wanted.free_variant(|key| taken.contains(key))
        } else {
            wanted.clone()
        }
    }

    /// Every live sheet id, unordered. It is each id the `sheets` CRDT map
    /// holds. A map with none reads the default sheet, so the workbook always
    /// has one (`TRAME-022`).
    pub(super) fn all_sheet_ids(&self) -> Vec<SheetId> {
        let stored = self.stored_sheet_ids();
        if stored.is_empty() {
            return vec![default_sheet()];
        }
        stored
    }

    /// Every sheet id the `sheets` CRDT map holds, unordered.
    fn stored_sheet_ids(&self) -> Vec<SheetId> {
        match self.doc.get_map(Root::Sheets.as_str()).get_value() {
            LoroValue::Map(sheets) => sheets.keys().cloned().map(SheetId).collect(),
            _ => Vec::new(),
        }
    }

    /// Store the default sheet when the map holds no sheet. Two peers that
    /// delete the last two sheets at once leave an empty map, and each reads
    /// the default sheet without a write (`TRAME-077`). The first local write
    /// stores it, so a sheet added next to it does not hide it. Two peers
    /// storing it at once write one key with one value. It returns whether it
    /// wrote, and the caller commits.
    pub(super) fn hold_default_sheet(&mut self) -> bool {
        if !self.stored_sheet_ids().is_empty() {
            return false;
        }
        let _ = self
            .doc
            .get_map(Root::Sheets.as_str())
            .insert(default_sheet().as_str(), SheetName::default().as_str());
        true
    }

    /// Canonical tab-strip order, deterministic on every peer. The implicit
    /// default sheet comes first. Then any sheet not yet present in
    /// `sheet_order`, sorted by id -- which covers a create whose order op has
    /// not merged. Then the explicit `sheet_order` entries that still correspond
    /// to a live sheet.
    pub(super) fn sheet_order_ids(&self) -> Vec<SheetId> {
        let listed: Vec<SheetId> = match self
            .doc
            .get_movable_list(Root::SheetOrder.as_str())
            .get_value()
        {
            LoroValue::List(items) => items
                .iter()
                .filter_map(|v| match v {
                    LoroValue::String(s) => Some(SheetId(s.to_string())),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        // Two peers materializing the order at once can both list one id: the
        // first occurrence is its position, every later one is ignored.
        let mut seen = HashSet::new();
        let listed: Vec<SheetId> = listed
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .collect();
        let in_list: HashSet<&SheetId> = listed.iter().collect();
        let mut unlisted: Vec<SheetId> = self
            .all_sheet_ids()
            .into_iter()
            .filter(|id| !in_list.contains(id))
            .collect();
        // Default sheet first, the rest by id for a stable, peer-independent order.
        let default = default_sheet();
        unlisted.sort_by(|a, b| {
            let a_def = *a == default;
            let b_def = *b == default;
            b_def.cmp(&a_def).then_with(|| a.cmp(b))
        });
        let existing: HashSet<SheetId> = self.all_sheet_ids().into_iter().collect();
        drop(in_list);
        unlisted
            .into_iter()
            .chain(listed.into_iter().filter(|id| existing.contains(id)))
            .collect()
    }

    /// Build the [`TrameEvent::SheetList`] the tab strip consumes, in canonical
    /// order with the active sheet flagged.
    pub(super) fn sheet_list_event(&self) -> TrameEvent {
        let mut names = self.sheet_names();
        let sheets = self
            .sheet_order_ids()
            .into_iter()
            .map(|id| {
                let name = names
                    .remove(&id)
                    .unwrap_or_else(|| SheetName::stored(id.to_string()));
                SheetTab { id, name }
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
        self.reseed_styles();
        self.rebuild_filter();
        self.reseed_metrics();
        conflicts
    }

    /// Switch the active sheet and re-derive its caches (`trame-sheets`). A no-op
    /// if the id
    /// is unknown or already active.
    pub(super) fn activate_sheet(&mut self, sheet_id: &SheetId) -> TrameOutcome {
        if *sheet_id == self.active_sheet || !self.all_sheet_ids().contains(sheet_id) {
            return TrameOutcome::display_only(vec![]);
        }
        self.active_sheet = sheet_id.to_owned();
        let mut display = self.reseed_active_sheet();
        display.push(TrameEvent::DataReset);
        display.push(self.sheet_list_event());
        display.push(self.style_state_event());
        TrameOutcome::display_only(display)
    }

    /// Add a sheet with a freshly minted peer-scoped id (`trame-sheets`). The new
    /// sheet is
    /// empty; its containers are created lazily on first write. Does not switch
    /// the active sheet (the UI sends [`TrameCommand::ActivateSheet`] for that).
    pub(super) fn create_sheet(&mut self, name: &SheetName) -> TrameOutcome {
        self.hold_default_sheet();
        let id = format!("{:016x}_{:08x}", self.doc.peer_id(), self.sheet_counter);
        self.sheet_counter += 1;
        let name = self.free_sheet_name(name);
        let _ = self
            .doc
            .get_map(Root::Sheets.as_str())
            .insert(&id, name.as_str());
        let _ = self
            .doc
            .get_movable_list(Root::SheetOrder.as_str())
            .push(id.as_str());
        self.doc.commit();
        // The new id sorts among the existing ones, so the engine index of
        // every sheet after it moves up. A compiled reference holds the old
        // index, so the engine is fed again under the new one, as a rename does.
        self.reseed_engine();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![self.sheet_list_event()],
            update,
            ephemeral: None,
        }
    }

    /// Rename a sheet's display label (LWW on the name). The stable id is
    /// untouched. The name -> index map changes, so reseed to recompile any
    /// cross-sheet reference (the display-name binding is the `trame-sheets` MVP
    /// gap).
    /// A no-op if the id is unknown. The name already obeys the rule, as
    /// [`SheetName::new`] is the only way to give one (`TRAME-160`,
    /// `TRAME-234`).
    pub(super) fn rename_sheet(&mut self, sheet_id: &SheetId, name: &SheetName) -> TrameOutcome {
        if !self.all_sheet_ids().iter().any(|id| id == sheet_id) {
            return TrameOutcome::display_only(vec![]);
        }
        // A taken name leaves two tabs no reader can tell apart, and two names
        // that differ only in case are one name (`TRAME-078`).
        let key = name.key();
        if self
            .sheet_names()
            .iter()
            .any(|(id, taken)| id != sheet_id && taken.key() == key)
        {
            return TrameOutcome::display_only(vec![TrameEvent::Error {
                message: format!("a sheet named \"{name}\" already exists"),
            }]);
        }
        let _ = self
            .doc
            .get_map(Root::Sheets.as_str())
            .insert(sheet_id.as_str(), name.as_str());
        self.doc.commit();
        self.reseed_engine();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![self.sheet_list_event(), TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    /// Move a sheet to `to_index` in the tab strip (`trame-sheets`). Pure
    /// presentation, no
    /// recompute. Materializes the canonical order into `sheet_order` first (the
    /// implicit default and unmerged entries are otherwise absent) so positions
    /// are well-defined, then issues one move op. A no-op if the id is unknown or
    /// already at the target position.
    pub(super) fn move_sheet(&mut self, sheet_id: &SheetId, to_index: SheetTabPos) -> TrameOutcome {
        let order = self.sheet_order_ids();
        let Some(from) = order.iter().position(|id| id == sheet_id) else {
            return TrameOutcome::display_only(vec![]);
        };
        let to = to_index.to_usize().min(order.len().saturating_sub(1));
        if from == to {
            return TrameOutcome::display_only(vec![]);
        }
        let list = self.doc.get_movable_list(Root::SheetOrder.as_str());
        // Bring the list to the canonical order if it diverges, so `mov` operates
        // on the positions the tab strip shows. The elements already listed are
        // kept: a concurrent move needs their identity to land (`TRAME-074`).
        let current: Vec<Option<SheetId>> = match list.get_value() {
            LoroValue::List(items) => items
                .iter()
                .map(|v| match v {
                    LoroValue::String(s) => Some(SheetId(s.to_string())),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        if current.iter().flatten().ne(order.iter()) || current.iter().any(Option::is_none) {
            // Drop, from the end, each entry naming no live sheet or repeating
            // an earlier one, as `sheet_order_ids` ignores them.
            let live: HashSet<SheetId> = self.all_sheet_ids().into_iter().collect();
            let mut seen = HashSet::new();
            let stale: Vec<usize> = current
                .iter()
                .enumerate()
                .filter(|(_, id)| match id {
                    Some(id) => !live.contains(id) || !seen.insert(id.clone()),
                    None => true,
                })
                .map(|(pos, _)| pos)
                .collect();
            for pos in stale.into_iter().rev() {
                let _ = list.delete(pos, 1);
            }
            // The ids the list lacks lead the canonical order: insert them first.
            let unlisted = order.iter().filter(|id| !seen.contains(*id));
            for (pos, id) in unlisted.enumerate() {
                let _ = list.insert(pos, id.as_str());
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

    /// Delete a sheet and its per-sheet containers (`trame-sheets`). Rejected
    /// locally if it
    /// is the last remaining sheet. If the active sheet is deleted, the active
    /// sheet falls back to the previous tab (else the first remaining). The sheet
    /// set changed, so a full reseed remaps engine indices and lets cross-sheet
    /// formulas targeting the gone id surface `#REF!`.
    pub(super) fn delete_sheet(&mut self, sheet_id: &SheetId) -> TrameOutcome {
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
        let list = self.doc.get_movable_list(Root::SheetOrder.as_str());
        if let LoroValue::List(items) = list.get_value()
            && let Some(pos) = items
                .iter()
                .position(|v| matches!(v, LoroValue::String(s) if s.as_str() == sheet_id.as_str()))
        {
            let _ = list.delete(pos, 1);
        }
        let _ = self
            .doc
            .get_map(Root::Sheets.as_str())
            .delete(sheet_id.as_str());
        self.drop_sheet_containers(sheet_id);
        self.doc.commit();

        if self.active_sheet == *sheet_id {
            self.active_sheet = self.fallback_sheet(&order);
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

    /// The sheet a reader lands on when the active sheet is deleted
    /// (`TRAME-075`). It is the nearest live tab before it in `order`, else the
    /// first live tab. `order` is the tab order the sheet was deleted from.
    fn fallback_sheet(&self, order: &[SheetId]) -> SheetId {
        let live = self.all_sheet_ids();
        let pos = order
            .iter()
            .position(|id| *id == self.active_sheet)
            .unwrap_or(0);
        order
            .iter()
            .take(pos)
            .rev()
            .find(|id| live.contains(id))
            .cloned()
            .or_else(|| self.sheet_order_ids().into_iter().next())
            .unwrap_or_else(default_sheet)
    }

    /// Leave the active sheet when a merge deleted it (`TRAME-075`). `order` is
    /// the tab order before the merge. The caller reseeds after.
    pub(super) fn leave_a_deleted_sheet(&mut self, order: &[SheetId]) {
        if !self.all_sheet_ids().contains(&self.active_sheet) {
            self.active_sheet = self.fallback_sheet(order);
        }
    }

    /// Empty every per-sheet container of `sheet_id` the schema builds. They are
    /// the cells, the styles, the merges, the metrics, the filter, its hidden
    /// rows, and the two axes. A deleted sheet leaves none of its data in the
    /// document (`TRAME-076`).
    pub(super) fn drop_sheet_containers(&mut self, sheet_id: &SheetId) {
        for root in [
            SheetRoot::Cells,
            SheetRoot::CellStyle,
            SheetRoot::Merges,
            SheetRoot::ColWidths,
            SheetRoot::RowHeights,
            SheetRoot::Filters,
            SheetRoot::FilterHidden,
        ] {
            let map = self
                .doc
                .get_map(SheetContainer::of(root, sheet_id).as_str());
            if let LoroValue::Map(entries) = map.get_value() {
                let keys: Vec<String> = entries.keys().cloned().collect();
                for key in keys {
                    let _ = map.delete(&key);
                }
            }
        }
        for root in [SheetRoot::Rows, SheetRoot::Cols] {
            let _ = self
                .doc
                .get_movable_list(SheetContainer::of(root, sheet_id).as_str())
                .clear();
        }
    }
}
