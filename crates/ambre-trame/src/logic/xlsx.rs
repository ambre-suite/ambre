// ambre-trame/src/logic/xlsx.rs
// Excel import and export (`trame-import-export`).
//
// Part of the `TrameState` impl, split out by concern. The Office converter
// module reads and writes the bytes (`ambre-spec-architecture.md section 2`),
// and answers the plain model of `ambre-xlsx`. This file maps that model onto
// the workbook, one slice of cells per pass, and the workbook back onto the
// model. No plaintext file ever reaches the relay (`ZK-001`).

use super::cells::SheetLines;
use super::*;
use crate::formula::ACTIVE_SHEET;
use crate::formula::functions::is_builtin;
use crate::formula::text::CellText;
use crate::formula::types::FormulaError;
use crate::message::{TrameConstruct, TrameReport};
use crate::{TOTAL_COLS, TOTAL_ROWS};
use ambre_bus::interop::{
    ConversionProgress, Direction, ExportedFile, FileFormat, InteropAnswer, InteropError,
    Occurrences, Outcome, Steps,
};
use ambre_core::id::{Id, Sheet};
use ambre_core::index::Pos;
use ambre_core::{Count, Payload};
use ambre_xlsx::book::{Book, BookCell, BookContent, BookSheet, StyleBytes};
use ambre_xlsx::formula::{AmbreFormula, ExcelFormula};
use ambre_xlsx::model as xl;
use ambre_xlsx::report::{Outcome as XlOutcome, Report as XlReport, XlsxConstruct};

/// The character an Excel formula Trame cannot run is stored behind, before
/// its `=` (`TRAME-IO-011`). The null byte: no user types it, so the export
/// finds the formula again, with no second container to hold a flag.
pub(crate) const VERBATIM_MARKER: char = '\0';

/// What an imported Excel cell becomes (`TRAME-IO-010`). `UnsupportedFormula`
/// and `Literal` are never mixed: they re-export differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImportedCellKind {
    /// Parsed, and executable by the engine: the source text, `=` first.
    Formula(CellText<SourceForm>),
    /// A formula the engine cannot run, held verbatim in Excel's dialect,
    /// without its `=`. Stored behind [`VERBATIM_MARKER`].
    UnsupportedFormula(Validated<ExcelFormula>),
    /// A value: text, number or boolean.
    Literal(CellText<SourceForm>),
}

impl ImportedCellKind {
    /// The text the cell is written with.
    pub(crate) fn source(&self) -> CellText<SourceForm> {
        match self {
            Self::Formula(text) | Self::Literal(text) => text.clone(),
            Self::UnsupportedFormula(excel) => {
                CellText::new(format!("{VERBATIM_MARKER}={}", excel.as_str()))
            }
        }
    }
}

/// The Excel formula a stored raw text holds verbatim, if it holds one.
pub(crate) fn verbatim_formula(raw: &CellText<StoredForm>) -> Option<&str> {
    raw.as_str()
        .strip_prefix(VERBATIM_MARKER)?
        .strip_prefix('=')
}

/// Sort one Excel cell into its state (`TRAME-IO-010`). `None` for a cell
/// that writes nothing: an empty value or an error value. `runs` answers
/// the source a Trame text stores when the engine parses it and runs it.
pub(crate) fn classify(
    content: &BookContent,
    runs: &impl Fn(&Validated<AmbreFormula>) -> Option<CellText<SourceForm>>,
) -> Option<ImportedCellKind> {
    match content {
        BookContent::Literal(text) if text.is_empty() => None,
        BookContent::Literal(text) => Some(ImportedCellKind::Literal(CellText::new(text.clone()))),
        BookContent::Formula { trame, excel, .. } => match (trame.as_ref().and_then(runs), excel) {
            (Some(source), _) => Some(ImportedCellKind::Formula(source)),
            (None, Some(excel)) => Some(ImportedCellKind::UnsupportedFormula(excel.clone())),
            (None, None) => trame.as_ref().and_then(|t| {
                Validated::parse(t.as_str().trim_start_matches('=').to_owned())
                    .ok()
                    .map(ImportedCellKind::UnsupportedFormula)
            }),
        },
    }
}

/// Whether every function `expr` calls is a built-in of the engine.
pub(super) fn calls_builtins(expr: &Expr) -> bool {
    match expr {
        Expr::FnCall { name, args } => is_builtin(name) && args.iter().all(calls_builtins),
        Expr::BinOp { lhs, rhs, .. } => calls_builtins(lhs) && calls_builtins(rhs),
        Expr::UnaryOp { operand, .. } => calls_builtins(operand),
        Expr::SheetRef { inner, .. } => calls_builtins(inner),
        _ => true,
    }
}

