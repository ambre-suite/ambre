// ambre-trame/src/logic/interop.rs
// CSV import / export (`trame-import-export`).
//
// Part of the `TrameState` impl, split out by concern. Submodule of `logic`, so
// it reaches the private fields/helpers of the parent module through the
// `use super::*` glob. The CSV reader and writer run in the converter module
// (`ambre-spec-architecture.md section 2`), which the Logic thread hands in.
// This file turns bytes into records through it, then records into cells.
// No plaintext file ever transits to the Blind Relay (Zero-Knowledge
// invariant).

use super::*;
use crate::formula::ACTIVE_SHEET;
use crate::formula::text::CellText;
use crate::message::{TrameConstruct, TrameReport};
use crate::message::{TrameExport, TrameImport};
use crate::{CsvDelimiter, TOTAL_COLS, TOTAL_ROWS};
use ambre_bus::interop::{
    Direction, ExportedFile, FileFormat, InteropAnswer, InteropError, InteropRequest, Occurrences,
    Outcome,
};
use ambre_core::Count;
use ambre_core::id::{Id, Sheet};
use ambre_core::index::{Len, Pos};

/// The answer to an import that writes nothing: the document is as it was,
/// and the user reads why (`INTEROP-011`, `TRAME-IO-028`).
fn import_failed(reason: String) -> TrameOutcome {
    TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::failed(
        Direction::Import,
        InteropError::Stopped(reason),
    ))])
}

/// The answer to a conversion the module refused or could not run: the
/// reason, shown, and the document as it was (`INTEROP-011`).
fn conversion_failed(direction: Direction, error: &ambre_convert::ConvertError) -> TrameOutcome {
    TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::failed(
        direction,
        InteropError::Stopped(error.to_string()),
    ))])
}

impl TrameState {
    /// Serve a command that needs the converter module: the CSV import and
    /// the CSV export (`trame-import-export`). The Logic thread loads the
    /// module and hands it in. Any other command is served as
    /// [`handle_command`](Self::handle_command) serves it.
    pub fn handle_conversion(
        &mut self,
        cmd: TrameCommand,
        converter: &ambre_convert::Converter,
    ) -> TrameOutcome {
        if !cmd.needs_converter() {
            return self.handle_command(cmd);
        }
        // The frame `handle_command` sets around every command: no cell
        // changed yet, and one step of the history per command (`TRAME-017`).
        self.engine.changed.clear();
        self.seal_history();
        let outcome = match cmd {
            TrameCommand::Interop(InteropRequest::Import(TrameImport::Csv(import))) => self
                .import_csv(
                    converter,
                    import.data,
                    import.delimiter,
                    import.header,
                    import.target_sheet_id,
                    CellPos::new(import.start_row, import.start_col),
                ),
            TrameCommand::Interop(InteropRequest::Export(TrameExport::Csv {
                sheet_id,
                delimiter,
                range,
            })) => {
                let sheet = sheet_id.unwrap_or_else(|| self.active_sheet.clone());
                self.export_csv(converter, &sheet, delimiter, range)
            }
            other => self.dispatch_command(other),
        };
        self.report_undo(outcome)
    }

