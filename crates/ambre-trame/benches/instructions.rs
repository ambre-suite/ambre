//! Operations whose budget is held by a count of instructions, not by a time.
//!
//! A time measured on a shared CI runner moves by tens of percent between two
//! runs, so a threshold on it fails on noise. An instruction count under
//! `callgrind` is stable, so `scripts/instructions.py` fails the build when an
//! operation outgrows its ceiling (`ambre-spec-testing.md section 2`).
//!
//! A structural edit is counted in two private methods of the state, each by
//! its own name. They are the re-anchor walk and the engine step after the
//! commit. A line delete is the same command counted whole: the purge, the
//! commit and every cache too. Its undo and its redo are each counted whole
//! in the same way. So are those of a row insert, of a delete of `MANY` rows
//! and of one cell write. A remote batch is counted whole, from the import to the events.
//!
//! A line delete while a block of `MANY` rows stays hidden is counted as the
//! line delete is. One slice of the sweep of that block is counted whole.
//!
//! Run with no operation named, as `cargo bench` does, every operation runs
//! once: a smoke test.

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
    // A pinned peer: `TrameState::new` draws one at random, and its ids would
    // move the count from run to run (`ambre-spec-testing.md section 2`).
    let mut st = TrameState::with_peer(ambre_core::PeerId::new(1));
    // One write on the last row materializes every row at once. A paste that
    // grows the sheet one row at a time costs far more under `callgrind`.
    st.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(FORMULAS - 1),
        col: Pos::<Col>::new(1),
        text: "0".to_owned(),
    });
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

/// Delete the middle row. The formulas of the deleted row go with it. No other
/// formula reads a cell of it, so the walk and the engine step carry the
/// removed cells alone.
#[inline(never)]
fn ambre_budget_trame_structural_edit(st: &mut TrameState) {
    black_box(st.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(FORMULAS / 2),
    }));
}

/// One step of the history, tagged. Each operation counts its step in a
/// function of its own, and the tag tells their bodies apart. Two functions of
/// one body fold into one symbol in the build, and the count goes to neither.
#[inline(always)]
fn step(st: &mut TrameState, command: TrameCommand, tag: u8) {
    black_box(tag);
    black_box(st.handle_command(command));
}

/// Undo the delete of the middle row: the row, its two cells and their
/// formula come back.
#[inline(never)]
fn ambre_budget_trame_line_delete_undo(st: &mut TrameState) {
    step(st, TrameCommand::Undo, 1);
}

/// Redo the delete of the middle row.
#[inline(never)]
fn ambre_budget_trame_line_delete_redo(st: &mut TrameState) {
    step(st, TrameCommand::Redo, 2);
}

/// Undo the insert of the middle row.
#[inline(never)]
fn ambre_budget_trame_line_insert_undo(st: &mut TrameState) {
    step(st, TrameCommand::Undo, 3);
}

/// Redo the insert of the middle row.
#[inline(never)]
fn ambre_budget_trame_line_insert_redo(st: &mut TrameState) {
    step(st, TrameCommand::Redo, 4);
}

/// Undo the delete of `MANY` rows.
#[inline(never)]
fn ambre_budget_trame_block_undo(st: &mut TrameState) {
    step(st, TrameCommand::Undo, 5);
}

/// Redo the delete of `MANY` rows.
#[inline(never)]
fn ambre_budget_trame_block_redo(st: &mut TrameState) {
    step(st, TrameCommand::Redo, 6);
}

/// Delete the middle row while `MANY` rows stay hidden: the axis view steps
/// over each hidden slot (`TRAME-308`). The tag keeps its body apart from the
/// line delete's, which would otherwise fold into one symbol.
#[inline(never)]
fn ambre_budget_trame_line_delete_after_block(st: &mut TrameState) {
    black_box(9u8);
    black_box(st.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(FORMULAS / 2),
    }));
}

/// One slice of the sweep of the `MANY` rows another session hid, from the
/// command to its outcome (`TRAME-307`).
#[inline(never)]
fn ambre_budget_trame_sweep_slice(st: &mut TrameState) {
    black_box(st.resume());
}

/// Undo the write of the middle cell.
#[inline(never)]
fn ambre_budget_trame_cell_write_undo(st: &mut TrameState) {
    step(st, TrameCommand::Undo, 7);
}

/// Redo the write of the middle cell.
#[inline(never)]
fn ambre_budget_trame_cell_write_redo(st: &mut TrameState) {
    step(st, TrameCommand::Redo, 8);
}

/// Insert a row in the middle of the sheet: the edit the next two undo.
fn insert_middle_row(st: &mut TrameState) {
    st.handle_command(TrameCommand::InsertRow {
        at: Pos::<Row>::new(FORMULAS / 2),
    });
}

/// Write a number into column A of the middle row, as one edit.
fn write_middle_cell(st: &mut TrameState) {
    st.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(FORMULAS / 2),
        col: Pos::<Col>::new(0),
        text: "7".to_owned(),
    });
    st.handle_command(TrameCommand::CommitEdit);
}

