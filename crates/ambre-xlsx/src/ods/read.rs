//! The reader: an OpenDocument spreadsheet into a [`Workbook`] and its
//! [`Report`], the same pair the xlsx reader answers.
//!
//! The styles part and the settings read as trees. The content part
//! streams. Its style declarations read as trees, and its tables cell by
//! cell. So a large sheet costs its cells and never a tree of its XML.

use std::collections::{HashMap, HashSet};

use ambre_core::index::Pos;
use ambre_core::{Count, Doc, FileBytes, Payload, Px, Validated};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::styles::{DEFAULT, Styles};
use super::{Node, formula, length_px, subtree, tree};
use crate::cellref::{CellRange, CellRef, LAST_COL, LAST_ROW};
use crate::formula::Formula;
use crate::model::{
    Cell, ColumnWidth, Content, ErrorValue, RowHeight, Sheet, SheetName, StyleId, Value,
    Visibility, Workbook,
};
use crate::opc::odf::OdfPackage;
use crate::opc::odf::{self, OdfType};
use crate::opc::{OpcError, PartName};
use crate::report::{Outcome, Report, XlsxConstruct};
use crate::xml::{self, XmlError};
use crate::{Imported, MAX_CELLS, XlsxError};

/// The width LibreOffice gives a column it was never asked to size, 0.889
/// inch. Its xlsx export writes no width for such a column, so neither does
/// this reader.
const LIBREOFFICE_COLUMN: Px<Doc> = Px::new(0.8889 * 96.0);

/// The most cells one run of repeated, styled, empty cells may cover. A
/// longer run styles a whole row or column, which Trame holds no style for.
const MAX_STYLED_RUN: u64 = 1024;

/// The elements of a drawing anchored in a sheet.
const SHAPES: &[&str] = &[
    "frame",
    "g",
    "custom-shape",
    "rect",
    "ellipse",
    "circle",
    "line",
    "polygon",
    "polyline",
    "regular-polygon",
    "path",
    "connector",
    "caption",
    "measure",
    "control",
];

