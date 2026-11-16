// ambre-trame/src/logic/interop.rs
// CSV import / export (`trame-import-export`).
//
// Part of the `TrameState` impl, split out by concern. Submodule of `logic`, so
// it reaches the private fields/helpers of the parent module through the
// `use super::*` glob. The CSV reader and writer run in the converter module
// (`ambre-spec-architecture.md section 2`), on the converter thread
// (`ARCH-123`). This file owes the request, then turns the records the
// answer carries into cells.
// No plaintext file ever transits to the Blind Relay (Zero-Knowledge
// invariant).

use super::*;
use crate::formula::ACTIVE_SHEET;
use crate::formula::text::CellText;
use crate::message::{TrameConstruct, TrameReport};
use crate::message::{TrameExport, TrameImport};
use crate::{CsvDelimiter, TOTAL_COLS, TOTAL_ROWS};
use ambre_bus::Request;
use ambre_bus::interop::{
    Direction, ExportedFile, FileFormat, InteropAnswer, InteropError, InteropRequest, Occurrences,
    Outcome,
};
use ambre_convert::{Conversion, ConvertError, Converted, FileWork};
use ambre_core::id::{Id, Sheet};
use ambre_core::index::{Len, Pos};
use ambre_core::{Count, Seq};

/// The answer to an import that writes nothing: the document is as it was,
/// and the user reads why (`INTEROP-011`, `TRAME-IO-028`).
fn import_failed(error: InteropError) -> TrameOutcome {
    TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::failed(
        Direction::Import,
        error,
    ))])
}

/// The error a converter error reads as on the bus: a file the module
/// refused on import, or the stop. The converter's words go to the log here
/// (`BUS-046`).
pub(super) fn converter_error(
    direction: Direction,
    error: &ambre_convert::ConvertError,
) -> InteropError {
    tracing::warn!("Trame: a conversion produced nothing: {error}");
    match (direction, error) {
        (Direction::Import, ambre_convert::ConvertError::Refused(refusal)) => {
            InteropError::ParseFailed(*refusal)
        }
        _ => InteropError::Stopped(error.stop_cause()),
    }
}

/// The answer to a conversion the module refused or could not run: the
/// reason, shown, and the document as it was (`INTEROP-011`).
fn conversion_failed(direction: Direction, error: &ambre_convert::ConvertError) -> TrameOutcome {
    TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::failed(
        direction,
        converter_error(direction, error),
    ))])
}

/// A file conversion the converter thread runs for this workbook, until its
/// answer lands (`ARCH-123`). Each variant holds what the answer needs and
/// the module call does not carry.
pub(super) enum FileJob {
    /// The module reads the records of a CSV import, which land from `start`
    /// on `target`, past a header row when one is present.
    ReadCsv {
        header: HeaderRow,
        target: Option<Id<Sheet>>,
        start: CellPos,
    },
    /// The module reads a workbook in this format.
    ReadWorkbook(super::xlsx::SheetFile),
    /// The module writes the records of a sheet, which leave under
    /// `filename`.
    WriteCsv {
        filename: ambre_client_core::ExportFilename,
    },
    /// The module writes the workbook in `file`, which leaves under
    /// `filename`. `report` holds what the walk of the workbook counted.
    WriteWorkbook {
        file: super::xlsx::SheetFile,
        filename: ambre_client_core::ExportFilename,
        report: Box<TrameReport>,
    },
    /// The PDF module writes the pages a PDF export recorded, which leave
    /// under `filename` (`ARCH-124`).
    WritePdf {
        filename: ambre_client_core::ExportFilename,
    },
}

/// The file conversions of one workbook on the converter thread: those in
/// flight by their ticket, and those the Logic dispatcher has not drained.
#[derive(Default)]
pub(super) struct FileJobs {
    in_flight: HashMap<Seq<Request>, FileJob>,
    outbox: Vec<(Seq<Request>, Conversion)>,
}

impl FileJobs {
    /// Owe the converter thread `conversion`, whose answer `job` lands.
    pub(super) fn ask(&mut self, conversion: Conversion, job: FileJob) {
        let ticket = ambre_bus::next_request();
        self.in_flight.insert(ticket, job);
        self.outbox.push((ticket, conversion));
    }
}

