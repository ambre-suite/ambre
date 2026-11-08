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
use crate::model::{Axes, CellRank, ElementId, LineRead};

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
        let map = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Cells, &self.active_sheet).as_str());
        if let Some(LoroValue::String(s)) = cell_value(&map, &self.index, at) {
            return s.to_string();
        }
        String::new()
    }

    /// Raw cell text as stored in the CRDT (the formula source for a formula
    /// cell, the literal otherwise). Empty for virtual/cleared cells.
    pub(super) fn cell_raw(&self, at: CellPos) -> CellText<StoredForm> {
        let map = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Cells, &self.active_sheet).as_str());
        if let Some(LoroValue::String(s)) = cell_value(&map, &self.index, at) {
            return CellText::<StoredForm>::new(s.to_string());
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
    /// cell with. Extends the dense used range by pushing one element per new
    /// line, each line under a uid derived from the line before it. The write is refused when the
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
        // Each minted id names the element that carries a line, never the line
        // itself. The line takes a uid derived from the line before it. So two
        // peers writing at once in one new line add one line (`TRAME-261`).
        let row_ids: Vec<ElementId> = row_uids.into_iter().map(ElementId::minted).collect();
        let col_ids: Vec<ElementId> = col_uids.into_iter().map(ElementId::minted).collect();
        let axes = Axes::of(&self.doc, &self.active_sheet);
        let (Some(rows), Some(cols)) = (
            axes.plan::<Row>(&self.index, &row_ids),
            axes.plan::<Col>(&self.index, &col_ids),
        ) else {
            self.uid_counter = before;
            return Err(Unmaterialized::IdsExhausted);
        };
        // A refused push leaves the index as the lines that landed; the resolve
        // below then answers the refusal.
        let _ = axes
            .grow(&mut self.index, rows)
            .and_then(|()| axes.grow(&mut self.index, cols));
        // A formula that read a position past the used range now reads a
        // line, as a reseed would bind it (`TRAME-289`).
        if grew {
            self.bind_grown_readers();
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

    /// Compile again each formula that reads, by a position, a line the
    /// active sheet just gained. The compile binds the position to the line,
    /// as a full reseed does. The next edit then finds the dependency on the
    /// line's cell, and not on a position (`TRAME-289`). A compile that meets
    /// a cycle reseeds, so the feed order decides which cell is refused.
    fn bind_grown_readers(&mut self) {
        let bound = self.engine.bound_by_growth(ACTIVE_SHEET, &self.index);
        if bound.is_empty() {
            return;
        }
        let mut indices = self.other_engine_indices();
        let mut reread = Vec::with_capacity(bound.len());
        for cell in bound {
            let index = if cell.sheet == ACTIVE_SHEET {
                &self.index
            } else {
                let Some(index) = indices.get(&cell.sheet) else {
                    continue;
                };
                index
            };
            let (Some(sheet), Some(row), Some(col)) = (
                self.sheets_by_idx.get(cell.sheet.to_usize()),
                cell.row_uid(),
                cell.col_uid(),
            ) else {
                continue;
            };
            let Some(at) = index.pos_of(&row, &col) else {
                continue;
            };
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, sheet).as_str());
            // The key the engine was fed: the first one the map holds, as
            // every reader takes it (`TRAME-279`).
            let Some(text) = index.keys_at(at).into_iter().find_map(|key| {
                match map.get(key.as_str()).map(|v| v.get_deep_value()) {
                    Some(LoroValue::String(text)) => Some(text.to_string()),
                    _ => None,
                }
            }) else {
                continue;
            };
            reread.push((cell, CellText::<StoredForm>::new(text)));
        }
        // The active sheet's layout is lent to the engine for the call, not
        // copied, as the structural step lends it.
        indices.insert(ACTIVE_SHEET, std::mem::take(&mut self.index));
        let ctx = TrameCtx {
            engine_sheet_by_name: &self.engine_sheet_by_name,
            engine_sheet_by_id: &self.engine_sheet_by_id,
            named: &self.named,
        };
        let carried = self
            .engine
            .apply_structural(&HashSet::default(), &reread, &indices, &ctx);
        if let Some(index) = indices.remove(&ACTIVE_SHEET) {
            self.index = index;
        }
        if carried == crate::formula::engine::Carry::CycleMet {
            self.reseed_engine();
        }
    }

    /// Rebuild [`Self::index`] from the active sheet's `rows_/cols_` lists.
    /// Called after any import, where peers' (or our own persisted) structural
    /// ops may have added rows/columns the cache does not yet know about.
    pub(super) fn rebuild_index(&mut self) {
        self.index = self.axes().index();
        self.note_index_read(&self.index);
    }

    /// Build the per-sheet position<->uid index map the formula engine needs to
    /// resolve references. The active sheet is local index 0 (reusing the
    /// maintained [`Self::index`]); other sheets are rebuilt on demand. Local
    /// indices match [`Self::sheets_by_idx`] (active pinned first).
    pub(super) fn engine_indices(&self) -> HashMap<Pos<EngineSheet>, SheetIndex> {
        let mut m = HashMap::default();
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
            // Clearing: only touch the CRDT if the cell was materialized. The
            // cell goes under every live element of its lines, so no value a
            // hidden element holds shows through (`TRAME-279`).
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, &self.active_sheet).as_str());
            for key in self.index.keys_at(at) {
                if map.get(key.as_str()).is_some() {
                    let _ = map.delete(key.as_str());
                }
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
            // The view drew the write already, and an answer clears its mark.
            self.engine.changed.insert(cell);
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
            // The cell is keyed by the element of each line this peer sees. A
            // concurrent delete of another element then spares it (`TRAME-265`).
            let Some(key) = self.index.key_at(at) else {
                return;
            };
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, &self.active_sheet).as_str());
            let _ = map.insert(key.as_str(), stored.as_str());
            // A value a peer wrote under a hidden element of the same lines
            // stays (`TRAME-279`), and the cell then sits under two keys. A
            // reseed leaves such a cell out and marks the engine, so the next
            // structural edit reseeds too (`TRAME-280`). The write marks it
            // the same way.
            if self
                .index
                .keys_at(at)
                .iter()
                .filter(|k| map.get(k.as_str()).is_some())
                .nth(1)
                .is_some()
            {
                self.engine_skipped = true;
            }
            let cell = CellId::bound(ACTIVE_SHEET, rl, cl);
            let indices = self.engine_indices();
            let ctx = TrameCtx {
                engine_sheet_by_name: &self.engine_sheet_by_name,
                engine_sheet_by_id: &self.engine_sheet_by_id,
                named: &self.named,
            };
            self.engine.on_cell_changed(cell, stored, &indices, &ctx);
            // The view drew the write already, and an answer clears its mark.
            self.engine.changed.insert(cell);
        }
    }

    /// Commit the pending cell write(s), recompute the dirty formula closure and
    /// the active filter, then return the standard write outcome. That outcome
    /// names the cells the writes and their readers changed, so the UI
    /// re-fetches those alone (`TRAME-284`), plus the CRDT delta. Shared tail
    /// of the single-cell and batch write paths.
    pub(super) fn finish_cell_writes(&mut self) -> TrameOutcome {
        self.doc.commit();
        let indices = self.engine_indices();
        self.engine.compute_all_dirty(&indices);
        // The edit may change the filtered column's value: recompute the active
        // filter so the row's hidden status tracks the data
        // (`trame-shared-filter`).
        let hidden = self.hidden_rows.clone();
        self.recompute_active_filter();
        let update = self.flush_delta();
        // The written cells and every formula they feed render their computed
        // result at once, without waiting for the network echo. A refused write
        // puts back what the UI drew optimistically. A row the filter hid or
        // showed moves every row below it. Both re-fetch the whole window.
        let refusals = self.take_write_refusals();
        let repaint = if refusals.is_empty() && hidden == self.hidden_rows {
            self.take_engine_changes(repaint::Repaint::Cells(None))
        } else {
            self.engine.changed.clear();
            repaint::Repaint::Whole
        };
        let mut display: Vec<TrameEvent> = self.repaint_event(repaint).into_iter().collect();
        display.extend(refusals);
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

/// The value a reader shows for the cell at `at` of `map`: the first key of
/// [`SheetIndex::keys_at`] the map holds (`TRAME-279`).
pub(super) fn cell_value(
    map: &loro::LoroMap,
    index: &SheetIndex,
    at: CellPos,
) -> Option<LoroValue> {
    index
        .keys_at(at)
        .iter()
        .find_map(|key| map.get(key.as_str()))
        .map(|v| v.get_deep_value())
}

/// The entries of a cell, style or merge map a reader shows (`TRAME-279`).
/// Each key reads on the cell of its two elements, and a key whose element is
/// gone reads nowhere. Of two keys on one cell, the smaller [`CellRank`] wins:
/// the one written under the visible element of each line. The other stays in
/// the document, unseen. The entries come back in key order, so a caller that
/// feeds them to the engine feeds them alike on every peer. The flag says
/// whether a key no build writes was skipped (`SCHEMA-003`).
pub(super) fn shown_cells<'a>(
    index: &SheetIndex,
    entries: impl IntoIterator<Item = (&'a String, &'a LoroValue)>,
) -> (Vec<(CellPos, &'a String, &'a LoroValue)>, LineRead) {
    let mut read = LineRead::Clean;
    let mut best: HashMap<CellPos, (CellRank, &'a String, &'a LoroValue)> = HashMap::default();
    for (key, value) in entries {
        if parse_cell_key(key).is_none() {
            read = LineRead::Skipped;
            continue;
        }
        let Some((at, rank)) = index.place(key) else {
            continue;
        };
        match best.get(&at) {
            Some((held, ..)) if *held <= rank => {}
            _ => {
                best.insert(at, (rank, key, value));
            }
        }
    }
    let mut cells: Vec<_> = best
        .into_iter()
        .map(|(at, (_, key, value))| (at, key, value))
        .collect();
    cells.sort_unstable_by(|a, b| a.1.cmp(b.1));
    (cells, read)
}
