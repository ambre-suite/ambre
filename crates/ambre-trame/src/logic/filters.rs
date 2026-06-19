// ambre-trame/src/logic/filters.rs.rs
// Shared filter apply / clear / recompute
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;

impl TrameState {
    // -------------------------------------------------------------------------
    // Shared filter (apply / clear / recompute) -- features spec §1.A
    // -------------------------------------------------------------------------

    /// Apply a shared filter to grid column `col` of the active sheet (§1.A).
    ///
    /// Evaluates each materialized row's cell in that column through the formula
    /// cache, writes the rows failing the filter into `filter_hidden_{sheet}`
    /// (keyed by `row_uid`), persists the filter definition for convergence and
    /// later recompute, and broadcasts the delta. A no-op for a non-active sheet
    /// or a virtual (never-materialized) column.
    pub(super) fn apply_filter(&mut self, sheet_id: &str, col: u32, kind: FilterKind) -> TrameOutcome {
        if sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        let Some(col_uid) = self.index.col_id(col as usize) else {
            // Virtual column: no data to filter.
            return TrameOutcome::display_only(vec![]);
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
                TrameEvent::FilterApplied { hidden_row_count: count, col },
                TrameEvent::DataReset,
            ],
            update,
            ephemeral: None,
        }
    }

    /// Collect the distinct visible values of the active cell's column for the
    /// filter popover's "by values" list (`-ui.md` sec.5.D). Evaluates every
    /// non-hidden materialized row through the formula cache, dedups by display
    /// text, sorts, and replies with [`TrameEvent::ColumnValues`]. A read-only
    /// query (no delta); replies with an empty list for a virtual column.
    pub(super) fn active_column_values(&mut self) -> TrameOutcome {
        let col = self.active.1;
        let Some(col_uid) = self.index.col_id(col as usize) else {
            return TrameOutcome::display_only(vec![TrameEvent::ColumnValues {
                col,
                values: vec![],
            }]);
        };
        // The popover lists current results; make sure the cache is up to date.
        let indices = self.engine_indices();
        self.engine.compute_all_dirty(&indices);
        let col_pos = self.index.col_pos(&col_uid).unwrap_or(col as usize);
        let mut values: Vec<String> = (0..self.index.row_len())
            // Skip rows already hidden by an active filter (distinct *visible*
            // values, matching Google Sheets).
            .filter(|&pos| !self.hidden_rows.contains(&(pos as u32)))
            .filter_map(|pos| {
                let v = self
                    .engine
                    .evaluated_value(self.active_cell_id(pos as u32, col_pos as u32))
                    .unwrap_or(CellValue::Empty);
                match v {
                    CellValue::Empty => None,
                    other => Some(other.to_display()),
                }
            })
            .collect();
        values.sort();
        values.dedup();
        TrameOutcome::display_only(vec![TrameEvent::ColumnValues { col, values }])
    }

    /// Clear the active shared filter: drop every `filter_hidden_{sheet}` entry
    /// and the stored `filters_{sheet}` definition, then broadcast (§1.A). A no-op
    /// (no delta) when nothing is filtered.
    pub(super) fn clear_filter(&mut self, sheet_id: &str) -> TrameOutcome {
        if sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        let hidden = self.doc.get_map(filter_hidden_map_name(&self.active_sheet));
        let filters = self.doc.get_map(filters_map_name(&self.active_sheet));
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

    /// Recompute the active filter's hidden rows after a data change (§1.A). Reads
    /// the stored definition; a no-op when no filter is active. Commits the
    /// updated `filter_hidden_{sheet}` so the caller's [`flush_delta`] broadcasts
    /// it. Assumes the formula cache is already current.
    ///
    /// [`flush_delta`]: Self::flush_delta
    pub(super) fn recompute_active_filter(&mut self) {
        let Some((col_uid, kind)) = self.load_active_filter() else {
            return;
        };
        self.write_filter_hidden(col_uid, &kind);
        self.doc.commit();
        self.rebuild_filter();
    }

    /// Recompute and rewrite `filter_hidden_{sheet}` from scratch for the given
    /// column/kind, returning the number of hidden rows. Clears the prior hidden
    /// set first. Does not commit (the caller does).
    pub(super) fn write_filter_hidden(&mut self, col_uid: Uid, kind: &FilterKind) -> u32 {
        let hidden = self.doc.get_map(filter_hidden_map_name(&self.active_sheet));
        clear_loro_map(&hidden);
        let Some(col_pos) = self.index.col_pos(&col_uid) else {
            return 0; // column deleted since the filter was set
        };
        let mut count = 0;
        for pos in 0..self.index.row_len() {
            let value = self
                .engine
                .evaluated_value(self.active_cell_id(pos as u32, col_pos as u32))
                .unwrap_or(CellValue::Empty);
            if !filter_pass(kind, &value)
                && let Some(uid) = self.index.row_id(pos)
            {
                let _ = hidden.insert(uid.to_key_part().as_str(), true);
                count += 1;
            }
        }
        count
    }

    /// Persist the active filter definition into `filters_{sheet}` (flat keys plus
    /// a child `values` list), replacing any prior one. Stored so concurrent
    /// filters converge via LWW and so [`recompute_active_filter`] can re-evaluate
    /// after a data change.
    ///
    /// [`recompute_active_filter`]: Self::recompute_active_filter
    pub(super) fn persist_filter_def(&self, col_uid: Uid, kind: &FilterKind) {
        let filters = self.doc.get_map(filters_map_name(&self.active_sheet));
        clear_loro_map(&filters);
        let _ = filters.insert("col", col_uid.to_key_part().as_str());
        match kind {
            FilterKind::Values(values) => {
                let _ = filters.insert("kind", "values");
                if let Ok(list) = filters.get_or_create_container("values", LoroList::new()) {
                    for v in values {
                        let _ = list.push(v.as_str());
                    }
                }
            }
            FilterKind::Condition { op, value } => {
                let _ = filters.insert("kind", "condition");
                let _ = filters.insert("op", op_to_str(*op));
                let _ = filters.insert("value", value.as_str());
            }
        }
    }

    /// Read the active filter definition back from `filters_{sheet}`. Returns the
    /// filtered column's uid and the reconstructed [`FilterKind`], or `None` when
    /// no (parseable) filter is stored.
    pub(super) fn load_active_filter(&self) -> Option<(Uid, FilterKind)> {
        let LoroValue::Map(filters) =
            self.doc.get_map(filters_map_name(&self.active_sheet)).get_value()
        else {
            return None;
        };
        let LoroValue::String(col) = filters.get("col")? else {
            return None;
        };
        let col_uid = Uid::parse(col.as_str())?;
        let LoroValue::String(kind) = filters.get("kind")? else {
            return None;
        };
        let kind = match kind.as_str() {
            "values" => {
                let values = match filters.get("values") {
                    Some(LoroValue::List(items)) => items
                        .iter()
                        .filter_map(|v| match v {
                            LoroValue::String(s) => Some(s.to_string()),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                FilterKind::Values(values)
            }
            "condition" => {
                let op = match filters.get("op") {
                    Some(LoroValue::String(s)) => op_from_str(s.as_str())?,
                    _ => return None,
                };
                let value = match filters.get("value") {
                    Some(LoroValue::String(s)) => s.to_string(),
                    _ => String::new(),
                };
                FilterKind::Condition { op, value }
            }
            _ => return None,
        };
        Some((col_uid, kind))
    }

    /// Rebuild [`Self::hidden_rows`] (grid positions) from the active sheet's
    /// `filter_hidden_{sheet}` map. Requires [`Self::index`] to be current (keys
    /// are stable `row_uid`s mapped back to positions); a uid no longer present
    /// (its row was deleted) is dropped.
    pub(super) fn rebuild_filter(&mut self) {
        self.hidden_rows.clear();
        let LoroValue::Map(entries) =
            self.doc.get_map(filter_hidden_map_name(&self.active_sheet)).get_value()
        else {
            return;
        };
        for (key, value) in entries.iter() {
            if matches!(value, LoroValue::Bool(true))
                && let Some(uid) = Uid::parse(key)
                && let Some(pos) = self.index.row_pos(&uid)
            {
                self.hidden_rows.insert(pos as u32);
            }
        }
    }

}
