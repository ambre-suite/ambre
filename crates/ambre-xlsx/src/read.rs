//! The reader: a package's bytes into a [`Workbook`] and its [`Report`].
//!
//! The small parts -- the workbook, the styles, the theme, the relationships
//! -- read as a tree. The worksheets and the shared strings stream, so a large
//! sheet costs its cells and never a tree of its XML.

use std::collections::BTreeMap;

use ambre_core::index::{Delta, Pos};
use ambre_core::{Cells, Count, Doc, FileBytes, Payload, Px, Validated};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::cellref::{CellRange, CellRef, Col, LAST_COL, LAST_ROW, Row};
use crate::formula::{self, Formula};
use crate::model::{
    Cell, CellStyle, ColumnWidth, Content, ErrorValue, RowHeight, Sheet, SheetName, StyleId, Value,
    Visibility, Workbook,
};
use crate::opc::{self, Package, PartBytes, PartName, RelKind, RelTarget, Relationship};
use crate::report::{Outcome, Report, XlsxConstruct};
use crate::styles::{self, Palette, Theme};
use crate::xml::{self, Element, XmlError};
use crate::{Imported, MAX_CELLS, XlsxError};

/// The first bytes of an OLE compound file: a legacy `.xls`, or an Excel
/// file under a password.
const COMPOUND_FILE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// The days between the two date systems' epochs: a 1904 serial plus this
/// count is the same day's 1900 serial.
const DAYS_1904_TO_1900: f64 = 1462.0;

/// Read the workbook `bytes` hold.
pub(crate) fn read(bytes: &Payload<FileBytes>) -> Result<Imported, XlsxError> {
    if bytes.as_bytes().starts_with(&COMPOUND_FILE) {
        return Err(XlsxError::CompoundFile);
    }
    let mut package = Package::open(bytes.as_bytes()).map_err(XlsxError::Package)?;
    let main = package
        .related(&opc::root(), RelKind::OFFICE_DOCUMENT)
        .map_err(XlsxError::Package)?
        .ok_or(XlsxError::NotAWorkbook)?;
    let root = tree(&mut package, &main)?.ok_or(XlsxError::NotAWorkbook)?;
    if root.name() != "workbook" {
        return Err(XlsxError::NotAWorkbook);
    }
    let rels = package.relationships(&main).map_err(XlsxError::Package)?;
    let mut report = Report::default();

    let theme = match related(&rels, "theme") {
        Some(name) => tree(&mut package, &name)?
            .map(|t| Theme::read(&t))
            .unwrap_or_default(),
        None => Theme::default(),
    };
    let styles_root = match related(&rels, "styles") {
        Some(name) => tree(&mut package, &name)?,
        None => None,
    };
    let palette = Palette::new(theme, styles_root.as_ref());
    let by_xf = styles::read(styles_root.as_ref(), &palette, &mut report).by_xf;
    let (styles, xf_to_style) = dedupe_styles(by_xf);

    let strings = match related(&rels, "sharedStrings") {
        Some(name) => match part(&mut package, &name)? {
            Some(data) => read_shared_strings(data.as_bytes(), &mut report)
                .map_err(|e| malformed(&name, e))?,
            None => Vec::new(),
        },
        None => Vec::new(),
    };

    read_workbook_extras(&root, &rels, &mut report);
    let on_1904 = root
        .child("workbookPr")
        .and_then(|p| p.attr("date1904"))
        .is_some_and(|v| v == "1" || v == "true");
    let dates = if on_1904 {
        DateSystem::From1904
    } else {
        DateSystem::From1900
    };

    let context = SheetContext {
        strings: &strings,
        xf_to_style: &xf_to_style,
        styles: &styles,
        dates,
    };
    let mut sheets = Vec::new();
    let mut cells_read: Count<Cells> = Count::ZERO;
    let listed = root
        .child("sheets")
        .map(|s| s.children("sheet").collect::<Vec<_>>())
        .unwrap_or_default();
    for (n, entry) in listed.into_iter().enumerate() {
        let Some(rel) = entry
            .attr("id")
            .and_then(|id| rels.iter().find(|r| r.id.as_str() == id))
        else {
            continue;
        };
        let RelTarget::Part(sheet_part) = &rel.target else {
            continue;
        };
        if is_kind(rel, "chartsheet") {
            report.record(XlsxConstruct::Chartsheet, Outcome::Dropped);
            continue;
        }
        if !is_kind(rel, "worksheet") {
            report.record(XlsxConstruct::Macro, Outcome::Dropped);
            continue;
        }
        let name = entry
            .attr("name")
            .and_then(|n| Validated::<SheetName>::parse(n.to_owned()).ok())
            .or_else(|| Validated::<SheetName>::parse(format!("Sheet{}", n + 1)).ok())
            .ok_or(XlsxError::NotAWorkbook)?;
        let Some(data) = part(&mut package, sheet_part)? else {
            return Err(XlsxError::MissingPart(sheet_part.clone()));
        };
        let mut sheet = Sheet::named(name);
        sheet.visibility = match entry.attr("state") {
            Some("hidden") => {
                report.record(XlsxConstruct::HiddenSheet, Outcome::Carried);
                Visibility::Hidden
            }
            Some("veryHidden") => {
                report.record(XlsxConstruct::VeryHiddenSheet, Outcome::Approximated);
                Visibility::Hidden
            }
            _ => Visibility::Visible,
        };
        let room = Count::new(MAX_CELLS.get().saturating_sub(cells_read.get()));
        read_sheet(data.as_bytes(), &context, room, &mut sheet, &mut report).map_err(
            |e| match e {
                SheetError::Xml(e) => malformed(sheet_part, e),
                SheetError::TooManyCells => XlsxError::TooManyCells(MAX_CELLS),
            },
        )?;
        cells_read = cells_read.plus(Count::of(sheet.cells.len()));
        read_sheet_parts(&mut package, sheet_part, &mut report)?;
        sheets.push(sheet);
    }
    if sheets.is_empty() {
        return Err(XlsxError::NoSheet);
    }
    if dates == DateSystem::From1904 {
        report.record(XlsxConstruct::DateSystem, Outcome::Approximated);
    }
    report.record_count(
        XlsxConstruct::Sheet,
        Outcome::Carried,
        Count::of(sheets.len()),
    );
    report.record_count(
        XlsxConstruct::Cell,
        Outcome::Carried,
        Count::of(sheets.iter().map(|s| s.cells.len()).sum()),
    );
    report.record_count(
        XlsxConstruct::Style,
        Outcome::Carried,
        Count::of(styles.len().saturating_sub(1)),
    );
    Ok(Imported {
        workbook: Workbook { sheets, styles },
        report,
    })
}

