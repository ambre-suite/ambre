// ambre-trame/src/logic/filters.rs.rs
// Shared filter apply / clear / recompute
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;
use ambre_core::id::{Id, Sheet};
use ambre_core::index::{Len, Pos, Span};

impl TrameState {
    // -------------------------------------------------------------------------
    // Shared filter (apply / clear / recompute) -- `trame-shared-filter`
    // -------------------------------------------------------------------------

    /// Apply a shared filter to grid column `col` of the active sheet
    /// (`trame-shared-filter`).
    ///
    /// Evaluates each materialized row's cell in that column through the formula
    /// cache. Writes the rows failing the filter into `filter_hidden_{sheet}`,
    /// keyed by `row_uid`. Persists the filter definition for convergence and
    /// later recompute, then broadcasts the delta. A no-op for a non-active
    /// sheet or a virtual (never-materialized) column.
    pub(super) fn apply_filter(
        &mut self,
        sheet_id: &Id<Sheet>,
        col: Pos<Col>,
        kind: FilterKind,
    ) -> TrameOutcome {
        // A filter that cannot apply says so (`TRAME-FEAT-037`): the popover
        // would otherwise close on a column that was never filtered.
        if *sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![TrameEvent::Error {
                message: "the filter applies to the sheet on screen only".to_owned(),
            }]);
        }
        let Some(col_uid) = self.index.col_id(col) else {
            return TrameOutcome::display_only(vec![TrameEvent::Error {
                message: "this column holds no data to filter".to_owned(),
            }]);
        };
        // The sort keys read from the formula cache; make sure it is current.
        let indices = self.engine_indices();
        self.engine.compute_all_dirty(&indices);

        let count = self.write_filter_hidden(col_uid, &kind);
        self.persist_filter_def(col_uid, &kind);
        self.doc.commit();
        self.rebuild_filter();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![
                TrameEvent::FilterApplied {
                    hidden_row_count: count,
                    col,
                },
                TrameEvent::DataReset,
            ],
            update,
            ephemeral: None,
        }
    }

    /// Collect the distinct visible values of the active cell's column for the
    /// filter popover's "by values" list (`trame-shared-filter`). Evaluates every
    /// non-hidden materialized row through the formula cache, dedups by display
    /// text, sorts, and replies with [`TrameEvent::ColumnValues`]. A read-only
    /// query (no delta); replies with an empty list for a virtual column.
    pub(super) fn active_column_values(&mut self) -> TrameOutcome {
        let col = self.active.col;
        let Some(col_uid) = self.index.col_id(col) else {
            return TrameOutcome::display_only(vec![TrameEvent::ColumnValues {
                col,
                values: vec![],
            }]);
        };
        // The popover lists current results; make sure the cache is up to date.
        let indices = self.engine_indices();
        self.engine.compute_all_dirty(&indices);
        let col_pos = self.index.col_pos(&col_uid).unwrap_or(col);
        let cells: Vec<CellValue> = Span::<Row>::from_len(Pos::<Row>::ZERO, self.index.row_len())
            .into_iter()
            // Skip rows already hidden by an active filter (distinct *visible*
            // values, matching Google Sheets).
            .filter(|pos| !self.hidden_rows.contains(pos))
            .filter_map(|pos| {
                match self
                    .engine
                    .evaluated_value(self.active_cell_id(CellPos::new(pos, col_pos)))
                {
                    None | Some(CellValue::Empty) => None,
                    Some(value) => Some(value),
                }
            })
            .collect();
        // Ordered as the column reads, numbers numerically, by the comparator
        // a sort uses (`TRAME-FEAT-041`). That comparator ties texts that
        // differ only in case, and their display text breaks the tie. Each
        // spelling lands next to its twin for the dedup, in one order
        // whatever the column order (`TRAME-257`).
        let mut keyed: Vec<(CellValue, String)> = cells
            .into_iter()
            .map(|value| {
                let text = value.to_display();
                (value, text)
            })
            .collect();
        keyed.sort_by(|(a, at), (b, bt)| compare_cell_values(a, b).then_with(|| at.cmp(bt)));
        let mut values: Vec<String> = keyed.into_iter().map(|(_, text)| text).collect();
        values.dedup();
        TrameOutcome::display_only(vec![TrameEvent::ColumnValues { col, values }])
    }

    /// Aggregate the tracked selection for the status bar (`trame-status-bar`).
    /// Walks the selection rectangle, pulling each cell's evaluated value through
    /// the formula cache. Rows a filter hides count, as they do in `SUM`, so the
    /// bar and a formula over the same cells read one total (`TRAME-FEAT-029`).
    /// The sum is the exact one `SUM` takes, for the same reason. `count`
    /// counts non-empty cells; numeric cells additionally feed the
    /// `NumericStats` totals. A read-only query: no CRDT delta.
    ///
    /// A number that is not finite counts as a non-empty cell and enters no
    /// total (`TRAME-UI-178`). The text of a cell comes from a keystroke, a
    /// paste, an import or a peer. A literal such as `NaN` or `1e999` parses
    /// as such a number.
    pub(super) fn selection_stats(&mut self) -> TrameOutcome {
        use crate::formula::functions::exact_sum;
        use crate::message::{CellFloat, Cells, NumericStats};
        use ambre_core::Bounded;
        // Make sure evaluated values are current before reading them.
        let indices = self.engine_indices();
        self.engine.compute_all_dirty(&indices);

        let mut count = ambre_core::Count::<Cells>::ZERO;
        let mut numbers: Vec<Bounded<CellFloat>> = Vec::new();
        // The whole selection, deduplicated across every rectangle (`TRAME-UI-104`).
        for at in self.selected_cells() {
            let v = self
                .engine
                .evaluated_value(self.active_cell_id(at))
                .unwrap_or(CellValue::Empty);
            match v {
                CellValue::Empty => {}
                CellValue::Number(n) => {
                    count = count.incremented();
                    if let Some(n) = Bounded::<CellFloat>::new(n) {
                        numbers.push(n);
                    }
                }
                _ => count = count.incremented(),
            }
        }
        // The exact sum `SUM` takes, so the band reads the total a formula
        // over the same cells reads, overflow included (`TRAME-UI-179`).
        let sum = Bounded::<CellFloat>::new(exact_sum(numbers.iter().map(|n| n.get())));
        let numbers = NumericStats::of_all(&numbers, sum);
        TrameOutcome::display_only(vec![TrameEvent::SelectionStats { count, numbers }])
    }

    /// Clear the active shared filter: drop every `filter_hidden_{sheet}` entry
    /// and the stored `filters_{sheet}` definition, then broadcast
    /// (`trame-shared-filter`). A no-op
    /// (no delta) when nothing is filtered.
    pub(super) fn clear_filter(&mut self, sheet_id: &Id<Sheet>) -> TrameOutcome {
        if *sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        let hidden = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::FilterHidden, &self.active_sheet).as_str());
        let filters = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Filters, &self.active_sheet).as_str());
        let has_hidden = matches!(hidden.get_value(), LoroValue::Map(ref m) if !m.is_empty());
        let has_def = matches!(filters.get_value(), LoroValue::Map(ref m) if !m.is_empty());
        if !has_hidden && !has_def {
            return TrameOutcome::display_only(vec![]);
        }
        clear_loro_map(&hidden);
        clear_loro_map(&filters);
        self.doc.commit();
        self.rebuild_filter();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![TrameEvent::FilterCleared, TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    /// Recompute the active filter's hidden rows after a data change
    /// (`trame-shared-filter`). Reads
    /// the stored definition; a no-op when no filter is active. Commits the
    /// updated `filter_hidden_{sheet}` so the caller's [`flush_delta`] broadcasts
    /// it. Assumes the formula cache is already current.
    ///
    /// [`flush_delta`]: Self::flush_delta
    pub(super) fn recompute_active_filter(&mut self) {
        let Some((col_uid, kind)) = self.stored_filter() else {
            return;
        };
        if self.index.col_pos(&col_uid).is_none() {
            // The column is gone: the filter goes with it (`TRAME-FEAT-030`).
            self.drop_filter();
        } else {
            self.write_filter_hidden(col_uid, &kind);
        }
        self.doc.commit();
        self.rebuild_filter();
    }

    /// Empty the filter definition and its hidden set, uncommitted.
    pub(super) fn drop_filter(&self) {
        for root in [SheetRoot::FilterHidden, SheetRoot::Filters] {
            let map = self
                .doc
                .get_map(SheetContainer::of(root, &self.active_sheet).as_str());
            clear_loro_map(&map);
        }
    }

    /// Bring `filter_hidden_{sheet}` to the rows the filter hides on the given
    /// column, returning how many. Only the rows whose mark changes are written,
    /// so an edit that hides and shows nothing broadcasts nothing
    /// (`TRAME-FEAT-036`). Does not commit (the caller does).
    pub(super) fn write_filter_hidden(&mut self, col_uid: Uid, kind: &FilterKind) -> Len<Row> {
        let hidden = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::FilterHidden, &self.active_sheet).as_str());
        let Some(col_pos) = self.index.col_pos(&col_uid) else {
            clear_loro_map(&hidden);
            return Len::<Row>::ZERO; // column deleted since the filter was set
        };
        let wanted: HashSet<String> = Span::<Row>::from_len(Pos::<Row>::ZERO, self.index.row_len())
            .into_iter()
            .filter(|&pos| {
                let value = self
                    .engine
                    .evaluated_value(self.active_cell_id(CellPos::new(pos, col_pos)))
                    .unwrap_or(CellValue::Empty);
                !filter_pass(kind, &value)
            })
            .filter_map(|pos| self.index.row_id(pos))
            .map(|uid| uid.to_key_part())
            .collect();
        let current: HashSet<String> = match hidden.get_value() {
            LoroValue::Map(entries) => entries.keys().cloned().collect(),
            _ => HashSet::default(),
        };
        for gone in current.difference(&wanted) {
            let _ = hidden.delete(gone);
        }
        for new in wanted.difference(&current) {
            let _ = hidden.insert(new.as_str(), true);
        }
        Len::<Row>::of(wanted.len())
    }

    /// Persist the active filter definition into `filters_{sheet}` as one whole
    /// register ([`FilterRecord`]), replacing any prior one. Stored so concurrent
    /// filters race exactly once and converge whole (`TRAME-FEAT-025`), and so
    /// [`recompute_active_filter`] can re-evaluate after a data change.
    ///
    /// [`recompute_active_filter`]: Self::recompute_active_filter
    pub(super) fn persist_filter_def(&self, col_uid: Uid, kind: &FilterKind) {
        let filters = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Filters, &self.active_sheet).as_str());
        clear_loro_map(&filters);
        let record = FilterRecord {
            col: col_uid,
            kind: kind.clone(),
        };
        let bytes = postcard::to_allocvec(&record).unwrap_or_default();
        let _ = Fields::<FilterKey>::new(filters).insert(FilterKey::Active, bytes);
    }

    /// The active filter: the stored definition, when its column still exists
    /// on the sheet. A filter never outlives the column it tests
    /// (`TRAME-FEAT-030`), even before a peer's column delete is cleaned up.
    pub(super) fn load_active_filter(&self) -> Option<(Uid, FilterKind)> {
        self.stored_filter()
            .filter(|(col_uid, _)| self.index.col_pos(col_uid).is_some())
    }

    /// Read the filter definition back from `filters_{sheet}`. Returns the
    /// filtered column's uid and the reconstructed [`FilterKind`], or `None` when
    /// no (parseable) filter is stored.
    fn stored_filter(&self) -> Option<(Uid, FilterKind)> {
        let LoroValue::Map(filters) = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Filters, &self.active_sheet).as_str())
            .get_value()
        else {
            return None;
        };
        let Some(LoroValue::Binary(bytes)) =
            FieldValues::<FilterKey>::new(&filters).get(FilterKey::Active)
        else {
            return None;
        };
        let Ok(record) = postcard::from_bytes::<FilterRecord>(bytes) else {
            // A register a newer client wrote is skipped, not silently read as
            // no filter (`SCHEMA-003`).
            self.note_undecodable(bytes);
            return None;
        };
        Some((record.col, record.kind))
    }

    /// Rebuild [`Self::hidden_rows`] (grid positions) from the active sheet's
    /// `filter_hidden_{sheet}` map. Requires [`Self::index`] to be current (keys
    /// are stable `row_uid`s mapped back to positions); a uid no longer present
    /// (its row was deleted) is dropped.
    pub(super) fn rebuild_filter(&mut self) {
        self.hidden_rows.clear();
        let LoroValue::Map(entries) = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::FilterHidden, &self.active_sheet).as_str())
            .get_value()
        else {
            return;
        };
        for (key, value) in entries.iter() {
            let Some(uid) = Uid::parse(key) else {
                // A key no build writes is said (`TRAME-248`).
                self.note_unreadable_key();
                continue;
            };
            if matches!(value, LoroValue::Bool(true))
                && let Some(pos) = self.index.row_pos(&uid)
            {
                self.hidden_rows.insert(pos);
            }
        }
    }
}
