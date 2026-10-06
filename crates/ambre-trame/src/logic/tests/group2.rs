// ambre-trame/src/logic/tests/group2.rs
// TrameState unit tests (group 2/7). Helpers via `use super::*`;
// logic-module items are reached with `super::super::` paths.

use super::*;

/// A multi-cell batch (Ctrl+Enter fill / Delete over a range) is a SINGLE
/// undo item. Every write shares one undo group, so one `Undo` restores all
/// the affected cells at once.
#[test]
fn batch_write_is_one_undo_item() {
    let mut state = TrameState::new();
    let undo_before = state.undo_manager.undo_count();

    state.handle_command(TrameCommand::SetCellsBatch {
        cells: vec![
            (0, 0, "x".to_owned()),
            (0, 1, "x".to_owned()),
            (1, 0, "x".to_owned()),
        ],
    });
    // Seal the group (cell-exit boundary), like the UI's next command would.
    state.handle_command(TrameCommand::CommitEdit);

    assert_eq!(read_cell(&mut state, 0, 0), "x");
    assert_eq!(read_cell(&mut state, 1, 0), "x");
    assert_eq!(
        state.undo_manager.undo_count(),
        undo_before + 1,
        "the whole batch collapses into a single undo item",
    );

    // One undo clears every cell of the batch.
    state.handle_command(TrameCommand::Undo);
    assert_eq!(
        read_cell(&mut state, 0, 0),
        "",
        "one undo restores the whole batch"
    );
    assert_eq!(read_cell(&mut state, 0, 1), "");
    assert_eq!(read_cell(&mut state, 1, 0), "");
}

/// A cell edit that also rewrites the active filter's hidden set commits the
/// `LoroDoc` twice (the cell write + the filter recompute). Yet the edit must
/// stay a single undo item (`trame-cell-editing`). `SetCellText` opens a group
/// that `CommitEdit` seals on cell exit, so both commits collapse into one
/// step. A single `Undo` restores the pre-edit value and the prior hidden set.
#[test]
fn filtered_column_edit_is_one_undo_item() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["1", "2", "3"]);
    // Hide every row whose column-A value is < 3 -> rows 0 and 1 hidden.
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: "s0".to_owned(),
        col: 0,
        kind: FilterKind::Condition {
            op: FilterOp::Gte,
            value: "3".to_owned(),
        },
    });
    let undo_before = state.undo_manager.undo_count();

    // Edit a filtered cell so the hidden set is recomputed (a second commit),
    // then exit the editor.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "9".to_owned(),
    });
    state.handle_command(TrameCommand::CommitEdit);

    assert_eq!(
        state.undo_manager.undo_count(),
        undo_before + 1,
        "cell write + filter recompute collapse into a single undo item",
    );

    state.handle_command(TrameCommand::Undo);
    assert_eq!(
        read_cell(&mut state, 0, 0),
        "1",
        "one undo restores the edited cell"
    );
}

