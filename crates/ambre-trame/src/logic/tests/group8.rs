// ambre-trame/src/logic/tests/group8.rs
// References stored by identity: the stored form, and what each structural
// edit does to it (`TRAME-008`, `TRAME-171`, `TRAME-226` to `TRAME-230`).

use super::*;

fn set(state: &mut TrameState, row: u32, col: u32, text: &str) {
    state.handle_command(TrameCommand::SetCellText {
        row,
        col,
        text: text.to_owned(),
    });
}

/// A column of numbers in A, from row 1 down.
fn column(values: &[&str]) -> TrameState {
    let mut state = TrameState::new();
    for (row, value) in values.iter().enumerate() {
        set(&mut state, row as u32, 0, value);
    }
    state
}

/// `TRAME-228`: a reference to a materialized cell is stored by its row and
/// column, never by its position. One past the used range stays a position.
#[test]
fn a_stored_formula_names_its_lines() {
    let mut state = column(&["1", "2"]);
    set(&mut state, 0, 1, "=A2+Z99");
    let stored = state.cell_raw(0, 1);
    assert!(
        !stored.contains("A2"),
        "no position for a used cell: {stored}"
    );
    assert!(stored.starts_with("={"), "a line pair: {stored}");
    assert!(
        stored.ends_with("+Z99"),
        "a virtual cell stays a position: {stored}"
    );
    assert_eq!(
        read_raw(&mut state, 0, 1),
        "=A2+Z99",
        "the bar reads positions"
    );
    assert_eq!(read_cell(&mut state, 0, 1), "2");
}

/// `TRAME-171`: a row inserted inside a range widens it, and the new row's
/// value counts.
#[test]
fn an_insert_inside_a_range_widens_it() {
    let mut state = column(&["1", "2", "3"]);
    set(&mut state, 0, 1, "=SUM(A1:A3)");
    assert_eq!(read_cell(&mut state, 0, 1), "6");
    state.handle_command(TrameCommand::InsertRow { at: 1 });
    set(&mut state, 1, 0, "10");
    assert_eq!(read_raw(&mut state, 0, 1), "=SUM(A1:A4)");
    assert_eq!(read_cell(&mut state, 0, 1), "16");
}

/// `TRAME-230`: a delete shrinks a range, and a range left with no line
/// reads `#REF!`.
#[test]
fn a_delete_shrinks_a_range_then_breaks_it() {
    let mut state = column(&["1", "2", "3", "4"]);
    set(&mut state, 0, 1, "=SUM(A2:A4)");
    assert_eq!(read_cell(&mut state, 0, 1), "9");
    // The range's last row goes: the end corner moves up.
    state.handle_command(TrameCommand::DeleteRow { at: 3 });
    assert_eq!(read_raw(&mut state, 0, 1), "=SUM(A2:A3)");
    assert_eq!(read_cell(&mut state, 0, 1), "5");
    // Its first row goes: the start corner moves down.
    state.handle_command(TrameCommand::DeleteRow { at: 1 });
    assert_eq!(read_raw(&mut state, 0, 1), "=SUM(A2:A2)");
    assert_eq!(read_cell(&mut state, 0, 1), "3");
    state.handle_command(TrameCommand::DeleteRow { at: 1 });
    assert_eq!(read_cell(&mut state, 0, 1), "#REF!");
}

/// `TRAME-227`: a sort keeps a range on its block of positions, while a
/// reference to one cell follows that cell's row.
#[test]
fn a_sort_keeps_the_range_and_moves_the_cell() {
    let mut state = column(&["3", "1", "2"]);
    set(&mut state, 4, 1, "=SUM(A1:A2)");
    set(&mut state, 4, 2, "=A1");
    assert_eq!(read_cell(&mut state, 4, 1), "4");
    state.handle_command(TrameCommand::SortRange {
        sheet_id: TEST_SHEET.to_owned(),
        row_start: 0,
        row_end: 2,
        col_start: 0,
        col_end: 0,
        sort_col: 0,
        ascending: true,
        has_header: false,
    });
    assert_eq!(read_raw(&mut state, 4, 1), "=SUM(A1:A2)", "the block stays");
    assert_eq!(read_cell(&mut state, 4, 1), "3", "and sums what is in it");
    assert_eq!(
        read_raw(&mut state, 4, 2),
        "=A3",
        "the cell moved with its row"
    );
    assert_eq!(read_cell(&mut state, 4, 2), "3");
}

