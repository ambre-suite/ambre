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
    /// records, resolves (or creates) the target sheet, writes each non-empty
    /// field as a literal cell, and recomputes. The whole import is a single
    /// undo group.
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
        // An unreadable record is counted and reported, never dropped in
        // silence (`TRAME-IO-039`).
        let mut unreadable = 0u32;
        let mut records: Vec<Vec<String>> = reader
            .records()
            .filter_map(|rec| rec.inspect_err(|_| unreadable += 1).ok())
            .map(|rec| rec.iter().map(str::to_owned).collect())
            .collect();
        let unreadable_notice = (unreadable > 0).then(|| TrameEvent::Error {
            message: format!(
                "{unreadable} record(s) of the file could not be read and were skipped"
            ),
        });
        if has_header && !records.is_empty() {
            records.remove(0);
        }
        if records.is_empty() {
            return TrameOutcome::display_only(vec![unreadable_notice.unwrap_or(
                TrameEvent::Error {
                    message: "the file holds no record to import".to_owned(),
                },
            )]);
        }

        // Open the undo group up front. A freshly created sheet and every cell
        // write then collapse into a single undo item, sealed by the next
        // command's fallback `group_end` (like `SetCellText`).
        let _ = self.undo_manager.group_start();

        // Resolve the target sheet: an existing id is reused, anything else
        // (including `None`) creates a fresh "CSV Import" sheet.
        let (sheet_id, created) = match target_sheet_id {
            Some(id) if self.all_sheet_ids().iter().any(|s| s == &id) => (id, false),
            _ => {
                self.hold_default_sheet();
                let id = format!("{:016x}_{:08x}", self.doc.peer_id(), self.sheet_counter);
                self.sheet_counter += 1;
                (id, true)
            }
        };
        // Make the target the active sheet so the write path targets its
        // containers, then rebuild the active-sheet caches against it.
        self.active_sheet = sheet_id.clone();
        self.reseed_active_sheet();

        let rows = records.len() as u32;
        let cols = records.iter().map(Vec::len).max().unwrap_or(0) as u32;

        // Pre-materialize every written cell first (same reasoning as
        // `SetCellsBatch`). A `materialize` commit carries the excluded origin,
        // so interleaving it between value commits would split the undo item.
        for (r, record) in records.iter().enumerate() {
            for (c, value) in record.iter().enumerate() {
                if !value.is_empty() {
                    let _ = self.materialize(start_row + r as u32, start_col + c as u32);
                }
            }
        }
        // The new sheet is listed only now, after the excluded commits, so the
        // commit that lists it is a local one inside the undo group. Listed
        // before them, it formed an undo item of its own (`TRAME-IO-025`).
        if created {
            let name = self.free_sheet_name("CSV Import");
            let _ = self
                .doc
                .get_map(SHEETS_MAP)
                .insert(&sheet_id, name.as_str());
            let _ = self
                .doc
                .get_movable_list(SHEET_ORDER)
                .push(sheet_id.as_str());
            self.reseed_active_sheet();
        }
        for (r, record) in records.into_iter().enumerate() {
            for (c, value) in record.into_iter().enumerate() {
                if !value.is_empty() {
                    self.write_cell(start_row + r as u32, start_col + c as u32, &value);
                }
            }
        }
        // A file whose fields are all empty writes no cell: the listing still
        // lands, in the group.
        self.doc.commit();

        let mut outcome = self.finish_cell_writes();
        outcome.display.push(self.sheet_list_event());
        outcome.display.push(self.style_state_event());
        outcome.display.push(TrameEvent::ImportComplete {
            sheet_id: SheetId(sheet_id),
            rows,
            cols,
        });
        outcome.display.extend(unreadable_notice);
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
        // An export that writes no file says why (`TRAME-IO-041`).
        let nothing = |message: &str| {
            TrameOutcome::display_only(vec![TrameEvent::Error {
                message: message.to_owned(),
            }])
        };
        let Some(&idx) = self.sheet_id_to_u16.get(sheet_id) else {
            return nothing("the sheet to export no longer exists");
        };
        let index = if idx == 0 {
            self.index.clone()
        } else {
            self.build_sheet_index(sheet_id)
        };
        let (row_start, row_end, col_start, col_end) = match range {
            Some(r) => (r.row, r.row_end, r.col, r.col_end),
            None => match self.used_extent(sheet_id, &index) {
                None => return nothing("the sheet is empty: there is nothing to export"),
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
    /// `sheet_id`, or `None` when the sheet is empty. Scans the `cells_{sheet}` map
    /// (the source of truth) rather than the row/column lists. Those lists stay
    /// materialized at their high-water mark after a scroll or a since-cleared
    /// edit. Using their length would therefore export a rectangle padded with
    /// empty trailing rows/columns.
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

    /// Displayed value of a cell on an arbitrary sheet, for CSV export. That is
    /// the formula result for a formula cell, otherwise the raw stored literal.
    /// The cell's number format then applies (`TRAME-108`).
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
        let text = match self.engine.evaluated_text(cell) {
            Some(text) if self.engine.is_formula_cell(cell) => text,
            _ => match self
                .doc
                .get_map(cells_map_name(sheet_id))
                .get(&cell_key(rl, cl))
                .map(|v| v.get_deep_value())
            {
                Some(LoroValue::String(s)) => s.to_string(),
                _ => String::new(),
            },
        };
        match self.stored_style(sheet_id, rl, cl) {
            Some(style) => self.format_display(row, col, text, &style),
            None => text,
        }
    }
}

/// Auto-detect the CSV field separator among the comma, the semicolon and the
/// tab, by frequency in the first kilobyte (`trame-import-export`). A separator
/// inside a quoted field does not count. A tie goes to the earlier of the
/// three. Any other separator, the caller names (`TRAME-IO-035`).
fn detect_csv_delimiter(data: &[u8]) -> u8 {
    let sample = data.get(..1024).unwrap_or(data);
    let mut counts = [(b',', 0usize), (b';', 0), (b'\t', 0)];
    let mut quoted = false;
    for &byte in sample {
        if byte == b'"' {
            // An escaped quote toggles twice, so it leaves the field quoted.
            quoted = !quoted;
        } else if !quoted && let Some(slot) = counts.iter_mut().find(|(sep, _)| *sep == byte) {
            slot.1 += 1;
        }
    }
    let mut best = counts[0];
    for candidate in &counts[1..] {
        if candidate.1 > best.1 {
            best = *candidate;
        }
    }
    best.0
}
