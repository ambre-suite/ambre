// ambre-trame/src/logic/cells.rs.rs
// Cell read/write, materialization, the stable-id index
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use std::cell::OnceCell;

use super::*;
use crate::formula::SheetQualifier;
use crate::formula::identity;

impl TrameState {
    /// Display text for a cell. The formula evaluation result if it's a formula
    /// cell, otherwise the edited value from the CRDT if present, otherwise
    /// empty. A virtual or cleared cell renders blank.
    pub(super) fn cell_text(&self, row: u32, col: u32) -> String {
        let cell = self.active_cell_id(row, col);
        // Formula cells: return cached evaluation result. Plain cells also have
        // a cached literal value now, so formula references resolve. Their raw
        // CRDT string is rendered verbatim, to avoid normalizing user input.
        if self.engine.is_formula_cell(cell)
            && let Some(text) = self.engine.evaluated_text(cell)
        {
            return text;
        }
        // Plain-text cells: resolve the grid position to its stable id pair and
        // read the CRDT value. A virtual (never-written) cell, or one cleared
        // back to empty, has no stored value and renders blank.
        if let Some((rl, cl)) = self.index.resolve(row as usize, col as usize) {
            let map = self.doc.get_map(cells_map_name(&self.active_sheet));
            if let Some(LoroValue::String(s)) =
                map.get(&cell_key(rl, cl)).map(|v| v.get_deep_value())
            {
                return s.to_string();
            }
        }
        String::new()
    }

    /// Raw cell text as stored in the CRDT (the formula source for a formula
    /// cell, the literal otherwise). Empty for virtual/cleared cells.
    pub(super) fn cell_raw(&self, row: u32, col: u32) -> String {
        if let Some((rl, cl)) = self.index.resolve(row as usize, col as usize) {
            let map = self.doc.get_map(cells_map_name(&self.active_sheet));
            if let Some(LoroValue::String(s)) =
                map.get(&cell_key(rl, cl)).map(|v| v.get_deep_value())
            {
                return s.to_string();
            }
        }
        String::new()
    }

    /// The text of an active-sheet cell with a formula's references at their
    /// positions ([`formula_source`](Self::formula_source)). A fill or a copy
    /// shifts this text, never the stored one (`TRAME-228`).
    pub(super) fn cell_source(&self, row: u32, col: u32) -> String {
        self.formula_source(
            &self.cell_raw(row, col),
            &self.active_sheet,
            &self.sheet_lines(),
        )
    }

    /// Rewrite user-entered formula text into its canonical CRDT form, for a
    /// cell of `sheet_id`. Each cross-sheet qualifier is bound to the stable
    /// `sheet_id` (`Sheet2!A1` -> `#s0!A1`), so a rename or a reorder never
    /// invalidates it (`trame-sheets`). Each position is bound to the row and
    /// column now at it, so an insert or a delete never moves it
    /// (`TRAME-228`). Plain text, and formulas that fail to parse, are stored
    /// verbatim (the user's exact input).
    ///
    /// # Errors
    ///
    /// The display name of a sheet the formula qualifies and no tab carries
    /// (`TRAME-211`). Stored by name, it would resolve the day such a sheet
    /// appears and break the day that sheet is renamed.
    pub(super) fn canonicalize_input(
        &self,
        text: &str,
        sheet_id: &str,
        lines: &SheetLines<'_>,
    ) -> Result<String, String> {
        let Some(body) = text.strip_prefix('=') else {
            return Ok(text.to_string());
        };
        let name_to_id = |name: &str| {
            self.sheet_name_to_u16
                .get(name)
                .and_then(|idx| self.sheets_by_idx.get(*idx as usize))
                .cloned()
        };
        let Ok(ast) = parse_formula(body) else {
            return Ok(text.to_string());
        };
        let canonical = render::canonicalize(ast, &name_to_id);
        if let Some(unknown) = render::first_name_qualifier(&canonical) {
            return Err(unknown);
        }
        let bound = identity::bind(canonical, &|q| lines.of(sheet_id, q));
        Ok(format!("={}", render::render(&bound)))
    }