/// `TRAME-229`: one peer types `=A2` while another inserts a row above A2.
/// After the merge, both read the cell the formula was typed against.
#[test]
fn a_reference_typed_during_a_concurrent_insert_keeps_its_cell() {
    let mut a = column(&["1", "7", "9"]);
    let mut b = TrameState::new();
    b.import_snapshot(&a.export_snapshot())
        .expect("a snapshot this build wrote reads back");

    let typed = a.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=A2".to_owned(),
    });
    let inserted = b.handle_command(TrameCommand::InsertRow { at: 1 });
    a.apply_remote(&inserted.update.expect("the insert broadcasts"));
    b.apply_remote(&typed.update.expect("the formula broadcasts"));

    for (peer, state) in [("a", &mut a), ("b", &mut b)] {
        assert_eq!(read_cell(state, 0, 1), "7", "{peer} reads the typed cell");
        assert_eq!(read_raw(state, 0, 1), "=A3", "{peer} shows where it went");
    }
}

/// `TRAME-229`, the delete twin: one peer types `=A3` while another deletes
/// row 1. After the merge, both read the cell the formula was typed against.
#[test]
fn a_reference_typed_during_a_concurrent_delete_keeps_its_cell() {
    let mut a = column(&["1", "7", "9"]);
    let mut b = TrameState::new();
    b.import_snapshot(&a.export_snapshot())
        .expect("a snapshot this build wrote reads back");

    let typed = a.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=A3".to_owned(),
    });
    let deleted = b.handle_command(TrameCommand::DeleteRow { at: 1 });
    a.apply_remote(&deleted.update.expect("the delete broadcasts"));
    b.apply_remote(&typed.update.expect("the formula broadcasts"));

    for (peer, state) in [("a", &mut a), ("b", &mut b)] {
        assert_eq!(read_cell(state, 0, 1), "9", "{peer} reads the typed cell");
        assert_eq!(read_raw(state, 0, 1), "=A2", "{peer} shows where it went");
    }
}

/// `TRAME-215`: a delete leaves the stored text of a reference to a cell
/// untouched; only its displayed position moves.
#[test]
fn a_delete_leaves_a_cell_reference_untouched() {
    let mut state = column(&["1", "2", "3", "4"]);
    set(&mut state, 0, 1, "=A4*2");
    let stored = state.cell_raw(0, 1);
    state.handle_command(TrameCommand::DeleteRow { at: 1 });
    assert_eq!(state.cell_raw(0, 1), stored, "the stored text is the same");
    assert_eq!(read_raw(&mut state, 0, 1), "=A3*2");
    assert_eq!(read_cell(&mut state, 0, 1), "8");
}

/// `TRAME-226`: a version-2 workbook stored `=A2`. It opens with the formula
/// naming the cell it read, so a later insert above it no longer moves it.
#[test]
fn a_positional_workbook_migrates_onto_its_cells() {
    use crate::logic::schema::{SCHEMA_VERSION, cells_map_name};
    use crate::model::cell_key;
    use ambre_core::schema::{SCHEMA_META_MAP, SCHEMA_VERSION_KEY};

    let mut old = column(&["1", "7", "9"]);
    set(&mut old, 0, 1, "=SUM(A2:A3)+A2");
    // Write the cell back as version 2 stored it: positions, and the number.
    let (rl, cl) = old.index.resolve(0, 1).expect("B1 is materialized");
    old.doc
        .get_map(cells_map_name(TEST_SHEET))
        .insert(&cell_key(rl, cl), "=SUM(A2:A3)+A2")
        .expect("the positional text");
    old.doc
        .get_map(SCHEMA_META_MAP)
        .insert(SCHEMA_VERSION_KEY, 2i64)
        .expect("the old version");
    old.doc.commit();

    let mut state = TrameState::new();
    state
        .import_snapshot(&old.export_snapshot())
        .expect("version 2 migrates");
    assert_eq!(state.schema_version(), SCHEMA_VERSION);
    let stored = state.cell_raw(0, 1);
    assert!(
        !stored.contains("A2"),
        "the migration named the lines: {stored}"
    );
    assert_eq!(read_cell(&mut state, 0, 1), "23");

    state.handle_command(TrameCommand::InsertRow { at: 1 });
    assert_eq!(read_raw(&mut state, 0, 1), "=SUM(A3:A4)+A3");
    assert_eq!(read_cell(&mut state, 0, 1), "23");
}

