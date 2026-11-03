// ambre-trame/src/logic/cells.rs.rs
// Cell read/write, materialization, the stable-id index
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use crate::formula::ACTIVE_SHEET;
use crate::formula::text::CellText;
use ambre_core::id::{Id, Sheet};
use ambre_core::index::Len;
use std::cell::OnceCell;

use super::*;
use crate::formula::SheetQualifier;
use crate::formula::identity;

impl TrameState {
    /// Display text for a cell. The formula evaluation result if it's a formula
    /// cell, otherwise the edited value from the CRDT if present, otherwise
    /// empty. A virtual or cleared cell renders blank.
    pub(super) fn cell_text(&self, at: CellPos) -> String {
        let cell = self.active_cell_id(at);
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
        if let Some((rl, cl)) = self.index.resolve(at) {
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, &self.active_sheet).as_str());
            if let Some(LoroValue::String(s)) = map
                .get(CellKey::of(rl, cl).as_str())
                .map(|v| v.get_deep_value())
            {
                return s.to_string();
            }
        }
        String::new()
    }

    /// Raw cell text as stored in the CRDT (the formula source for a formula
    /// cell, the literal otherwise). Empty for virtual/cleared cells.
    pub(super) fn cell_raw(&self, at: CellPos) -> CellText<StoredForm> {
        if let Some((rl, cl)) = self.index.resolve(at) {
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, &self.active_sheet).as_str());
            if let Some(LoroValue::String(s)) = map
                .get(CellKey::of(rl, cl).as_str())
                .map(|v| v.get_deep_value())
            {
                return CellText::<StoredForm>::new(s.to_string());
            }
        }
        CellText::<StoredForm>::default()
    }

    /// The text of an active-sheet cell with a formula's references at their
    /// positions ([`formula_source`](Self::formula_source)). A fill or a copy
    /// shifts this text, never the stored one (`TRAME-228`).
    pub(super) fn cell_source(&self, at: CellPos) -> CellText<SourceForm> {
        self.formula_source(&self.cell_raw(at), &self.active_sheet, &self.sheet_lines())
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
        text: &CellText<SourceForm>,
        sheet_id: &Id<Sheet>,
        lines: &SheetLines<'_>,
    ) -> Result<CellText<StoredForm>, SheetName> {
        let verbatim = || CellText::<StoredForm>::new(text.as_str().to_owned());
        let Some(body) = text.formula_body() else {
            return Ok(verbatim());
        };
        let name_to_id = |name: &SheetName| {
            self.engine_sheet_by_name
                .get(&name.key())
                .and_then(|idx| self.sheets_by_idx.get(idx.to_usize()))
                .cloned()
        };
        let Ok(ast) = parse_formula(body) else {
            return Ok(verbatim());
        };
        let canonical = render::canonicalize(ast, &name_to_id);
        if let Some(unknown) = render::first_name_qualifier(&canonical) {
            return Err(unknown);
        }
        let bound = identity::bind(canonical, &|q| lines.of(sheet_id, q));
        Ok(CellText::<StoredForm>::new(format!(
            "={}",
            render::render(&bound)
        )))
    }

    /// The source of a stored formula with each reference back at its position
    /// in today's layout, its qualifiers still sheet ids (`#s0!A2`). A line
    /// that is gone reads `#REF!` (`TRAME-008`). This is the text a paste or a
    /// fill shifts, and what a copy carries. Plain text passes through.
    pub(super) fn formula_source(
        &self,
        stored: &CellText<StoredForm>,
        sheet_id: &Id<Sheet>,
        lines: &SheetLines<'_>,
    ) -> CellText<SourceForm> {
        let verbatim = || CellText::<SourceForm>::new(stored.as_str().to_owned());
        let Some(body) = stored.formula_body() else {
            return verbatim();
        };
        // A stored line is the only `{` the canonical form writes outside a
        // string; a formula without one is already positional.
        if !body.contains('{') {
            return verbatim();
        }
        match parse_formula(body) {
            Ok(ast) => CellText::<SourceForm>::new(format!(
                "={}",
                render::render(&identity::unbind(ast, &|q| lines.of(sheet_id, q)))
            )),
            Err(_) => verbatim(),
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
        stored: &CellText<StoredForm>,
        sheet_id: &Id<Sheet>,
        lines: &SheetLines<'_>,
    ) -> CellText<DisplayForm> {
        let source = self.formula_source(stored, sheet_id, lines);
        let verbatim = || CellText::<DisplayForm>::new(source.as_str().to_owned());
        let Some(body) = source.formula_body() else {
            return verbatim();
        };
        // The canonical id qualifier is the only other `#` we persist in `v`.
        if !body.contains('#') {
            return verbatim();
        }
        let id_to_name = |id: &Id<Sheet>| self.sheet_id_to_name.get(id).cloned();
        match parse_formula(body) {
            Ok(ast) => CellText::<DisplayForm>::new(format!(
                "={}",
                render::render(&render::to_display_form(ast, &id_to_name))
            )),
            Err(_) => verbatim(),
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

    /// Mint a fresh, peer-scoped row/column [`Uid`] (`trame-rows-and-columns`).
    /// The pair
    /// `(document PeerID, per-session counter)` is unique across peers, so
    /// concurrent materialize/insert ops never collide. `None` once the session
    /// has minted its last counter: a repeated uid would make two lines one.
    pub(super) fn mint_uid(&mut self) -> Option<Uid> {
        let Some(counter) = self.uid_counter else {
            tracing::warn!("[Logic] the session has no fresh row or column uid; write dropped");
            return None;
        };
        self.uid_counter = counter.next();
        Some(Uid {
            peer: ambre_core::PeerId::new(self.doc.peer_id()),
            counter,
        })
    }

    /// Ensure the cell `at` is materialized (its row and column exist in the
    /// `rows_/cols_` lists) and return the `(row_lid, col_lid)` pair to key the
    /// cell with. Extends the dense used range by pushing placeholder elements,
    /// capturing each new element's stable id. The write is refused when the
    /// jump past the high-water mark exceeds [`MATERIALIZE_ROWS`]. It is refused
    /// too when the session cannot mint every uid the cell needs. Every uid is minted
    /// before the first push, so a refusal for want of one pushes no line.
    pub(super) fn materialize(&mut self, at: CellPos) -> Result<(Uid, Uid), Unmaterialized> {
        let (rows, cols) = (self.index.row_len(), self.index.col_len());
        if at.row.len_from(rows.as_end()) >= MATERIALIZE_ROWS
            || at.col.len_from(cols.as_end()) >= MATERIALIZE_COLS
        {
            tracing::warn!(
                "[Logic] materialize {at:?} exceeds cap past hwm \
                 (rows {rows:?}, cols {cols:?}); write dropped",
            );
            return Err(Unmaterialized::BeyondCap);
        }
        let before = self.uid_counter;
        let mut row_uids = Vec::new();
        let mut next_row = rows.as_end();
        let mut col_uids = Vec::new();
        let mut next_col = cols.as_end();
        let minted = loop {
            if next_row <= at.row {
                let Some(uid) = self.mint_uid() else {
                    break false;
                };
                row_uids.push(uid);
                next_row += Len::<Row>::ONE;
            } else if next_col <= at.col {
                let Some(uid) = self.mint_uid() else {
                    break false;
                };
                col_uids.push(uid);
                next_col += Len::<Col>::ONE;
            } else {
                break true;
            }
        };
        if !minted {
            // No uid minted here reached the document: hand them back.
            self.uid_counter = before;
            return Err(Unmaterialized::IdsExhausted);
        }
        let grew = !row_uids.is_empty() || !col_uids.is_empty();
        let rows = self
            .doc
            .get_movable_list(SheetContainer::of(SheetRoot::Rows, &self.active_sheet).as_str());
        for uid in row_uids {
            if rows.push(uid.to_key_part().as_str()).is_err() {
                break;
            }
            self.index.push_row(uid);
        }
        let cols = self
            .doc
            .get_movable_list(SheetContainer::of(SheetRoot::Cols, &self.active_sheet).as_str());
        for uid in col_uids {
            if cols.push(uid.to_key_part().as_str()).is_err() {
                break;
            }
            self.index.push_col(uid);
        }
        // Commit the implicit scaffolding under the excluded origin so it stays
        // OUT of the undo stack (see `CommitOrigin::Materialize`). A cell edit's intent
        // is the value, not the row/column placeholders it forced into existence.
        // Keeping those off the stack means undo reverts only the value. On redo,
        // the cell's key then still resolves to the same (never-removed) list
        // element ids. Without this split, Loro would recreate those elements with
        // fresh ids on redo and orphan the cell. A no-op when nothing grew.
        if grew {
            self.doc
                .set_next_commit_origin(ambre_core::crdt::CommitOrigin::Materialize.as_str());
            self.doc.commit();
        }
        self.index.resolve(at).ok_or(Unmaterialized::Unwritten)
    }

    /// Rebuild [`Self::index`] from the active sheet's `rows_/cols_` lists.
    /// Called after any import, where peers' (or our own persisted) structural
    /// ops may have added rows/columns the cache does not yet know about.
    pub(super) fn rebuild_index(&mut self) {
        let rows = self
            .doc
            .get_movable_list(SheetContainer::of(SheetRoot::Rows, &self.active_sheet).as_str());
        let cols = self
            .doc
            .get_movable_list(SheetContainer::of(SheetRoot::Cols, &self.active_sheet).as_str());
        self.index = SheetIndex::build(&rows, &cols);
        self.note_index_read(&self.index);
    }

    /// Build the per-sheet position<->uid index map the formula engine needs to
    /// resolve references. The active sheet is local index 0 (reusing the
    /// maintained [`Self::index`]); other sheets are rebuilt on demand. Local
    /// indices match [`Self::sheets_by_idx`] (active pinned first).
    pub(super) fn engine_indices(&self) -> SheetIndices {
        let mut m = SheetIndices::new();
        for (idx, id) in self.sheets_by_idx.iter().enumerate() {
            let index = if idx == ACTIVE_SHEET.to_usize() {
                self.index.clone()
            } else {
                self.build_sheet_index(id)
            };
            m.insert(Pos::<EngineSheet>::from_usize(idx), index);
        }
        m
    }

    /// Engine [`CellId`] for the cell `at` on the active sheet (local index
    /// 0). Each axis binds to its row/column `Uid` when materialized, else stays
    /// a virtual grid position.
    pub(super) fn active_cell_id(&self, at: CellPos) -> CellId {
        let r = match self.index.row_id(at.row) {
            Some(u) => AxisId::Bound(u),
            None => AxisId::Virtual(at.row),
        };
        let c = match self.index.col_id(at.col) {
            Some(u) => AxisId::Bound(u),
            None => AxisId::Virtual(at.col),
        };
        CellId {
            sheet: ACTIVE_SHEET,
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
    /// A `materialize` commit carries the excluded [`CommitOrigin::Materialize`](ambre_core::crdt::CommitOrigin::Materialize). An
    /// excluded commit interleaved between two value commits splits them into
    /// separate undo items. Materializing up front keeps all value writes in one
    /// final commit, hence one undo item.
    pub(super) fn write_cell(&mut self, at: CellPos, text: &CellText<SourceForm>) {
        if text.is_empty() {
            // Clearing: only touch the CRDT if the cell was materialized.
            if let Some((rl, cl)) = self.index.resolve(at) {
                let _ = self
                    .doc
                    .get_map(SheetContainer::of(SheetRoot::Cells, &self.active_sheet).as_str())
                    .delete(CellKey::of(rl, cl).as_str());
            }
            let cell = self.active_cell_id(at);
            let indices = self.engine_indices();
            let ctx = TrameCtx {
                engine_sheet_by_name: &self.engine_sheet_by_name,
                engine_sheet_by_id: &self.engine_sheet_by_id,
                named: &self.named,
            };
            self.engine
                .on_cell_changed(cell, CellText::<StoredForm>::default(), &indices, &ctx);
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
            let (rl, cl) = match self.materialize(at) {
                Ok(lines) => lines,
                Err(refused) => {
                    // The write is refused, and `finish_cell_writes` says so
                    // (`TRAME-UI-164`).
                    self.write_refusals.push(WriteRefusal::from(refused));
                    return;
                }
            };
            // Bound once the cell's own lines exist, so a reference to its row
            // or column names that line rather than a position.
            let stored = self
                .canonicalize_input(text, &sheet, &self.sheet_lines())
                .unwrap_or_else(|_| CellText::<StoredForm>::new(text.as_str().to_owned()));
            let _ = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, &self.active_sheet).as_str())
                .insert(CellKey::of(rl, cl).as_str(), stored.as_str());
            let cell = CellId::bound(ACTIVE_SHEET, rl, cl);
            let indices = self.engine_indices();
            let ctx = TrameCtx {
                engine_sheet_by_name: &self.engine_sheet_by_name,
                engine_sheet_by_id: &self.engine_sheet_by_id,
                named: &self.named,
            };
            self.engine.on_cell_changed(cell, stored, &indices, &ctx);
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
        let spent = refusals.contains(&WriteRefusal::IdsExhausted);
        let unwritten_lines = refusals.contains(&WriteRefusal::Unwritten);
        let past_largest = refusals.contains(&WriteRefusal::PastLargestNumber);
        let mut unknown: Vec<SheetName> = refusals
            .into_iter()
            .filter_map(|r| match r {
                WriteRefusal::UnknownSheet(name) => Some(name),
                WriteRefusal::BeyondCap
                | WriteRefusal::IdsExhausted
                | WriteRefusal::Unwritten
                | WriteRefusal::PastLargestNumber => None,
            })
            .collect();
        unknown.sort();
        unknown.dedup();
        let mut events = Vec::new();
        if beyond_cap > 0 {
            events.push(TrameEvent::Error {
                message: format!(
                    "{beyond_cap} cell(s) lie beyond the sheet's {}-line limit and were not written",
                    MATERIALIZE_ROWS.get()
                ),
            });
        }
        if spent {
            events.push(ids_exhausted());
        }
        if unwritten_lines {
            events.push(unwritten());
        }
        if past_largest {
            events.push(TrameEvent::Error {
                message: "the series runs past the largest number; the fill stops there".to_owned(),
            });
        }
        events.extend(unknown.into_iter().map(|name| TrameEvent::Error {
            message: format!("no sheet is named \"{name}\": the formula was not written"),
        }));
        events
    }
}

/// Why [`TrameState::materialize`] gave a cell no lines. Each refusal names
/// its own cause, so a batch that hits the cap and then spends the counter
/// reports both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Unmaterialized {
    /// The cell lies past the materialization cap ([`MATERIALIZE_ROWS`]).
    BeyondCap,
    /// The cell needs a new line, and the session cannot mint its uid.
    IdsExhausted,
    /// The document refused a new line, so the cell still has none.
    Unwritten,
}

impl From<Unmaterialized> for WriteRefusal {
    /// The refusal a batch reports for a cell given no lines.
    fn from(refused: Unmaterialized) -> Self {
        match refused {
            Unmaterialized::BeyondCap => Self::BeyondCap,
            Unmaterialized::IdsExhausted => Self::IdsExhausted,
            Unmaterialized::Unwritten => Self::Unwritten,
        }
    }
}

impl Unmaterialized {
    /// The notice for one write refused for this cause (`TRAME-UI-164`).
    pub(super) fn notice(self) -> TrameEvent {
        match self {
            Self::BeyondCap => TrameEvent::Error {
                message: format!(
                    "the cell lies beyond the sheet's {}-line limit and was not written",
                    MATERIALIZE_ROWS.get()
                ),
            },
            Self::IdsExhausted => ids_exhausted(),
            Self::Unwritten => unwritten(),
        }
    }
}

/// The notice for a write whose new row or column the document refused.
fn unwritten() -> TrameEvent {
    TrameEvent::Error {
        message: "the sheet refused a new row or column; the write was dropped".to_owned(),
    }
}

/// The notice for a write that needs a new row or column after the session
/// minted its last uid. A reopened sheet mints under a new PeerID.
pub(super) fn ids_exhausted() -> TrameEvent {
    TrameEvent::Error {
        message: "this session has no fresh row or column id; reopen the sheet to add more"
            .to_owned(),
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
    pub(super) fn of(
        &self,
        sheet_id: &Id<Sheet>,
        q: Option<&SheetQualifier>,
    ) -> Option<&SheetIndex> {
        let state = self.state;
        let idx = match q {
            None => state.engine_sheet_by_id.get(sheet_id),
            Some(SheetQualifier::Id(id)) => state.engine_sheet_by_id.get(id),
            Some(SheetQualifier::Name(name)) => state.engine_sheet_by_name.get(&name.key()),
        }
        .copied()?;
        if idx == ACTIVE_SHEET {
            return Some(&state.index);
        }
        let id = state.sheets_by_idx.get(idx.to_usize())?;
        let cell = self.built.get(idx.to_usize())?;
        Some(cell.get_or_init(|| state.build_sheet_index(id)))
    }
}