/// Every mutating command refreshes the Undo/Redo availability, so the
/// chrome enables the Undo button right after the first edit. It does not
/// wait for the user to trigger an undo. A non-mutating command emits no
/// `UndoState`.
#[test]
fn mutating_command_reports_undo_state() {
    let mut state = TrameState::new();

    let evts = state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "x".to_owned(),
    });
    assert!(
        matches!(
            evts.display.last(),
            Some(TrameEvent::UndoState {
                can_undo: true,
                can_redo: false
            }),
        ),
        "an edit enables Undo"
    );

    // A pure selection move produces no CRDT delta -> no UndoState refresh.
    let evts = state.handle_command(TrameCommand::SetActiveCell {
        row: 1,
        col: 1,
        anchor_row: 1,
        anchor_col: 1,
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    assert!(
        !evts
            .display
            .iter()
            .any(|e| matches!(e, TrameEvent::UndoState { .. })),
        "a non-mutating command does not touch the history stacks",
    );
}

/// Undo with an empty stack is a no-op: no `DataReset`, no broadcast, just the
/// (unchanged) availability so the chrome can disable the button.
#[test]
fn undo_with_empty_stack_is_a_noop() {
    let mut state = TrameState::new();
    let evts = state.handle_command(TrameCommand::Undo);
    assert!(evts.update.is_none());
    assert_eq!(evts.display.len(), 1);
    assert!(matches!(
        evts.display[0],
        TrameEvent::UndoState {
            can_undo: false,
            can_redo: false
        }
    ));
}

/// Undo is local-only (`trame-undo`): `UndoManager` tracks only this peer's
/// commits, never imported remote ops. Peer B writes its own cell, receives a
/// remote edit from A, then undoes. Only B's own edit is reverted; A's remote
/// value stays put (Google Sheets / collaborative-editor behavior).
#[test]
fn undo_local_only_not_remote() {
    // Shared base so both peers address the same row/column uids: materialize
    // A1 and B1 with placeholders before either peer diverges.
    let mut base = TrameState::new();
    base.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "base_a".to_owned(),
    });
    base.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "base_b".to_owned(),
    });
    let base_snap = base.export_snapshot();

    let mut a = TrameState::new();
    a.import_snapshot(&base_snap)
        .expect("a snapshot this build wrote reads back at the current schema");
    let mut b = TrameState::new();
    b.import_snapshot(&base_snap)
        .expect("a snapshot this build wrote reads back at the current schema");

    // A edits A1; B edits B1 locally.
    a.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "alice".to_owned(),
    });
    b.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "bob".to_owned(),
    });

    // B receives A's edit (the remote op enters B's doc but not its undo stack).
    // A full snapshot carries A's history from the shared base, so importing it
    // yields the union without dangling deps.
    b.apply_remote(&a.export_snapshot());
    assert_eq!(read_cell(&mut b, 0, 0), "alice", "B sees A's remote value");
    assert_eq!(read_cell(&mut b, 0, 1), "bob", "B sees its own value");

    // B undoes: only its own last local op (B1) is reverted; A1 is untouched.
    b.handle_command(TrameCommand::Undo);
    assert_eq!(
        read_cell(&mut b, 0, 0),
        "alice",
        "remote op is never undone"
    );
    assert_eq!(
        read_cell(&mut b, 0, 1),
        "base_b",
        "only the local op is reverted"
    );
}

/// Undoing an `InsertRow` must rebuild the `SheetIndex`, so positions resolve
/// against the restored row order. The row that had shifted down returns to its
/// original position, and the inserted blank row is gone.
#[test]
fn undo_insert_row_rebuilds_sheet_index() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "top".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "mid".to_owned(),
    });

    // Insert a blank row at 1: "mid" shifts down to row 2.
    state.handle_command(TrameCommand::InsertRow { at: 1 });
    assert_eq!(read_cell(&mut state, 2, 0), "mid", "mid shifted down");

    // Undo: the index must rebuild so "mid" resolves back at row 1 and the
    // inserted blank row no longer occupies a position.
    let evts = state.handle_command(TrameCommand::Undo);
    assert!(matches!(evts.display[0], TrameEvent::DataReset));
    assert_eq!(read_cell(&mut state, 0, 0), "top");
    assert_eq!(read_cell(&mut state, 1, 0), "mid", "row order restored");
    assert_eq!(read_cell(&mut state, 2, 0), "", "inserted row removed");
}

