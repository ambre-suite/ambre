//! The writer: a [`Workbook`] into the bytes of an OpenDocument
//! spreadsheet.
//!
//! The package holds the parts a reader needs and nothing else: the
//! content, the styles and the manifest. No timestamp enters it, so one
//! workbook always writes the same bytes.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use ambre_core::{Count, Doc, Payload, Px};

use super::{escaped, formula, length_pt, push_escaped, styles};
use crate::cellref::CellRange;
use crate::formula::Formula;
use crate::model::{Cell, Content, Sheet, Value, Visibility, Workbook};
use crate::opc::PartName;
use crate::opc::odf::{OdfType, OdfWriter};
use crate::report::{Outcome, Report, XlsxConstruct};
use crate::{Exported, XlsxError};

/// The namespaces every written part declares.
const NAMESPACES: &str = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
     xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
     xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
     xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" \
     xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
     xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" \
     xmlns:number=\"urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0\" \
     xmlns:of=\"urn:oasis:names:tc:opendocument:xmlns:of:1.2\" \
     xmlns:calcext=\"urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0\" \
     office:version=\"1.3\"";

/// Write `workbook` as an OpenDocument spreadsheet.
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
    let styles = if workbook.styles.is_empty() {
        vec![crate::model::CellStyle::default()]
    } else {
        workbook.styles.clone()
    };
    let written = styles::write(&styles, &mut report);
    report.record_count(
        XlsxConstruct::Style,
        Outcome::Carried,
        Count::of(styles.len().saturating_sub(1)),
    );

    // A reader opens on a visible sheet: a workbook whose every sheet is
    // hidden shows its first one.
    let first_visible = workbook
        .sheets
        .iter()
        .position(|s| s.visibility == Visibility::Visible);
    if first_visible.is_none() {
        report.record(XlsxConstruct::HiddenSheet, Outcome::Approximated);
    }
    let active = first_visible.unwrap_or(0);

    let mut layout = Layout::default();
    let mut tables = String::new();
    for (n, sheet) in workbook.sheets.iter().enumerate() {
        let shown = if sheet.visibility == Visibility::Hidden && n != active {
            report.record(XlsxConstruct::HiddenSheet, Outcome::Carried);
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        table_xml(&mut tables, sheet, shown, &styles, &mut layout, &mut report);
    }
    report.record_count(
        XlsxConstruct::Sheet,
        Outcome::Carried,
        Count::of(workbook.sheets.len()),
    );

    let mut automatic = String::from(
        "<style:style style:name=\"ta1\" style:family=\"table\"><style:table-properties table:display=\"true\"/></style:style>\
         <style:style style:name=\"ta2\" style:family=\"table\"><style:table-properties table:display=\"false\"/></style:style>",
    );
    for (at, width) in layout.widths.iter().enumerate() {
        let _ = write!(
            automatic,
            "<style:style style:name=\"co{}\" style:family=\"table-column\"><style:table-column-properties style:column-width=\"{}\"/></style:style>",
            at + 1,
            length_pt(*width)
        );
    }
    for (at, height) in layout.heights.iter().enumerate() {
        let _ = write!(
            automatic,
            "<style:style style:name=\"ro{}\" style:family=\"table-row\"><style:table-row-properties style:row-height=\"{}\" style:use-optimal-row-height=\"false\"/></style:style>",
            at + 1,
            length_pt(*height)
        );
    }
    automatic.push_str(&written.automatic);

    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <office:document-content {NAMESPACES}>\
         <office:font-face-decls>{}</office:font-face-decls>\
         <office:automatic-styles>{automatic}</office:automatic-styles>\
         <office:body><office:spreadsheet>{tables}</office:spreadsheet></office:body>\
         </office:document-content>",
        written.fonts
    );
    let styles_part = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <office:document-styles {NAMESPACES}>\
         <office:font-face-decls>{}</office:font-face-decls>\
         <office:styles>{}</office:styles>\
         </office:document-styles>",
        written.fonts, written.named
    );

    let mut package = OdfWriter::new(OdfType::SPREADSHEET);
    package.part(
        part("content.xml")?,
        OdfType::XML,
        Payload::new(content.into_bytes()),
    );
    package.part(
        part("styles.xml")?,
        OdfType::XML,
        Payload::new(styles_part.into_bytes()),
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

/// The column widths and row heights of the workbook, each distinct size
/// once: the column style `co{i + 1}` and the row style `ro{i + 1}`.
#[derive(Default)]
struct Layout {
    widths: Vec<Px<Doc>>,
    heights: Vec<Px<Doc>>,
}

impl Layout {
    fn slot(list: &mut Vec<Px<Doc>>, size: Px<Doc>) -> usize {
        list.iter()
            .position(|held| held.get().to_bits() == size.get().to_bits())
            .unwrap_or_else(|| {
                list.push(size);
                list.len() - 1
            })
            + 1
    }
}

/// One element of a written row, in column order.
enum Slot<'a> {
    /// A cell of the model, the anchor of a merge when `span` is set.
    Cell {
        cell: Option<&'a Cell>,
        span: Option<CellRange>,
        covered: Covered,
    },
    /// Columns no cell holds, covered by a merge or not.
    Gap { count: u32, covered: Covered },
}

