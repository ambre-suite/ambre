//! The map between the model and the [`Book`], both ways. It runs in the
//! OOXML module alone, so the Main instance links none of it.

use std::collections::BTreeMap;

use ambre_core::index::Pos;
use ambre_core::{Bounded, Payload, Validated};

use super::{Book, BookCell, BookContent, BookSheet, TrameCellStyle, TrameFormat, TrameStyle};
use crate::cellref::Col;
use crate::formula::Formula;
use crate::model::{
    CellStyle, ColumnWidth, Content, Decimals, NumberFormat, Sheet, SheetName, StyleId, Value,
    Workbook,
};
use crate::report::{Outcome, Report, XlsxConstruct};

/// The Trame twin of a cell style.
fn trame_style(style: &CellStyle) -> TrameStyle {
    let (format, decimals) = match &style.format {
        NumberFormat::General => (TrameFormat::General, None),
        NumberFormat::Number(d) => (TrameFormat::Number, Some(*d)),
        NumberFormat::Currency(d) => (TrameFormat::Currency, Some(*d)),
        NumberFormat::Percent(d) => (TrameFormat::Percent, Some(*d)),
        NumberFormat::Scientific(d) => (TrameFormat::Scientific, Some(*d)),
        NumberFormat::Fraction => (TrameFormat::Fraction, None),
        NumberFormat::Date => (TrameFormat::Date, None),
        NumberFormat::Time => (TrameFormat::Time, None),
        NumberFormat::Text => (TrameFormat::Text, None),
        NumberFormat::Custom(code) => (TrameFormat::Custom(code.as_str().to_owned()), None),
    };
    TrameStyle {
        bold: style.weight,
        italic: style.slant,
        style: TrameCellStyle {
            underline: style.underline,
            strikethrough: style.strikethrough,
            h_align: style.h_align,
            v_align: style.v_align,
            wrap: style.wrap,
            fg: style.fg,
            bg: style.bg,
            font: style.font.clone(),
            size: style.size,
            format,
            decimals,
            borders: style.borders,
        },
    }
}

/// The cell style of a Trame twin. A family with no count of its own takes
/// two places, as Trame draws it.
fn cell_style(twin: &TrameStyle) -> CellStyle {
    let style = &twin.style;
    let places = || style.decimals.unwrap_or(Bounded::<Decimals>::of_u8::<2>());
    CellStyle {
        weight: twin.bold,
        slant: twin.italic,
        underline: style.underline,
        strikethrough: style.strikethrough,
        h_align: style.h_align,
        v_align: style.v_align,
        wrap: style.wrap,
        fg: style.fg,
        bg: style.bg,
        font: style.font.clone(),
        size: style.size,
        format: match &style.format {
            TrameFormat::General => NumberFormat::General,
            TrameFormat::Number => NumberFormat::Number(places()),
            TrameFormat::Currency => NumberFormat::Currency(places()),
            TrameFormat::Percent => NumberFormat::Percent(places()),
            TrameFormat::Scientific => NumberFormat::Scientific(places()),
            TrameFormat::Fraction => NumberFormat::Fraction,
            TrameFormat::Date => NumberFormat::Date,
            TrameFormat::Time => NumberFormat::Time,
            TrameFormat::Text => NumberFormat::Text,
            TrameFormat::Custom(code) => {
                Validated::parse(code.clone()).map_or(NumberFormat::General, NumberFormat::Custom)
            }
        },
        borders: style.borders,
    }
}