impl TrameState {
    /// Serve a command that needs a converter module: the CSV, Excel and
    /// OpenDocument imports and exports (`trame-import-export`). The module
    /// call runs on the converter thread, never here (`ARCH-123`). This owes
    /// the request and changes nothing yet. The answer comes back through
    /// [`handle_file_converted`](Self::handle_file_converted). An export
    /// reads the workbook now, so its file holds the cells as the command
    /// found them. Any other command is served as
    /// [`handle_command`](Self::handle_command) serves it.
    pub fn handle_conversion(&mut self, cmd: TrameCommand) -> TrameOutcome {
        if cmd.converter_family().is_none() {
            return self.handle_command(cmd);
        }
        // The frame `handle_command` sets around every command: no cell
        // changed yet, and one step of the history per command (`TRAME-017`).
        // A running Excel import lands whole first, so an export holds it.
        self.engine.changed.clear();
        let settled = self.settle_xlsx();
        self.seal_history();
        let outcome = match cmd {
            TrameCommand::Interop(InteropRequest::Import(TrameImport::Xlsx(import))) => {
                self.ask_workbook_read(super::xlsx::SheetFile::Xlsx, import.data)
            }
            TrameCommand::Interop(InteropRequest::Import(TrameImport::Ods(import))) => {
                self.ask_workbook_read(super::xlsx::SheetFile::Ods, import.data)
            }
            TrameCommand::Interop(InteropRequest::Export(TrameExport::Xlsx)) => {
                self.ask_workbook_write(super::xlsx::SheetFile::Xlsx)
            }
            TrameCommand::Interop(InteropRequest::Export(TrameExport::Ods)) => {
                self.ask_workbook_write(super::xlsx::SheetFile::Ods)
            }
            TrameCommand::Interop(InteropRequest::Import(TrameImport::Csv(import))) => {
                let delimiter = import
                    .delimiter
                    .unwrap_or_else(|| detect_csv_delimiter(&import.data));
                self.file_jobs.ask(
                    Conversion::ReadCsv {
                        data: import.data,
                        delimiter,
                    },
                    FileJob::ReadCsv {
                        header: import.header,
                        target: import.target_sheet_id,
                        start: CellPos::new(import.start_row, import.start_col),
                    },
                );
                TrameOutcome::display_only(Vec::new())
            }
            TrameCommand::Interop(InteropRequest::Export(TrameExport::Csv {
                sheet_id,
                delimiter,
                range,
            })) => {
                let sheet = sheet_id.unwrap_or_else(|| self.active_sheet.clone());
                self.ask_csv_write(&sheet, delimiter, range)
            }
            other => self.dispatch_command(other),
        };
        self.carry_settled(settled, outcome)
    }

    /// `outcome`, after the events of the import that landed whole before
    /// it, with the delta of both, then the undo state.
    fn carry_settled(
        &mut self,
        mut settled: Vec<TrameEvent>,
        mut outcome: TrameOutcome,
    ) -> TrameOutcome {
        if !settled.is_empty() {
            settled.append(&mut outcome.display);
            outcome.display = settled;
            if outcome.update.is_none() {
                outcome.update = self.flush_delta();
            }
        }
        self.report_undo(outcome)
    }

    /// The file conversions this workbook owes the converter thread,
    /// drained. The Logic dispatcher sends each one and never waits for it
    /// (`ARCH-123`).
    pub fn take_file_requests(&mut self) -> Vec<(Seq<Request>, Conversion)> {
        std::mem::take(&mut self.file_jobs.outbox)
    }

    /// What the file conversion under `ticket` does, or `None` when the
    /// ticket is not one of this workbook's. A read writes the workbook once
    /// it lands; a write never does.
    #[must_use]
    pub fn file_work(&self, ticket: Seq<Request>) -> Option<FileWork> {
        self.file_jobs.in_flight.get(&ticket).map(|job| match job {
            FileJob::ReadCsv { .. } | FileJob::ReadWorkbook(_) => FileWork::Read,
            FileJob::WriteCsv { .. }
            | FileJob::WriteWorkbook { .. }
            | FileJob::WritePdf { .. } => FileWork::Write,
        })
    }