    /// The source of a stored formula with each reference back at its position
    /// in today's layout, its qualifiers still sheet ids (`#s0!A2`). A line
    /// that is gone reads `#REF!` (`TRAME-008`). This is the text a paste or a
    /// fill shifts, and what a copy carries. Plain text passes through.
    pub(super) fn formula_source(
        &self,
        stored: &str,
        sheet_id: &str,
        lines: &SheetLines<'_>,
    ) -> String {
        let Some(body) = stored.strip_prefix('=') else {
            return stored.to_string();
        };
        // A stored line is the only `{` the canonical form writes outside a
        // string; a formula without one is already positional.
        if !body.contains('{') {
            return stored.to_string();
        }
        match parse_formula(body) {
            Ok(ast) => format!(
                "={}",
                render::render(&identity::unbind(ast, &|q| lines.of(sheet_id, q)))
            ),
            Err(_) => stored.to_string(),
        }
    }

    /// Reconstruct the formula-bar text for a stored CRDT value of a cell of
    /// `sheet_id`. Each line is rendered at its position
    /// ([`formula_source`](Self::formula_source)), and each `#id` qualifier
    /// through the current `sheet_id -> name` map (`#s0!A1` -> `Data!A1`). A
    /// rename or an insert is then reflected immediately, with no CRDT
    /// mutation. A deleted sheet stays in `#id` form, signalling the dangling
    /// reference. Plain text passes through.
    pub(super) fn formula_display(
        &self,
        stored: &str,
        sheet_id: &str,
        lines: &SheetLines<'_>,
    ) -> String {
        let source = self.formula_source(stored, sheet_id, lines);
        let Some(body) = source.strip_prefix('=') else {
            return source;
        };
        // The canonical id qualifier is the only other `#` we persist in `v`.
        if !body.contains('#') {
            return source;
        }
        let id_to_name = |id: &str| self.sheet_id_to_name.get(id).cloned();
        match parse_formula(body) {
            Ok(ast) => format!(
                "={}",
                render::render(&render::to_display_form(ast, &id_to_name))
            ),
            Err(_) => source,
        }
    }

