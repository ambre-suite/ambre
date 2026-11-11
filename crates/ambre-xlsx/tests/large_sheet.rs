//! A sheet of 100 000 cells, written and read back: half numbers, a quarter
//! texts, a quarter formulas. The test holds the round trip and prints the
//! time of each half, the figures of `ambre-spec-budgets.md` section 1.
//!
//! Run: `cargo test --release -p ambre-xlsx --test large_sheet -- --nocapture`.

#![expect(
    clippy::panic,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use std::time::Instant;

use ambre_core::Validated;
use ambre_core::index::Pos;
use ambre_xlsx::cellref::CellRef;
use ambre_xlsx::formula::Formula;
use ambre_xlsx::model::{Cell, CellStyle, Content, Sheet, SheetName, StyleId, Value, Workbook};

const CELLS: u32 = 100_000;
const COLS: u32 = 10;

fn workbook() -> Workbook {
    let name = Validated::<SheetName>::parse("Data".to_owned()).unwrap_or_else(|e| panic!("{e}"));
    let mut sheet = Sheet::named(name);
    sheet.cells = (0..CELLS)
        .map(|i| {
            let (row, col) = (i / COLS, i % COLS);
            let content = match col % 4 {
                0 | 1 => Content::Value(Value::Number(f64::from(i) * 1.5)),
                2 => Content::Value(Value::Text(format!("item {i}"))),
                _ => Content::Formula {
                    formula: Formula::from_ambre(&format!("=A{}*2+B{}", row + 1, row + 1))
                        .unwrap_or_else(|e| panic!("{e:?}")),
                    cached: Value::Number(f64::from(i)),
                },
            };
            Cell {
                at: CellRef {
                    row: Pos::new(row),
                    col: Pos::new(col),
                },
                content,
                style: StyleId::DEFAULT,
            }
        })
        .collect();
    Workbook {
        sheets: vec![sheet],
        styles: vec![CellStyle::default()],
    }
}

#[test]
fn a_sheet_of_100k_cells_reads_back_unchanged() {
    let book = workbook();
    let start = Instant::now();
    let file = ambre_xlsx::write(&book).unwrap_or_else(|e| panic!("{e}"));
    let written = start.elapsed();
    let start = Instant::now();
    let back = ambre_xlsx::read(&file.bytes).unwrap_or_else(|e| panic!("{e}"));
    let read = start.elapsed();
    println!(
        "{CELLS} cells: {} bytes, write {:.1} ms, read {:.1} ms",
        file.bytes.as_bytes().len(),
        written.as_secs_f64() * 1e3,
        read.as_secs_f64() * 1e3
    );
    assert_eq!(back.workbook, book);
}

#[test]
fn a_sheet_of_100k_cells_reads_back_unchanged_through_ods() {
    let book = workbook();
    let start = Instant::now();
    let file = ambre_xlsx::write_ods(&book).unwrap_or_else(|e| panic!("{e}"));
    let written = start.elapsed();
    let start = Instant::now();
    let back = ambre_xlsx::read_ods(&file.bytes).unwrap_or_else(|e| panic!("{e}"));
    let read = start.elapsed();
    println!(
        "{CELLS} cells as ODS: {} bytes, write {:.1} ms, read {:.1} ms",
        file.bytes.as_bytes().len(),
        written.as_secs_f64() * 1e3,
        read.as_secs_f64() * 1e3
    );
    assert_eq!(back.workbook, book);
}