fn malformed(part: &PartName, e: XmlError) -> XlsxError {
    XlsxError::Malformed(part.clone(), e)
}

/// The bytes of the part `name`, or `None` when the package lacks it.
fn part(
    package: &mut Package<'_>,
    name: &PartName,
) -> Result<Option<ambre_core::Payload<PartBytes>>, XlsxError> {
    package.part(name).map_err(XlsxError::Package)
}

/// The tree of the part `name`, or `None` when the package lacks it.
fn tree(package: &mut Package<'_>, name: &PartName) -> Result<Option<Element>, XlsxError> {
    part(package, name)?
        .map(|data| xml::parse_tree(data.as_bytes()).map_err(|e| malformed(name, e)))
        .transpose()
}

fn is_kind(rel: &Relationship, last_segment: &str) -> bool {
    rel.kind.as_str().rsplit('/').next() == Some(last_segment)
}

/// The part the first relationship of type `last_segment` points to.
fn related(rels: &[Relationship], last_segment: &str) -> Option<PartName> {
    rels.iter().find_map(|r| match &r.target {
        RelTarget::Part(name) if is_kind(r, last_segment) => Some(name.clone()),
        _ => None,
    })
}

/// The distinct styles of `by_xf`, the default first, and the style each
/// cell format index names.
fn dedupe_styles(by_xf: Vec<CellStyle>) -> (Vec<CellStyle>, Vec<StyleId>) {
    let mut styles: Vec<CellStyle> = Vec::with_capacity(by_xf.len());
    let ids = by_xf
        .into_iter()
        .map(|style| {
            let at = styles.iter().position(|s| *s == style).unwrap_or_else(|| {
                styles.push(style);
                styles.len() - 1
            });
            StyleId::at(Pos::from_usize(at))
        })
        .collect();
    if styles.is_empty() {
        styles.push(CellStyle::default());
    }
    (styles, ids)
}

