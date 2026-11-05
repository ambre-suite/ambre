//! Operations whose budget is held by a count of instructions, not by a time.
//!
//! A time measured on a shared CI runner moves by tens of percent between two
//! runs, so a threshold on it fails on noise. An instruction count under
//! `callgrind` is stable, so `scripts/instructions.py` fails the build when an
//! operation outgrows its ceiling (`ambre-spec-testing.md section 2`).
//!
//! The re-anchor walk is a private method of the state. The script counts
//! inside that method by its own name. The rest of the delete falls outside
//! the count: the purge, and the engine reseed after it. Run with no operation
//! named, as `cargo bench` does, every operation runs once: a smoke test.

use ambre_core::index::{Len, Pos};
use ambre_trame::logic::TrameState;
use ambre_trame::{ClipboardCell, ClipboardPayload, Col, Row, TrameCommand, TrameEvent};
use std::hint::black_box;

/// Formula cells in the sheet the "Trame structural edit" row names.
const FORMULAS: u32 = 10_000;

/// A sheet of `FORMULAS` rows: a number in column A, and in column B a
/// formula that reads the cell beside it. One paste writes it, so the setup
/// stays short under `callgrind`.
fn sheet() -> TrameState {
    let mut cells = Vec::new();
    for row in 0..FORMULAS {
        for (col, raw) in [(0, row.to_string()), (1, format!("=A{}*2", row + 1))] {
            cells.push(ClipboardCell {
                dr: Len::<Row>::new(row),
                dc: Len::<Col>::new(col),
                raw,
                ..ClipboardCell::default()
            });
        }
    }
    let mut st = TrameState::new();
    st.handle_command(TrameCommand::PasteRange {
        target_row: Pos::<Row>::new(0),
        target_col: Pos::<Col>::new(0),
        payload: ClipboardPayload {
            src_row: Pos::<Row>::new(0),
            src_col: Pos::<Col>::new(0),
            rows: Len::<Row>::new(FORMULAS),
            cols: Len::<Col>::new(2),
            cells,
        },
    });
    st
}

/// Delete the middle row: the walk parses each formula of the sheet, and
/// re-anchors none but the one on the deleted row.
#[inline(never)]
fn ambre_budget_trame_reanchor_walk(st: &mut TrameState) {
    black_box(st.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(FORMULAS / 2),
    }));
}

/// The sheet holds every formula the paste wrote, so the walk has its work.
fn checked(st: &mut TrameState) {
    let out = st.handle_command(TrameCommand::ScrollViewport {
        version: ambre_core::Seq::nth(u64::MAX),
        row_start: Pos::<Row>::new(FORMULAS - 1),
        row_end: Pos::<Row>::new(FORMULAS - 1),
        col_start: Pos::<Col>::new(1),
        col_end: Pos::<Col>::new(1),
    });
    let want = (2 * (FORMULAS - 1)).to_string();
    let computed = out.display.iter().any(|e| match e {
        TrameEvent::ViewportData { cells, .. } => cells.iter().any(|c| c.text == want),
        _ => false,
    });
    assert!(computed, "the last formula computes");
}

/// Each operation the script may name, with the function that runs it.
const OPERATIONS: &[(&str, fn())] = &[("reanchor_walk", || {
    let mut st = sheet();
    checked(&mut st);
    ambre_budget_trame_reanchor_walk(&mut st);
})];

fn main() {
    // `cargo bench` passes flags such as `--bench`; only a bare word names an
    // operation.
    let named: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    for (name, run) in OPERATIONS {
        if named.is_empty() || named.iter().any(|n| n == name) {
            run();
        }
    }
}