/// Whether the cell of column A on `row` shows `want`.
fn shows_a(st: &mut TrameState, row: u32, want: &str) -> bool {
    let out = st.handle_command(TrameCommand::ScrollViewport {
        version: ambre_core::Seq::nth(u64::MAX),
        row_start: Pos::<Row>::new(row),
        row_end: Pos::<Row>::new(row),
        col_start: Pos::<Col>::new(0),
        col_end: Pos::<Col>::new(0),
    });
    out.display.iter().any(|e| match e {
        TrameEvent::ViewportData { cells, .. } => cells.iter().any(|c| c.text == want),
        _ => false,
    })
}

/// Rows the many-line delete takes out, from the middle of the sheet.
const MANY: u32 = 500;

/// Delete `MANY` rows from the middle of the sheet as one selection: the
/// selection, then the delete, counted alone.
fn delete_many_rows(st: &mut TrameState) {
    select_many_rows(st);
    ambre_budget_trame_block_remove(st);
}

/// The delete of the `MANY` rows the selection holds, counted whole.
#[inline(never)]
fn ambre_budget_trame_block_remove(st: &mut TrameState) {
    black_box(st.handle_command(TrameCommand::DeleteSelectedRows));
}

/// Select `MANY` rows from the middle of the sheet.
fn select_many_rows(st: &mut TrameState) {
    st.handle_command(TrameCommand::SetActiveCell {
        row: Pos::<Row>::new(FORMULAS / 2 + MANY - 1),
        col: Pos::<Col>::new(0),
        anchor_row: Pos::<Row>::new(FORMULAS / 2),
        anchor_col: Pos::<Col>::new(0),
        extra: Vec::new(),
        kind: ambre_trame::SelectionKind::Cells,
    });
}

/// The rows below the inserted one moved down one, each formula with its row.
fn checked_after_insert(st: &mut TrameState) {
    assert!(shows(st, FORMULAS / 2, ""), "the inserted row is blank");
    let middle = (2 * (FORMULAS / 2)).to_string();
    assert!(
        shows(st, FORMULAS / 2 + 1, &middle),
        "the next row moved down"
    );
}

/// The `MANY` rows are gone, and the rows below them moved up.
fn checked_after_many_delete(st: &mut TrameState) {
    let next = (2 * (FORMULAS / 2 + MANY)).to_string();
    assert!(shows(st, FORMULAS / 2, &next), "the rows below moved up");
    let last = (2 * (FORMULAS - 1)).to_string();
    assert!(
        shows(st, FORMULAS - 1 - MANY, &last),
        "the last row moved up"
    );
}

/// The sheet holds every formula the paste wrote, so the edit meets them.
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

/// The batch a peer receives from the author of `sheet()`: one cell edit,
/// then one row delete. The edit writes a number into the column the
/// formulas read, so one formula recomputes.
fn remote_batch(author: &mut TrameState) -> Vec<ambre_core::Payload<ambre_core::DeltaBytes>> {
    let edit = author.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(FORMULAS / 4),
        col: Pos::<Col>::new(0),
        text: "7".to_owned(),
    });
    let delete = author.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(FORMULAS / 2),
    });
    [edit.update, delete.update].into_iter().flatten().collect()
}

/// Apply the batch on the peer: the import, every cache and the events.
#[inline(never)]
fn ambre_budget_trame_remote_batch(
    st: &mut TrameState,
    batch: &[ambre_core::Payload<ambre_core::DeltaBytes>],
) {
    black_box(st.apply_remote_batch(batch));
}

/// Whether the cell of column B on `row` shows `want`.
fn shows(st: &mut TrameState, row: u32, want: &str) -> bool {
    let out = st.handle_command(TrameCommand::ScrollViewport {
        version: ambre_core::Seq::nth(u64::MAX),
        row_start: Pos::<Row>::new(row),
        row_end: Pos::<Row>::new(row),
        col_start: Pos::<Col>::new(1),
        col_end: Pos::<Col>::new(1),
    });
    out.display.iter().any(|e| match e {
        TrameEvent::ViewportData { cells, .. } => cells.iter().any(|c| c.text == want),
        _ => false,
    })
}

/// The peer shows the edit, and the rows below the delete moved up one.
fn checked_after_batch(st: &mut TrameState) {
    assert!(shows(st, FORMULAS / 4, "14"), "the edited cell recomputes");
    let last = (2 * (FORMULAS - 1)).to_string();
    assert!(shows(st, FORMULAS - 2, &last), "the last row moved up one");
}

/// The rows below the deleted one moved up one, each formula with its row.
fn checked_after_delete(st: &mut TrameState) {
    let middle = (2 * (FORMULAS / 2 + 1)).to_string();
    assert!(shows(st, FORMULAS / 2, &middle), "the next row moved up");
    let last = (2 * (FORMULAS - 1)).to_string();
    assert!(shows(st, FORMULAS - 2, &last), "the last row moved up");
}