    /// Import CSV bytes into the workbook (`trame-import-export`). The
    /// converter reads the records, then this resolves (or creates) the
    /// target sheet, writes each non-empty field as a literal cell, and
    /// recomputes. The whole import is a single undo group.
    fn import_csv(
        &mut self,
        converter: &ambre_convert::Converter,
        data: ambre_core::Payload<ambre_core::FileBytes>,
        delimiter: Option<CsvDelimiter>,
        header: HeaderRow,
        target_sheet_id: Option<Id<Sheet>>,
        start: CellPos,
    ) -> TrameOutcome {
        let delimiter = delimiter.unwrap_or_else(|| detect_csv_delimiter(&data));
        let (mut records, unreadable) =
            match converter.convert(&ambre_convert::Conversion::ReadCsv { data, delimiter }) {
                Ok(ambre_convert::Converted::Records {
                    records,
                    unreadable,
                }) => (records, unreadable.get()),
                Ok(ambre_convert::Converted::File(_)) => {
                    return conversion_failed(
                        Direction::Import,
                        &ambre_convert::ConvertError::Malformed,
                    );
                }
                Err(error) => return conversion_failed(Direction::Import, &error),
            };
        // An unreadable record is counted and reported, never dropped in
        // silence (`TRAME-IO-039`).
        let mut report = TrameReport::new(Direction::Import, FileFormat::Csv);
        report.record_count(
            TrameConstruct::UnreadableRecord,
            Outcome::Dropped,
            Count::<Occurrences>::new(unreadable),
        );
        if header == HeaderRow::Present && !records.is_empty() {
            records.remove(0);
        }
        if records.is_empty() {
            let reason = if unreadable > 0 {
                format!("none of the {unreadable} record(s) of the file could be read")
            } else {
                "the file holds no record to import".to_owned()
            };
            return import_failed(reason);
        }
        // The grid ends at `LAST_ROW` and `LAST_COL`, and a field past either
        // would land outside it: cut the records there, and say so
        // (`TRAME-IO-050`).
        let room_rows = TOTAL_ROWS.as_end().len_from(start.row).to_usize();
        let room_cols = TOTAL_COLS.as_end().len_from(start.col).to_usize();
        let cut_records = records.len().saturating_sub(room_rows);
        records.truncate(room_rows);
        let cut_fields: usize = records
            .iter_mut()
            .map(|record| {
                let cut = record.len().saturating_sub(room_cols);
                record.truncate(room_cols);
                cut
            })
            .sum();
        report.record_count(
            TrameConstruct::RecordPastLastRow,
            Outcome::Dropped,
            Count::<Occurrences>::of(cut_records),
        );
        report.record_count(
            TrameConstruct::FieldPastLastColumn,
            Outcome::Dropped,
            Count::<Occurrences>::of(cut_fields),
        );
        // A start past the edge cuts every field: nothing is left to write,
        // so the document stays untouched, as for an empty file
        // (`TRAME-IO-028`).
        if records.iter().all(Vec::is_empty) && (cut_records > 0 || cut_fields > 0) {
            return import_failed(format!(
                "the grid ends at row {} and column {}, and no field of the file fits",
                TOTAL_ROWS.get(),
                TOTAL_COLS.get()
            ));
        }

        // Resolve the target sheet: an existing id is reused, anything else
        // (including `None`) creates a fresh "CSV Import" sheet. The id is
        // minted before the undo group opens, so a spent counter refuses the
        // import with the document and the undo stack untouched (`TRAME-243`).
        let (sheet_id, created) = match target_sheet_id {
            Some(id) if self.all_sheet_ids().iter().any(|s| s == &id) => (id, false),
            _ => {
                let Some(id) = self.mint_sheet_id() else {
                    return TrameOutcome::display_only(vec![super::sheets::sheet_ids_exhausted()]);
                };
                (id, true)
            }
        };

        if created {
            self.hold_default_sheet();
        }
        // Make the target the active sheet so the write path targets its
        // containers, then rebuild the active-sheet caches against it.
        self.active_sheet = sheet_id.clone();
        self.reseed_active_sheet();

        let rows = Count::<Occurrences>::of(records.len());
        let cols = Count::<Occurrences>::of(records.iter().map(Vec::len).max().unwrap_or(0));
        let land = |r: usize, c: usize| {
            CellPos::new(start.row + Len::<Row>::of(r), start.col + Len::<Col>::of(c))
        };

        // Pre-materialize every written cell first (same reasoning as
        // `SetCellsBatch`). A `materialize` commit carries the excluded origin,
        // so interleaving it between value commits would split the undo item.
        for (r, record) in records.iter().enumerate() {
            for (c, value) in record.iter().enumerate() {
                if !value.is_empty() {
                    let _ = self.materialize(land(r, c));
                }
            }
        }
        // The new sheet is listed only now, after the excluded commits, so the
        // commit that lists it is a local one inside the undo group. Listed
        // before them, it formed an undo item of its own (`TRAME-IO-025`).
        if created {
            let name = self.free_sheet_name(&SheetName::new("CSV Import").unwrap_or_default());
            let _ = self
                .doc
                .get_map(Root::Sheets.as_str())
                .insert(sheet_id.as_str(), name.as_str());
            let _ = self
                .doc
                .get_movable_list(Root::SheetOrder.as_str())
                .push(sheet_id.as_str());
            self.reseed_active_sheet();
        }
        for (r, record) in records.into_iter().enumerate() {
            for (c, value) in record.into_iter().enumerate() {
                if !value.is_empty() {
                    self.write_cell(land(r, c), &CellText::<SourceForm>::new(value));
                }
            }
        }
        // A file whose fields are all empty writes no cell: the listing still
        // lands, in the group.
        self.doc.commit();

        let mut outcome = self.finish_cell_writes();
        outcome.display.push(self.sheet_list_event());
        outcome.display.push(self.style_state_event());
        report.record_count(TrameConstruct::Row, Outcome::Carried, rows);
        report.record_count(TrameConstruct::Column, Outcome::Carried, cols);
        outcome
            .display
            .push(TrameEvent::Interop(InteropAnswer::ImportCompleted {
                report,
            }));
        outcome
    }