/// The undo stack lives for the open-document session only (`trame-undo`: "no
/// undo stack persistence between sessions"). Reopening a document from its
/// persisted snapshot starts with a fresh, empty `UndoManager`. A prior edit is
/// not undoable, so Ctrl+Z is a no-op on the reopened doc.
#[test]
fn undo_stack_cleared_on_doc_close() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "hello".to_owned(),
    });
    assert!(state.undo_manager.can_undo(), "edit is undoable in-session");

    // Close + reopen: persist a snapshot, restore into a fresh state.
    let snapshot = state.export_snapshot();
    let mut reopened = TrameState::new();
    reopened
        .import_snapshot(&snapshot)
        .expect("a snapshot this build wrote reads back at the current schema");

    assert!(
        !reopened.undo_manager.can_undo(),
        "undo history does not survive a reopen"
    );
    // The value is intact (data persisted), but Ctrl+Z cannot revert it.
    assert_eq!(read_cell(&mut reopened, 0, 0), "hello");
    let evts = reopened.handle_command(TrameCommand::Undo);
    assert!(evts.update.is_none(), "no inverse op on an empty stack");
    assert_eq!(read_cell(&mut reopened, 0, 0), "hello", "value unchanged");
}

/// Inserting a row shifts the cells below it down by one position; cells are
/// keyed by stable ids so their content follows the shift.
#[test]
fn insert_row_shifts_cells_down() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "top".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "mid".to_owned(),
    });

    let evts = state.handle_command(TrameCommand::InsertRow { at: 1 });
    assert!(matches!(evts.display[0], TrameEvent::DataReset));

    assert_eq!(read_cell(&mut state, 0, 0), "top", "row above stays put");
    assert_eq!(read_cell(&mut state, 1, 0), "", "new blank row is empty");
    assert_eq!(
        read_cell(&mut state, 2, 0),
        "mid",
        "old row 1 shifted to row 2"
    );
}

/// Deleting a row shifts the rows below it up and drops the deleted row's
/// cell content (no orphan entry resurfaces).
#[test]
fn delete_row_shifts_cells_up_and_purges() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "keep".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "gone".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 2,
        col: 0,
        text: "last".to_owned(),
    });

    state.handle_command(TrameCommand::DeleteRow { at: 1 });

    assert_eq!(read_cell(&mut state, 0, 0), "keep");
    assert_eq!(
        read_cell(&mut state, 1, 0),
        "last",
        "row 2 shifted up to row 1"
    );
    // The purged content is gone for good, even after a snapshot round-trip.
    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored
        .import_snapshot(&snapshot)
        .expect("a snapshot this build wrote reads back at the current schema");
    assert_eq!(read_cell(&mut restored, 0, 0), "keep");
    assert_eq!(read_cell(&mut restored, 1, 0), "last");
}

/// Column insert/delete mirror the row behavior on the horizontal axis.
#[test]
fn insert_and_delete_column_shift_cells() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "a".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "b".to_owned(),
    });

    state.handle_command(TrameCommand::InsertColumn { at: 1 });
    assert_eq!(read_cell(&mut state, 0, 0), "a");
    assert_eq!(read_cell(&mut state, 0, 1), "", "inserted blank column");
    assert_eq!(read_cell(&mut state, 0, 2), "b", "old col 1 shifted right");

    state.handle_command(TrameCommand::DeleteColumn { at: 1 });
    assert_eq!(read_cell(&mut state, 0, 0), "a");
    assert_eq!(read_cell(&mut state, 0, 1), "b", "shift back after delete");
}

/// Drag reorder moves a whole row by uid: its cells travel with it. A formula referencing the
/// moved data follows it to the new position (`trame-formula-engine`). That is the same guarantee
/// as sort: no reference rewrite, no reseed.
#[test]
fn move_row_carries_cells_and_formula_follows_data() {
    let mut state = TrameState::new();
    for (r, t) in ["a", "b", "c"].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: (*t).to_owned(),
        });
    }
    // B1 references A1 ("a"). After moving row 0 down to position 2 the value
    // "a" lives at A3, and the formula must resolve to it at its new home.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=A1".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 1), "a", "formula before move");

    state.handle_command(TrameCommand::MoveRow { from: 0, to: 2 });
    // Order is now b, c, a on column 0; the formula traveled with row "a".
    assert_eq!(read_cell(&mut state, 0, 0), "b");
    assert_eq!(read_cell(&mut state, 1, 0), "c");
    assert_eq!(read_cell(&mut state, 2, 0), "a", "moved data at new row");
    assert_eq!(
        read_cell(&mut state, 2, 1),
        "a",
        "formula followed its row and still reads its source"
    );
}

