//! The writer: a [`Workbook`] into the bytes of an xlsx package.
//!
//! The package holds the parts Excel needs and nothing else: the workbook,
//! one worksheet per sheet, the styles and the shared strings. No timestamp
//! enters it, so one workbook always writes the same bytes.

use std::collections::HashMap;
use std::fmt::Write as _;

use ambre_core::{Count, Payload};

use crate::formula::Formula;
use crate::model::{Cell, Content, Sheet, Value, Visibility, Workbook};
use crate::opc::{self, PackageWriter, PartName, RelKind, RelTarget};
use crate::read::{column_width_attr, row_height_attr};
use crate::report::{Outcome, Report, XlsxConstruct};
use crate::{Exported, XlsxError, styles, xml};

const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

const WORKBOOK_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const WORKSHEET_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const STYLES_TYPE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";
const STRINGS_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";

const WORKSHEET_REL: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet");
const STYLES_REL: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles");
const STRINGS_REL: RelKind = RelKind::new(
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings",
);

/// The shared strings of a written workbook, each text once.
#[derive(Default)]
struct Strings {
    index: HashMap<String, usize>,
    list: Vec<String>,
    uses: usize,
}

impl Strings {
    fn intern(&mut self, text: &str) -> usize {
        self.uses += 1;
        if let Some(&at) = self.index.get(text) {
            return at;
        }
        let at = self.list.len();
        self.index.insert(text.to_owned(), at);
        self.list.push(text.to_owned());
        at
    }
}

/// Write `workbook` as an xlsx package.
pub(crate) fn write(workbook: &Workbook) -> Result<Exported, XlsxError> {
    if workbook.sheets.is_empty() {
        return Err(XlsxError::NoSheet);
    }
    let mut seen: Vec<String> = Vec::new();
    for sheet in &workbook.sheets {
        let folded = sheet.name.as_str().to_lowercase();
        if seen.contains(&folded) {
            return Err(XlsxError::DuplicateSheetName(sheet.name.clone()));
        }
        seen.push(folded);
    }
    let mut report = Report::default();
    let mut package = PackageWriter::new();
    let main = part("xl/workbook.xml")?;
    package.relate(
        &opc::root(),
        RelKind::OFFICE_DOCUMENT,
        RelTarget::Part(main.clone()),
    );

    // Excel opens on a visible tab: a workbook whose every sheet is hidden
    // shows its first one.
    let first_visible = workbook
        .sheets
        .iter()
        .position(|s| s.visibility == Visibility::Visible);
    if first_visible.is_none() {
        report.record(XlsxConstruct::HiddenSheet, Outcome::Approximated);
    }
    let active = first_visible.unwrap_or(0);

    let mut strings = Strings::default();
    let mut sheet_entries = String::new();
    for (n, sheet) in workbook.sheets.iter().enumerate() {
        let name = part(&format!("xl/worksheets/sheet{}.xml", n + 1))?;
        let tab = if n == active {
            Tab::Selected
        } else {
            Tab::Background
        };
        let xml = sheet_xml(sheet, tab, workbook, &mut strings, &mut report);
        package.part(
            name.clone(),
            Some(WORKSHEET_TYPE),
            Payload::new(xml.into_bytes()),
        );
        let id = package.relate(&main, WORKSHEET_REL, RelTarget::Part(name));
        let id = id.as_str();
        let hidden = sheet.visibility == Visibility::Hidden && n != active;
        if hidden {
            report.record(XlsxConstruct::HiddenSheet, Outcome::Carried);
        }
        let _ = write!(
            sheet_entries,
            "<sheet name=\"{}\" sheetId=\"{}\"{} r:id=\"{id}\"/>",
            xml::escape(sheet.name.as_str()),
            n + 1,
            if hidden { " state=\"hidden\"" } else { "" },
        );
    }

    let styles_part = part("xl/styles.xml")?;
    let styles = if workbook.styles.is_empty() {
        vec![crate::model::CellStyle::default()]
    } else {
        workbook.styles.clone()
    };
    package.part(
        styles_part.clone(),
        Some(STYLES_TYPE),
        Payload::new(styles::write(&styles, &mut report).into_bytes()),
    );
    package.relate(&main, STYLES_REL, RelTarget::Part(styles_part));
    report.record_count(
        XlsxConstruct::Style,
        Outcome::Carried,
        Count::of(styles.len().saturating_sub(1)),
    );

    if !strings.list.is_empty() {
        let strings_part = part("xl/sharedStrings.xml")?;
        package.part(
            strings_part.clone(),
            Some(STRINGS_TYPE),
            Payload::new(strings_xml(&strings).into_bytes()),
        );
        package.relate(&main, STRINGS_REL, RelTarget::Part(strings_part));
    }

    let workbook_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <workbook xmlns=\"{MAIN_NS}\" xmlns:r=\"{REL_NS}\">\
         <bookViews><workbookView activeTab=\"{active}\"/></bookViews>\
         <sheets>{sheet_entries}</sheets>\
         <calcPr calcId=\"191029\" fullCalcOnLoad=\"1\"/></workbook>"
    );
    package.part(
        main,
        Some(WORKBOOK_TYPE),
        Payload::new(workbook_xml.into_bytes()),
    );

    report.record_count(
        XlsxConstruct::Sheet,
        Outcome::Carried,
        Count::of(workbook.sheets.len()),
    );
    let bytes = package.finish().map_err(XlsxError::Package)?;
    Ok(Exported {
        bytes: bytes.into_role(),
        report,
    })
}