    /// Export a sheet (or sub-range) to CSV bytes (`trame-import-export`).
    /// Read-only: emits no
    /// CRDT delta. Cells are written by their displayed value (formula results
    /// included); the converter writes the bytes. Returns an
    /// [`InteropAnswer::ExportReady`] carrying them.
    fn export_csv(
        &self,
        converter: &ambre_convert::Converter,
        sheet_id: &Id<Sheet>,
        delimiter: Option<CsvDelimiter>,
        range: Option<CellRect>,
    ) -> TrameOutcome {
        // An export that writes no file says why (`TRAME-IO-041`).
        let nothing = |message: &str| {
            TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::failed(
                Direction::Export,
                InteropError::Stopped(message.to_owned()),
            ))])
        };
        let Some(&idx) = self.engine_sheet_by_id.get(sheet_id) else {
            return nothing("the sheet to export no longer exists");
        };
        let index = if idx == ACTIVE_SHEET {
            self.index.clone()
        } else {
            self.build_sheet_index(sheet_id)
        };
        let rect = match range {
            Some(rect) => rect,
            None => match self.used_extent(sheet_id, &index) {
                None => return nothing("the sheet is empty: there is nothing to export"),
                Some(last) => CellRect::spanning(CellPos::ORIGIN, last),
            },
        };

        let records: Vec<Vec<String>> = rect
            .rows()
            .into_iter()
            .map(|row| {
                rect.cols()
                    .into_iter()
                    .map(|col| self.export_cell_text(sheet_id, idx, &index, CellPos::new(row, col)))
                    .collect()
            })
            .collect();
        let data = match converter.convert(&ambre_convert::Conversion::WriteCsv {
            records,
            delimiter: delimiter.unwrap_or(CsvDelimiter::COMMA),
        }) {
            Ok(ambre_convert::Converted::File(data)) => data,
            Ok(ambre_convert::Converted::Records { .. }) => {
                return conversion_failed(
                    Direction::Export,
                    &ambre_convert::ConvertError::Malformed,
                );
            }
            Err(error) => return conversion_failed(Direction::Export, &error),
        };

        let name = match self
            .doc
            .get_map(Root::Sheets.as_str())
            .get(sheet_id.as_str())
            .map(|v| v.get_deep_value())
        {
            Some(LoroValue::String(s)) => s.to_string(),
            _ => sheet_id.to_string(),
        };
        // A sheet name is user text, so the file name is cleaned of what a
        // host refuses and bounded (`DOCFILE-019`, `DOCFILE-020`).
        // Every displayed value leaves as text, so CSV loses nothing the grid
        // shows: the report is clean.
        TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::ExportReady {
            file: ExportedFile {
                filename: ambre_core::ExportFilename::from_title(
                    &name,
                    "sheet",
                    ambre_core::ExportExtension::Csv,
                ),
                media_type: ambre_core::MediaType::from("text/csv; charset=utf-8"),
                data,
            },
            report: TrameReport::new(Direction::Export, FileFormat::Csv),
        })])
    }

    /// The bottom-right corner of the content on
    /// `sheet_id`, or `None` when the sheet is empty. Scans the `cells_{sheet}` map
    /// (the source of truth) rather than the row/column lists. Those lists stay
    /// materialized at their high-water mark after a scroll or a since-cleared
    /// edit. Using their length would therefore export a rectangle padded with
    /// empty trailing rows/columns.
    fn used_extent(&self, sheet_id: &Id<Sheet>, index: &SheetIndex) -> Option<CellPos> {
        let LoroValue::Map(cells) = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Cells, sheet_id).as_str())
            .get_value()
        else {
            return None;
        };
        let mut max_row = None;
        let mut max_col = None;
        let (shown, _) = super::cells::shown_cells(index, cells.iter());
        for (at, _, value) in shown {
            // A cleared cell is deleted from the map, but guard against a stray
            // empty value so it never widens the exported rectangle.
            if matches!(value, LoroValue::String(s) if s.is_empty()) {
                continue;
            }
            max_row = Some(max_row.map_or(at.row, |m: Pos<Row>| m.max(at.row)));
            max_col = Some(max_col.map_or(at.col, |m: Pos<Col>| m.max(at.col)));
        }
        Some(CellPos::new(max_row?, max_col?))
    }

    /// Displayed value of a cell on an arbitrary sheet, for CSV export. That is
    /// the formula result for a formula cell, otherwise the raw stored literal.
    /// The cell's number format then applies (`TRAME-108`).
    fn export_cell_text(
        &self,
        sheet_id: &Id<Sheet>,
        idx: Pos<EngineSheet>,
        index: &SheetIndex,
        at: CellPos,
    ) -> String {
        let Some((rl, cl)) = index.resolve(at) else {
            return String::new();
        };
        let cell = CellId::bound(idx, rl, cl);
        let text = match self.engine.evaluated_text(cell) {
            Some(text) if self.engine.is_formula_cell(cell) => text,
            _ => match super::cells::cell_value(
                &self
                    .doc
                    .get_map(SheetContainer::of(SheetRoot::Cells, sheet_id).as_str()),
                index,
                at,
            ) {
                Some(LoroValue::String(s)) => s.to_string(),
                _ => String::new(),
            },
        };
        // An export writes the workbook's forms, whoever runs it
        // (`LOCALE-020`).
        let style = self.stored_style(sheet_id, index, at).unwrap_or_default();
        self.format_display(at, text, &style, &self.export_forms())
    }
}

/// Auto-detect the CSV field separator among the comma, the semicolon and the
/// tab, by frequency in the first kilobyte (`trame-import-export`). A separator
/// inside a quoted field does not count. A tie goes to the earlier of the
/// three. Any other separator, the caller names (`TRAME-IO-035`).
fn detect_csv_delimiter(data: &ambre_core::Payload<ambre_core::FileBytes>) -> CsvDelimiter {
    let data = data.as_bytes();
    let sample = data.get(..1024).unwrap_or(data);
    let mut counts = [
        (CsvDelimiter::COMMA, 0usize),
        (CsvDelimiter::SEMICOLON, 0),
        (CsvDelimiter::TAB, 0),
    ];
    let mut quoted = false;
    for &byte in sample {
        if byte == b'"' {
            // An escaped quote toggles twice, so it leaves the field quoted.
            quoted = !quoted;
        } else if !quoted && let Some(slot) = counts.iter_mut().find(|(sep, _)| sep.byte() == byte)
        {
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