/// Rewrite each sheet qualifier of `expr` that names no live sheet as `#REF!`:
/// a reference to a deleted sheet (`TRAME-IO-018`).
fn break_dangling(expr: Expr, live: &HashSet<Id<Sheet>>) -> Expr {
    let again = |e: Box<Expr>| Box::new(break_dangling(*e, live));
    match expr {
        Expr::SheetRef {
            sheet: crate::formula::types::SheetQualifier::Id(id),
            ..
        } if !live.contains(&id) => Expr::Literal(CellValue::Error(FormulaError::BrokenRef)),
        Expr::BinOp { kind, lhs, rhs } => Expr::BinOp {
            kind,
            lhs: again(lhs),
            rhs: again(rhs),
        },
        Expr::UnaryOp { kind, operand } => Expr::UnaryOp {
            kind,
            operand: again(operand),
        },
        Expr::FnCall { name, args } => Expr::FnCall {
            name,
            args: args.into_iter().map(|a| break_dangling(a, live)).collect(),
        },
        other => other,
    }
}

/// How many units an import applies per call. A unit is one cell, one merge,
/// one column width or one row height.
pub enum XlsxUnits {}

impl ambre_core::NarrowCount for XlsxUnits {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("XlsxUnitCount");
}

/// An import reports its progress in its own units (`INTEROP-013`).
impl ambre_core::Recounts<XlsxUnits> for Steps {}

/// The units one call of a running import applies (`TRAME-IO-013`). The
/// `xlsx_slice` test reads the cost of one slice against the keystroke pass.
pub(super) const XLSX_SLICE: Count<XlsxUnits> = Count::new(256);

/// How much of a running import one call applies.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ImportPace {
    /// One slice, so the grid's other commands wait no longer than that.
    Slice,
    /// Every unit left, before an edit or an export.
    Whole,
}

/// One sheet of a running import: its id, and what is left to write.
struct ImportSheet {
    id: Id<Sheet>,
    cells: std::vec::IntoIter<ImportedCell>,
    merges: Vec<CellRect>,
    widths: Vec<(Pos<Col>, Px<Doc>)>,
    heights: Vec<(Pos<Row>, Px<Doc>)>,
}

/// One cell of a running import: where it lands, the text it is written
/// with, and the bytes of its style. The text is empty for a style alone.
type ImportedCell = (CellPos, CellText<SourceForm>, Option<Payload<StyleBytes>>);

impl ImportSheet {
    fn units(&self) -> Count<XlsxUnits> {
        Count::of(self.cells.len() + self.merges.len() + self.widths.len() + self.heights.len())
    }
}

/// Whether the formula counts of the codec reach the report as they are.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FormulaCounts {
    /// An export: the codec counted each formula it wrote.
    Kept,
    /// An import: the engine sorts each formula, and the import counts it
    /// again (`TRAME-IO-010`).
    Recounted,
}

/// An Excel import being applied one slice at a time (`TRAME-IO-013`). The
/// Logic thread resumes it between its messages.
pub(super) struct XlsxJob {
    /// The sheets left, the one being written at the end of the list. A
    /// slice pops it once it is written.
    sheets: Vec<ImportSheet>,
    /// The sheet to show once the import ends: the first one shown.
    shown: Id<Sheet>,
    report: TrameReport,
    done: Count<XlsxUnits>,
    total: Count<XlsxUnits>,
}

/// What the export reads one sheet through.
struct ExportSheet<'a, 'l> {
    id: &'a Id<Sheet>,
    idx: Pos<EngineSheet>,
    index: &'a SheetIndex,
    lines: &'a SheetLines<'l>,
    /// Every live sheet: a qualifier naming another is a deleted sheet.
    live: &'a HashSet<Id<Sheet>>,
}

/// The answer to an import or an export that produced nothing.
fn failed(direction: Direction, error: InteropError) -> TrameOutcome {
    TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::failed(
        direction, error,
    ))])
}

/// The bus outcome of a codec outcome.
const fn outcome_of(outcome: XlOutcome) -> Outcome {
    match outcome {
        XlOutcome::Carried => Outcome::Carried,
        XlOutcome::Approximated => Outcome::Approximated,
        XlOutcome::Dropped => Outcome::Dropped,
    }
}

/// Every entry of the codec's `report` into `into`, but the formula counts,
/// which the import counts again once the engine sorted each formula.
fn carry_report(report: &XlReport, into: &mut TrameReport, formulas: FormulaCounts) {
    for entry in report.entries() {
        if formulas == FormulaCounts::Recounted
            && matches!(
                entry.construct,
                XlsxConstruct::Formula | XlsxConstruct::ForeignFormula
            )
        {
            continue;
        }
        into.record_count(
            TrameConstruct::Excel(entry.construct),
            outcome_of(entry.outcome),
            Count::<Occurrences>::new(entry.count.get()),
        );
    }
}