/// Column drag reorder is the horizontal analogue: cells travel by col uid.
#[test]
fn move_column_carries_cells() {
    let mut state = TrameState::new();
    for (c, t) in ["x", "y", "z"].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: 0,
            col: c as u32,
            text: (*t).to_owned(),
        });
    }
    state.handle_command(TrameCommand::MoveColumn { from: 2, to: 0 });
    assert_eq!(read_cell(&mut state, 0, 0), "z", "moved column at front");
    assert_eq!(read_cell(&mut state, 0, 1), "x");
    assert_eq!(read_cell(&mut state, 0, 2), "y");
}

/// A reference names its row and column, so deleting the row that held a
/// formula's source breaks it to `#REF!`. It never reads the cell that slid
/// up into the position (`TRAME-008`).
#[test]
fn a_reference_to_a_deleted_row_reads_ref() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "=7".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=A2".to_owned(),
    });
    assert_eq!(
        read_cell(&mut state, 0, 1),
        "7",
        "B1 reads A2 before delete"
    );

    // Something sits below the source, ready to slide into its position.
    state.handle_command(TrameCommand::SetCellText {
        row: 2,
        col: 0,
        text: "9".to_owned(),
    });
    // Delete row 1 (the source A2): B1 breaks, it does not read the 9.
    state.handle_command(TrameCommand::DeleteRow { at: 1 });
    assert_eq!(read_cell(&mut state, 0, 1), "#REF!", "deleted source");
    assert_eq!(read_raw(&mut state, 0, 1), "=#REF!", "the bar shows it");
}

/// Inserting past the materialized used range is a no-op: virtual rows are
/// already blank, there is nothing to shift, and no CRDT delta is produced.
#[test]
fn insert_beyond_used_range_is_noop() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "x".to_owned(),
    });
    let evts = state.handle_command(TrameCommand::InsertRow { at: 50 });
    assert!(evts.display.is_empty(), "no DataReset");
    assert!(evts.update.is_none(), "no CRDT delta");
    assert_eq!(read_cell(&mut state, 0, 0), "x");
}

/// Merging a range marks the top-left master with its span and renders the
/// covered cells blank; the merge survives a snapshot round-trip.
#[test]
fn merge_marks_master_span_and_blanks_covered() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "title".to_owned(),
    });

    // Merge A1:B2 (2 cols x 2 rows), master = A1.
    let evts = state.handle_command(TrameCommand::MergeCells {
        row: 0,
        col: 0,
        row_end: 1,
        col_end: 1,
    });
    assert!(
        evts.display
            .iter()
            .any(|e| matches!(e, TrameEvent::DataReset))
    );

    let master = read_display(&mut state, 0, 0);
    assert_eq!(master.merge_span, Some((2, 2)), "master spans 2x2");
    assert!(master.is_merged);
    assert_eq!(master.text, "title");

    let covered = read_display(&mut state, 1, 1);
    assert!(covered.is_merged, "covered cell flags merged");
    assert_eq!(covered.merge_span, None, "covered cell has no span");
    assert_eq!(covered.text, "", "covered cell renders blank");

    // Round-trip: the merge is persisted in the CRDT.
    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored
        .import_snapshot(&snapshot)
        .expect("a snapshot this build wrote reads back at the current schema");
    assert_eq!(read_display(&mut restored, 0, 0).merge_span, Some((2, 2)));
}