/// Read the spreadsheet `bytes` hold.
pub(crate) fn read(bytes: &Payload<FileBytes>) -> Result<Imported, XlsxError> {
    let mut package = odf::open(bytes.as_bytes(), OdfType::SPREADSHEET).map_err(|e| match e {
        OpcError::CompoundFile => XlsxError::CompoundFile,
        OpcError::OtherDocument => XlsxError::NotAWorkbook,
        e => XlsxError::Package(e),
    })?;
    let mut report = Report::default();
    let styles_root = match named_part(&mut package, "styles.xml")? {
        Some((name, data)) => Some(tree(data.as_bytes()).map_err(|e| malformed(&name, e))?),
        None => None,
    };
    if let Some((name, data)) = named_part(&mut package, "settings.xml")? {
        let settings = tree(data.as_bytes()).map_err(|e| malformed(&name, e))?;
        record_frozen_panes(&settings, &mut report);
    }
    let scripted = package
        .part_names()
        .iter()
        .any(|n| n.as_str().starts_with("Basic/") || n.as_str().starts_with("Scripts/"));
    if scripted {
        report.record(XlsxConstruct::Macro, Outcome::Dropped);
    }
    let (name, content) = named_part(&mut package, "content.xml")?.ok_or_else(|| {
        PartName::new("content.xml").map_or(XlsxError::NotAWorkbook, XlsxError::MissingPart)
    })?;
    let sheets = read_content(content.as_bytes(), styles_root.as_ref(), &mut report).map_err(
        |e| match e {
            ContentError::Xml(e) => malformed(&name, e),
            ContentError::TooManyCells => XlsxError::TooManyCells(MAX_CELLS),
        },
    )?;
    let (sheets, styles) = sheets;
    if sheets.is_empty() {
        return Err(XlsxError::NoSheet);
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

/// The part `name` and its bytes, or `None` when the package lacks it.
fn named_part(
    package: &mut OdfPackage<'_>,
    name: &str,
) -> Result<Option<(PartName, Payload<crate::opc::PartBytes>)>, XlsxError> {
    let Some(part) = PartName::new(name) else {
        return Ok(None);
    };
    Ok(package
        .part(&part)
        .map_err(XlsxError::Package)?
        .map(|data| (part, data)))
}

/// Report each sheet the settings freeze a pane of.
fn record_frozen_panes(settings: &Node, report: &mut Report) {
    for node in settings.walk() {
        if node.name() != "config-item-map-named" || node.attr("name") != Some("Tables") {
            continue;
        }
        for table in node.children() {
            let frozen = table.children().iter().any(|item| {
                matches!(
                    item.attr("name"),
                    Some("HorizontalSplitMode" | "VerticalSplitMode")
                ) && item.text().trim() == "2"
            });
            if frozen {
                report.record(XlsxConstruct::FrozenPane, Outcome::Dropped);
            }
        }
    }
}

/// Why the content part did not read.
enum ContentError {
    Xml(XmlError),
    TooManyCells,
}

impl From<XmlError> for ContentError {
    fn from(e: XmlError) -> Self {
        Self::Xml(e)
    }
}

impl From<quick_xml::Error> for ContentError {
    fn from(e: quick_xml::Error) -> Self {
        Self::Xml(e.into())
    }
}

/// The layout the automatic styles give the tables: each column width, each
/// row height the user set, and each table style that hides its sheet.
#[derive(Default)]
struct Layout {
    column_px: HashMap<String, Px<Doc>>,
    row_px: HashMap<String, Px<Doc>>,
    hidden: HashSet<String>,
}

impl Layout {
    fn declare(&mut self, node: &Node) {
        let (Some(name), Some(family)) = (node.attr("name"), node.attr("family")) else {
            return;
        };
        match family {
            "table-column" => {
                let width = node
                    .child("table-column-properties")
                    .and_then(|p| p.attr("column-width"))
                    .and_then(length_px);
                if let Some(px) = width {
                    self.column_px.insert(name.to_owned(), px);
                }
            }
            "table-row" => {
                let props = node.child("table-row-properties");
                let custom = props.and_then(|p| p.attr("use-optimal-row-height")) == Some("false");
                let height = props.and_then(|p| p.attr("row-height")).and_then(length_px);
                if let Some(px) = height.filter(|_| custom) {
                    self.row_px.insert(name.to_owned(), px);
                }
            }
            "table" => {
                let hidden = node
                    .child("table-properties")
                    .and_then(|p| p.attr("display"))
                    == Some("false");
                if hidden {
                    self.hidden.insert(name.to_owned());
                }
            }
            _ => {}
        }
    }
}

/// One run of columns and the style their cells take.
struct ColumnRun {
    first: u64,
    last: u64,
    style: Option<String>,
}

/// One attribute of a start tag: its prefix, its local name and its value.
struct Attr {
    prefix: String,
    local: String,
    value: String,
}

fn attributes(e: &BytesStart<'_>) -> Result<Vec<Attr>, XmlError> {
    e.attributes()
        .map(|a| {
            let a = a?;
            let prefix = a
                .key
                .prefix()
                .map(|p| AsRef::<str>::as_ref(&p).to_owned())
                .unwrap_or_default();
            let local = AsRef::<str>::as_ref(&a.key.local_name()).to_owned();
            let value = a
                .normalized_value(quick_xml::XmlVersion::Explicit1_0)?
                .into_owned();
            Ok(Attr {
                prefix,
                local,
                value,
            })
        })
        .collect()
}

fn get<'a>(attrs: &'a [Attr], local: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| a.local == local && a.prefix != "calcext")
        .map(|a| a.value.as_str())
}

fn count_of(attrs: &[Attr], local: &str) -> u64 {
    get(attrs, local)
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1)
}