/// Report what the workbook part holds and Trame drops.
fn read_workbook_extras(root: &Element, rels: &[Relationship], report: &mut Report) {
    let names = root
        .child("definedNames")
        .map(|d| {
            d.children("definedName")
                .filter(|n| {
                    !n.attr("name")
                        .is_some_and(|name| name.starts_with("_xlnm."))
                })
                .count()
        })
        .unwrap_or(0);
    report.record_count(
        XlsxConstruct::DefinedName,
        Outcome::Dropped,
        Count::of(names),
    );
    let locked = root.child("workbookProtection").is_some_and(|p| {
        [
            "lockStructure",
            "lockWindows",
            "workbookPassword",
            "workbookHashValue",
        ]
        .iter()
        .any(|a| p.attr(a).is_some_and(|v| v != "0" && v != "false"))
    });
    if locked {
        report.record(XlsxConstruct::Protection, Outcome::Dropped);
    }
    for rel in rels {
        if is_kind(rel, "vbaProject") {
            report.record(XlsxConstruct::Macro, Outcome::Dropped);
        } else if is_kind(rel, "externalLink") {
            report.record(XlsxConstruct::ExternalLink, Outcome::Dropped);
        }
    }
}

/// The strings of the shared-string part. A string item whose runs carry
/// formatting is read as its text, and reported.
fn read_shared_strings(data: &[u8], report: &mut Report) -> Result<Vec<String>, XmlError> {
    let mut reader = Reader::from_str(xml::utf8(data)?);
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut state = TextState::default();
    let mut rich = false;
    loop {
        match reader.read_event()? {
            Event::Start(e) => match e.local_name().as_ref() {
                "si" => {
                    current.clear();
                    rich = false;
                }
                "rPr" => rich = true,
                name => state.open(name),
            },
            Event::Empty(e) if e.local_name().as_ref() == "si" => strings.push(String::new()),
            Event::Empty(e) if e.local_name().as_ref() == "rPr" => rich = true,
            Event::End(e) => match e.local_name().as_ref() {
                "si" => {
                    if rich {
                        report.record(XlsxConstruct::RichText, Outcome::Dropped);
                    }
                    strings.push(xml::decode_ooxml_escapes(&current).into_owned());
                }
                name => state.close(name),
            },
            Event::Text(t) if state.is_text() => {
                current.push_str(&t.xml10_content());
            }
            Event::CData(t) if state.is_text() => {
                current.push_str(&t.xml10_content());
            }
            Event::GeneralRef(r) if state.is_text() => xml::push_reference(&mut current, &r)?,
            Event::Eof => return Ok(strings),
            _ => {}
        }
    }
}

/// Where a reader stands in a string item: inside a `t` element that is the
/// text, or inside a phonetic run whose `t` is not.
#[derive(Debug, Default)]
struct TextState {
    in_t: usize,
    in_phonetic: usize,
}

impl TextState {
    fn open(&mut self, name: &str) {
        match name {
            "t" => self.in_t += 1,
            "rPh" => self.in_phonetic += 1,
            _ => {}
        }
    }

    fn close(&mut self, name: &str) {
        match name {
            "t" => self.in_t = self.in_t.saturating_sub(1),
            "rPh" => self.in_phonetic = self.in_phonetic.saturating_sub(1),
            _ => {}
        }
    }

    fn is_text(&self) -> bool {
        self.in_t > 0 && self.in_phonetic == 0
    }
}

/// What every sheet of one workbook reads against.
struct SheetContext<'a> {
    strings: &'a [String],
    xf_to_style: &'a [StyleId],
    styles: &'a [CellStyle],
    dates: DateSystem,
}

/// The epoch a workbook counts its date serials from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateSystem {
    From1900,
    /// Macintosh Excel's epoch, four years and a day later.
    From1904,
}

/// The cell being read: its attributes, then its children.
#[derive(Debug, Default)]
struct PendingCell {
    at: Option<CellRef>,
    style: StyleId,
    kind: CellKind,
    value: String,
    formula: Option<PendingFormula>,
    inline: String,
}

