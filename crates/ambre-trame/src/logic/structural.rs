// ambre-trame/src/logic/structural.rs.rs
// Row/column insert/delete, axis purge, structural reseed
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::line_cells;
use super::line_history::ListEdit;
use super::*;
use crate::formula::ACTIVE_SHEET;
use crate::formula::SheetQualifier;
use crate::formula::engine::Carry;
use crate::formula::identity::{self, AxisEdit, LineEdit};
use crate::formula::text::{CellText, StoredForm};
use crate::model::{Axes, CellKey, Removed};
use ambre_core::index::{Len, Pos};

impl TrameState {
    /// Carry every formula that reads the active sheet across one structural
    /// edit of it. The lists already hold the new layout and `self.index`
    /// still the old one. Ranges keep their rectangle through a reorder
    /// (`TRAME-227`), widen on an insert inside them (`TRAME-171`) and shrink
    /// on a delete (`TRAME-230`). Runs before the commit, so the rewrite and
    /// the edit are one undo step. Only a formula whose text changes is
    /// written.
    ///
    /// `gone` names the lines the edit removed. The walk reads only the
    /// formulas the engine names for them. Those read a cell of a gone line,
    /// or read positions (`TRAME-280`). Any other
    /// formula names its cells by lines the edit leaves in place, so its text
    /// cannot change. When the engine cannot name them, the walk reads every
    /// formula, and the engine reseeds.
    ///
    /// `new` is the layout after the edit, which the caller derived.
    ///
    /// Never inlined: `scripts/instructions.py` counts it by its name.
    #[inline(never)]
    pub(super) fn reanchor_formulas(&self, edit: AxisEdit, gone: &[Uid], new: SheetIndex) -> Walk {
        if self.engine_needs_reseed() {
            self.reanchor_every_formula(edit, &new);
            return Walk {
                index: new,
                step: EngineStep::Reseed,
            };
        }
        let removed = self.removed_cells(edit, gone);
        let mut others: HashMap<Pos<EngineSheet>, SheetIndex> = HashMap::new();
        let mut reread = Vec::new();
        for cell in self.engine.structural_reach(&removed) {
            let Some(sheet) = self.sheets_by_idx.get(cell.sheet.to_usize()) else {
                continue;
            };
            let index = if cell.sheet == ACTIVE_SHEET {
                &self.index
            } else {
                others
                    .entry(cell.sheet)
                    .or_insert_with(|| self.build_sheet_index(sheet))
            };
            let (Some(row), Some(col)) = (cell.row_uid(), cell.col_uid()) else {
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
            let Some((key, text)) = index.keys_at(at).into_iter().find_map(|key| {
                match map.get(key.as_str()).map(|v| v.get_deep_value()) {
                    Some(LoroValue::String(text)) => Some((key, text.to_string())),
                    _ => None,
                }
            }) else {
                continue;
            };
            reread.push((
                Reread {
                    cell,
                    sheet: sheet.clone(),
                    key,
                },
                text,
            ));
        }
        // A formula that places on no cell is one the engine names nowhere,
        // and the full walk reads it all the same. Its text changes like any
        // other: a reference to a gone line is frozen (`TRAME-277`).
        let mut walked: Vec<(Id<Sheet>, CellKey, String)> = reread
            .iter()
            .map(|(r, text)| (r.sheet.clone(), r.key.clone(), text.clone()))
            .collect();
        for sheet in &self.sheets_by_idx {
            let Some(keys) = self.unplaced.get(sheet) else {
                continue;
            };
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, sheet).as_str());
            walked.extend(keys.iter().filter_map(|key| {
                match map.get(key.as_str()).map(|v| v.get_deep_value()) {
                    Some(LoroValue::String(text)) => {
                        Some((sheet.clone(), key.clone(), text.to_string()))
                    }
                    _ => None,
                }
            }));
        }
        // The reach comes out of a hash set. Writing in key order makes the
        // operations of one edit the same on every run, whatever the order the
        // set hands the formulas out in.
        walked.sort_unstable_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        for (sheet, key, text) in &walked {
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, sheet).as_str());
            self.reanchor_one(&map, sheet, key.as_str(), text, &new, edit);
        }
        let reread = reread.into_iter().map(|(r, _)| r).collect();
        Walk {
            index: new,
            step: EngineStep::Carry { removed, reread },
        }
    }

    /// The layout of the active sheet after `edit`, whose lists already hold
    /// it. An insert of the line `inserted` derives it from the layout before,
    /// with no read of the lists. A reorder, or a view the derivation cannot
    /// follow, reads the lists. A delete derives its layout as it removes each
    /// line ([`Axes::delete`]).
    pub(super) fn next_index(&self, edit: AxisEdit, inserted: Option<Uid>) -> SheetIndex {
        let derived = match (edit, inserted) {
            (AxisEdit::Rows(LineEdit::Insert { at, .. }), Some(uid)) => {
                self.index.clone().with_line::<Row>(at, uid)
            }
            (AxisEdit::Cols(LineEdit::Insert { at, .. }), Some(uid)) => {
                self.index.clone().with_line::<Col>(at, uid)
            }
            _ => None,
        };
        derived.unwrap_or_else(|| self.build_sheet_index(&self.active_sheet))
    }

    /// Take what the walk left once the edit commits. Its layout becomes the
    /// position cache. Its engine step goes back to the caller, which runs it
    /// after the merges are rebuilt.
    pub(super) fn adopt_walk(&mut self, walk: Walk) -> EngineStep {
        self.adopt_index(walk.index);
        walk.step
    }

    /// Take the layout the walk derived as the position cache. Each test
    /// checks it against a read of the lists.
    fn adopt_index(&mut self, index: SheetIndex) {
        #[cfg(test)]
        assert_eq!(
            index,
            self.axes().index(),
            "the derived layout is the lists'"
        );
        self.note_index_read(&index);
        self.index = index;
    }

    /// The walk over every formula of every sheet, for an engine that cannot
    /// name the formulas an edit moves.
    fn reanchor_every_formula(&self, edit: AxisEdit, new: &SheetIndex) {
        for sheet in &self.sheets_by_idx {
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, sheet).as_str());
            let LoroValue::Map(cells) = map.get_value() else {
                continue;
            };
            for (key, value) in cells.iter() {
                let LoroValue::String(text) = value else {
                    continue;
                };
                self.reanchor_one(&map, sheet, key, text, new, edit);
            }
        }
    }

    /// Re-anchor the stored text `text` of the cell `key` of `sheet`, and
    /// write it back when it changed.
    fn reanchor_one(
        &self,
        map: &loro::LoroMap,
        sheet: &Id<Sheet>,
        key: &str,
        text: &str,
        new: &SheetIndex,
        edit: AxisEdit,
    ) {
        let active = &self.active_sheet;
        let Some(body) = text.strip_prefix('=') else {
            return;
        };
        // Another sheet's formula reads this one only through a qualifier.
        if sheet != active && !may_qualify(body) {
            return;
        }
        let Ok(ast) = parse_formula(body) else {
            return;
        };
        let edited = |q: Option<&SheetQualifier>| match q {
            None => sheet == active,
            Some(SheetQualifier::Id(id)) => id == active,
            Some(SheetQualifier::Name(name)) => {
                self.engine_sheet_by_name
                    .get(&name.key())
                    .and_then(|idx| self.sheets_by_idx.get(idx.to_usize()))
                    == Some(active)
            }
        };
        // Most formulas name no line the edit moves: those render and write
        // nothing.
        let Some(after) = identity::reanchor(ast, &edited, &self.index, new, edit) else {
            return;
        };
        let after = render::render(&after);
        if after != body {
            self.note_write(map, key);
            let _ = map.insert(key, format!("={after}").as_str());
        }
    }

    /// Whether the engine cannot carry a structural edit and must reseed.
    ///
    /// A formula refused for a cycle keeps no edge, so the graph cannot say
    /// whether the edit frees it. A reseed decides it by its feed order.
    /// A cell the last reseed left out of the engine, a masked one or one
    /// under a second key, is a formula no dependency names.
    fn engine_needs_reseed(&self) -> bool {
        #[cfg(test)]
        if self.structural_path == StructuralPath::Reseed {
            return true;
        }
        self.engine_holds_a_skip()
    }

    /// Whether the engine holds what no dependency names: a formula refused
    /// for a cycle, a masked cell, or a cell the last reseed left out. Neither
    /// a structural edit nor a remote batch can carry such an engine.
    pub(super) fn engine_holds_a_skip(&self) -> bool {
        self.engine.has_cyclic() || self.engine_skipped || !self.masked_cells.is_empty()
    }

    /// The engine cells of the active sheet on the lines `gone`: each line
    /// crossed with every line of the other axis in the old layout. The
    /// engine feeds a stored cell under its two lines, so no other cell of a
    /// gone line holds a value.
    fn removed_cells(&self, edit: AxisEdit, gone: &[Uid]) -> HashSet<CellId> {
        let mut cells = HashSet::new();
        for &line in gone {
            match edit {
                AxisEdit::Rows(_) => cells.extend(
                    self.index
                        .cols()
                        .lines()
                        .iter()
                        .map(|&col| CellId::bound(ACTIVE_SHEET, line, col)),
                ),
                AxisEdit::Cols(_) => cells.extend(
                    self.index
                        .rows()
                        .lines()
                        .iter()
                        .map(|&row| CellId::bound(ACTIVE_SHEET, row, line)),
                ),
            }
        }
        cells
    }

    /// Carry the engine across a committed structural edit: the step the walk
    /// chose. A carried step that meets a cycle reseeds after all, so the
    /// feed order decides which cell of it is refused.
    ///
    /// Never inlined: `scripts/instructions.py` counts it by its name.
    #[inline(never)]
    pub(super) fn carry_engine(&mut self, step: EngineStep) {
        let EngineStep::Carry { removed, reread } = step else {
            self.reseed_engine();
            return;
        };
        // An edit can mask a cell that held a value: the masked cell leaves the
        // engine, which only a reseed does.
        if !self.masked_cells.is_empty() {
            self.reseed_engine();
            return;
        }
        // The active sheet's layout is lent to the engine for the call, not
        // copied, as the remote step lends it.
        let mut indices = self.other_engine_indices();
        indices.insert(ACTIVE_SHEET, std::mem::take(&mut self.index));
        let texts: Vec<(CellId, CellText<StoredForm>)> = reread
            .into_iter()
            .map(|r| {
                let text = match self
                    .doc
                    .get_map(SheetContainer::of(SheetRoot::Cells, &r.sheet).as_str())
                    .get(r.key.as_str())
                    .map(|v| v.get_deep_value())
                {
                    Some(LoroValue::String(text)) => text.to_string(),
                    _ => String::new(),
                };
                (r.cell, CellText::<StoredForm>::new(text))
            })
            .collect();
        let ctx = TrameCtx {
            engine_sheet_by_name: &self.engine_sheet_by_name,
            engine_sheet_by_id: &self.engine_sheet_by_id,
            named: &self.named,
        };
        let carried = self
            .engine
            .apply_structural(&removed, &texts, &indices, &ctx);
        if let Some(index) = indices.remove(&ACTIVE_SHEET) {
            self.index = index;
        }
        if carried == Carry::CycleMet {
            self.reseed_engine();
        }
    }

    /// Insert a blank row/column at grid position `at`, shifting later elements
    /// down/right. One `LoroMovableList` op; cells keyed by stable id pairs
    /// follow their element automatically. A no-op past the materialized used
    /// range (those rows/columns are already virtual blanks, nothing to shift).
    pub(super) fn insert_axis<L: SheetLine>(&mut self, at: Pos<L>) -> TrameOutcome {
        if at > L::len(&self.index).as_end() {
            return TrameOutcome::display_only(vec![]);
        }
        // The new line carries a freshly minted, move-invariant uid value
        // (`trame-rows-and-columns`).
        let Some(uid) = self.mint_uid() else {
            return TrameOutcome::display_only(vec![super::cells::ids_exhausted()]);
        };
        // The insert records its own inverse, as a delete does (`TRAME-289`).
        self.open_journal();
        let Some(spot) = self.axes().insert(at, uid) else {
            self.drop_journal();
            return TrameOutcome::display_only(vec![]);
        };
        self.note_list(self.axes().list_id::<L>(), ListEdit::Inserted, &[spot]);
        let edit = L::edit(LineEdit::Insert { at, n: Len::ONE });
        let walk = self.reanchor_formulas(edit, &[], self.next_index(edit, Some(uid)));
        self.commit_recorded();
        self.after_structural(Some(walk))
    }

    /// Delete the row/column at grid position `at`, shifting later elements
    /// up/left and purging the removed element's cells and formatting. A
    /// reference names its line, so one to a deleted line reads `#REF!`
    /// (`TRAME-008`). One below it follows its cell up, and a range shrinks
    /// ([`reanchor_formulas`](Self::reanchor_formulas)). A no-op in virtual
    /// space.
    pub(super) fn delete_axis<L: SheetLine>(&mut self, at: Pos<L>) -> TrameOutcome {
        // The delete reads the layout it already holds, not the lists.
        let mut layout = Some(self.index.clone());
        self.open_journal();
        let Some(removal) = self.remove_line(&mut layout, at) else {
            self.drop_journal();
            return TrameOutcome::display_only(vec![]);
        };
        let new = self.layout_after(layout);
        let walk = self.reanchor_formulas(
            L::edit(LineEdit::Delete { at, n: Len::ONE }),
            &[removal.uid],
            new,
        );
        self.commit_recorded();
        self.after_line_removal(walk, removal.filter_dropped)
    }

    /// The outcome of a committed removal: the structural refresh, and the
    /// notice that the filter went with its column when it did.
    fn after_line_removal(&mut self, walk: Walk, filter_dropped: bool) -> TrameOutcome {
        let mut outcome = self.after_structural(Some(walk));
        if filter_dropped {
            outcome.display.push(TrameEvent::FilterCleared);
        }
        outcome
    }

    /// The axes of the active sheet, the one door to its line order.
    pub(super) fn axes(&self) -> Axes<'_> {
        Axes::of(&self.doc, &self.active_sheet)
    }

    /// The layout after a delete: the one its removals derived, or a read of
    /// the lists when they could not derive it.
    fn layout_after(&self, layout: Option<SheetIndex>) -> SheetIndex {
        layout.unwrap_or_else(|| self.build_sheet_index(&self.active_sheet))
    }

    /// Remove the row/column at grid position `at` and purge what it carried,
    /// with no commit, so several removals land as one. `layout` is the layout
    /// before the removal, and the line leaves it ([`Axes::delete`]). `None`
    /// when the line is virtual and nothing was removed.
    fn remove_line<L: SheetLine>(
        &mut self,
        layout: &mut Option<SheetIndex>,
        at: Pos<L>,
    ) -> Option<Removal> {
        // A virtual line has nothing materialized to remove. Every element of
        // the line goes, the hidden ones too (`TRAME-269`).
        let removed = self.axes().delete(layout, at)?;
        self.note_list(
            self.axes().list_id::<L>(),
            ListEdit::Removed,
            &removed.spots,
        );
        let filter_dropped = self.purge_axis(&removed, L::AXIS);
        Some(Removal {
            uid: removed.uid,
            filter_dropped,
        })
    }

    /// Move the row/column at grid position `from` to grid position `to`
    /// (Excel-style drag reorder), realized as a single `LoroMovableList::mov` on
    /// `rows_/cols_{sheet}`. Cells keyed by the move-invariant `(row_uid, col_uid)`
    /// pair travel with their line for free. So does every reference to a
    /// cell, while a range keeps its rectangle (`after_reorder`, `TRAME-227`). A no-op
    /// if `from == to` or either index is outside the materialized used range
    /// (virtual lines carry nothing to move).
    pub(super) fn move_axis<L: SheetLine>(&mut self, from: Pos<L>, to: Pos<L>) -> TrameOutcome {
        let end = L::len(&self.index).as_end();
        if from == to || from >= end || to >= end {
            return TrameOutcome::display_only(vec![]);
        }
        if self.axes().move_line(from, to).is_none() {
            return TrameOutcome::display_only(vec![]);
        }
        let edit = L::edit(LineEdit::Reorder);
        let walk = self.reanchor_formulas(edit, &[], self.next_index(edit, None));
        self.doc.commit();
        self.after_reorder(walk)
    }

    /// Delete every row/column spanned by the current selection rectangle,
    /// iterating from the far edge inward so earlier positions stay valid as the
    /// list shifts. The removals share one commit: one undo step (`TRAME-088`)
    /// and one delta carrying every line to the peers (`TRAME-175`). The
    /// selection is left untouched -- the UI owns it and re-reports it on the
    /// next cursor move.
    pub(super) fn delete_selected_axis<L: SheetLine>(&mut self) -> TrameOutcome {
        // The span of every selected rectangle across the axis (`TRAME-UI-129`).
        let Some((start, end)) = self
            .selection_rects()
            .into_iter()
            .map(L::bounds)
            .reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b)))
        else {
            return TrameOutcome::display_only(vec![]);
        };
        let mut removals: Vec<Removal> = Vec::new();
        let mut layout = Some(self.index.clone());
        let mut at = Some(end);
        self.open_journal();
        while let Some(line) = at.filter(|line| *line >= start) {
            removals.extend(self.remove_line(&mut layout, line));
            at = line.prev();
        }
        if removals.is_empty() {
            self.drop_journal();
            return TrameOutcome::display_only(vec![]);
        }
        // The removed lines are the materialized run from `start`.
        let n = Len::of(removals.len());
        let gone: Vec<Uid> = removals.iter().map(|r| r.uid).collect();
        let new = self.layout_after(layout);
        let walk = self.reanchor_formulas(L::edit(LineEdit::Delete { at: start, n }), &gone, new);
        self.commit_recorded();
        self.after_line_removal(walk, removals.iter().any(|r| r.filter_dropped))
    }

    /// The range [`sort_selection`](Self::sort_selection) and the multi-key sort
    /// would order, for the sort dialog (`TRAME-UI-101`). It reads the same
    /// [`selection_bounds`](Self::selection_bounds), so the columns the dialog
    /// offers are the columns the sort orders.
    pub(super) fn sort_range_query(&self) -> TrameOutcome {
        let rect = self.selection_bounds();
        TrameOutcome::display_only(vec![TrameEvent::SortRange {
            r1: rect.top_left.row,
            c1: rect.top_left.col,
            r2: rect.bottom_right.row,
            c2: rect.bottom_right.col,
        }])
    }

    /// Single-column shared sort resolved against the tracked selection rectangle
    /// (`trame-sort`). Sorts on the range's first column with no header row,
    /// exactly the parameterless context-menu intent. Whole rows are permuted, so
    /// cells outside the selection move with their row (Google Sheets "sort
    /// range"). Delegates to [`sort_range`](Self::sort_range) on the active
    /// sheet, which guards the empty/single-row cases and replies with
    /// [`TrameEvent::SortComplete`].
    pub(super) fn sort_selection(&mut self, order: SortOrder) -> TrameOutcome {
        let rect = self.selection_bounds();
        let sheet = self.active_sheet.clone();
        self.sort_range(&sheet, rect, Len::<Col>::ZERO, order, HeaderRow::Absent)
    }

    /// Whole-sheet single-column sort by the active cell's column (`trame-sort`,
    /// the "sort by column" affordance for a whole-column selection).
    /// Sorts every materialized row of the sheet across the full used-column span,
    /// so a row's cells outside the active column travel with it. The active
    /// column is passed as `sort_col` over a range anchored at column 0, with no
    /// header row. Delegates to [`sort_range`](Self::sort_range), which guards the
    /// empty range and replies with [`TrameEvent::SortComplete`].
    pub(super) fn sort_sheet(&mut self, order: SortOrder) -> TrameOutcome {
        let rect = CellRect::spanning(
            CellPos::ORIGIN,
            CellPos::new(
                Row::len(&self.index).last().unwrap_or(Pos::<Row>::ZERO),
                Col::len(&self.index).last().unwrap_or(Pos::<Col>::ZERO),
            ),
        );
        let sort_col = self.active.col.len_from(Pos::<Col>::ZERO);
        let sheet = self.active_sheet.clone();
        self.sort_range(&sheet, rect, sort_col, order, HeaderRow::Absent)
    }

    /// Multi-column sort over the tracked selection rectangle (`trame-sort`,
    /// the "Sort range..." dialog). Parameterless sibling of
    /// [`sort_selection`](Self::sort_selection): resolves the range against the
    /// tracked selection and delegates to
    /// [`sort_range_multi`](Self::sort_range_multi) on the active sheet. Keys are
    /// in decreasing priority; each `col` is range-relative (`0` = the selection's
    /// first column).
    pub(super) fn sort_selection_multi(
        &mut self,
        sort_keys: &[SortKey],
        header: HeaderRow,
    ) -> TrameOutcome {
        let rect = self.selection_bounds();
        let sheet = self.active_sheet.clone();
        self.sort_range_multi(&sheet, rect, sort_keys, header)
    }

    /// Drop every cell and formatting entry whose key references the removed
    /// row/column id, so a deleted line leaves no orphan entries behind.
    /// `true` when the removed column carried the active filter, which went
    /// with it.
    ///
    /// The keys come from the index by line, not from a walk of the maps
    /// (`TRAME-081`). Each test checks them against that walk.
    pub(super) fn purge_axis(&mut self, removed: &Removed, axis: Axis) -> bool {
        for root in line_cells::PURGED {
            let mut stale: Vec<CellKey> = {
                let lines = line_cells::read(&self.lines);
                removed
                    .elements
                    .iter()
                    .flat_map(|element| lines.keys_on(root, &self.active_sheet, axis, element))
                    .collect()
            };
            stale.sort_unstable();
            #[cfg(test)]
            assert_eq!(
                stale,
                self.walked_stale(root, removed, axis),
                "the index by line holds the keys a walk finds"
            );
            let map = self
                .doc
                .get_map(SheetContainer::of(root, &self.active_sheet).as_str());
            for key in stale {
                self.note_write(&map, key.as_str());
                let _ = map.delete(key.as_str());
                // A formula under a gone line of the other axis goes too, and
                // the walk no longer reads it (`TRAME-280`).
                if root == SheetRoot::Cells
                    && let Some(keys) = self.unplaced.get_mut(&self.active_sheet)
                {
                    keys.remove(&key);
                }
            }
        }
        // A filter on the removed column goes with it (`TRAME-FEAT-030`).
        let filter_dropped = matches!(axis, Axis::Col)
            && self
                .load_active_filter()
                .is_some_and(|(col, _)| col == removed.uid);
        if filter_dropped {
            self.drop_filter();
        }
        // The generations recorded after the line go with it.
        self.axes()
            .purge_record(&removed.uid, |map, key| self.note_write(map, key));
        // The line's own formatting is keyed by its uid alone: its size, and
        // for a row its filter mark (`TRAME-082`).
        let line_maps = match axis {
            Axis::Row => vec![SheetRoot::RowHeights, SheetRoot::FilterHidden],
            Axis::Col => vec![SheetRoot::ColWidths],
        };
        let key = removed.uid.to_key_part();
        for root in line_maps {
            let map = self
                .doc
                .get_map(SheetContainer::of(root, &self.active_sheet).as_str());
            if map.get(&key).is_some() {
                self.note_write(&map, &key);
                let _ = map.delete(&key);
            }
        }
        filter_dropped
    }

    /// The keys a walk of the whole map `root` finds under the elements
    /// `removed` of `axis`, sorted. It is the reference model of the index
    /// by line.
    #[cfg(test)]
    fn walked_stale(&self, root: SheetRoot, removed: &Removed, axis: Axis) -> Vec<CellKey> {
        let map = self
            .doc
            .get_map(SheetContainer::of(root, &self.active_sheet).as_str());
        let LoroValue::Map(entries) = map.get_value() else {
            return Vec::new();
        };
        let mut keys: Vec<CellKey> = entries
            .keys()
            .filter_map(|k| parse_cell_key(k))
            .filter(|(row, col)| {
                removed.elements.contains(match axis {
                    Axis::Row => row,
                    Axis::Col => col,
                })
            })
            .map(|(row, col)| CellKey::of(row, col))
            .collect();
        keys.sort_unstable();
        keys
    }

    /// Shared tail of every structural op. It rebuilds the position <-> id
    /// cache, since positions shifted. It carries the formula engine across
    /// the edit through `step`, and reseeds the formatting sets from the new
    /// layout. It then flushes the CRDT delta and asks the UI to re-fetch its
    /// viewport. With no walk, as after an undo, it reads the lists and
    /// reseeds the engine.
    pub(super) fn after_structural(&mut self, walk: Option<Walk>) -> TrameOutcome {
        let step = match walk {
            Some(walk) => self.adopt_walk(walk),
            None => {
                self.rebuild_index();
                EngineStep::Reseed
            }
        };
        // Merges resolve through the freshly rebuilt index; do this before the
        // engine step so masked cells are excluded from the DAG.
        let mut display = self.rebuild_merges();
        self.carry_engine(step);
        self.reseed_styles();
        // The hidden set is keyed by stable row uid, so a structural shift just
        // remaps it to the new positions (no recompute).
        self.rebuild_filter();
        // So are the sizes: each follows its own line (`TRAME-170`,
        // `TRAME-UI-037`).
        self.reseed_metrics();
        let update = self.flush_delta();
        display.push(TrameEvent::DataReset);
        TrameOutcome {
            display,
            update,
            ephemeral: None,
        }
    }
}