fn part(name: &str) -> Result<PartName, XlsxError> {
    PartName::new(name).ok_or(XlsxError::NotAWorkbook)
}

/// Whether a sheet's tab is the one Excel opens on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Background,
    Selected,
}

/// The worksheet part of `sheet`.
fn sheet_xml(
    sheet: &Sheet,
    tab: Tab,
    workbook: &Workbook,
    strings: &mut Strings,
    report: &mut Report,
) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <worksheet xmlns=\"{MAIN_NS}\" xmlns:r=\"{REL_NS}\">"
    );
    if let (Some(first), Some(last)) = (sheet.cells.first(), sheet.cells.last()) {
        let (top, bottom) = (first.at.row, last.at.row);
        let left = sheet
            .cells
            .iter()
            .map(|c| c.at.col)
            .min()
            .unwrap_or(first.at.col);
        let right = sheet
            .cells
            .iter()
            .map(|c| c.at.col)
            .max()
            .unwrap_or(last.at.col);
        let range = crate::cellref::CellRange::spanning(
            crate::cellref::CellRef {
                row: top,
                col: left,
            },
            crate::cellref::CellRef {
                row: bottom,
                col: right,
            },
        );
        let _ = write!(out, "<dimension ref=\"{range}\"/>");
    }
    let _ = write!(
        out,
        "<sheetViews><sheetView{} workbookViewId=\"0\"/></sheetViews>\
         <sheetFormatPr defaultRowHeight=\"15\"/>",
        match tab {
            Tab::Background => "",
            Tab::Selected => " tabSelected=\"1\"",
        }
    );
    if !sheet.column_widths.is_empty() {
        out.push_str("<cols>");
        for c in &sheet.column_widths {
            let _ = write!(
                out,
                "<col min=\"{}\" max=\"{}\" width=\"{}\" customWidth=\"1\"/>",
                u64::from(c.first.get()) + 1,
                u64::from(c.last.get()) + 1,
                column_width_attr(c.width)
            );
            report.record(XlsxConstruct::ColumnWidth, Outcome::Carried);
        }
        out.push_str("</cols>");
    }
    out.push_str("<sheetData>");
    let mut heights = sheet.row_heights.iter().peekable();
    let mut cells = sheet.cells.iter().peekable();
    loop {
        let next_cell_row = cells.peek().map(|c| c.at.row);
        let next_height_row = heights.peek().map(|h| h.row);
        let row = match (next_cell_row, next_height_row) {
            (None, None) => break,
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
        };
        let _ = write!(out, "<row r=\"{}\"", u64::from(row.get()) + 1);
        if let Some(h) = heights.next_if(|h| h.row == row) {
            let _ = write!(
                out,
                " ht=\"{}\" customHeight=\"1\"",
                row_height_attr(h.height)
            );
            report.record(XlsxConstruct::RowHeight, Outcome::Carried);
        }
        out.push('>');
        while let Some(cell) = cells.next_if(|c| c.at.row == row) {
            cell_xml(&mut out, cell, workbook, strings, report);
        }
        out.push_str("</row>");
    }
    out.push_str("</sheetData>");
    if !sheet.merges.is_empty() {
        let _ = write!(out, "<mergeCells count=\"{}\">", sheet.merges.len());
        for m in &sheet.merges {
            let _ = write!(out, "<mergeCell ref=\"{m}\"/>");
            report.record(XlsxConstruct::Merge, Outcome::Carried);
        }
        out.push_str("</mergeCells>");
    }
    out.push_str(
        "<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>\
         </worksheet>",
    );
    out
}