/// Whether a written cell sits under a merge it does not anchor.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Covered {
    No,
    Yes,
}

/// Write the table of `sheet`.
fn table_xml(
    out: &mut String,
    sheet: &Sheet,
    shown: Visibility,
    styles: &[crate::model::CellStyle],
    layout: &mut Layout,
    report: &mut Report,
) {
    out.push_str("<table:table table:name=\"");
    push_escaped(out, sheet.name.as_str());
    let _ = write!(
        out,
        "\" table:style-name=\"{}\">",
        match shown {
            Visibility::Hidden => "ta2",
            Visibility::Visible => "ta1",
        }
    );
    // The columns: each sized run, the runs between them at the default.
    let mut col = 0u32;
    for width in &sheet.column_widths {
        if width.first.get() < col {
            continue;
        }
        if width.first.get() > col {
            let _ = write!(
                out,
                "<table:table-column table:number-columns-repeated=\"{}\"/>",
                width.first.get() - col
            );
        }
        let span = width.last.get().saturating_sub(width.first.get()) + 1;
        let style = Layout::slot(&mut layout.widths, width.width);
        let _ = write!(
            out,
            "<table:table-column table:style-name=\"co{style}\" table:number-columns-repeated=\"{span}\"/>"
        );
        report.record(XlsxConstruct::ColumnWidth, Outcome::Carried);
        col = width.last.get().saturating_add(1);
    }
    if col == 0 {
        out.push_str("<table:table-column/>");
    }

    // The rows that hold something: a cell, a height, the edges of a merge.
    let mut rows: BTreeSet<u32> = BTreeSet::new();
    rows.extend(sheet.cells.iter().map(|c| c.at.row.get()));
    rows.extend(sheet.row_heights.iter().map(|h| h.row.get()));
    for m in &sheet.merges {
        rows.insert(m.first().row.get());
        rows.insert(m.first().row.get().saturating_add(1));
        rows.insert(m.last().row.get().saturating_add(1));
    }
    let last_cell_row = sheet
        .cells
        .iter()
        .map(|c| c.at.row.get())
        .chain(sheet.row_heights.iter().map(|h| h.row.get()))
        .chain(sheet.merges.iter().map(|m| m.last().row.get()))
        .max();
    let Some(last_row) = last_cell_row else {
        out.push_str("<table:table-row><table:table-cell/></table:table-row></table:table>");
        return;
    };
    let rows: Vec<u32> = rows.into_iter().filter(|r| *r <= last_row).collect();
    let mut cells = sheet.cells.iter().peekable();
    let mut written = 0u32;
    for (at, &row) in rows.iter().enumerate() {
        if row > written {
            empty_rows(out, written..row, sheet);
        }
        let mut row_cells = Vec::new();
        while let Some(cell) = cells.next_if(|c| c.at.row.get() == row) {
            row_cells.push(cell);
        }
        let height = sheet.row_heights.iter().find(|h| h.row.get() == row);
        let style = height.map(|h| Layout::slot(&mut layout.heights, h.height));
        if height.is_some() {
            report.record(XlsxConstruct::RowHeight, Outcome::Carried);
        }
        row_xml(out, row, style, &row_cells, sheet, styles, report);
        written = row + 1;
        // The rows up to the next one that holds something repeat this
        // row's merges, and nothing else.
        let next = rows.get(at + 1).copied().unwrap_or(last_row + 1);
        if next > written {
            empty_rows(out, written..next, sheet);
            written = next;
        }
    }
    out.push_str("</table:table>");
}

