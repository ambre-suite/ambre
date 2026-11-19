//! The reader: a package's bytes into a [`Workbook`] and its [`Report`].
//!
//! The small parts -- the workbook, the styles, the theme, the relationships
//! -- read as a tree. The worksheets and the shared strings stream, so a large
//! sheet costs its cells and never a tree of its XML.

use std::collections::BTreeMap;

use ambre_client_core::FileBytes;
use ambre_core::index::{Delta, Len, Pos};
use ambre_core::{
    Cells, Count, Doc, FracDay, Instant, Octets, Payload, Px, SheetDateTime, TimeSpan, Validated,
};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::cellref::{CellRange, CellRef, Col, LAST_COL, LAST_ROW, Row};
use crate::formula::{self, Formula};
use crate::model::{
    Cell, CellStyle, ColumnWidth, Content, ErrorValue, FrozenPane, RowHeight, Sheet, SheetName,
    StyleId, Value, Visibility, Workbook,
};
use crate::opc::{self, Package, PartBytes, PartName, RelKind, RelTarget, Relationship};
use crate::report::{Outcome, Report, XlsxConstruct};
use crate::styles::{self, Palette, Theme};
use crate::xml::{self, Element, XmlError};
use crate::{Imported, MAX_CELLS, MAX_SHEETS, MAX_TEXT_BYTES, XlsxError};