/// The cell being read: its attributes, then its text.
#[derive(Default)]
struct PendingCell {
    col: u64,
    repeat: u64,
    style: Option<String>,
    value_type: Option<String>,
    error: bool,
    value: Option<String>,
    date: Option<String>,
    time: Option<String>,
    boolean: Option<String>,
    string: Option<String>,
    formula: Option<String>,
    span: (u64, u64),
    matrix: (u64, u64),
    text: String,
    paragraphs: usize,
    /// Raw white space waiting to collapse into one space.
    pending_space: bool,
    rich: bool,
}

impl PendingCell {
    fn open(e: &BytesStart<'_>, col: u64) -> Result<Self, XmlError> {
        let attrs = attributes(e)?;
        let pair = |a: &str, b: &str| (count_of(&attrs, a), count_of(&attrs, b));
        Ok(Self {
            col,
            repeat: count_of(&attrs, "number-columns-repeated"),
            style: get(&attrs, "style-name").map(str::to_owned),
            value_type: get(&attrs, "value-type").map(str::to_owned),
            error: attrs
                .iter()
                .any(|a| a.prefix == "calcext" && a.local == "value-type" && a.value == "error"),
            value: get(&attrs, "value").map(str::to_owned),
            date: get(&attrs, "date-value").map(str::to_owned),
            time: get(&attrs, "time-value").map(str::to_owned),
            boolean: get(&attrs, "boolean-value").map(str::to_owned),
            string: get(&attrs, "string-value").map(str::to_owned),
            formula: get(&attrs, "formula").map(str::to_owned),
            span: pair("number-columns-spanned", "number-rows-spanned"),
            matrix: pair(
                "number-matrix-columns-spanned",
                "number-matrix-rows-spanned",
            ),
            ..Self::default()
        })
    }

    /// Note that the cell's text carries formatting runs, which the read
    /// drops and reports.
    fn mark_rich(&mut self) {
        self.rich = true;
    }

    fn start_paragraph(&mut self) {
        if self.paragraphs > 0 {
            self.text.push('\n');
        }
        self.paragraphs += 1;
        self.pending_space = false;
    }

    /// Append raw text. Each run of white space reads as one space. None
    /// reads at the start or the end of a paragraph (ODF 1.3 part 3, 6.1.2).
    fn push_raw(&mut self, raw: &str) {
        for c in raw.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r') {
                let at_start = self.text.is_empty() || self.text.ends_with('\n');
                if !at_start {
                    self.pending_space = true;
                }
            } else {
                self.flush_space();
                self.text.push(c);
            }
        }
    }

    /// Append text that keeps its every character: a spaces element, a tab,
    /// a character reference.
    fn push_kept(&mut self, kept: &str) {
        self.flush_space();
        self.text.push_str(kept);
    }

    fn flush_space(&mut self) {
        if self.pending_space {
            self.text.push(' ');
            self.pending_space = false;
        }
    }

    /// The value the cell holds.
    fn value(&self) -> Value {
        let number = |text: &Option<String>| {
            text.as_deref()
                .and_then(|t| t.trim().parse::<f64>().ok())
                .filter(|n| n.is_finite())
                .map_or(Value::Empty, Value::Number)
        };
        if self.error {
            return ErrorValue::from_code(self.text.trim()).map_or(Value::Empty, Value::Error);
        }
        match self.value_type.as_deref() {
            Some("float" | "percentage" | "currency") => number(&self.value),
            Some("date") => self
                .date
                .as_deref()
                .and_then(|d| crate::read::iso_to_serial(d.trim()))
                .map_or(Value::Empty, Value::Number),
            Some("time") => self
                .time
                .as_deref()
                .and_then(duration_days)
                .map_or(Value::Empty, Value::Number),
            Some("boolean") => match self.boolean.as_deref().map(str::trim) {
                Some("true") => Value::Boolean(true),
                Some("false") => Value::Boolean(false),
                _ => Value::Empty,
            },
            Some("string") => Value::Text(self.string.clone().unwrap_or_else(|| self.text.clone())),
            _ => Value::Empty,
        }
    }
}