/// Write the `rows` that hold no cell: the cells a merge covers, and gaps.
fn empty_rows(out: &mut String, rows: std::ops::Range<u32>, sheet: &Sheet) {
    let count = rows.end.saturating_sub(rows.start);
    let slots = slots(rows.start, &[], sheet);
    if slots.is_empty() {
        let _ = write!(
            out,
            "<table:table-row table:number-rows-repeated=\"{count}\"><table:table-cell/></table:table-row>"
        );
        return;
    }
    let _ = write!(
        out,
        "<table:table-row table:number-rows-repeated=\"{count}\">"
    );
    for slot in &slots {
        if let Slot::Gap { count, covered } = slot {
            gap_xml(out, *count, *covered);
        }
    }
    out.push_str("</table:table-row>");
}

/// The slots of the row `row`, given its cells.
fn slots<'a>(row: u32, cells: &[&'a Cell], sheet: &Sheet) -> Vec<Slot<'a>> {
    let mut merges: Vec<&CellRange> = sheet
        .merges
        .iter()
        .filter(|m| m.first().row.get() <= row && row <= m.last().row.get())
        .collect();
    merges.sort_by_key(|m| m.first().col);
    let mut slots = Vec::new();
    let mut col = 0u32;
    let mut cells = cells.iter().peekable();
    let mut merges = merges.into_iter().peekable();
    loop {
        let next_cell = cells.peek().map(|c| c.at.col.get());
        let next_merge = merges.peek().map(|m| m.first().col.get());
        let start = match (next_cell, next_merge) {
            (None, None) => break,
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
        };
        if start > col {
            slots.push(Slot::Gap {
                count: start - col,
                covered: Covered::No,
            });
        }
        if next_merge == Some(start) {
            let Some(merge) = merges.next() else {
                break;
            };
            let anchor_row = merge.first().row.get() == row;
            let mut at = start;
            while at <= merge.last().col.get() {
                let cell = cells.next_if(|c| c.at.col.get() == at).copied();
                let anchor = anchor_row && at == start;
                if anchor || cell.is_some() {
                    slots.push(Slot::Cell {
                        cell,
                        span: anchor.then_some(*merge),
                        covered: if anchor { Covered::No } else { Covered::Yes },
                    });
                    at += 1;
                } else {
                    let until = cells
                        .peek()
                        .map(|c| c.at.col.get())
                        .filter(|c| *c <= merge.last().col.get())
                        .unwrap_or(merge.last().col.get() + 1);
                    slots.push(Slot::Gap {
                        count: until - at,
                        covered: Covered::Yes,
                    });
                    at = until;
                }
            }
            col = merge.last().col.get().saturating_add(1);
        } else if let Some(cell) = cells.next() {
            slots.push(Slot::Cell {
                cell: Some(cell),
                span: None,
                covered: Covered::No,
            });
            col = start.saturating_add(1);
        }
    }
    slots
}

fn gap_xml(out: &mut String, count: u32, covered: Covered) {
    let element = match covered {
        Covered::No => "table:table-cell",
        Covered::Yes => "table:covered-table-cell",
    };
    if count == 1 {
        let _ = write!(out, "<{element}/>");
    } else {
        let _ = write!(
            out,
            "<{element} table:number-columns-repeated=\"{count}\"/>"
        );
    }
}

fn row_xml(
    out: &mut String,
    row: u32,
    height_style: Option<usize>,
    cells: &[&Cell],
    sheet: &Sheet,
    styles: &[crate::model::CellStyle],
    report: &mut Report,
) {
    out.push_str("<table:table-row");
    if let Some(style) = height_style {
        let _ = write!(out, " table:style-name=\"ro{style}\"");
    }
    out.push('>');
    let slots = slots(row, cells, sheet);
    if slots.is_empty() {
        out.push_str("<table:table-cell/>");
    }
    for slot in slots {
        match slot {
            Slot::Gap { count, covered } => gap_xml(out, count, covered),
            Slot::Cell {
                cell,
                span,
                covered,
            } => cell_xml(out, cell, span, covered, styles, report),
        }
    }
    out.push_str("</table:table-row>");
}