/// The Excel value of an evaluated Trame value. A circular reference, which
/// Excel spells with no error, leaves as `#VALUE!` and is reported.
fn excel_value(value: &CellValue, report: &mut TrameReport) -> xl::Value {
    match value {
        CellValue::Number(n) => xl::Value::Number(*n),
        CellValue::Text(text) => xl::Value::Text(text.clone()),
        CellValue::Boolean(b) => xl::Value::Boolean(*b),
        CellValue::Empty | CellValue::Range(_) => xl::Value::Empty,
        CellValue::Error(e) => xl::Value::Error(match e {
            FormulaError::BrokenRef => xl::ErrorValue::Ref,
            FormulaError::DivisionByZero => xl::ErrorValue::DivisionByZero,
            FormulaError::TypeError => xl::ErrorValue::Value,
            FormulaError::NumError => xl::ErrorValue::Num,
            FormulaError::NaError => xl::ErrorValue::NotAvailable,
            FormulaError::NameError | FormulaError::ParseError(_) => xl::ErrorValue::Name,
            FormulaError::CircularReference => {
                report.record(
                    TrameConstruct::Excel(XlsxConstruct::ErrorValue),
                    Outcome::Approximated,
                );
                xl::ErrorValue::Value
            }
        }),
    }
}

/// The Excel position of a grid position.
fn excel_ref(at: CellPos) -> Option<ambre_xlsx::cellref::CellRef> {
    ambre_xlsx::cellref::CellRef::new(Pos::new(at.row.get()), Pos::new(at.col.get()))
}

/// The office format a workbook leaves in or comes from. The Office module
/// reads both into one form, so the import and the export below serve both
/// (`TRAME-IO-029`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SheetFile {
    Xlsx,
    Ods,
}

impl SheetFile {
    fn read(self, data: ambre_core::Payload<ambre_core::FileBytes>) -> ambre_convert::Conversion {
        match self {
            Self::Xlsx => ambre_convert::Conversion::ReadXlsx { data },
            Self::Ods => ambre_convert::Conversion::ReadOds { data },
        }
    }

    fn write(
        self,
        workbook: ambre_core::Payload<ambre_convert::WorkbookBytes>,
    ) -> ambre_convert::Conversion {
        match self {
            Self::Xlsx => ambre_convert::Conversion::WriteXlsx { workbook },
            Self::Ods => ambre_convert::Conversion::WriteOds { workbook },
        }
    }

    const fn format(self) -> FileFormat {
        match self {
            Self::Xlsx => FileFormat::Xlsx,
            Self::Ods => FileFormat::Ods,
        }
    }

    const fn extension(self) -> ambre_core::ExportExtension {
        match self {
            Self::Xlsx => ambre_core::ExportExtension::Xlsx,
            Self::Ods => ambre_core::ExportExtension::Ods,
        }
    }

    const fn media_type(self) -> &'static str {
        match self {
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Ods => "application/vnd.oasis.opendocument.spreadsheet",
        }
    }
}