    /// Land the answer of a file conversion this workbook asked for, or
    /// answer `None` when the ticket is not one of its own (`ARCH-123`). A
    /// read lands in the frame of a command. A running Excel import lands
    /// whole first, and the read is one step of the history. A write
    /// answers the file, or why there is none.
    pub fn handle_file_converted(
        &mut self,
        ticket: Seq<Request>,
        answer: Result<Converted, ConvertError>,
    ) -> Option<TrameOutcome> {
        let job = self.file_jobs.in_flight.remove(&ticket)?;
        Some(match job {
            FileJob::ReadCsv {
                header,
                target,
                start,
            } => {
                self.engine.changed.clear();
                let settled = self.settle_xlsx();
                self.seal_history();
                let outcome = self.import_csv(answer, header, target, start);
                self.carry_settled(settled, outcome)
            }
            FileJob::ReadWorkbook(file) => {
                self.engine.changed.clear();
                let settled = self.settle_xlsx();
                self.seal_history();
                let outcome = self.import_xlsx(answer, file, super::xlsx::ImportPace::Slice);
                self.carry_settled(settled, outcome)
            }
            FileJob::WriteCsv { filename } => csv_written(answer, filename),
            FileJob::WriteWorkbook {
                file,
                filename,
                report,
            } => super::xlsx::workbook_written(answer, file, filename, *report),
            FileJob::WritePdf { filename } => super::print::pdf_written(answer, filename),
        })
    }

    /// Owe the converter thread the read of a workbook in `file`.
    fn ask_workbook_read(
        &mut self,
        file: super::xlsx::SheetFile,
        data: ambre_core::Payload<ambre_client_core::FileBytes>,
    ) -> TrameOutcome {
        self.file_jobs
            .ask(file.read(data), FileJob::ReadWorkbook(file));
        TrameOutcome::display_only(Vec::new())
    }

    /// Owe the converter thread the write of the workbook in `file`. The
    /// walk of the workbook runs now; a form that does not encode fails at
    /// once.
    fn ask_workbook_write(&mut self, file: super::xlsx::SheetFile) -> TrameOutcome {
        match self.workbook_write_request(file) {
            Ok((conversion, filename, report)) => {
                self.file_jobs.ask(
                    conversion,
                    FileJob::WriteWorkbook {
                        file,
                        filename,
                        report: Box::new(report),
                    },
                );
                TrameOutcome::display_only(Vec::new())
            }
            Err(outcome) => outcome,
        }
    }

    /// Owe the converter thread the write of `sheet_id`, or of `range` on
    /// it, as CSV. The cells are read now; a sheet with nothing to write
    /// fails at once.
    fn ask_csv_write(
        &mut self,
        sheet_id: &Id<Sheet>,
        delimiter: Option<CsvDelimiter>,
        range: Option<CellRect>,
    ) -> TrameOutcome {
        match self.csv_write_request(sheet_id, delimiter, range) {
            Ok((conversion, filename)) => {
                self.file_jobs
                    .ask(conversion, FileJob::WriteCsv { filename });
                TrameOutcome::display_only(Vec::new())
            }
            Err(outcome) => outcome,
        }
    }