/// A merge is rejected locally when a covered cell is non-empty, and no CRDT
/// delta is produced (Excel/Sheets behavior, `trame-cell-merges`).
#[test]
fn merge_rejected_when_covered_cell_non_empty() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "busy".to_owned(),
    });

    let evts = state.handle_command(TrameCommand::MergeCells {
        row: 0,
        col: 0,
        row_end: 0,
        col_end: 1,
    });
    assert!(
        matches!(&evts.display[0], TrameEvent::MergeRejected { .. }),
        "non-empty covered cell rejects the merge"
    );
    assert!(evts.update.is_none(), "no CRDT mutation on reject");
    // The cells stay independent.
    assert_eq!(read_display(&mut state, 0, 0).merge_span, None);
}

/// Unmerging restores the covered cells (span and blanking cleared).
#[test]
fn unmerge_restores_covered_cells() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::MergeCells {
        row: 0,
        col: 0,
        row_end: 0,
        col_end: 2,
    });
    assert_eq!(read_display(&mut state, 0, 0).merge_span, Some((3, 1)));

    // Unmerge by pointing at any cell inside the range.
    state.handle_command(TrameCommand::UnmergeCells { row: 0, col: 2 });
    assert_eq!(read_display(&mut state, 0, 0).merge_span, None);
    assert!(
        !read_display(&mut state, 0, 1).is_merged,
        "covered cell freed"
    );
}

