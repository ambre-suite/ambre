// ambre-trame/src/logic/interop.rs
// CSV import / export (`trame-import-export`).
//
// Part of the `TrameState` impl, split out by concern. Submodule of `logic`, so
// it reaches the private fields/helpers of the parent module through the
// `use super::*` glob. All conversion runs here, in the Logic thread, so no
// plaintext file ever transits to the Blind Relay (Zero-Knowledge invariant).

use super::*;

impl TrameState {
    /// Import CSV bytes into the workbook (`trame-import-export`). Parses the
    /// records, resolves
    /// (or creates) the target sheet, writes each non-empty field as a literal
    /// cell, and recomputes -- the whole import is a single undo group.
    pub(super) fn import_csv(
        &mut self,
        data: Vec<u8>,
        delimiter: Option<u8>,
        has_header: bool,
        target_sheet_id: Option<String>,
        start_row: u32,
        start_col: u32,
    ) -> TrameOutcome {
        let delim = delimiter.unwrap_or_else(|| detect_csv_delimiter(&data));
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delim)
            // We skip the header ourselves so every record is read uniformly.
            .has_headers(false)
            // CSV rows of varying field counts are common in real exports.
            .flexible(true)
            .from_reader(data.as_slice());
        let mut records: Vec<Vec<String>> = reader
            .records()
            .filter_map(Result::ok)
            .map(|rec| rec.iter().map(str::to_owned).collect())
            .collect();
        if has_header && !records.is_empty() {
            records.remove(0);
        }
        if records.is_empty() {
            return TrameOutcome::display_only(vec![]);
        }

        // Open the undo group up front so a freshly created sheet and every cell
        // write collapse into a single undo item (sealed by the next command's
        // fallback `group_end`, like `SetCellText`).
        let _ = self.undo_manager.group_start();

        // Resolve the target sheet: an existing id is reused, anything else
        // (including `None`) creates a fresh "CSV Import" sheet.
        let sheet_id = match target_sheet_id {
            Some(id) if self.all_sheet_ids().iter().any(|s| s == &id) => id,
            _ => {
                let id = format!("{:016x}_{:08x}", self.doc.peer_id(), self.sheet_counter);
                self.sheet_counter += 1;
                let _ = self.doc.get_map(SHEETS_MAP).insert(&id, "CSV Import");
                let _ = self.doc.get_movable_list(SHEET_ORDER).push(id.as_str());
                self.doc.commit();
                id
            }
        };
        // Make the target the active sheet so the write path targets its
        // containers, then rebuild the active-sheet caches against it.
        self.active_sheet = sheet_id.clone();
        self.reseed_active_sheet();

        let rows = records.len() as u32;
        let cols = records.iter().map(Vec::len).max().unwrap_or(0) as u32;

        // Pre-materialize every written cell first (same reasoning as
        // `SetCellsBatch`): a `materialize` commit carries the excluded origin,
        // so interleaving it between value commits would split the undo item.
        for (r, record) in records.iter().enumerate() {
            for (c, value) in record.iter().enumerate() {
                if !value.is_empty() {
                    let _ = self.materialize(start_row + r as u32, start_col + c as u32);
                }
            }
        }
        for (r, record) in records.into_iter().enumerate() {
            for (c, value) in record.into_iter().enumerate() {
                if !value.is_empty() {
                    self.write_cell(start_row + r as u32, start_col + c as u32, &value);
                }
            }
        }