    /// Import the records the module read into the workbook
    /// (`trame-import-export`). This resolves (or creates) the target sheet,
    /// writes each non-empty field as a literal cell, and recomputes. The
    /// whole import is a single undo group.
    fn import_csv(
        &mut self,
        answer: Result<Converted, ConvertError>,
        header: HeaderRow,
        target_sheet_id: Option<Id<Sheet>>,
        start: CellPos,
    ) -> TrameOutcome {
        let (mut records, unreadable) = match answer {
            Ok(Converted::Records {
                records,
                unreadable,
            }) => (records, unreadable.get()),
            Ok(_) => return conversion_failed(Direction::Import, &ConvertError::Malformed),
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
            let refusal = if unreadable > 0 {
                ambre_core::FileRefusal::Malformed
            } else {
                ambre_core::FileRefusal::Empty
            };
            return import_failed(InteropError::ParseFailed(refusal));
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
            return import_failed(InteropError::Stopped(ambre_core::StopCause::PastBound));
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

    /// The name a sheet shows on its tab, or its id when the listing holds
    /// none. It is the stem of the file a sheet is exported to.
    pub(super) fn sheet_title(&self, sheet_id: &Id<Sheet>) -> SheetName {
        match self
            .doc
            .get_map(Root::Sheets.as_str())
            .get(sheet_id.as_str())
            .map(|v| v.get_deep_value())
        {
            Some(LoroValue::String(s)) => SheetName::stored(s.to_string()),
            _ => SheetName::of_id(sheet_id),
        }
    }

    /// The request that writes a sheet (or sub-range) as CSV
    /// (`trame-import-export`), and the name its file leaves under. Read-only:
    /// emits no CRDT delta. Cells are written by their displayed value
    /// (formula results included).
    ///
    /// # Errors
    ///
    /// The answer of an export that writes no file, and why
    /// (`TRAME-IO-041`).
    fn csv_write_request(
        &self,
        sheet_id: &Id<Sheet>,
        delimiter: Option<CsvDelimiter>,
        range: Option<CellRect>,
    ) -> Result<(Conversion, ambre_client_core::ExportFilename), TrameOutcome> {
        let nothing = |cause: ambre_core::StopCause| {
            TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::failed(
                Direction::Export,
                InteropError::Stopped(cause),
            ))])
        };
        let Some(&idx) = self.engine_sheet_by_id.get(sheet_id) else {
            return Err(nothing(ambre_core::StopCause::SourceGone));
        };
        let index = if idx == ACTIVE_SHEET {
            self.index.clone()
        } else {
            self.build_sheet_index(sheet_id)
        };
        let rect = match range {
            Some(rect) => rect,
            None => match self.used_extent(sheet_id, &index) {
                None => return Err(nothing(ambre_core::StopCause::NothingToConvert)),
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
        let name = self.sheet_title(sheet_id);
        // A sheet name is user text, so the file name is cleaned of what a
        // host refuses and bounded (`DOCFILE-019`, `DOCFILE-020`).
        let filename = ambre_client_core::ExportFilename::from_title(
            name.as_str(),
            "sheet",
            ambre_client_core::ExportExtension::Csv,
        );
        Ok((
            Conversion::WriteCsv {
                records,
                delimiter: delimiter.unwrap_or(CsvDelimiter::COMMA),
            },
            filename,
        ))
    }

    /// The bottom-right corner of the content on
    /// `sheet_id`, or `None` when the sheet is empty. Scans the `cells_{sheet}` map
    /// (the source of truth) rather than the row/column lists. Those lists stay
    /// materialized at their high-water mark after a scroll or a since-cleared
    /// edit. Using their length would therefore export a rectangle padded with
    /// empty trailing rows/columns.
    pub(super) fn used_extent(&self, sheet_id: &Id<Sheet>, index: &SheetIndex) -> Option<CellPos> {
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

/// The answer to a CSV export once the module wrote the file: an
/// [`InteropAnswer::ExportReady`] carrying it under `filename`, or why there
/// is none. Every displayed value leaves as text, so CSV loses nothing the
/// grid shows: the report is clean.
fn csv_written(
    answer: Result<Converted, ConvertError>,
    filename: ambre_client_core::ExportFilename,
) -> TrameOutcome {
    let data = match answer {
        Ok(Converted::File(data)) => data,
        Ok(_) => return conversion_failed(Direction::Export, &ConvertError::Malformed),
        Err(error) => return conversion_failed(Direction::Export, &error),
    };
    TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::ExportReady {
        file: ExportedFile {
            filename,
            media_type: ambre_core::MediaType::from("text/csv; charset=utf-8"),
            data,
        },
        report: TrameReport::new(Direction::Export, FileFormat::Csv),
    })])
}

/// Auto-detect the CSV field separator among the comma, the semicolon and the
/// tab, by frequency in the first kilobyte (`trame-import-export`). A separator
/// inside a quoted field does not count. A tie goes to the earlier of the
/// three. Any other separator, the caller names (`TRAME-IO-035`).
fn detect_csv_delimiter(data: &ambre_core::Payload<ambre_client_core::FileBytes>) -> CsvDelimiter {
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

#[cfg(test)]
impl TrameState {
    /// [`Self::handle_conversion`], then the file conversions it owes the
    /// converter thread, run in place through `converter` as the thread runs
    /// them.
    pub(crate) fn convert_in_place(
        &mut self,
        cmd: TrameCommand,
        converter: &ambre_convert::Converter,
    ) -> TrameOutcome {
        let mut outcome = self.handle_conversion(cmd);
        for (ticket, conversion) in self.take_file_requests() {
            if let Some(next) = self.handle_file_converted(ticket, converter.convert(&conversion)) {
                outcome.display.extend(next.display);
                if next.update.is_some() {
                    outcome.update = next.update;
                }
            }
        }
        outcome
    }
}