/// The days an ISO 8601 duration such as `PT12H30M00S` spans.
fn duration_days(text: &str) -> Option<f64> {
    let text = text.trim();
    let (sign, text) = match text.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, text),
    };
    let text = text.strip_prefix('P')?;
    let (days_part, time_part) = text.split_once('T').unwrap_or((text, ""));
    let mut seconds = 0.0;
    if let Some(days) = days_part.strip_suffix('D') {
        seconds += days.parse::<f64>().ok()? * 86_400.0;
    } else if !days_part.is_empty() {
        return None;
    }
    let mut rest = time_part;
    for (unit, scale) in [('H', 3600.0), ('M', 60.0), ('S', 1.0)] {
        if let Some((number, tail)) = rest.split_once(unit) {
            seconds += number.parse::<f64>().ok()? * scale;
            rest = tail;
        }
    }
    rest.is_empty()
        .then_some(sign * seconds / 86_400.0)
        .filter(|d| d.is_finite())
}

/// One table being read.
struct Table {
    sheet: Sheet,
    columns: Vec<ColumnRun>,
    next_col: u64,
    row: u64,
    row_repeat: u64,
    row_style: Option<String>,
    row_cells: Vec<Cell>,
    col: u64,
}

/// The content of the drawing being read: a frame classified by its first
/// child, or a shape.
#[derive(Default)]
struct Drawing {
    depth: usize,
    frame: Option<Option<XlsxConstruct>>,
}