/// The text Trame stores for a value, never in scientific notation: Rust
/// prints a finite `f64` in full. A date is its serial already. An error and
/// an empty value write nothing.
fn literal(value: &Value) -> String {
    match value {
        Value::Number(n) if n.is_finite() => n.to_string(),
        Value::Text(text) => text.clone(),
        Value::Boolean(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        _ => String::new(),
    }
}

/// The value a raw text Trame stores reads as: a finite number, a boolean,
/// or a text, as Trame's engine reads it.
fn value_of(raw: &str) -> Value {
    match raw {
        "" => Value::Empty,
        "TRUE" => Value::Boolean(true),
        "FALSE" => Value::Boolean(false),
        _ => match raw.parse::<f64>() {
            Ok(n) if n.is_finite() => Value::Number(n),
            _ => Value::Text(raw.to_owned()),
        },
    }
}

/// The [`Book`] an import answers. A column width or a row height past the
/// last cell of its sheet sizes a line that holds nothing. It is dropped,
/// and counted in `report`.
#[must_use]
pub fn book_of(workbook: &Workbook, report: &mut Report) -> Book {
    let styles = workbook
        .styles
        .iter()
        .map(|s| Payload::new(postcard::to_allocvec(&trame_style(s)).unwrap_or_default()))
        .collect();
    let sheets = workbook
        .sheets
        .iter()
        .map(|sheet| {
            let cells: Vec<BookCell> = sheet
                .cells
                .iter()
                .map(|cell| BookCell {
                    at: cell.at,
                    content: match &cell.content {
                        Content::Value(value) => BookContent::Literal(literal(value)),
                        Content::Formula { formula, cached } => BookContent::Formula {
                            trame: formula.ambre_text().cloned(),
                            excel: Some(formula.excel_text().clone()),
                            cached: cached.clone(),
                        },
                        Content::TrameFormula { formula, cached } => BookContent::Formula {
                            trame: Some(formula.clone()),
                            excel: None,
                            cached: cached.clone(),
                        },
                    },
                    style: cell.style,
                })
                .collect();
            let last_row = cells.iter().map(|c| c.at.row).max();
            let last_col = cells.iter().map(|c| c.at.col).max();
            let mut dropped = 0u32;
            let mut column_widths = Vec::new();
            for w in &sheet.column_widths {
                for col in w.first.get()..=w.last.get() {
                    if last_col.is_some_and(|last| col <= last.get()) {
                        let col = Pos::<Col>::new(col);
                        column_widths.push(ColumnWidth {
                            first: col,
                            last: col,
                            width: w.width,
                        });
                    } else {
                        dropped += 1;
                    }
                }
            }
            report.record_count(
                XlsxConstruct::ColumnWidth,
                Outcome::Dropped,
                ambre_core::Count::new(dropped),
            );
            let (row_heights, far): (Vec<_>, Vec<_>) = sheet
                .row_heights
                .iter()
                .partition(|h| last_row.is_some_and(|last| h.row <= last));
            report.record_count(
                XlsxConstruct::RowHeight,
                Outcome::Dropped,
                ambre_core::Count::of(far.len()),
            );
            BookSheet {
                name: sheet.name.as_str().to_owned(),
                visibility: sheet.visibility,
                cells,
                merges: sheet.merges.clone(),
                column_widths,
                row_heights,
            }
        })
        .collect();
    Book { sheets, styles }
}

/// The name a Trame sheet takes in Excel, unique in `taken`, case ignored. A
/// name Excel refuses has each refused character replaced, its length cut to
/// 31, and is counted in `report`.
fn excel_name(name: &str, taken: &mut Vec<String>, report: &mut Report) -> Validated<SheetName> {
    let mut wanted = match Validated::<SheetName>::parse(name.to_owned()) {
        Ok(name) => name.as_str().to_owned(),
        Err(_) => {
            report.record(XlsxConstruct::Sheet, Outcome::Approximated);
            let cleaned: String = name
                .chars()
                .map(|c| if "\\/?*[]:".contains(c) { '_' } else { c })
                .take(31)
                .collect();
            cleaned.trim_matches('\'').to_owned()
        }
    };
    let mut n = 1u32;
    loop {
        let folded = wanted.to_lowercase();
        if !wanted.is_empty()
            && !taken.contains(&folded)
            && let Ok(valid) = Validated::<SheetName>::parse(wanted.clone())
        {
            taken.push(folded);
            return valid;
        }
        n += 1;
        let suffix = format!(" ({n})");
        let keep = 31usize.saturating_sub(suffix.chars().count());
        let base: String = name
            .chars()
            .filter(|c| !"\\/?*[]:'".contains(*c))
            .take(keep)
            .collect();
        let base = if base.is_empty() {
            "Sheet".to_owned()
        } else {
            base
        };
        wanted = format!("{base}{suffix}");
    }
}

/// The model of a [`Book`] Trame wrote, for the writer. Each sheet name is
/// brought to Excel's rule, and the cells and the line sizes are put in
/// order. A Trame formula keeps its text for the writer to translate.
#[must_use]
pub fn workbook_of(book: &Book, report: &mut Report) -> Workbook {
    let styles = book
        .styles
        .iter()
        .map(|bytes| cell_style(&postcard::from_bytes(bytes.as_bytes()).unwrap_or_default()))
        .collect();
    let mut taken = Vec::new();
    let sheets = book
        .sheets
        .iter()
        .map(|sheet| {
            let mut out = Sheet::named(excel_name(&sheet.name, &mut taken, report));
            out.visibility = sheet.visibility;
            let mut cells = BTreeMap::new();
            for cell in &sheet.cells {
                let content = match &cell.content {
                    BookContent::Literal(raw) => Content::Value(value_of(raw)),
                    BookContent::Formula {
                        excel: Some(excel), ..
                    } => Content::Formula {
                        formula: Formula::kept(excel.clone()),
                        cached: Value::Empty,
                    },
                    BookContent::Formula {
                        trame: Some(trame),
                        cached,
                        ..
                    } => Content::TrameFormula {
                        formula: trame.clone(),
                        cached: cached.clone(),
                    },
                    BookContent::Formula { cached, .. } => Content::Value(cached.clone()),
                };
                if matches!(content, Content::Value(Value::Empty)) && cell.style == StyleId::DEFAULT
                {
                    continue;
                }
                let style = cell.style;
                cells.insert(
                    (cell.at.row, cell.at.col),
                    crate::model::Cell {
                        at: cell.at,
                        content,
                        style,
                    },
                );
            }
            out.cells = cells.into_values().collect();
            out.merges = sheet.merges.clone();
            let mut widths = sheet.column_widths.clone();
            widths.sort_by_key(|w| w.first);
            out.column_widths = widths;
            let mut heights = sheet.row_heights.clone();
            heights.sort_by_key(|h| h.row);
            out.row_heights = heights;
            out
        })
        .collect();
    Workbook { sheets, styles }
}