/// One `c` element.
fn cell_xml(
    out: &mut String,
    cell: &Cell,
    workbook: &Workbook,
    strings: &mut Strings,
    report: &mut Report,
) {
    let style = cell.style.slot().get();
    let style_attr = if style == 0 || style as usize >= workbook.styles.len() {
        String::new()
    } else {
        format!(" s=\"{style}\"")
    };
    report.record(XlsxConstruct::Cell, Outcome::Carried);
    match &cell.content {
        Content::Value(value) => value_cell_xml(out, cell, &style_attr, value, strings),
        Content::TrameFormula { formula, cached } => match Formula::from_ambre(formula.as_str()) {
            Ok(formula) => formula_xml(out, cell, &style_attr, &formula, cached, strings, report),
            // A Trame text whose Excel spelling passes the cap of the format
            // leaves as its value, and the report names the loss.
            Err(_) => {
                report.record(XlsxConstruct::Formula, Outcome::Approximated);
                value_cell_xml(out, cell, &style_attr, cached, strings);
            }
        },
        Content::Formula { formula, cached } => {
            formula_xml(out, cell, &style_attr, formula, cached, strings, report);
        }
    }
}

/// One `c` element that holds a value.
fn value_cell_xml(
    out: &mut String,
    cell: &Cell,
    style_attr: &str,
    value: &Value,
    strings: &mut Strings,
) {
    let (t, v) = value_xml(value, strings, ValueSlot::Typed);
    let _ = write!(out, "<c r=\"{}\"{style_attr}{t}", cell.at);
    match v {
        Some(v) => {
            let _ = write!(out, "><v>{v}</v></c>");
        }
        None => out.push_str("/>"),
    }
}

/// One `c` element that holds a formula and its last value.
fn formula_xml(
    out: &mut String,
    cell: &Cell,
    style_attr: &str,
    formula: &Formula,
    cached: &Value,
    strings: &mut Strings,
    report: &mut Report,
) {
    {
        {
            let (t, v) = value_xml(cached, strings, ValueSlot::FormulaResult);
            let _ = write!(out, "<c r=\"{}\"{style_attr}{t}>", cell.at);
            match formula.array() {
                Some(range) => {
                    let _ = write!(out, "<f t=\"array\" ref=\"{range}\">");
                }
                None => out.push_str("<f>"),
            }
            out.push_str(&xml::escape(formula.excel()));
            out.push_str("</f>");
            if let Some(v) = v {
                let _ = write!(out, "<v>{v}</v>");
            }
            out.push_str("</c>");
            let construct = if formula.ambre().is_some() {
                XlsxConstruct::Formula
            } else {
                XlsxConstruct::ForeignFormula
            };
            report.record(construct, Outcome::Carried);
        }
    }
}

/// Where a value sits: as the cell's own value, or as a formula's result.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueSlot {
    Typed,
    FormulaResult,
}

/// The `t` attribute and the `v` text of a value.
fn value_xml(
    value: &Value,
    strings: &mut Strings,
    slot: ValueSlot,
) -> (&'static str, Option<String>) {
    match value {
        Value::Empty => ("", None),
        Value::Number(n) if n.is_finite() => ("", Some(n.to_string())),
        Value::Number(_) => (" t=\"e\"", Some("#NUM!".to_owned())),
        Value::Text(text) => match slot {
            ValueSlot::Typed => (" t=\"s\"", Some(strings.intern(text).to_string())),
            ValueSlot::FormulaResult => (" t=\"str\"", Some(xml::escape(text).into_owned())),
        },
        Value::Boolean(b) => (" t=\"b\"", Some(if *b { "1" } else { "0" }.to_owned())),
        Value::Error(e) => (" t=\"e\"", Some(xml::escape(e.code()).into_owned())),
    }
}

fn strings_xml(strings: &Strings) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <sst xmlns=\"{MAIN_NS}\" count=\"{}\" uniqueCount=\"{}\">",
        strings.uses,
        strings.list.len()
    );
    for text in &strings.list {
        let keep_space =
            text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace);
        let _ = write!(
            out,
            "<si><t{}>{}</t></si>",
            if keep_space {
                " xml:space=\"preserve\""
            } else {
                ""
            },
            xml::escape(text)
        );
    }
    out.push_str("</sst>");
    out
}