/// The parameterless structural intents (grid context menu) resolve their
/// geometry against the active cell / selection the Logic thread tracks.
/// `InsertRowAbove` / `InsertRowBelow` land relative to the active row.
#[test]
fn insert_intents_land_relative_to_active_cell() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "r0".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "r1".to_owned(),
    });

    // Active cell = row 1; "insert above" pushes r1 down to row 2.
    state.handle_command(TrameCommand::SetActiveCell {
        row: 1,
        col: 0,
        anchor_row: 1,
        anchor_col: 0,
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    state.handle_command(TrameCommand::InsertRowAbove);
    assert_eq!(
        read_cell(&mut state, 0, 0),
        "r0",
        "row above the active stays"
    );
    assert_eq!(
        read_cell(&mut state, 1, 0),
        "",
        "blank row inserted at the active row"
    );
    assert_eq!(read_cell(&mut state, 2, 0), "r1", "active row shifted down");
}

/// `DeleteSelectedRows` drops every row spanned by the selection rectangle.
#[test]
fn delete_selected_rows_drops_the_whole_span() {
    let mut state = TrameState::new();
    for (r, t) in [(0, "keep"), (1, "a"), (2, "b"), (3, "c"), (4, "tail")] {
        state.handle_command(TrameCommand::SetCellText {
            row: r,
            col: 0,
            text: t.to_owned(),
        });
    }
    // Select rows 1..=3 (active corner row 3, anchor row 1).
    state.handle_command(TrameCommand::SetActiveCell {
        row: 3,
        col: 0,
        anchor_row: 1,
        anchor_col: 0,
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    state.handle_command(TrameCommand::DeleteSelectedRows);
    assert_eq!(read_cell(&mut state, 0, 0), "keep");
    assert_eq!(
        read_cell(&mut state, 1, 0),
        "tail",
        "rows 1..=3 gone, row 4 shifted up"
    );
    assert_eq!(read_cell(&mut state, 2, 0), "");
}

/// Five rows selected: seeded, then the selection set over them.
fn five_selected_rows() -> TrameState {
    let mut state = TrameState::new();
    for r in 0..6 {
        state.handle_command(TrameCommand::SetCellText {
            row: r,
            col: 0,
            text: format!("r{r}"),
        });
    }
    state.handle_command(TrameCommand::CommitEdit);
    state.handle_command(TrameCommand::SetActiveCell {
        row: 4,
        col: 0,
        anchor_row: 0,
        anchor_col: 0,
        extra: vec![],
        kind: crate::SelectionKind::Rows,
    });
    state
}

/// Deleting a selection of lines undoes as one step (`TRAME-088`).
#[test]
fn delete_selected_rows_undoes_as_one_step() {
    let mut state = five_selected_rows();
    state.handle_command(TrameCommand::DeleteSelectedRows);
    assert_eq!(read_cell(&mut state, 0, 0), "r5");
    state.handle_command(TrameCommand::Undo);
    let restored: Vec<String> = (0..6).map(|r| read_cell(&mut state, r, 0)).collect();
    assert_eq!(restored, vec!["r0", "r1", "r2", "r3", "r4", "r5"]);
}

/// Every line a selection deletes reaches the collaborators (`TRAME-175`):
/// the one delta the command returns carries all five.
#[test]
fn delete_selected_rows_reaches_peers_whole() {
    let mut a = five_selected_rows();
    let mut b = TrameState::new();
    b.import_snapshot(&a.export_snapshot())
        .expect("a snapshot this build wrote reads back at the current schema");
    let outcome = a.handle_command(TrameCommand::DeleteSelectedRows);
    let update = outcome.update.expect("the delete broadcasts");
    b.apply_remote(&update);
    assert_eq!(read_cell(&mut b, 0, 0), "r5");
    assert_eq!(read_cell(&mut b, 1, 0), "");
}

/// A deleted row leaves none of its formatting behind (`TRAME-082`): its
/// height and its filter mark go with it, as a deleted column's width does.
#[test]
fn deleted_line_leaves_no_formatting() {
    use crate::logic::schema::{col_widths_map_name, filter_hidden_map_name, row_heights_map_name};
    let mut state = TrameState::new();
    for r in 0..3 {
        state.handle_command(TrameCommand::SetCellText {
            row: r,
            col: 0,
            text: format!("{r}"),
        });
    }
    state.handle_command(TrameCommand::ResizeRow {
        version: 1,
        sheet_id: TEST_SHEET.to_owned(),
        row: 1,
        height_px: 40.0,
    });
    state.handle_command(TrameCommand::ResizeCol {
        version: 2,
        sheet_id: TEST_SHEET.to_owned(),
        col: 0,
        width_px: 150.0,
    });
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: TEST_SHEET.to_owned(),
        col: 0,
        kind: FilterKind::Values(vec!["0".to_owned(), "2".to_owned()]),
    });
    let len = |state: &TrameState, name: String| state.doc.get_map(name).len();
    assert_eq!(len(&state, row_heights_map_name(TEST_SHEET)), 1);
    assert_eq!(
        len(&state, filter_hidden_map_name(TEST_SHEET)),
        1,
        "row 1 is hidden"
    );
    state.handle_command(TrameCommand::DeleteRow { at: 1 });
    assert_eq!(len(&state, row_heights_map_name(TEST_SHEET)), 0);
    assert_eq!(len(&state, filter_hidden_map_name(TEST_SHEET)), 0);
    state.handle_command(TrameCommand::ClearFilter {
        sheet_id: TEST_SHEET.to_owned(),
    });
    state.handle_command(TrameCommand::DeleteColumn { at: 0 });
    assert_eq!(len(&state, col_widths_map_name(TEST_SHEET)), 0);
}

/// `MergeSelection` merges the tracked selection; `UnmergeActive` splits the
/// merge under the active cell.
#[test]
fn merge_and_unmerge_intents_use_the_selection() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "title".to_owned(),
    });
    // Select A1:B2.
    state.handle_command(TrameCommand::SetActiveCell {
        row: 1,
        col: 1,
        anchor_row: 0,
        anchor_col: 0,
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    state.handle_command(TrameCommand::MergeSelection);
    assert_eq!(
        read_display(&mut state, 0, 0).merge_span,
        Some((2, 2)),
        "selection merged"
    );

    // Active cell is inside the merge; UnmergeActive frees it.
    state.handle_command(TrameCommand::UnmergeActive);
    assert_eq!(
        read_display(&mut state, 0, 0).merge_span,
        None,
        "merge split"
    );
}