        let mut outcome = self.finish_cell_writes();
        outcome.display.push(self.sheet_list_event());
        outcome.display.push(self.style_state_event());
        outcome.display.push(TrameEvent::ImportComplete {
            sheet_id,
            rows,
            cols,
        });
        outcome
    }

    /// Export a sheet (or sub-range) to CSV bytes (`trame-import-export`).
    /// Read-only: emits no
    /// CRDT delta. Cells are written by their displayed value (formula results
    /// included). Returns an [`TrameEvent::ExportComplete`] carrying the bytes.
    pub(super) fn export_csv(
        &self,
        sheet_id: &str,
        delimiter: Option<u8>,
        range: Option<ambre_bus::GridRange>,
    ) -> TrameOutcome {
        // Unknown sheet: nothing to export.
        let Some(&idx) = self.sheet_id_to_u16.get(sheet_id) else {
            return TrameOutcome::display_only(vec![]);
        };
        let index = if idx == 0 {
            self.index.clone()
        } else {
            self.build_sheet_index(sheet_id)
        };
        let (row_start, row_end, col_start, col_end) = match range {
            Some(r) => (r.row, r.row_end, r.col, r.col_end),
            None => match self.used_extent(sheet_id, &index) {
                // No content at all: nothing to export.
                None => return TrameOutcome::display_only(vec![]),
                Some((max_row, max_col)) => (0, max_row, 0, max_col),
            },
        };

        let delim = delimiter.unwrap_or(b',');
        let mut writer = csv::WriterBuilder::new()
            .delimiter(delim)
            .from_writer(Vec::new());
        for row in row_start..=row_end {
            let record: Vec<String> = (col_start..=col_end)
                .map(|col| self.export_cell_text(sheet_id, idx, &index, row, col))
                .collect();
            let _ = writer.write_record(&record);
        }
        let data = writer.into_inner().unwrap_or_default();

        let name = match self
            .doc
            .get_map(SHEETS_MAP)
            .get(sheet_id)
            .map(|v| v.get_deep_value())
        {
            Some(LoroValue::String(s)) => s.to_string(),
            _ => sheet_id.to_owned(),
        };
        TrameOutcome::display_only(vec![TrameEvent::ExportComplete {
            filename: format!("{name}.csv"),
            data,
        }])
    }

    /// Inclusive `(max_row, max_col)` grid position holding actual content on
    /// `sheet_id`, or `None` when the sheet is empty. Scans the `cells_{sheet}`
    /// map (the source of truth) rather than the row/column lists: those lists
    /// stay materialized at their high-water mark after a scroll or a since-cleared
    /// edit, so using their length would export a rectangle padded with empty
    /// trailing rows/columns.
    fn used_extent(&self, sheet_id: &str, index: &SheetIndex) -> Option<(u32, u32)> {
        let LoroValue::Map(cells) = self.doc.get_map(cells_map_name(sheet_id)).get_value() else {
            return None;
        };
        let mut max_row = None;
        let mut max_col = None;
        for (key, value) in cells.iter() {
            // A cleared cell is deleted from the map, but guard against a stray
            // empty value so it never widens the exported rectangle.
            if matches!(value, LoroValue::String(s) if s.is_empty()) {
                continue;
            }
            let Some((rl, cl)) = parse_cell_key(key) else {
                continue;
            };
            if let (Some(r), Some(c)) = (index.row_pos(&rl), index.col_pos(&cl)) {
                max_row = Some(max_row.map_or(r, |m: usize| m.max(r)));
                max_col = Some(max_col.map_or(c, |m: usize| m.max(c)));
            }
        }
        Some((max_row? as u32, max_col? as u32))
    }

    /// Displayed value of a cell on an arbitrary sheet, for CSV export: the
    /// formula result for a formula cell, otherwise the raw stored literal.
    fn export_cell_text(
        &self,
        sheet_id: &str,
        idx: u16,
        index: &SheetIndex,
        row: u32,
        col: u32,
    ) -> String {
        let Some((rl, cl)) = index.resolve(row as usize, col as usize) else {
            return String::new();
        };
        let cell = CellId::bound(idx, rl, cl);
        if self.engine.is_formula_cell(cell)
            && let Some(text) = self.engine.evaluated_text(cell)
        {
            return text;
        }
        match self
            .doc
            .get_map(cells_map_name(sheet_id))
            .get(&cell_key(rl, cl))
            .map(|v| v.get_deep_value())
        {
            Some(LoroValue::String(s)) => s.to_string(),
            _ => String::new(),
        }
    }
}

/// Auto-detect the CSV field separator: comma vs semicolon, by frequency in the
/// first kilobyte (`trame-import-export`). Defaults to comma on a tie.
fn detect_csv_delimiter(data: &[u8]) -> u8 {
    let sample = &data[..data.len().min(1024)];
    let commas = sample.iter().filter(|&&b| b == b',').count();
    let semicolons = sample.iter().filter(|&&b| b == b';').count();
    if semicolons > commas { b';' } else { b',' }
}