/// Read the content part: every table, and the styles their cells name.
fn read_content(
    data: &[u8],
    styles_root: Option<&Node>,
    report: &mut Report,
) -> Result<(Vec<Sheet>, Vec<crate::model::CellStyle>), ContentError> {
    let mut reader = Reader::from_str(xml::utf8(data)?);
    let mut declared: Vec<Node> = Vec::new();
    let mut layout = Layout::default();
    let mut styles: Option<Styles> = None;
    let mut sheets: Vec<Sheet> = Vec::new();
    let mut table: Option<Table> = None;
    let mut cell: Option<PendingCell> = None;
    // Open annotations and drawings inside the cell being read, whose text
    // is not the cell's.
    let mut hidden: usize = 0;
    let mut drawing = Drawing::default();
    let mut cells_read: u64 = 0;
    loop {
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let name = xml::local_name(e);
                if !empty && matches!(name.as_str(), "automatic-styles" | "font-face-decls") {
                    let node = subtree(&mut reader, e)?;
                    for n in node.walk() {
                        layout.declare(n);
                    }
                    declared.push(node);
                    continue;
                }
                if SHAPES.contains(&name.as_str()) || drawing.depth > 0 {
                    read_drawing(&name, empty, &mut drawing, report);
                    if cell.is_some() && !empty && SHAPES.contains(&name.as_str()) {
                        hidden += 1;
                    }
                    if SHAPES.contains(&name.as_str()) || drawing.depth > 0 {
                        continue;
                    }
                }
                if styles.is_none() && name == "body" {
                    let mut roots: Vec<&Node> = styles_root.into_iter().collect();
                    roots.extend(declared.iter());
                    styles = Some(Styles::new(&roots));
                }
                match name.as_str() {
                    "table" => {
                        let attrs = attributes(e)?;
                        let n = sheets.len() + 1;
                        let sheet_name = get(&attrs, "name")
                            .and_then(|n| Validated::<SheetName>::parse(n.to_owned()).ok())
                            .or_else(|| Validated::<SheetName>::parse(format!("Sheet{n}")).ok());
                        let Some(sheet_name) = sheet_name else {
                            continue;
                        };
                        let mut sheet = Sheet::named(sheet_name);
                        if get(&attrs, "style-name").is_some_and(|s| layout.hidden.contains(s)) {
                            sheet.visibility = Visibility::Hidden;
                            report.record(XlsxConstruct::HiddenSheet, Outcome::Carried);
                        }
                        if get(&attrs, "protected") == Some("true") {
                            report.record(XlsxConstruct::Protection, Outcome::Dropped);
                        }
                        let new = Table {
                            sheet,
                            columns: Vec::new(),
                            next_col: 0,
                            row: 0,
                            row_repeat: 1,
                            row_style: None,
                            row_cells: Vec::new(),
                            col: 0,
                        };
                        if empty {
                            sheets.push(finish_sheet(new));
                        } else {
                            table = Some(new);
                        }
                    }
                    "table-column" => {
                        if let Some(t) = table.as_mut() {
                            read_column(e, t, &layout, report)?;
                        }
                    }
                    "table-row" => {
                        if let Some(t) = table.as_mut() {
                            start_row(e, t, &layout, report)?;
                            if empty {
                                finish_row(t, &mut cells_read)?;
                            }
                        }
                    }
                    "table-cell" | "covered-table-cell" => {
                        if let (Some(t), Some(styles)) = (table.as_mut(), styles.as_mut()) {
                            let pending = PendingCell::open(e, t.col)?;
                            t.col = t.col.saturating_add(pending.repeat);
                            if empty {
                                finish_cell(pending, t, styles, report);
                            } else {
                                cell = Some(pending);
                                hidden = 0;
                            }
                        }
                    }
                    "annotation" => {
                        report.record(XlsxConstruct::Comment, Outcome::Dropped);
                        if !empty {
                            hidden += 1;
                        }
                    }
                    "p" | "h" if hidden == 0 => {
                        if let Some(c) = cell.as_mut() {
                            c.start_paragraph();
                        }
                    }
                    "s" if hidden == 0 => {
                        if let Some(c) = cell.as_mut() {
                            let n = attributes(e)?
                                .iter()
                                .find(|a| a.local == "c")
                                .and_then(|a| a.value.parse::<usize>().ok())
                                .unwrap_or(1)
                                .min(1024);
                            c.push_kept(&" ".repeat(n));
                        }
                    }
                    "tab" if hidden == 0 => {
                        if let Some(c) = cell.as_mut() {
                            c.push_kept("\t");
                        }
                    }
                    "line-break" if hidden == 0 => {
                        if let Some(c) = cell.as_mut() {
                            c.push_kept("\n");
                        }
                    }
                    "span" if hidden == 0 => {
                        if let Some(c) = cell.as_mut() {
                            c.mark_rich();
                        }
                    }
                    "a" if hidden == 0 && cell.is_some() => {
                        report.record(XlsxConstruct::Hyperlink, Outcome::Dropped);
                    }
                    "named-range" | "named-expression" => {
                        report.record(XlsxConstruct::DefinedName, Outcome::Dropped);
                    }
                    "database-range" => {
                        let filtered = attributes(e)?
                            .iter()
                            .any(|a| a.local == "display-filter-buttons" && a.value == "true");
                        if filtered {
                            report.record(XlsxConstruct::AutoFilter, Outcome::Dropped);
                        }
                    }
                    "data-pilot-table" => {
                        report.record(XlsxConstruct::PivotTable, Outcome::Dropped);
                    }
                    "content-validation" => {
                        report.record(XlsxConstruct::DataValidation, Outcome::Dropped);
                    }
                    "conditional-format" => {
                        report.record(XlsxConstruct::ConditionalFormat, Outcome::Dropped);
                    }
                    "table-source" => {
                        report.record(XlsxConstruct::ExternalLink, Outcome::Dropped);
                    }
                    "spreadsheet" => {
                        let locked = attributes(e)?
                            .iter()
                            .any(|a| a.local == "structure-protected" && a.value == "true");
                        if locked {
                            report.record(XlsxConstruct::Protection, Outcome::Dropped);
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref e) => {
                let name = AsRef::<str>::as_ref(&e.local_name()).to_owned();
                if drawing.depth > 0 && SHAPES.contains(&name.as_str()) {
                    end_drawing(&mut drawing, report);
                    if cell.is_some() {
                        hidden = hidden.saturating_sub(1);
                    }
                    continue;
                }
                match name.as_str() {
                    "annotation" => hidden = hidden.saturating_sub(1),
                    "table-cell" | "covered-table-cell" => {
                        if let (Some(t), Some(c), Some(s)) =
                            (table.as_mut(), cell.take(), styles.as_mut())
                        {
                            finish_cell(c, t, s, report);
                        }
                    }
                    "table-row" => {
                        if let Some(t) = table.as_mut() {
                            finish_row(t, &mut cells_read)?;
                        }
                    }
                    "table" => {
                        if let Some(t) = table.take() {
                            sheets.push(finish_sheet(t));
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(ref t) if hidden == 0 => {
                if let Some(c) = cell.as_mut().filter(|c| c.paragraphs > 0) {
                    c.push_raw(&t.xml10_content());
                }
            }
            Event::CData(ref t) if hidden == 0 => {
                if let Some(c) = cell.as_mut().filter(|c| c.paragraphs > 0) {
                    c.push_raw(&t.xml10_content());
                }
            }
            Event::GeneralRef(ref r) if hidden == 0 => {
                if let Some(c) = cell.as_mut().filter(|c| c.paragraphs > 0) {
                    push_reference(c, r)?;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let list = styles.map(|s| s.list).unwrap_or_default();
    let list = if list.is_empty() {
        vec![crate::model::CellStyle::default()]
    } else {
        list
    };
    Ok((sheets, list))
}

/// Append the character or the entity `r` stands for. A character
/// reference keeps its character, white space included.
fn push_reference(
    cell: &mut PendingCell,
    r: &quick_xml::events::BytesRef<'_>,
) -> Result<(), XmlError> {
    let mut text = String::new();
    xml::push_reference(&mut text, r)?;
    if r.resolve_char_ref()?.is_some() {
        cell.push_kept(&text);
    } else {
        cell.push_raw(&text);
    }
    Ok(())
}

/// Count one drawing element that opens: a frame waits for its first child
/// to say what it holds, and a shape counts at once.
fn read_drawing(name: &str, empty: bool, drawing: &mut Drawing, report: &mut Report) {
    let opens_shape = SHAPES.contains(&name);
    if drawing.depth == 0 && opens_shape {
        if name == "frame" {
            drawing.frame = Some(None);
        } else {
            report.record(XlsxConstruct::Shape, Outcome::Dropped);
        }
    } else if let Some(None) = drawing.frame {
        let kind = match name {
            "object" | "object-ole" => Some(XlsxConstruct::Chart),
            "image" => Some(XlsxConstruct::Picture),
            _ => None,
        };
        if kind.is_some() {
            drawing.frame = Some(kind);
        }
    }
    if opens_shape {
        if empty {
            if drawing.depth == 0 {
                end_drawing_at_top(drawing, report);
            }
        } else {
            drawing.depth += 1;
        }
    }
}

fn end_drawing(drawing: &mut Drawing, report: &mut Report) {
    drawing.depth = drawing.depth.saturating_sub(1);
    if drawing.depth == 0 {
        end_drawing_at_top(drawing, report);
    }
}

fn end_drawing_at_top(drawing: &mut Drawing, report: &mut Report) {
    if let Some(kind) = drawing.frame.take() {
        report.record(kind.unwrap_or(XlsxConstruct::Shape), Outcome::Dropped);
    }
}

fn read_column(
    e: &BytesStart<'_>,
    table: &mut Table,
    layout: &Layout,
    report: &mut Report,
) -> Result<(), XmlError> {
    let attrs = attributes(e)?;
    let repeat = count_of(&attrs, "number-columns-repeated");
    let first = table.next_col;
    let last = first.saturating_add(repeat - 1);
    table.next_col = last.saturating_add(1);
    if matches!(get(&attrs, "visibility"), Some("collapse" | "filter")) {
        report.record(XlsxConstruct::HiddenLine, Outcome::Dropped);
    }
    table.columns.push(ColumnRun {
        first,
        last,
        style: get(&attrs, "default-cell-style-name").map(str::to_owned),
    });
    let width = get(&attrs, "style-name").and_then(|s| layout.column_px.get(s).copied());
    let sized = width.filter(|px| (px.get() - LIBREOFFICE_COLUMN.get()).abs() > 0.05);
    let last_in_grid = last.min(u64::from(LAST_COL.get()));
    if let (Some(px), Ok(first), Ok(last)) =
        (sized, u32::try_from(first), u32::try_from(last_in_grid))
        && first <= last
    {
        table.sheet.column_widths.push(ColumnWidth {
            first: Pos::new(first),
            last: Pos::new(last),
            width: px,
        });
        report.record(XlsxConstruct::ColumnWidth, Outcome::Carried);
    }
    Ok(())
}

fn start_row(
    e: &BytesStart<'_>,
    table: &mut Table,
    layout: &Layout,
    report: &mut Report,
) -> Result<(), XmlError> {
    let attrs = attributes(e)?;
    table.row_repeat = count_of(&attrs, "number-rows-repeated");
    table.row_style = get(&attrs, "default-cell-style-name").map(str::to_owned);
    table.col = 0;
    table.row_cells.clear();
    if matches!(get(&attrs, "visibility"), Some("collapse" | "filter")) {
        report.record(XlsxConstruct::HiddenLine, Outcome::Dropped);
    }
    let height = get(&attrs, "style-name").and_then(|s| layout.row_px.get(s).copied());
    if let Some(px) = height {
        if table.row_repeat > MAX_STYLED_RUN {
            report.record(XlsxConstruct::RowHeight, Outcome::Dropped);
            return Ok(());
        }
        for k in 0..table.row_repeat {
            let row = table.row.saturating_add(k);
            let Some(row) = u32::try_from(row).ok().filter(|r| *r <= LAST_ROW.get()) else {
                break;
            };
            table.sheet.row_heights.push(RowHeight {
                row: Pos::new(row),
                height: px,
            });
            report.record(XlsxConstruct::RowHeight, Outcome::Carried);
        }
    }
    Ok(())
}

/// Close a row: its cells land once per row it repeats over.
fn finish_row(table: &mut Table, cells_read: &mut u64) -> Result<(), ContentError> {
    let cells = std::mem::take(&mut table.row_cells);
    let copies = table.row_repeat;
    let room = MAX_CELLS.get();
    if !cells.is_empty() {
        let total = (cells.len() as u64).saturating_mul(copies);
        *cells_read = cells_read.saturating_add(total);
        if *cells_read > room {
            return Err(ContentError::TooManyCells);
        }
        for k in 0..copies {
            for c in &cells {
                let Some(row) = u32::try_from(u64::from(c.at.row.get()).saturating_add(k))
                    .ok()
                    .filter(|r| *r <= LAST_ROW.get())
                else {
                    continue;
                };
                let mut copy = c.clone();
                copy.at.row = Pos::new(row);
                table.sheet.cells.push(copy);
            }
        }
    }
    table.row = table.row.saturating_add(copies);
    Ok(())
}

/// The style a cell of `table` at `col` takes when it names none: its row's,
/// else its column's, else the default.
fn inherited_style(table: &Table, col: u64) -> String {
    table
        .row_style
        .clone()
        .or_else(|| {
            table
                .columns
                .iter()
                .find(|r| r.first <= col && col <= r.last)
                .and_then(|r| r.style.clone())
        })
        .unwrap_or_else(|| DEFAULT.to_owned())
}

fn finish_cell(cell: PendingCell, table: &mut Table, styles: &mut Styles, report: &mut Report) {
    let style_name = cell
        .style
        .clone()
        .unwrap_or_else(|| inherited_style(table, cell.col));
    let style = styles.id(&style_name, report);
    let Some(at) = position(Spot {
        row: table.row,
        col: cell.col,
    }) else {
        return;
    };
    if cell.rich {
        report.record(XlsxConstruct::RichText, Outcome::Dropped);
    }
    let cached = cell.value();
    let mut content = Content::Value(cached.clone());
    if let Some(attribute) = &cell.formula {
        let array = (cell.matrix != (1, 1))
            .then(|| {
                position(Spot {
                    row: table.row + cell.matrix.1 - 1,
                    col: cell.col + cell.matrix.0 - 1,
                })
            })
            .flatten()
            .map(|end| CellRange::spanning(at, end));
        match formula::excel_of(attribute) {
            Some(excel) => {
                if let Ok(formula) = Formula::from_excel(&excel, array, report) {
                    content = Content::Formula { formula, cached };
                }
            }
            None => {
                if attribute.contains("'file:") || attribute.contains("'http") {
                    report.record(XlsxConstruct::ExternalLink, Outcome::Dropped);
                }
                report.record(XlsxConstruct::ForeignFormula, Outcome::Dropped);
            }
        }
    }
    if cell.span != (1, 1) {
        let end = position(Spot {
            row: table.row.saturating_add(cell.span.1 - 1),
            col: cell.col.saturating_add(cell.span.0 - 1),
        });
        if let Some(end) = end {
            table.sheet.merges.push(CellRange::spanning(at, end));
            report.record(XlsxConstruct::Merge, Outcome::Carried);
        }
    }
    let empty = matches!(content, Content::Value(Value::Empty));
    if empty && style == StyleId::DEFAULT {
        return;
    }
    if empty && cell.repeat.saturating_mul(table.row_repeat) > MAX_STYLED_RUN {
        report.record(XlsxConstruct::Style, Outcome::Dropped);
        return;
    }
    for k in 0..cell.repeat {
        let Some(at) = position(Spot {
            row: table.row,
            col: cell.col.saturating_add(k),
        }) else {
            break;
        };
        table.row_cells.push(Cell {
            at,
            content: content.clone(),
            style,
        });
    }
}

/// A place a table names as it counts its rows and columns, which a
/// repetition can carry past the grid.
#[derive(Clone, Copy)]
struct Spot {
    row: u64,
    col: u64,
}

/// The cell at `spot`, or `None` past the grid.
fn position(spot: Spot) -> Option<CellRef> {
    let row = u32::try_from(spot.row).ok()?;
    let col = u32::try_from(spot.col).ok()?;
    CellRef::new(Pos::new(row), Pos::new(col))
}

fn finish_sheet(table: Table) -> Sheet {
    let mut sheet = table.sheet;
    sheet.cells.sort_by_key(|c| c.at);
    sheet.cells.dedup_by_key(|c| c.at);
    sheet.merges.sort();
    sheet.merges.dedup();
    sheet.column_widths.sort_by_key(|c| c.first);
    sheet.row_heights.sort_by_key(|r| r.row);
    sheet.row_heights.dedup_by_key(|r| r.row);
    sheet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_reads_as_its_days() {
        assert_eq!(duration_days("PT12H00M00S"), Some(0.5));
        assert_eq!(duration_days("PT36H"), Some(1.5));
        assert_eq!(duration_days("P1DT6H"), Some(1.25));
        assert_eq!(duration_days("-PT6H"), Some(-0.25));
        assert_eq!(duration_days("12:00"), None);
    }

    #[test]
    fn white_space_collapses_and_a_spaces_element_keeps_its_count() {
        let mut cell = PendingCell::default();
        cell.start_paragraph();
        cell.push_raw("  a  \n b ");
        cell.push_kept("  ");
        cell.push_raw("c ");
        cell.start_paragraph();
        cell.push_raw(" d");
        assert_eq!(cell.text, "a b   c\nd");
    }
}