/// The deleted row came back, and every row below it with it.
fn checked_after_undo(st: &mut TrameState) {
    let middle = (2 * (FORMULAS / 2)).to_string();
    assert!(shows(st, FORMULAS / 2, &middle), "the deleted row is back");
    checked(st);
}

/// Each operation the script may name, with the function that runs it.
const OPERATIONS: &[(&str, fn())] = &[
    ("structural_edit", || {
        let mut st = sheet();
        checked(&mut st);
        ambre_budget_trame_structural_edit(&mut st);
    }),
    ("line_delete", || {
        let mut st = sheet();
        checked(&mut st);
        ambre_budget_trame_structural_edit(&mut st);
        checked_after_delete(&mut st);
    }),
    ("line_delete_undo", || {
        let mut st = sheet();
        checked(&mut st);
        ambre_budget_trame_structural_edit(&mut st);
        ambre_budget_trame_line_delete_undo(&mut st);
        checked_after_undo(&mut st);
    }),
    ("line_delete_redo", || {
        let mut st = sheet();
        checked(&mut st);
        ambre_budget_trame_structural_edit(&mut st);
        ambre_budget_trame_line_delete_undo(&mut st);
        ambre_budget_trame_line_delete_redo(&mut st);
        checked_after_delete(&mut st);
    }),
    ("line_insert_undo", || {
        let mut st = sheet();
        checked(&mut st);
        insert_middle_row(&mut st);
        checked_after_insert(&mut st);
        ambre_budget_trame_line_insert_undo(&mut st);
        checked_after_undo(&mut st);
    }),
    ("line_insert_redo", || {
        let mut st = sheet();
        checked(&mut st);
        insert_middle_row(&mut st);
        ambre_budget_trame_line_insert_undo(&mut st);
        ambre_budget_trame_line_insert_redo(&mut st);
        checked_after_insert(&mut st);
    }),
    ("many_line_delete", || {
        let mut st = sheet();
        checked(&mut st);
        delete_many_rows(&mut st);
        checked_after_many_delete(&mut st);
    }),
    ("many_line_delete_undo", || {
        let mut st = sheet();
        checked(&mut st);
        delete_many_rows(&mut st);
        checked_after_many_delete(&mut st);
        ambre_budget_trame_block_undo(&mut st);
        checked_after_undo(&mut st);
    }),
    ("many_line_delete_redo", || {
        let mut st = sheet();
        checked(&mut st);
        delete_many_rows(&mut st);
        ambre_budget_trame_block_undo(&mut st);
        ambre_budget_trame_block_redo(&mut st);
        checked_after_many_delete(&mut st);
    }),
    ("line_delete_after_block", || {
        let mut st = sheet();
        checked(&mut st);
        delete_many_rows(&mut st);
        ambre_budget_trame_line_delete_after_block(&mut st);
        let next = (2 * (FORMULAS / 2 + MANY + 1)).to_string();
        assert!(shows(&mut st, FORMULAS / 2, &next), "the next row moved up");
    }),
    ("many_line_sweep_slice", || {
        let mut author = sheet();
        checked(&mut author);
        delete_many_rows(&mut author);
        // A second session, which never met the author's user: the block
        // reads as hidden by an ended session.
        let mut st = TrameState::with_peer(ambre_core::PeerId::new(2));
        let opened = st.import_snapshot(&author.export_snapshot());
        assert!(opened.is_ok(), "a snapshot this build wrote reads back");
        st.start_sweep();
        assert!(st.has_pending_work(), "a register is left to sweep");
        ambre_budget_trame_sweep_slice(&mut st);
        assert!(
            st.has_pending_work(),
            "five hundred rows take several slices"
        );
        while st.has_pending_work() {
            st.resume();
        }
        checked_after_many_delete(&mut st);
    }),
    ("cell_write_undo", || {
        let mut st = sheet();
        checked(&mut st);
        write_middle_cell(&mut st);
        ambre_budget_trame_cell_write_undo(&mut st);
        assert!(
            shows_a(&mut st, FORMULAS / 2, &(FORMULAS / 2).to_string()),
            "the old value is back"
        );
    }),
    ("cell_write_redo", || {
        let mut st = sheet();
        checked(&mut st);
        write_middle_cell(&mut st);
        ambre_budget_trame_cell_write_undo(&mut st);
        ambre_budget_trame_cell_write_redo(&mut st);
        assert!(shows_a(&mut st, FORMULAS / 2, "7"), "the write is back");
    }),
    ("remote_batch", || {
        let mut author = sheet();
        let mut peer = TrameState::with_peer(ambre_core::PeerId::new(2));
        let opened = peer.import_snapshot(&author.export_snapshot());
        assert!(opened.is_ok(), "a snapshot this build wrote reads back");
        checked(&mut peer);
        let batch = remote_batch(&mut author);
        assert_eq!(batch.len(), 2, "the edit and the delete each send a delta");
        ambre_budget_trame_remote_batch(&mut peer, &batch);
        checked_after_batch(&mut peer);
    }),
];

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