/// What a structural edit leaves the engine to do once it commits.
pub(super) enum EngineStep {
    /// Reseed the whole engine from the document.
    Reseed,
    /// Clear the cells of the gone lines, and compile the re-read formulas
    /// again from their text.
    Carry {
        removed: HashSet<CellId>,
        reread: Vec<Reread>,
    },
}

/// What the walk leaves for the tail of a structural edit.
pub(super) struct Walk {
    /// The layout of the active sheet after the edit.
    index: SheetIndex,
    /// What the engine does once the edit commits.
    step: EngineStep,
}

/// A formula the walk read, and where its text lives.
pub(super) struct Reread {
    cell: CellId,
    sheet: Id<Sheet>,
    key: CellKey,
}

/// The path a structural edit takes, for the test that holds the carried
/// engine against a full rebuild (`TRAME-280`).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StructuralPath {
    /// The walk narrowed to the formulas the engine names, then a carried step.
    Carry,
    /// The reference model: the walk over every formula, then a full reseed.
    Reseed,
}

/// One line removed, before the commit.
struct Removal {
    uid: Uid,
    /// Whether the active filter went with the line.
    filter_dropped: bool,
}

/// Whether a stored formula body may name another sheet. Every qualifier ends
/// on `!`, and the element separator `#` of a derived reference is no
/// qualifier. A formula of another sheet that cites its own lines alone is
/// then never parsed by a structural edit of this one.
fn may_qualify(body: &str) -> bool {
    body.contains('!')
}

#[cfg(test)]
mod tests {
    use super::may_qualify;

    /// A derived reference carries `#`, and a qualifier `!`: only the second
    /// sends a formula of another sheet through the walk.
    #[test]
    fn only_a_qualifier_sends_another_sheets_formula_through_the_walk() {
        assert!(
            !may_qualify("{a1_2#b3_4|c5_6#d7_8}*2"),
            "an element separator"
        );
        assert!(!may_qualify("SUM({a1_2|c5_6}:{a1_3|c5_6})"));
        assert!(may_qualify("#s1_2!{a1_2#b3_4|c5_6}+1"), "an id qualifier");
        assert!(may_qualify("'My Sheet'!A1"), "a name qualifier");
    }
}