/// The `t` attribute of a cell: how its value text reads.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum CellKind {
    #[default]
    Number,
    SharedString,
    FormulaString,
    InlineString,
    Boolean,
    Error,
    Date,
}

impl CellKind {
    fn of(t: Option<&str>) -> Self {
        match t {
            Some("s") => Self::SharedString,
            Some("str") => Self::FormulaString,
            Some("inlineStr") => Self::InlineString,
            Some("b") => Self::Boolean,
            Some("e") => Self::Error,
            Some("d") => Self::Date,
            _ => Self::Number,
        }
    }
}

/// The `f` element of a cell.
#[derive(Debug, Default)]
struct PendingFormula {
    kind: FormulaKind,
    text: String,
    range: Option<CellRange>,
    shared_index: Option<u32>,
}

/// The `t` attribute of a formula.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum FormulaKind {
    #[default]
    Normal,
    Shared,
    Array,
    DataTable,
}

/// Which text of a cell the reader stands in.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Place {
    #[default]
    Elsewhere,
    /// The `v` element: the cached value.
    Value,
    /// The `f` element: the formula text.
    Formula,
    /// The `is` element: an inline string.
    Inline,
}

/// Where the streaming reader of a sheet stands.
#[derive(Debug, Default)]
struct SheetCursor {
    row: Option<Pos<Row>>,
    next_col: Pos<Col>,
    place: Place,
    inline: TextState,
}

/// A cell whose shared formula names a master the reader has not met yet.
struct Deferred {
    cell: usize,
    index: u32,
}

/// Why a worksheet part did not read.
enum SheetError {
    Xml(XmlError),
    /// The sheet holds more cells than the room the workbook left it.
    TooManyCells,
}

impl From<XmlError> for SheetError {
    fn from(e: XmlError) -> Self {
        Self::Xml(e)
    }
}

impl From<quick_xml::Error> for SheetError {
    fn from(e: quick_xml::Error) -> Self {
        Self::Xml(e.into())
    }
}