/// `TRAME-048`: a whole-column range reads the used part of its column. A
/// value written below the last row, or in a row inserted above, is summed.
#[test]
fn a_whole_column_range_reads_the_used_rows() {
    let mut state = column(&["1", "2", "3"]);
    set(&mut state, 0, 1, "=SUM(A:A)");
    assert_eq!(read_cell(&mut state, 0, 1), "6");
    let stored = state.cell_raw(0, 1);
    assert!(stored.contains("{*|"), "rows are stored whole: {stored}");
    assert_eq!(read_raw(&mut state, 0, 1), "=SUM(A:A)");
    set(&mut state, 99, 0, "10");
    assert_eq!(read_cell(&mut state, 0, 1), "16", "a row past the old end");
    state.handle_command(TrameCommand::InsertRow { at: 0 });
    set(&mut state, 0, 0, "100");
    assert_eq!(read_raw(&mut state, 1, 1), "=SUM(A:A)");
    assert_eq!(read_cell(&mut state, 1, 1), "116", "a row inserted above");
}

/// `TRAME-048`: a whole-row range reads the used part of its rows.
#[test]
fn a_whole_row_range_reads_the_used_columns() {
    let mut state = TrameState::new();
    set(&mut state, 1, 0, "1");
    set(&mut state, 2, 3, "2");
    set(&mut state, 0, 0, "=SUM(2:3)");
    assert_eq!(read_cell(&mut state, 0, 0), "3");
    set(&mut state, 1, 40, "4");
    assert_eq!(
        read_cell(&mut state, 0, 0),
        "7",
        "a column past the old end"
    );
    assert_eq!(read_raw(&mut state, 0, 0), "=SUM(2:3)");
}

/// `TRAME-048`: a column insert or delete moves a whole-column range as it
/// moves a range. An insert inside widens it and a delete shrinks it. With no
/// column left it reads `#REF!`.
#[test]
fn a_column_edit_carries_a_whole_column_range() {
    let mut state = column(&["1", "2"]);
    set(&mut state, 0, 1, "3");
    set(&mut state, 5, 3, "=SUM(A:B)");
    assert_eq!(read_cell(&mut state, 5, 3), "6");
    state.handle_command(TrameCommand::InsertColumn { at: 1 });
    assert_eq!(
        read_raw(&mut state, 5, 4),
        "=SUM(A:C)",
        "an insert inside widens"
    );
    assert_eq!(read_cell(&mut state, 5, 4), "6");
    state.handle_command(TrameCommand::DeleteColumn { at: 0 });
    assert_eq!(read_raw(&mut state, 5, 3), "=SUM(A:B)");
    assert_eq!(read_cell(&mut state, 5, 3), "3");
    state.handle_command(TrameCommand::DeleteColumn { at: 0 });
    state.handle_command(TrameCommand::DeleteColumn { at: 0 });
    assert_eq!(read_cell(&mut state, 5, 1), "#REF!");
}

/// `TRAME-042`: a whole-column range over its own cell closes a loop the cell
/// graph cannot see. The evaluator catches it and settles.
#[test]
fn a_whole_column_range_over_itself_is_a_cycle() {
    let mut state = column(&["1", "2"]);
    set(&mut state, 2, 0, "=SUM(A:A)");
    assert_eq!(read_cell(&mut state, 2, 0), "#CIRCULAR!");
    assert_eq!(read_cell(&mut state, 0, 0), "1");
}