impl TrameState {
    /// Read an Excel workbook or an OpenDocument spreadsheet through the
    /// Office module, then start applying it. Every sheet is listed now, and
    /// the cells land one slice per call (`TRAME-IO-013`). The whole import is
    /// one undo step.
    pub(super) fn import_xlsx(
        &mut self,
        converter: &ambre_convert::Converter,
        file: SheetFile,
        data: ambre_core::Payload<ambre_core::FileBytes>,
        pace: ImportPace,
    ) -> TrameOutcome {
        let (workbook, codec_report): (Book, XlReport) = match converter
            .convert(&file.read(data))
            .and_then(|answer| match answer {
                ambre_convert::Converted::Workbook(bytes) => ambre_convert::decode_workbook(&bytes),
                _ => Err(ambre_convert::ConvertError::Malformed),
            }) {
            Ok(read) => read,
            Err(error) => {
                return failed(
                    Direction::Import,
                    super::interop::converter_error(Direction::Import, &error),
                );
            }
        };
        if workbook.sheets.is_empty() {
            return failed(
                Direction::Import,
                InteropError::ParseFailed(ambre_core::FileRefusal::Empty),
            );
        }
        // One sheet id per sheet, minted before anything is written, so a
        // spent counter refuses the import with the document untouched
        // (`TRAME-243`).
        let mut ids = Vec::with_capacity(workbook.sheets.len());
        for _ in &workbook.sheets {
            match self.mint_sheet_id() {
                Some(id) => ids.push(id),
                None => {
                    return TrameOutcome::display_only(vec![super::sheets::sheet_ids_exhausted()]);
                }
            }
        }
        let mut display = self.settle_xlsx();
        self.doc.commit();
        self.seal_history();
        self.hold_default_sheet();

        // The sheets are listed first, so a formula reads a sheet the
        // import writes later. A name the workbook already carries takes a
        // free variant, and each formula names its sheet by id.
        let mut by_name: Vec<(SheetNameKey, Id<Sheet>)> = Vec::with_capacity(ids.len());
        for (sheet, id) in workbook.sheets.iter().zip(&ids) {
            let wanted = SheetName::new(&sheet.name).unwrap_or_default();
            let name = self.free_sheet_name(&wanted);
            by_name.push((wanted.key(), id.clone()));
            let _ = self
                .doc
                .get_map(Root::Sheets.as_str())
                .insert(id.as_str(), name.as_str());
            let _ = self
                .doc
                .get_movable_list(Root::SheetOrder.as_str())
                .push(id.as_str());
            if sheet.visibility == xl::Visibility::Hidden {
                let _ = self
                    .doc
                    .get_map(Root::HiddenSheets.as_str())
                    .insert(id.as_str(), true);
            }
        }
        // The engine learns the new sheets, so a qualifier binds to them.
        self.reseed_active_sheet();

        let name_to_id = |name: &SheetName| {
            let key = name.key();
            by_name
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, id)| id.clone())
        };
        let runs = |text: &Validated<AmbreFormula>| -> Option<CellText<SourceForm>> {
            let body = text.as_str().strip_prefix('=')?;
            let ast = parse_formula(body).ok()?;
            if !calls_builtins(&ast) {
                return None;
            }
            let canonical = render::canonicalize(ast, &name_to_id);
            // A qualifier no sheet of the workbook carries stays Excel's.
            if render::first_name_qualifier(&canonical).is_some() {
                return None;
            }
            Some(CellText::new(format!("={}", render::render(&canonical))))
        };
        let mut report = TrameReport::new(Direction::Import, file.format());
        carry_report(&codec_report, &mut report, FormulaCounts::Recounted);
        let (mut parsed, mut verbatim, mut refused) = (0usize, 0usize, 0usize);
        let mut sheets = Vec::with_capacity(ids.len());
        let mut shown = None;
        let styles = &workbook.styles;
        for (sheet, id) in workbook.sheets.iter().zip(ids) {
            if shown.is_none() && sheet.visibility == xl::Visibility::Visible {
                shown = Some(id.clone());
            }
            let in_grid = |row: u32, col: u32| {
                (row < TOTAL_ROWS.get() && col < TOTAL_COLS.get())
                    .then(|| CellPos::new(Pos::new(row), Pos::new(col)))
            };
            let mut cells = Vec::with_capacity(sheet.cells.len());
            for cell in &sheet.cells {
                let Some(at) = in_grid(cell.at.row.get(), cell.at.col.get()) else {
                    refused += 1;
                    continue;
                };
                let kind = classify(&cell.content, &runs);
                match kind {
                    Some(ImportedCellKind::Formula(_)) => parsed += 1,
                    Some(ImportedCellKind::UnsupportedFormula(_)) => verbatim += 1,
                    _ => {}
                }
                let style = (cell.style != xl::StyleId::DEFAULT)
                    .then(|| cell.style.slot().to_index())
                    .flatten()
                    .and_then(|i| styles.get(i))
                    .cloned();
                if kind.is_none() && style.is_none() {
                    continue;
                }
                cells.push((at, kind.map(|k| k.source()).unwrap_or_default(), style));
            }
            let merges = sheet
                .merges
                .iter()
                .filter_map(|m| {
                    let a = in_grid(m.first().row.get(), m.first().col.get())?;
                    let b = in_grid(m.last().row.get(), m.last().col.get())?;
                    Some(CellRect::spanning(a, b))
                })
                .collect::<Vec<_>>();
            let dropped_merges = sheet.merges.len() - merges.len();
            report.record_count(
                TrameConstruct::Excel(XlsxConstruct::Merge),
                Outcome::Dropped,
                Count::<Occurrences>::of(dropped_merges),
            );
            // The module dropped each size past the last cell. One past the
            // grid is dropped here, as its cell is.
            let widths = sheet
                .column_widths
                .iter()
                .filter(|w| w.first.get() < TOTAL_COLS.get())
                .map(|w| (Pos::<Col>::new(w.first.get()), w.width))
                .collect();
            let heights = sheet
                .row_heights
                .iter()
                .filter(|h| h.row.get() < TOTAL_ROWS.get())
                .map(|h| (Pos::<Row>::new(h.row.get()), h.height))
                .collect();
            sheets.push(ImportSheet {
                id,
                cells: cells.into_iter(),
                merges,
                widths,
                heights,
            });
        }
        report.record_count(
            TrameConstruct::Excel(XlsxConstruct::Formula),
            Outcome::Carried,
            Count::<Occurrences>::of(parsed),
        );
        report.record_count(
            TrameConstruct::Excel(XlsxConstruct::ForeignFormula),
            Outcome::Approximated,
            Count::<Occurrences>::of(verbatim),
        );
        report.record_count(
            TrameConstruct::RefusedCell,
            Outcome::Dropped,
            Count::<Occurrences>::of(refused),
        );
        let total = sheets
            .iter()
            .map(ImportSheet::units)
            .fold(Count::<XlsxUnits>::ZERO, |a, b| a.plus(b));
        // A slice writes the last entry, so the first sheet of the file sits
        // last and lands first.
        sheets.reverse();
        // A workbook whose every sheet is hidden leaves the sheet shown before.
        let shown = shown.unwrap_or_else(|| self.active_sheet.clone());
        self.xlsx_job = Some(Box::new(XlsxJob {
            sheets,
            shown,
            report,
            done: Count::ZERO,
            total,
        }));
        // The listing commits inside the step the cells join.
        self.doc.commit();
        let mut outcome = self.run_xlsx(pace);
        display.append(&mut outcome.display);
        outcome.display = display;
        outcome.update = self.flush_delta();
        outcome
    }

    /// Whether an Excel import, a sweep or a PDF export is still running
    /// (`TRAME-IO-013`, `TRAME-305`, `TRAME-IO-076`). The
    /// Logic thread reads it to come back to this workbook without waiting
    /// for a message.
    #[must_use]
    pub fn has_pending_work(&self) -> bool {
        self.xlsx_job.is_some() || self.sweeping() || self.is_printing()
    }

    /// Apply the next slice of the running import, and flush its delta. With
    /// no import running it removes the next slice of a running sweep
    /// (`TRAME-305`). With neither it answers nothing.
    pub fn resume(&mut self) -> TrameOutcome {
        if self.xlsx_job.is_none() {
            if self.sweeping() {
                return self.sweep_slice();
            }
            return TrameOutcome::display_only(Vec::new());
        }
        let mut outcome = self.run_xlsx(ImportPace::Slice);
        outcome.update = self.flush_delta();
        self.report_undo(outcome)
    }

    /// Land the rest of a running import now, and flush its delta: before a
    /// remote delta merges or the workbook closes (`ARCH-031`).
    pub fn settle(&mut self) -> TrameOutcome {
        if self.xlsx_job.is_none() {
            return TrameOutcome::display_only(Vec::new());
        }
        let mut outcome = self.run_xlsx(ImportPace::Whole);
        outcome.update = self.flush_delta();
        self.report_undo(outcome)
    }

    /// Land the rest of a running import without flushing it: the caller's
    /// own flush carries it. Called before a command that is no scroll, so an
    /// edit never lands inside the import's undo step.
    pub(super) fn settle_xlsx(&mut self) -> Vec<TrameEvent> {
        if self.xlsx_job.is_none() {
            return Vec::new();
        }
        self.run_xlsx(ImportPace::Whole).display
    }

    /// Apply `pace` of the running import and commit. The caller flushes.
    /// A slice short of the end reports its progress (`INTEROP-013`), and the
    /// last one closes the import with its report.
    fn run_xlsx(&mut self, pace: ImportPace) -> TrameOutcome {
        let Some(mut job) = self.xlsx_job.take() else {
            return TrameOutcome::display_only(Vec::new());
        };
        let end = match pace {
            ImportPace::Slice => job.done.plus(XLSX_SLICE).min(job.total),
            ImportPace::Whole => job.total,
        };
        while job.done < end {
            let Some(sheet) = job.sheets.last_mut() else {
                break;
            };
            if sheet.units() == Count::ZERO {
                job.sheets.pop();
                continue;
            }
            if sheet.id != self.active_sheet {
                self.active_sheet = sheet.id.clone();
                self.reseed_active_sheet();
            }
            let room = Count::<XlsxUnits>::new(end.get() - job.done.get());
            let applied = self.apply_units(sheet, room);
            job.done = job.done.plus(applied);
        }
        let refused = self.write_refusals.len();
        self.write_refusals.clear();
        job.report.record_count(
            TrameConstruct::RefusedCell,
            Outcome::Dropped,
            Count::<Occurrences>::of(refused),
        );
        if job.done < job.total {
            self.doc.commit();
            let indices = self.engine_indices();
            self.engine.compute_all_dirty(&indices);
            let progress = ConversionProgress::new(
                Direction::Import,
                Count::<Steps>::recount(job.done),
                Count::<Steps>::recount(job.total),
            );
            self.xlsx_job = Some(job);
            return TrameOutcome::display_only(vec![
                TrameEvent::DataReset,
                TrameEvent::Interop(InteropAnswer::Progress(progress)),
            ]);
        }
        // The last slice: show the first sheet shown, rebuild every cache
        // of it, and answer the report.
        self.doc.commit();
        self.active_sheet = job.shown.clone();
        self.reseed_active_sheet();
        let indices = self.engine_indices();
        self.engine.compute_all_dirty(&indices);
        self.engine.changed.clear();
        TrameOutcome::display_only(vec![
            TrameEvent::DataReset,
            self.sheet_list_event(),
            self.style_state_event(),
            TrameEvent::Interop(InteropAnswer::ImportCompleted { report: job.report }),
        ])
    }

    /// Write at most `room` units of `sheet`, the active sheet, and return
    /// how many it wrote. Every cell of the slice is materialized before the
    /// first value lands, as the CSV import does (`TRAME-IO-025`).
    fn apply_units(&mut self, sheet: &mut ImportSheet, room: Count<XlsxUnits>) -> Count<XlsxUnits> {
        let room = room.get() as usize;
        let cells: Vec<_> = sheet.cells.by_ref().take(room).collect();
        // The far corner of the slice first: every line the slice needs
        // lands in one growth, and each cell then finds its lines. A growth
        // per cell costs a plan of the whole axis each time.
        if let (Some(row), Some(col)) = (
            cells.iter().map(|c| c.0.row).max(),
            cells.iter().map(|c| c.0.col).max(),
        ) {
            let _ = self.materialize(CellPos::new(row, col));
        }
        for (at, _, _) in &cells {
            let _ = self.materialize(*at);
        }
        let mut applied = cells.len();
        let mut styled = false;
        self.write_imported_cells(&cells);
        for (at, _, style) in cells {
            if let Some(record) = style {
                styled |= self.write_style_record(at, &record).is_some();
            }
        }
        if styled {
            self.reseed_styles();
        }
        // The cells of the sheet are all written: its merges and its line
        // sizes follow, in the same slice while room is left.
        if sheet.cells.len() == 0 {
            let left = room - applied;
            let merges: Vec<_> = sheet.merges.drain(..sheet.merges.len().min(left)).collect();
            applied += merges.len();
            for rect in merges {
                let _ = self.insert_merge_entry(rect);
            }
            let left = room - applied;
            let widths: Vec<_> = sheet.widths.drain(..sheet.widths.len().min(left)).collect();
            applied += widths.len();
            for (col, width) in widths {
                if let Ok((_, uid)) = self.materialize(CellPos::new(Pos::<Row>::ZERO, col)) {
                    let width = super::metrics::COL_SIZES.clamp(width);
                    let _ = self
                        .doc
                        .get_map(SheetContainer::of(SheetRoot::ColWidths, &sheet.id).as_str())
                        .insert(&uid.to_key_part(), f64::from(width.get()));
                }
            }
            let left = room - applied;
            let heights: Vec<_> = sheet
                .heights
                .drain(..sheet.heights.len().min(left))
                .collect();
            applied += heights.len();
            for (row, height) in heights {
                if let Ok((uid, _)) = self.materialize(CellPos::new(row, Pos::<Col>::ZERO)) {
                    let height = super::metrics::ROW_SIZES.clamp(height);
                    let _ = self
                        .doc
                        .get_map(SheetContainer::of(SheetRoot::RowHeights, &sheet.id).as_str())
                        .insert(&uid.to_key_part(), f64::from(height.get()));
                }
            }
            if sheet.units() == Count::ZERO {
                self.rebuild_merges();
                self.reseed_metrics();
            }
        }
        Count::of(applied)
    }

    /// Write the texts of one slice into the active sheet, each cell
    /// already materialized. The write of one cell, [`write_cell`], reads
    /// the line order of every sheet. Here the slice reads it once. So a
    /// slice costs its own cells, and not the size of the sheet
    /// (`TRAME-IO-013`). The caller commits.
    ///
    /// [`write_cell`]: Self::write_cell
    fn write_imported_cells(&mut self, cells: &[ImportedCell]) {
        let sheet = self.active_sheet.clone();
        let indices = self.engine_indices();
        let map = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Cells, &sheet).as_str());
        let mut written = Vec::with_capacity(cells.len());
        {
            let lines = self.sheet_lines();
            for (at, text, _) in cells {
                if text.is_empty() {
                    continue;
                }
                let (Some((rl, cl)), Some(key)) = (self.index.resolve(*at), self.index.key_at(*at))
                else {
                    // A cell the grid refused to materialize (`TRAME-IO-012`).
                    continue;
                };
                let stored = self
                    .canonicalize_input(text, &sheet, &lines)
                    .unwrap_or_else(|_| CellText::<StoredForm>::new(text.as_str().to_owned()));
                let _ = map.insert(key.as_str(), stored.as_str());
                written.push((CellId::bound(ACTIVE_SHEET, rl, cl), stored));
            }
        }
        let ctx = TrameCtx {
            engine_sheet_by_name: &self.engine_sheet_by_name,
            engine_sheet_by_id: &self.engine_sheet_by_id,
            named: &self.named,
        };
        for (cell, stored) in written {
            self.engine.on_cell_changed(cell, stored, &indices, &ctx);
        }
    }

    /// Store `style`, the bytes of a style record the module wrote, as the
    /// style of the active-sheet cell `at`. The caller commits.
    fn write_style_record(&mut self, at: CellPos, style: &Payload<StyleBytes>) -> Option<()> {
        self.materialize(at).ok()?;
        let key = self.index.key_at(at)?;
        let _ = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::CellStyle, &self.active_sheet).as_str())
            .insert(key.as_str(), style.as_bytes().to_vec());
        Some(())
    }

    /// Write the workbook as an Excel file through the Office module. Every
    /// sheet leaves, hidden ones hidden. Each formula leaves in its display
    /// form: each reference at its position, each sheet by its name
    /// (`TRAME-IO-017`, `TRAME-IO-048`). A reference to a deleted line or
    /// sheet leaves as `#REF!` (`TRAME-IO-018`). The export writes nothing
    /// (`TRAME-IO-047`).
    pub(super) fn export_xlsx(
        &self,
        converter: &ambre_convert::Converter,
        file: SheetFile,
    ) -> TrameOutcome {
        let mut report = TrameReport::new(Direction::Export, file.format());
        let (workbook, title) = self.excel_workbook(&mut report);
        let written = ambre_convert::encode_workbook(&workbook)
            .and_then(|bytes| converter.convert(&file.write(bytes)))
            .and_then(|answer| match answer {
                ambre_convert::Converted::WrittenWorkbook { file, report } => {
                    ambre_convert::decode_workbook::<XlReport>(&report).map(|r| (file, r))
                }
                _ => Err(ambre_convert::ConvertError::Malformed),
            });
        let (data, codec_report) = match written {
            Ok(written) => written,
            Err(error) => {
                return failed(
                    Direction::Export,
                    super::interop::converter_error(Direction::Export, &error),
                );
            }
        };
        carry_report(&codec_report, &mut report, FormulaCounts::Kept);
        TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::ExportReady {
            file: ExportedFile {
                filename: ambre_core::ExportFilename::from_title(
                    title.as_ref().map_or("", SheetName::as_str),
                    "workbook",
                    file.extension(),
                ),
                media_type: ambre_core::MediaType::from(file.media_type()),
                data,
            },
            report,
        })])
    }

    /// Trame's form of the whole workbook, and the name of its first sheet,
    /// which titles the file. The module brings each name to Excel's rule
    /// and orders the cells and the sizes.
    pub(super) fn excel_workbook(&self, report: &mut TrameReport) -> (Book, Option<SheetName>) {
        let names = self.sheet_names();
        let hidden = self.hidden_sheet_ids();
        let live: HashSet<Id<Sheet>> = self.all_sheet_ids().into_iter().collect();
        let lines = self.sheet_lines();
        let mut book = Book {
            sheets: Vec::new(),
            styles: vec![Payload::new(Vec::new())],
        };
        let mut title = None;
        for id in self.sheet_order_ids() {
            let Some(&idx) = self.engine_sheet_by_id.get(&id) else {
                continue;
            };
            let name = names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| SheetName::of_id(&id));
            if title.is_none() {
                title = Some(name.clone());
            }
            let index = if id == self.active_sheet {
                self.index.clone()
            } else {
                self.build_sheet_index(&id)
            };
            let cells_map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, &id).as_str());
            let style_map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::CellStyle, &id).as_str());
            let mut places: Vec<CellPos> = Vec::new();
            let mut seen: HashSet<CellPos> = HashSet::default();
            for map in [&cells_map, &style_map] {
                if let LoroValue::Map(entries) = map.get_value() {
                    let (shown, _) = super::cells::shown_cells(&index, entries.iter());
                    places.extend(
                        shown
                            .into_iter()
                            .filter_map(|(at, _, _)| seen.insert(at).then_some(at)),
                    );
                }
            }
            let read = ExportSheet {
                id: &id,
                idx,
                index: &index,
                lines: &lines,
                live: &live,
            };
            let mut cells = Vec::with_capacity(places.len());
            for at in places {
                let Some(excel_at) = excel_ref(at) else {
                    continue;
                };
                // Each distinct style leaves once, by its stored bytes.
                let style = match super::cells::cell_value(&style_map, &index, at) {
                    Some(LoroValue::Binary(bytes)) => {
                        match book
                            .styles
                            .iter()
                            .position(|s| s.as_bytes() == bytes.as_slice())
                        {
                            Some(slot) => slot,
                            None => {
                                book.styles.push(Payload::new(bytes.to_vec()));
                                book.styles.len() - 1
                            }
                        }
                    }
                    _ => 0,
                };
                let raw = match super::cells::cell_value(&cells_map, &index, at) {
                    Some(LoroValue::String(s)) => CellText::new(s.to_string()),
                    _ => CellText::default(),
                };
                cells.push(BookCell {
                    at: excel_at,
                    content: self.excel_content(&read, at, raw, report),
                    style: Pos::from_index(style).map_or(xl::StyleId::DEFAULT, xl::StyleId::at),
                });
            }
            book.sheets.push(BookSheet {
                name: name.as_str().to_owned(),
                visibility: if hidden.contains(&id) {
                    xl::Visibility::Hidden
                } else {
                    xl::Visibility::Visible
                },
                cells,
                merges: self
                    .resolved_merges(&id, &index)
                    .into_iter()
                    .filter_map(|r| {
                        Some(ambre_xlsx::cellref::CellRange::spanning(
                            excel_ref(r.top_left)?,
                            excel_ref(r.bottom_right)?,
                        ))
                    })
                    .collect(),
                column_widths: self.excel_widths(&id, &index),
                row_heights: self.excel_heights(&id, &index),
            });
        }
        (book, title)
    }

    /// Trame's form of the cell `at` of `sheet`, from its raw text.
    fn excel_content(
        &self,
        sheet: &ExportSheet<'_, '_>,
        at: CellPos,
        raw: CellText<StoredForm>,
        report: &mut TrameReport,
    ) -> BookContent {
        let ExportSheet {
            id,
            idx,
            index,
            lines,
            live,
        } = sheet;
        if let Some(excel) = verbatim_formula(&raw) {
            // Restored as it came, byte for byte (`TRAME-IO-011`).
            return BookContent::Formula {
                trame: None,
                excel: Validated::parse(excel.to_owned()).ok(),
                cached: xl::Value::Empty,
            };
        }
        if !raw.is_formula() {
            return BookContent::Literal(raw.into_string());
        }
        let display = self.formula_display(&raw, id, lines);
        let text = match display.formula_body().map(parse_formula) {
            Some(Ok(ast)) => format!("={}", render::render(&break_dangling(ast, live))),
            _ => display.as_str().to_owned(),
        };
        let cached = index
            .resolve(at)
            .and_then(|(rl, cl)| self.engine.evaluated_value(CellId::bound(*idx, rl, cl)))
            .map_or(xl::Value::Empty, |v| excel_value(&v, report));
        BookContent::Formula {
            trame: Validated::parse(text).ok(),
            excel: None,
            cached,
        }
    }

    /// The column widths `col_widths_{sheet}` stores for the sheet `id`, one
    /// entry per column.
    fn excel_widths(&self, id: &Id<Sheet>, index: &SheetIndex) -> Vec<xl::ColumnWidth> {
        let LoroValue::Map(widths) = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::ColWidths, id).as_str())
            .get_value()
        else {
            return Vec::new();
        };
        widths
            .iter()
            .filter_map(|(key, value)| {
                let col = Pos::new(index.col_pos(&Uid::parse(key)?)?.get());
                Some(xl::ColumnWidth {
                    first: col,
                    last: col,
                    width: super::metrics::COL_SIZES.read(value)?,
                })
            })
            .collect()
    }

    /// The row heights `row_heights_{sheet}` stores for the sheet `id`.
    fn excel_heights(&self, id: &Id<Sheet>, index: &SheetIndex) -> Vec<xl::RowHeight> {
        let LoroValue::Map(heights) = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::RowHeights, id).as_str())
            .get_value()
        else {
            return Vec::new();
        };
        heights
            .iter()
            .filter_map(|(key, value)| {
                Some(xl::RowHeight {
                    row: Pos::new(index.row_pos(&Uid::parse(key)?)?.get()),
                    height: super::metrics::ROW_SIZES.read(value)?,
                })
            })
            .collect()
    }
}