/// Read the worksheet part `data` into `sheet`, refusing it past `room`
/// cells before the next one is read.
fn read_sheet(
    data: &[u8],
    context: &SheetContext<'_>,
    room: Count<Cells>,
    sheet: &mut Sheet,
    report: &mut Report,
) -> Result<(), SheetError> {
    let mut reader = Reader::from_str(xml::utf8(data)?);
    let mut cursor = SheetCursor::default();
    let mut cell = PendingCell::default();
    let mut masters: BTreeMap<u32, (String, CellRef)> = BTreeMap::new();
    let mut deferred: Vec<Deferred> = Vec::new();
    loop {
        if Count::<Cells>::of(sheet.cells.len()) > room {
            return Err(SheetError::TooManyCells);
        }
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                match e.local_name().as_ref() {
                    "row" => start_row(e, &mut cursor, sheet, report)?,
                    "c" => {
                        cell = start_cell(e, &mut cursor, context)?;
                        if empty {
                            finish_cell(
                                &mut cell,
                                context,
                                sheet,
                                &mut masters,
                                &mut deferred,
                                report,
                            );
                        }
                    }
                    "v" if !empty => cursor.place = Place::Value,
                    "f" => {
                        cell.formula = Some(start_formula(e)?);
                        if !empty {
                            cursor.place = Place::Formula;
                        }
                    }
                    "is" if !empty => cursor.place = Place::Inline,
                    "col" => read_col(e, sheet, report)?,
                    "mergeCell" => {
                        let range = xml::attr(e, "ref")?.as_deref().and_then(CellRange::parse);
                        if let Some(range) = range.filter(|r| !r.is_single()) {
                            sheet.merges.push(range);
                            report.record(XlsxConstruct::Merge, Outcome::Carried);
                        }
                    }
                    "pane" if xml::attr(e, "state")?.is_some_and(|s| s.starts_with("frozen")) => {
                        report.record(XlsxConstruct::FrozenPane, Outcome::Dropped);
                    }
                    "conditionalFormatting" => {
                        report.record(XlsxConstruct::ConditionalFormat, Outcome::Dropped);
                    }
                    "dataValidation" => {
                        report.record(XlsxConstruct::DataValidation, Outcome::Dropped)
                    }
                    "hyperlink" => report.record(XlsxConstruct::Hyperlink, Outcome::Dropped),
                    "sheetProtection"
                        if xml::attr(e, "sheet")?.is_some_and(|v| v == "1" || v == "true") =>
                    {
                        report.record(XlsxConstruct::Protection, Outcome::Dropped);
                    }
                    "autoFilter" => report.record(XlsxConstruct::AutoFilter, Outcome::Dropped),
                    "picture" => report.record(XlsxConstruct::Picture, Outcome::Dropped),
                    name if cursor.place == Place::Inline => {
                        if name == "rPr" {
                            report.record(XlsxConstruct::RichText, Outcome::Dropped);
                        }
                        if !empty {
                            cursor.inline.open(name);
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref e) => match e.local_name().as_ref() {
                "c" => finish_cell(
                    &mut cell,
                    context,
                    sheet,
                    &mut masters,
                    &mut deferred,
                    report,
                ),
                "v" | "f" | "is" => cursor.place = Place::Elsewhere,
                name if cursor.place == Place::Inline => cursor.inline.close(name),
                _ => {}
            },
            Event::Text(ref t) => {
                let text = t.xml10_content();
                push_cell_text(&cursor, &mut cell, &text);
            }
            Event::CData(ref t) => {
                let text = t.xml10_content();
                push_cell_text(&cursor, &mut cell, &text);
            }
            Event::GeneralRef(ref r) => {
                let mut text = String::new();
                xml::push_reference(&mut text, r)?;
                push_cell_text(&cursor, &mut cell, &text);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    for d in deferred {
        let Some((master, origin)) = masters.get(&d.index) else {
            continue;
        };
        if let Some(target) = sheet.cells.get_mut(d.cell) {
            let text = shifted(master, *origin, target.at);
            set_formula(target, &text, None, report);
            report.record(XlsxConstruct::SharedFormula, Outcome::Carried);
        }
    }
    sheet.cells.sort_by_key(|c| c.at);
    sheet.cells.dedup_by_key(|c| c.at);
    sheet.merges.sort();
    sheet.column_widths.sort_by_key(|c| c.first);
    sheet.row_heights.sort_by_key(|r| r.row);
    Ok(())
}

fn push_cell_text(cursor: &SheetCursor, cell: &mut PendingCell, text: &str) {
    match cursor.place {
        Place::Value => cell.value.push_str(text),
        Place::Formula => {
            if let Some(f) = cell.formula.as_mut() {
                f.text.push_str(text);
            }
        }
        Place::Inline if cursor.inline.is_text() => cell.inline.push_str(text),
        Place::Inline | Place::Elsewhere => {}
    }
}

fn start_row(
    e: &BytesStart<'_>,
    cursor: &mut SheetCursor,
    sheet: &mut Sheet,
    report: &mut Report,
) -> Result<(), XmlError> {
    let row = match xml::attr(e, "r")?
        .as_deref()
        .and_then(crate::cellref::row_from_number)
    {
        Some(row) => Some(row),
        None => match cursor.row {
            Some(prev) if prev < LAST_ROW => Some(prev.next()),
            Some(_) => None,
            None => Some(Pos::new(0)),
        },
    };
    cursor.row = row;
    cursor.next_col = Pos::new(0);
    let Some(row) = row else {
        return Ok(());
    };
    if xml::attr(e, "hidden")?.is_some_and(|h| h == "1" || h == "true") {
        report.record(XlsxConstruct::HiddenLine, Outcome::Dropped);
    }
    let custom = xml::attr(e, "customHeight")?.is_some_and(|h| h == "1" || h == "true");
    let height = xml::attr(e, "ht")?.as_deref().and_then(row_height_px);
    if let Some(height) = height.filter(|_| custom) {
        sheet.row_heights.push(RowHeight { row, height });
        report.record(XlsxConstruct::RowHeight, Outcome::Carried);
    }
    Ok(())
}

fn start_cell(
    e: &BytesStart<'_>,
    cursor: &mut SheetCursor,
    context: &SheetContext<'_>,
) -> Result<PendingCell, XmlError> {
    let at = match xml::attr(e, "r")?.as_deref().and_then(CellRef::parse) {
        Some(at) => Some(at),
        None => cursor
            .row
            .and_then(|row| CellRef::new(row, cursor.next_col)),
    };
    if let Some(at) = at {
        cursor.row = Some(at.row);
        cursor.next_col = if at.col < LAST_COL {
            at.col.next()
        } else {
            at.col
        };
    }
    let style = xml::attr(e, "s")?
        .and_then(|s| s.parse::<usize>().ok())
        .and_then(|s| context.xf_to_style.get(s).copied())
        .unwrap_or_default();
    Ok(PendingCell {
        at,
        style,
        kind: CellKind::of(xml::attr(e, "t")?.as_deref()),
        ..PendingCell::default()
    })
}

fn start_formula(e: &BytesStart<'_>) -> Result<PendingFormula, XmlError> {
    Ok(PendingFormula {
        kind: match xml::attr(e, "t")?.as_deref() {
            Some("shared") => FormulaKind::Shared,
            Some("array") => FormulaKind::Array,
            Some("dataTable") => FormulaKind::DataTable,
            _ => FormulaKind::Normal,
        },
        text: String::new(),
        range: xml::attr(e, "ref")?.as_deref().and_then(CellRange::parse),
        shared_index: xml::attr(e, "si")?.and_then(|s| s.parse().ok()),
    })
}

/// The cached value of a pending cell.
fn value_of(cell: &PendingCell, context: &SheetContext<'_>) -> Value {
    let text = cell.value.as_str();
    match cell.kind {
        CellKind::SharedString => text
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|i| context.strings.get(i))
            .map_or(Value::Empty, |s| Value::Text(s.clone())),
        CellKind::FormulaString => Value::Text(xml::decode_ooxml_escapes(text).into_owned()),
        CellKind::InlineString => {
            let source = if cell.inline.is_empty() {
                text
            } else {
                cell.inline.as_str()
            };
            Value::Text(xml::decode_ooxml_escapes(source).into_owned())
        }
        CellKind::Boolean => match text.trim() {
            "1" | "true" | "TRUE" => Value::Boolean(true),
            "0" | "false" | "FALSE" => Value::Boolean(false),
            _ => Value::Empty,
        },
        CellKind::Error => ErrorValue::from_code(text.trim()).map_or(Value::Empty, Value::Error),
        CellKind::Date => iso_to_serial(text.trim()).map_or(Value::Empty, Value::Number),
        CellKind::Number => match text.trim() {
            "" => Value::Empty,
            t => t
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .map_or(Value::Empty, Value::Number),
        },
    }
}

fn finish_cell(
    pending: &mut PendingCell,
    context: &SheetContext<'_>,
    sheet: &mut Sheet,
    masters: &mut BTreeMap<u32, (String, CellRef)>,
    deferred: &mut Vec<Deferred>,
    report: &mut Report,
) {
    let cell = std::mem::take(pending);
    let Some(at) = cell.at else {
        return;
    };
    let mut cached = value_of(&cell, context);
    if let (DateSystem::From1904, Value::Number(n)) = (context.dates, &cached) {
        let format = context
            .styles
            .get(cell.style.slot().to_usize())
            .map(|s| &s.format);
        if format.is_some_and(styles::is_calendar_date) {
            cached = Value::Number(n + DAYS_1904_TO_1900);
        }
    }
    let mut out = Cell {
        at,
        content: Content::Value(cached.clone()),
        style: cell.style,
    };
    let mut defer = None;
    if let Some(f) = cell.formula {
        match f.kind {
            FormulaKind::DataTable => {
                report.record(XlsxConstruct::ForeignFormula, Outcome::Dropped);
            }
            FormulaKind::Array if !f.text.is_empty() => {
                set_formula(&mut out, &f.text, f.range, report);
            }
            FormulaKind::Shared => match (f.shared_index, f.text.is_empty()) {
                (Some(index), false) => {
                    masters.insert(index, (f.text.clone(), at));
                    set_formula(&mut out, &f.text, None, report);
                }
                (Some(index), true) => match masters.get(&index) {
                    Some((master, origin)) => {
                        let text = shifted(master, *origin, at);
                        set_formula(&mut out, &text, None, report);
                        report.record(XlsxConstruct::SharedFormula, Outcome::Carried);
                    }
                    None => defer = Some(index),
                },
                _ => {}
            },
            _ if !f.text.is_empty() => set_formula(&mut out, &f.text, None, report),
            _ => {}
        }
    }
    let keep = !matches!(out.content, Content::Value(Value::Empty))
        || out.style != StyleId::DEFAULT
        || defer.is_some();
    if keep {
        if let Some(index) = defer {
            deferred.push(Deferred {
                cell: sheet.cells.len(),
                index,
            });
        }
        sheet.cells.push(out);
    }
}

/// The text of `master`, stored at `origin`, as the cell `at` computes it.
fn shifted(master: &str, origin: CellRef, at: CellRef) -> String {
    let rows = Delta::new(i64::from(at.row.get()) - i64::from(origin.row.get()));
    let cols = Delta::new(i64::from(at.col.get()) - i64::from(origin.col.get()));
    formula::shift(master, rows, cols)
}

/// Give `cell` the formula `excel`, its cached value kept.
fn set_formula(cell: &mut Cell, excel: &str, array: Option<CellRange>, report: &mut Report) {
    let cached = match &cell.content {
        Content::Value(v) => v.clone(),
        Content::Formula { cached, .. } => cached.clone(),
    };
    if let Ok(formula) = Formula::from_excel(excel, array, report) {
        cell.content = Content::Formula { formula, cached };
    }
}

fn read_col(e: &BytesStart<'_>, sheet: &mut Sheet, report: &mut Report) -> Result<(), XmlError> {
    let bound = |v: Option<String>| -> Option<Pos<Col>> {
        let n: u32 = v?.parse().ok()?;
        let pos = Pos::new(n.checked_sub(1)?);
        (pos <= LAST_COL).then_some(pos)
    };
    let (Some(first), Some(last)) = (bound(xml::attr(e, "min")?), bound(xml::attr(e, "max")?))
    else {
        return Ok(());
    };
    if xml::attr(e, "hidden")?.is_some_and(|h| h == "1" || h == "true") {
        report.record(XlsxConstruct::HiddenLine, Outcome::Dropped);
    }
    let custom = xml::attr(e, "customWidth")?.is_none_or(|c| c == "1" || c == "true");
    let width = xml::attr(e, "width")?.as_deref().and_then(column_width_px);
    if let Some(width) = width.filter(|_| custom && first <= last) {
        sheet.column_widths.push(ColumnWidth { first, last, width });
        report.record(XlsxConstruct::ColumnWidth, Outcome::Carried);
    }
    Ok(())
}

/// Report what a sheet's related parts hold: drawings, comments, tables and
/// pivot tables, all of which Trame drops.
fn read_sheet_parts(
    package: &mut Package<'_>,
    sheet: &PartName,
    report: &mut Report,
) -> Result<(), XlsxError> {
    for rel in package.relationships(sheet).map_err(XlsxError::Package)? {
        let RelTarget::Part(part) = &rel.target else {
            continue;
        };
        if is_kind(&rel, "drawing") {
            if let Some(root) = tree(package, part)? {
                count_drawing(&root, report);
            }
        } else if is_kind(&rel, "comments") {
            let count = tree(package, part)?
                .and_then(|root| {
                    root.child("commentList")
                        .map(|l| l.children("comment").count())
                })
                .unwrap_or(0);
            report.record_count(XlsxConstruct::Comment, Outcome::Dropped, Count::of(count));
        } else if is_kind(&rel, "table") {
            report.record(XlsxConstruct::Table, Outcome::Dropped);
        } else if is_kind(&rel, "pivotTable") {
            report.record(XlsxConstruct::PivotTable, Outcome::Dropped);
        }
    }
    Ok(())
}

/// Count the charts, pictures and shapes a drawing part anchors.
fn count_drawing(root: &Element, report: &mut Report) {
    for anchor in root.all_children() {
        for item in anchor.all_children() {
            let construct = match item.name() {
                "graphicFrame" => Some(XlsxConstruct::Chart),
                "pic" => Some(XlsxConstruct::Picture),
                "sp" | "cxnSp" | "grpSp" => Some(XlsxConstruct::Shape),
                _ => None,
            };
            if let Some(construct) = construct {
                report.record(construct, Outcome::Dropped);
            }
        }
    }
}

/// The screen width of a column whose `width` attribute reads `attr`. The
/// format counts in widths of the digit of the default font, 7 pixels for
/// Calibri 11, with a padding (ECMA-376 part 1, 18.3.1.13).
pub(crate) fn column_width_px(attr: &str) -> Option<Px<Doc>> {
    let digits = attr
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|w| w.is_finite() && *w >= 0.0)?;
    Some(Px::new((((256.0 * digits + 18.0) / 256.0) * 7.0).trunc()))
}

/// The `width` attribute of a column `px` wide, the inverse of
/// [`column_width_px`] on whole pixels.
pub(crate) fn column_width_attr(px: Px<Doc>) -> String {
    ((px.get().max(0.0) / 7.0 * 256.0).round() / 256.0).to_string()
}

/// The screen height of a row whose `ht` attribute reads `attr`, in points,
/// at 96 dots per inch.
pub(crate) fn row_height_px(attr: &str) -> Option<Px<Doc>> {
    let points = attr
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|p| p.is_finite() && *p >= 0.0)?;
    Some(Px::new(points * 4.0 / 3.0))
}

/// The `ht` attribute of a row `px` high.
pub(crate) fn row_height_attr(px: Px<Doc>) -> String {
    (px.get().max(0.0) * 0.75).to_string()
}

/// The 1900 serial of an ISO 8601 date and time, such as
/// `2024-03-01T12:00:00`, as a `t="d"` cell stores it.
fn iso_to_serial(text: &str) -> Option<f64> {
    let (date, time) = text.split_once('T').unwrap_or((text, ""));
    let mut parts = date.splitn(3, '-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from 1970-01-01, by the civil-from-days inverse.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let unix_days = era * 146_097 + doe - 719_468;
    // 1970-01-01 is serial 25569; Excel counts the phantom 1900-02-29.
    let mut serial = unix_days as f64 + 25_569.0;
    if serial < 61.0 {
        serial -= 1.0;
    }
    let mut seconds = 0.0;
    let time = time.trim_end_matches('Z');
    if !time.is_empty() {
        let mut hms = time.splitn(3, ':');
        let h: f64 = hms.next()?.parse().ok()?;
        let m: f64 = hms.next().unwrap_or("0").parse().ok()?;
        let s: f64 = hms.next().unwrap_or("0").parse().ok()?;
        seconds = h * 3600.0 + m * 60.0 + s;
    }
    Some(serial + seconds / 86_400.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sheet_past_its_room_is_refused_before_the_next_cell() {
        let xml = b"<worksheet><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c>\
            <c r=\"B1\"><v>2</v></c><c r=\"C1\"><v>3</v></c></row></sheetData></worksheet>";
        let context = SheetContext {
            strings: &[],
            xf_to_style: &[],
            styles: &[],
            dates: DateSystem::From1900,
        };
        let name = Validated::<crate::model::SheetName>::parse("S".to_owned());
        let Ok(name) = name else {
            panic!("a sheet name");
        };
        let mut sheet = Sheet::named(name);
        let mut report = Report::default();
        let read = read_sheet(xml, &context, Count::new(2), &mut sheet, &mut report);
        assert!(matches!(read, Err(SheetError::TooManyCells)));
        let mut sheet = Sheet::named(sheet.name.clone());
        let read = read_sheet(xml, &context, Count::new(3), &mut sheet, &mut report);
        assert!(read.is_ok());
        assert_eq!(sheet.cells.len(), 3);
    }

    #[test]
    fn an_iso_date_reads_as_its_1900_serial() {
        assert_eq!(iso_to_serial("1970-01-01"), Some(25_569.0));
        assert_eq!(iso_to_serial("2024-03-01T12:00:00"), Some(45_352.5));
        assert_eq!(iso_to_serial("1900-01-01"), Some(1.0));
        assert_eq!(iso_to_serial("2024-13-01"), None);
    }

    #[test]
    fn a_whole_pixel_width_survives_the_round_trip_through_digit_widths() {
        for px in [0.0f32, 5.0, 64.0, 100.0, 255.0, 1000.0] {
            let back = column_width_px(&column_width_attr(Px::new(px)));
            assert_eq!(back.map(Px::get), Some(px));
        }
        // The default Excel column, 8.43 digits of text, stores 9.140625
        // with its padding and draws 64 pixels wide.
        assert_eq!(column_width_px("9.140625").map(Px::get), Some(64.0));
        assert_eq!(column_width_px("-1"), None);
    }
}