/// One cell element: its style, its spans, its value or formula, its text.
fn cell_xml(
    out: &mut String,
    cell: Option<&Cell>,
    span: Option<CellRange>,
    covered: Covered,
    styles: &[crate::model::CellStyle],
    report: &mut Report,
) {
    let element = match covered {
        Covered::No => "table:table-cell",
        Covered::Yes => "table:covered-table-cell",
    };
    let _ = write!(out, "<{element}");
    if let Some(span) = span {
        let _ = write!(
            out,
            " table:number-columns-spanned=\"{}\" table:number-rows-spanned=\"{}\"",
            span.last().col.get() - span.first().col.get() + 1,
            span.last().row.get() - span.first().row.get() + 1
        );
        report.record(XlsxConstruct::Merge, Outcome::Carried);
    }
    let Some(cell) = cell else {
        out.push_str("/>");
        return;
    };
    report.record(XlsxConstruct::Cell, Outcome::Carried);
    let style = cell.style.slot().to_usize();
    if style != 0 && style < styles.len() {
        let _ = write!(out, " table:style-name=\"ce{style}\"");
    }
    let (value, formula) = match &cell.content {
        Content::Value(value) => (value.clone(), None),
        Content::Formula { formula, cached } => (cached.clone(), Some(formula.clone())),
        Content::TrameFormula { formula, cached } => match Formula::from_ambre(formula.as_str()) {
            Ok(formula) => (cached.clone(), Some(formula)),
            // A Trame text whose Excel spelling passes the cap of the format
            // leaves as its value, and the report names the loss.
            Err(_) => {
                report.record(XlsxConstruct::Formula, Outcome::Approximated);
                (cached.clone(), None)
            }
        },
    };
    if let Some(formula) = &formula {
        out.push_str(" table:formula=\"");
        push_escaped(out, &formula::attribute_of(formula.excel()));
        out.push('"');
        if let Some(range) = formula.array() {
            let _ = write!(
                out,
                " table:number-matrix-columns-spanned=\"{}\" table:number-matrix-rows-spanned=\"{}\"",
                range.last().col.get() - range.first().col.get() + 1,
                range.last().row.get() - range.first().row.get() + 1
            );
        }
        let construct = if formula.ambre().is_some() {
            XlsxConstruct::Formula
        } else {
            XlsxConstruct::ForeignFormula
        };
        report.record(construct, Outcome::Carried);
    }
    let text = match &value {
        Value::Empty => None,
        Value::Number(n) if n.is_finite() => {
            let _ = write!(
                out,
                " office:value-type=\"float\" office:value=\"{n}\" calcext:value-type=\"float\""
            );
            Some(n.to_string())
        }
        Value::Number(_) => {
            out.push_str(" office:value-type=\"string\" calcext:value-type=\"error\"");
            Some("#NUM!".to_owned())
        }
        Value::Text(text) => {
            out.push_str(" office:value-type=\"string\"");
            if formula.is_some() {
                let _ = write!(out, " office:string-value=\"{}\"", escaped(text));
            }
            out.push_str(" calcext:value-type=\"string\"");
            Some(text.clone())
        }
        Value::Boolean(b) => {
            let _ = write!(
                out,
                " office:value-type=\"boolean\" office:boolean-value=\"{b}\" calcext:value-type=\"boolean\""
            );
            Some(if *b { "TRUE" } else { "FALSE" }.to_owned())
        }
        Value::Error(e) => {
            out.push_str(" office:value-type=\"string\" office:string-value=\"\" calcext:value-type=\"error\"");
            Some(e.code().to_owned())
        }
    };
    match text {
        Some(text) => {
            out.push('>');
            paragraphs(out, &text);
            let _ = write!(out, "</{element}>");
        }
        None => out.push_str("/>"),
    }
}

/// Write `text` as paragraphs, one per line, each space a reader would
/// collapse written as a spaces element.
fn paragraphs(out: &mut String, text: &str) {
    for line in text.split('\n') {
        out.push_str("<text:p>");
        let chars: Vec<char> = line.chars().collect();
        let mut at = 0;
        while let Some(&c) = chars.get(at) {
            match c {
                ' ' => {
                    let run = chars
                        .get(at..)
                        .map_or(0, |rest| rest.iter().take_while(|&&x| x == ' ').count());
                    let inside = at > 0 && chars.get(at + run).is_some();
                    if run == 1 && inside {
                        out.push(' ');
                    } else if run == 1 {
                        out.push_str("<text:s/>");
                    } else {
                        let _ = write!(out, "<text:s text:c=\"{run}\"/>");
                    }
                    at += run;
                }
                '\t' => {
                    out.push_str("<text:tab/>");
                    at += 1;
                }
                c => {
                    let mut one = [0u8; 4];
                    push_escaped(out, c.encode_utf8(&mut one));
                    at += 1;
                }
            }
        }
        out.push_str("</text:p>");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_space_a_reader_would_collapse_leaves_as_a_spaces_element() {
        let mut out = String::new();
        paragraphs(&mut out, " a  b c \tz\nnext\r");
        assert_eq!(
            out,
            "<text:p><text:s/>a<text:s text:c=\"2\"/>b c <text:tab/>z</text:p>\
             <text:p>next&#13;</text:p>"
        );
    }
}