    /// The row and column order of every sheet, built on first use (see
    /// [`SheetLines`]).
    pub(super) fn sheet_lines(&self) -> SheetLines<'_> {
        SheetLines {
            state: self,
            built: self.sheets_by_idx.iter().map(|_| OnceCell::new()).collect(),
        }
    }

    /// Ensure grid `(row, col)` is materialized (its row and column exist in the
    /// `rows_/cols_` lists) and return the `(row_lid, col_lid)` pair to key the
    /// cell with. Extends the dense used range by pushing placeholder elements,
    /// capturing each new element's stable id. Returns `None` -- dropping the
    /// write -- if the jump past the high-water mark exceeds [`MATERIALIZE_CAP`].
    /// Mint a fresh, peer-scoped row/column [`Uid`] (`trame-rows-and-columns`).
    /// The pair
    /// `(document PeerID, per-session counter)` is unique across peers, so
    /// concurrent materialize/insert ops never collide.
    pub(super) fn mint_uid(&mut self) -> Uid {
        let uid = Uid {
            peer: ambre_core::PeerId(self.doc.peer_id()),
            counter: self.uid_counter,
        };
        self.uid_counter += 1;
        uid
    }

    pub(super) fn materialize(&mut self, row: u32, col: u32) -> Option<(Uid, Uid)> {
        let (row, col) = (row as usize, col as usize);
        if row >= self.index.row_len() && row - self.index.row_len() >= MATERIALIZE_CAP
            || col >= self.index.col_len() && col - self.index.col_len() >= MATERIALIZE_CAP
        {
            tracing::warn!(
                "[Logic] materialize ({row},{col}) exceeds cap past hwm \
                 (rows {}, cols {}); write dropped",
                self.index.row_len(),
                self.index.col_len(),
            );
            return None;
        }
        let mut grew = false;
        while self.index.row_len() <= row {
            let uid = self.mint_uid();
            let rows = self
                .doc
                .get_movable_list(rows_list_name(&self.active_sheet));
            rows.push(uid.to_key_part().as_str()).ok()?;
            self.index.push_row(uid);
            grew = true;
        }
        while self.index.col_len() <= col {
            let uid = self.mint_uid();
            let cols = self
                .doc
                .get_movable_list(cols_list_name(&self.active_sheet));
            cols.push(uid.to_key_part().as_str()).ok()?;
            self.index.push_col(uid);
            grew = true;
        }
        // Commit the implicit scaffolding under the excluded origin so it stays
        // OUT of the undo stack (see [`MATERIALIZE_ORIGIN`]). A cell edit's intent
        // is the value, not the row/column placeholders it forced into existence.
        // Keeping those off the stack means undo reverts only the value. On redo,
        // the cell's key then still resolves to the same (never-removed) list
        // element ids. Without this split, Loro would recreate those elements with
        // fresh ids on redo and orphan the cell. A no-op when nothing grew.
        if grew {
            self.doc.set_next_commit_origin(MATERIALIZE_ORIGIN);
            self.doc.commit();
        }
        self.index.resolve(row, col)
    }

    /// Rebuild [`Self::index`] from the active sheet's `rows_/cols_` lists.
    /// Called after any import, where peers' (or our own persisted) structural
    /// ops may have added rows/columns the cache does not yet know about.
    pub(super) fn rebuild_index(&mut self) {
        let rows = self
            .doc
            .get_movable_list(rows_list_name(&self.active_sheet));
        let cols = self
            .doc
            .get_movable_list(cols_list_name(&self.active_sheet));
        self.index = SheetIndex::build(&rows, &cols);
    }

    /// Build the per-sheet position<->uid index map the formula engine needs to
    /// resolve references. The active sheet is local index 0 (reusing the
    /// maintained [`Self::index`]); other sheets are rebuilt on demand. Local
    /// indices match [`Self::sheets_by_idx`] (active pinned first).
    pub(super) fn engine_indices(&self) -> SheetIndices {
        let mut m = SheetIndices::new();
        for (idx, id) in self.sheets_by_idx.iter().enumerate() {
            let index = if idx == 0 {
                self.index.clone()
            } else {
                self.build_sheet_index(id)
            };
            m.insert(idx as u16, index);
        }
        m
    }

    /// Engine [`CellId`] for grid `(row, col)` on the active sheet (local index
    /// 0). Each axis binds to its row/column `Uid` when materialized, else stays
    /// a virtual grid position.
    pub(super) fn active_cell_id(&self, row: u32, col: u32) -> CellId {
        let r = match self.index.row_id(row as usize) {
            Some(u) => AxisId::Bound(u),
            None => AxisId::Virtual(row),
        };
        let c = match self.index.col_id(col as usize) {
            Some(u) => AxisId::Bound(u),
            None => AxisId::Virtual(col),
        };
        CellId {
            sheet: 0,
            row: r,
            col: c,
        }
    }

    /// Apply a single cell write to the CRDT and the formula engine, without
    /// committing, recomputing, or touching the undo group. Shared by the
    /// single-cell [`SetCellText`](TrameCommand::SetCellText) path and the
    /// multi-cell [`SetCellsBatch`](TrameCommand::SetCellsBatch) path so a batch
    /// can wrap many writes in one undo group. An empty `text` clears the cell; a
    /// write beyond the materialization cap is silently dropped.
    ///
    /// Does not commit -- the caller commits once after the write(s). The batch
    /// path must pre-materialize every cell first (see the `SetCellsBatch` arm).
    /// A `materialize` commit carries the excluded [`MATERIALIZE_ORIGIN`]. An
    /// excluded commit interleaved between two value commits splits them into
    /// separate undo items. Materializing up front keeps all value writes in one
    /// final commit, hence one undo item.
    pub(super) fn write_cell(&mut self, row: u32, col: u32, text: &str) {
        if text.is_empty() {
            // Clearing: only touch the CRDT if the cell was materialized.
            if let Some((rl, cl)) = self.index.resolve(row as usize, col as usize) {
                let _ = self
                    .doc
                    .get_map(cells_map_name(&self.active_sheet))
                    .delete(&cell_key(rl, cl));
            }
            let cell = self.active_cell_id(row, col);
            let indices = self.engine_indices();
            let ctx = TrameCtx {
                sheet_name_to_u16: &self.sheet_name_to_u16,
                sheet_id_to_u16: &self.sheet_id_to_u16,
                named: &self.named,
            };
            self.engine.on_cell_changed(cell, "", &indices, &ctx);
        } else {
            // Persist the canonical form: sheet ids, so cross-sheet references
            // survive rename/reorder (`trame-sheets`), and lines, so references
            // survive an insert or a delete (`TRAME-228`).
            let sheet = self.active_sheet.clone();
            let refused = self
                .canonicalize_input(text, &sheet, &self.sheet_lines())
                .err();
            if let Some(unknown) = refused {
                self.write_refusals
                    .push(WriteRefusal::UnknownSheet(unknown));
                return;
            }
            let Some((rl, cl)) = self.materialize(row, col) else {
                // Beyond the materialization cap: the write is refused, and
                // `finish_cell_writes` says so (`TRAME-UI-164`).
                self.write_refusals.push(WriteRefusal::BeyondCap);
                return;
            };
            // Bound once the cell's own lines exist, so a reference to its row
            // or column names that line rather than a position.
            let stored = self
                .canonicalize_input(text, &sheet, &self.sheet_lines())
                .unwrap_or_else(|_| text.to_string());
            let _ = self
                .doc
                .get_map(cells_map_name(&self.active_sheet))
                .insert(&cell_key(rl, cl), stored.as_str());
            let cell = CellId::bound(0, rl, cl);
            let indices = self.engine_indices();
            let ctx = TrameCtx {
                sheet_name_to_u16: &self.sheet_name_to_u16,
                sheet_id_to_u16: &self.sheet_id_to_u16,
                named: &self.named,
            };
            self.engine.on_cell_changed(cell, &stored, &indices, &ctx);
        }
    }

    /// Commit the pending cell write(s), recompute the dirty formula closure and
    /// the active filter, then return the standard write outcome. That outcome is
    /// a `DataReset`, so the UI re-fetches its viewport, plus the CRDT delta.
    /// Shared tail of the single-cell and batch write paths.
    pub(super) fn finish_cell_writes(&mut self) -> TrameOutcome {
        self.doc.commit();
        let indices = self.engine_indices();
        self.engine.compute_all_dirty(&indices);
        // The edit may change the filtered column's value: recompute the active
        // filter so the row's hidden status tracks the data
        // (`trame-shared-filter`).
        self.recompute_active_filter();
        let update = self.flush_delta();
        // Ask the UI to re-fetch its viewport. The edit, and any formula it
        // feeds, then renders its computed result immediately, without waiting
        // for the network echo. The optimistic UI-side update covers plain
        // literals; formulas need this round-trip. The re-fetch also puts back
        // what the UI drew optimistically for a refused write.
        let mut display = vec![TrameEvent::DataReset];
        display.extend(self.take_write_refusals());
        TrameOutcome {
            display,
            update,
            ephemeral: None,
        }
    }

    /// One [`TrameEvent::Error`] per kind of refused write since the last call,
    /// the refusals cleared.
    fn take_write_refusals(&mut self) -> Vec<TrameEvent> {
        let refusals = std::mem::take(&mut self.write_refusals);
        let beyond_cap = refusals
            .iter()
            .filter(|r| **r == WriteRefusal::BeyondCap)
            .count();
        let mut unknown: Vec<String> = refusals
            .into_iter()
            .filter_map(|r| match r {
                WriteRefusal::UnknownSheet(name) => Some(name),
                WriteRefusal::BeyondCap => None,
            })
            .collect();
        unknown.sort();
        unknown.dedup();
        let mut events = Vec::new();
        if beyond_cap > 0 {
            events.push(TrameEvent::Error {
                message: format!(
                    "{beyond_cap} cell(s) lie beyond the sheet's {MATERIALIZE_CAP}-line limit and were not written"
                ),
            });
        }
        events.extend(unknown.into_iter().map(|name| TrameEvent::Error {
            message: format!("no sheet is named \"{name}\": the formula was not written"),
        }));
        events
    }
}

/// The row and column order of every sheet, for the rewrites between a stored
/// formula and its positions (`TRAME-228`). The active sheet reads the
/// maintained index. Another sheet's index is built from its lists on first
/// read, then kept for the life of this value.
pub(super) struct SheetLines<'a> {
    state: &'a TrameState,
    built: Vec<OnceCell<SheetIndex>>,
}

impl SheetLines<'_> {
    /// The index a reference of a formula on `sheet_id` reads: its qualifier's
    /// sheet, else the formula's own. `None` for a sheet that does not exist.
    pub(super) fn of(&self, sheet_id: &str, q: Option<&SheetQualifier>) -> Option<&SheetIndex> {
        let state = self.state;
        let idx = match q {
            None => state.sheet_id_to_u16.get(sheet_id),
            Some(SheetQualifier::Id(id)) => state.sheet_id_to_u16.get(id),
            Some(SheetQualifier::Name(name)) => state.sheet_name_to_u16.get(name),
        }
        .copied()?;
        if idx == 0 {
            return Some(&state.index);
        }
        let id = state.sheets_by_idx.get(idx as usize)?;
        let cell = self.built.get(idx as usize)?;
        Some(cell.get_or_init(|| state.build_sheet_index(id)))
    }
}