/// The first bytes of an OLE compound file: a legacy `.xls`, or an Excel
/// file under a password.
const COMPOUND_FILE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// The days between the two date systems' epochs: a 1904 serial plus this
/// count is the same day's 1900 serial.
const DAYS_1904_TO_1900: TimeSpan<FracDay> = TimeSpan::new(1462.0);

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
    let mut text_read: Count<Octets> = Count::ZERO;
    let listed = root
        .child("sheets")
        .map(|s| s.children("sheet").collect::<Vec<_>>())
        .unwrap_or_default();
    // A sheet is listed in a few bytes: the count is refused before any
    // sheet is read (`P51`).
    if Count::<crate::WorkbookSheets>::of(listed.len()) > MAX_SHEETS {
        return Err(XlsxError::TooManySheets(MAX_SHEETS));
    }
    // Each relationship by its id, the first of an id kept, so a long list
    // of sheets costs no scan of the relationships per sheet.
    let mut by_id: BTreeMap<&str, &Relationship> = BTreeMap::new();
    for rel in &rels {
        by_id.entry(rel.id.as_str()).or_insert(rel);
    }
    for (n, entry) in listed.into_iter().enumerate() {
        let Some(rel) = entry.attr("id").and_then(|id| by_id.get(id).copied()) else {
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
        let room = Room {
            cells: MAX_CELLS.minus(cells_read),
            text: MAX_TEXT_BYTES.minus(text_read),
        };
        read_sheet(data.as_bytes(), &context, room, &mut sheet, &mut report).map_err(
            |e| match e {
                SheetError::Xml(e) => malformed(sheet_part, e),
                SheetError::TooManyCells => XlsxError::TooManyCells(MAX_CELLS),
                SheetError::TooMuchText => XlsxError::TooMuchText(MAX_TEXT_BYTES),
            },
        )?;
        cells_read = cells_read.plus(Count::of(sheet.cells.len()));
        text_read = text_read.plus(sheet.cells.iter().map(Cell::text_bytes).sum());
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
///
/// A style finds its twin by its postcard bytes in a map, not by a scan of
/// the styles before it. So a part of many distinct formats costs no
/// quadratic walk (`W305`). Two styles equal under `PartialEq` write the
/// same bytes, save a zero and a negative zero, which stay two styles.
fn dedupe_styles(by_xf: Vec<CellStyle>) -> (Vec<CellStyle>, Vec<StyleId>) {
    let mut styles: Vec<CellStyle> = Vec::with_capacity(by_xf.len());
    let mut seen: std::collections::HashMap<Vec<u8>, usize> = std::collections::HashMap::new();
    let ids = by_xf
        .into_iter()
        .map(|style| {
            let key = postcard::to_allocvec(&style).ok();
            let held = key.as_ref().and_then(|key| seen.get(key).copied());
            let at = held.unwrap_or_else(|| {
                styles.push(style);
                let at = styles.len() - 1;
                if let Some(key) = key {
                    seen.insert(key, at);
                }
                at
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
/// formatting is read as its text, and reported. Each item is charged to a
/// tree budget as one node: an empty item costs four bytes of XML and a
/// string record (`W72`).
fn read_shared_strings(data: &[u8], report: &mut Report) -> Result<Vec<String>, XmlError> {
    let mut reader = Reader::from_str(xml::utf8(data)?);
    let mut budget = ambre_opc::TreeBudget::default();
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
            Event::Empty(e) if e.local_name().as_ref() == "si" => {
                budget.node()?;
                strings.push(String::new());
            }
            Event::Empty(e) if e.local_name().as_ref() == "rPr" => rich = true,
            Event::End(e) => match e.local_name().as_ref() {
                "si" => {
                    if rich {
                        report.record(XlsxConstruct::RichText, Outcome::Dropped);
                    }
                    budget.node()?;
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
            Event::DocType(_) => return Err(xml::document_type()),
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
    /// The cells of the sheet hold more text than the room left.
    TooMuchText,
}

/// What the workbook leaves a sheet still to read: cells, and bytes of
/// text in its values and formulas.
#[derive(Clone, Copy)]
struct Room {
    cells: Count<Cells>,
    text: Count<Octets>,
}

/// The text the cells of `sheet` past `counted` hold, added to `text`, and
/// `counted` moved to the end. Each cell is counted once, as it lands.
fn count_text(sheet: &Sheet, counted: &mut Count<Cells>, text: &mut Count<Octets>) {
    let from = usize::try_from(counted.get()).unwrap_or(usize::MAX);
    for cell in sheet.cells.iter().skip(from) {
        *text = text.plus(cell.text_bytes());
    }
    *counted = Count::of(sheet.cells.len());
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

/// Read the worksheet part `data` into `sheet`, refusing it past the cells
/// or the text of `room` before the next cell is read.
fn read_sheet(
    data: &[u8],
    context: &SheetContext<'_>,
    room: Room,
    sheet: &mut Sheet,
    report: &mut Report,
) -> Result<(), SheetError> {
    let mut reader = Reader::from_str(xml::utf8(data)?);
    let mut cursor = SheetCursor::default();
    let mut cell = PendingCell::default();
    let mut masters: BTreeMap<u32, (String, CellRef)> = BTreeMap::new();
    let mut deferred: Vec<Deferred> = Vec::new();
    let mut counted: Count<Cells> = Count::ZERO;
    let mut text: Count<Octets> = Count::ZERO;
    loop {
        if Count::<Cells>::of(sheet.cells.len()) > room.cells {
            return Err(SheetError::TooManyCells);
        }
        count_text(sheet, &mut counted, &mut text);
        if text > room.text {
            return Err(SheetError::TooMuchText);
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
                        read_frozen_pane(e, sheet, report)?;
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
            Event::DocType(_) => return Err(xml::document_type().into()),
            Event::Eof => break,
            _ => {}
        }
    }
    count_text(sheet, &mut counted, &mut text);
    if text > room.text {
        return Err(SheetError::TooMuchText);
    }
    for d in deferred {
        let Some((master, origin)) = masters.get(&d.index) else {
            continue;
        };
        if let Some(target) = sheet.cells.get_mut(d.cell) {
            let before = target.text_bytes();
            let formula = shifted(master, *origin, target.at);
            set_formula(target, &formula, None, report);
            report.record(XlsxConstruct::SharedFormula, Outcome::Carried);
            text = text.plus(target.text_bytes()).minus(before);
            if text > room.text {
                return Err(SheetError::TooMuchText);
            }
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
        CellKind::Date => {
            iso_to_serial(text.trim()).map_or(Value::Empty, |at| Value::Number(at.get()))
        }
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
            cached = Value::Number(n + DAYS_1904_TO_1900.get());
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
                    set_formula(&mut out, &f.text, None, report);
                    // A master past the formula cap reads as no formula.
                    // It is never stored, so no cell that names it lexes
                    // the whole text again (`W305`).
                    if matches!(out.content, Content::Formula { .. }) {
                        masters.insert(index, (f.text, at));
                    }
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
        Content::Formula { cached, .. } | Content::TrameFormula { cached, .. } => cached.clone(),
    };
    if let Ok(formula) = Formula::from_excel(excel, array, report) {
        cell.content = Content::Formula { formula, cached };
    }
}

/// Read a frozen pane: `ySplit` counts the rows frozen at the top and
/// `xSplit` the columns frozen at the left. A count that is no whole number,
/// or that leaves no line of the grid to scroll, freezes nothing on its axis.
/// A pane that freezes nothing is dropped from the report too.
fn read_frozen_pane(
    e: &BytesStart<'_>,
    sheet: &mut Sheet,
    report: &mut Report,
) -> Result<(), XmlError> {
    // Excel writes a split as a decimal number, often with a fraction of
    // zero: "2" and "2.0" freeze the same two lines.
    let split = |v: Option<String>| -> Option<u32> {
        let n: f64 = v?.trim().parse().ok()?;
        if n.fract() != 0.0 || n < 1.0 || n > f64::from(u32::MAX) {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the guard above holds a whole number inside the range of a u32"
        )]
        let whole = n as u32;
        Some(whole)
    };
    let rows = split(xml::attr(e, "ySplit")?)
        .map(Len::<Row>::new)
        .filter(|rows| *rows < Len::through(LAST_ROW))
        .unwrap_or_default();
    let cols = split(xml::attr(e, "xSplit")?)
        .map(Len::<Col>::new)
        .filter(|cols| *cols < Len::through(LAST_COL))
        .unwrap_or_default();
    sheet.frozen = FrozenPane { rows, cols };
    if !sheet.frozen.is_none() {
        report.record(XlsxConstruct::FrozenPane, Outcome::Carried);
    }
    Ok(())
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
pub(crate) fn iso_to_serial(text: &str) -> Option<Instant<SheetDateTime>> {
    let (date, time) = text.split_once('T').unwrap_or((text, ""));
    let mut parts = date.splitn(3, '-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from 1970-01-01, by the civil-from-days inverse. A year the file
    // spells near the top of the width overflows: it reads as no date, never
    // as a wrapped one (`W306`).
    let y = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let unix_days = era
        .checked_mul(146_097)?
        .checked_add(doe)?
        .checked_sub(719_468)?;
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
    Some(Instant::new(serial + seconds / 86_400.0))
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
        let room = |cells| Room {
            cells: Count::new(cells),
            text: MAX_TEXT_BYTES,
        };
        let read = read_sheet(xml, &context, room(2), &mut sheet, &mut report);
        assert!(matches!(read, Err(SheetError::TooManyCells)));
        let mut sheet = Sheet::named(sheet.name.clone());
        let read = read_sheet(xml, &context, room(3), &mut sheet, &mut report);
        assert!(read.is_ok());
        assert_eq!(sheet.cells.len(), 3);
    }

    #[test]
    fn an_iso_date_reads_as_its_1900_serial() {
        assert_eq!(
            iso_to_serial("1970-01-01").map(Instant::get),
            Some(25_569.0)
        );
        assert_eq!(
            iso_to_serial("2024-03-01T12:00:00").map(Instant::get),
            Some(45_352.5)
        );
        assert_eq!(iso_to_serial("1900-01-01").map(Instant::get), Some(1.0));
        assert_eq!(iso_to_serial("2024-13-01"), None);
        // `W306`: a year near the top of the width reads as no date, in a
        // debug build as in a release one.
        assert_eq!(iso_to_serial("9223372036854775807-07-01"), None);
        assert_eq!(iso_to_serial("-9223372036854775808-01-01"), None);
    }

    /// The room of a workbook that read nothing yet.
    const WHOLE: Room = Room {
        cells: MAX_CELLS,
        text: MAX_TEXT_BYTES,
    };

    fn sheet_context() -> SheetContext<'static> {
        SheetContext {
            strings: &[],
            xf_to_style: &[],
            styles: &[],
            dates: DateSystem::From1900,
        }
    }

    fn empty_sheet() -> Sheet {
        let Ok(name) = Validated::<crate::model::SheetName>::parse("S".to_owned()) else {
            panic!("a sheet name");
        };
        Sheet::named(name)
    }

    /// `P53`: one shared string named by many cells is copied into each.
    /// The sheet is refused once the copies pass the text room, before the
    /// next cell is read.
    #[test]
    fn a_shared_string_copied_past_the_text_room_is_refused() {
        let strings = vec!["x".repeat(1_000)];
        let context = SheetContext {
            strings: &strings,
            xf_to_style: &[],
            styles: &[],
            dates: DateSystem::From1900,
        };
        let rows: String = (1..=20)
            .map(|r| format!("<row r=\"{r}\"><c r=\"A{r}\" t=\"s\"><v>0</v></c></row>"))
            .collect();
        let xml = format!("<worksheet><sheetData>{rows}</sheetData></worksheet>");
        let mut report = Report::default();
        let mut sheet = empty_sheet();
        let tight = Room {
            cells: MAX_CELLS,
            text: Count::new(10_000),
        };
        let read = read_sheet(xml.as_bytes(), &context, tight, &mut sheet, &mut report);
        assert!(matches!(read, Err(SheetError::TooMuchText)));
        assert!(sheet.cells.len() <= 11);
        let mut sheet = empty_sheet();
        let read = read_sheet(xml.as_bytes(), &context, WHOLE, &mut sheet, &mut report);
        assert!(read.is_ok());
        assert_eq!(sheet.cells.len(), 20);
    }

    /// `W307`: a worksheet that declares a document type is refused, as the
    /// four other codecs refuse it.
    #[test]
    fn a_sheet_that_declares_a_document_type_is_refused() {
        let xml = b"<?xml version='1.0'?><!DOCTYPE w [<!ENTITY e 'x'>]>\
            <worksheet><sheetData/></worksheet>";
        let mut sheet = empty_sheet();
        let mut report = Report::default();
        let read = read_sheet(xml, &sheet_context(), WHOLE, &mut sheet, &mut report);
        assert!(matches!(read, Err(SheetError::Xml(_))));
    }

    /// `W307` and `W72`: a shared-string part that declares a document type
    /// is refused, and so is one of more empty items than the node bound.
    #[test]
    fn a_hostile_shared_string_part_is_refused() {
        let mut report = Report::default();
        let typed = b"<!DOCTYPE s [<!ENTITY e 'x'>]><sst><si><t>a</t></si></sst>";
        assert!(read_shared_strings(typed, &mut report).is_err());
        let past = usize::try_from(ambre_opc::MAX_TREE_NODES.get()).expect("fits") + 1;
        let wide = format!("<sst>{}</sst>", "<si/>".repeat(past));
        assert!(read_shared_strings(wide.as_bytes(), &mut report).is_err());
    }

    /// `W305`: a shared formula master past the formula cap is never
    /// stored, so a cell that names it lexes nothing and stays a value.
    #[test]
    fn a_shared_master_past_the_formula_cap_is_never_stored() {
        let master = "1+".repeat(5_000) + "1";
        let xml = format!(
            "<worksheet><sheetData><row r=\"1\">\
             <c r=\"A1\"><f t=\"shared\" si=\"0\" ref=\"A1:A2\">{master}</f><v>1</v></c>\
             <c r=\"A2\"><f t=\"shared\" si=\"0\"/><v>2</v></c>\
             </row></sheetData></worksheet>"
        );
        let mut sheet = empty_sheet();
        let mut report = Report::default();
        let read = read_sheet(
            xml.as_bytes(),
            &sheet_context(),
            WHOLE,
            &mut sheet,
            &mut report,
        );
        assert!(read.is_ok());
        assert!(
            sheet
                .cells
                .iter()
                .all(|cell| matches!(cell.content, Content::Value(_)))
        );
    }

    /// `W305`: many distinct styles and their twins dedupe by a lookup, each
    /// twin to the style it equals.
    #[test]
    fn many_styles_dedupe_to_their_first_twin() {
        let count = 20_000;
        let styles: Vec<CellStyle> = (0..count)
            .map(|at| CellStyle {
                fg: Some(ambre_core::Rgb::from_channels(
                    u8::try_from(at % 256).expect("fits"),
                    u8::try_from(at / 256 % 256).expect("fits"),
                    0,
                )),
                ..CellStyle::default()
            })
            .collect();
        let twice: Vec<CellStyle> = styles.iter().chain(styles.iter()).cloned().collect();
        let (distinct, ids) = dedupe_styles(twice);
        assert_eq!(distinct.len(), count);
        assert_eq!(ids.get(..count), ids.get(count..));
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
