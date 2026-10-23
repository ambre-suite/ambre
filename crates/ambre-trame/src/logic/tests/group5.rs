// ambre-trame/src/logic/tests/group5.rs
// TrameState unit tests (group 5/7). Helpers via `use super::*`;
// logic-module items are reached with `super::super::` paths.

use super::*;
use crate::logic::schema::{SheetContainer, SheetRoot};

/// Hidden rows are excluded from the viewport but still counted by `=SUM`
/// (`trame-shared-filter`): the filter is a display concern, not a data one.
#[test]
fn shared_filter_does_not_affect_sum() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["1", "2", "3"]);
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=SUM(A1:A3)".to_owned(),
    });

    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(0),
        kind: FilterKind::Condition {
            op: FilterOp::Gt,
            value: "1".to_owned(),
        },
    });
    assert!(is_filtered(&mut state, 0, 0), "row with 1 is hidden");
    assert_eq!(
        read_cell(&mut state, 0, 1),
        "6",
        "SUM still totals every row"
    );
}

/// Clearing the filter unhides every row and broadcasts the removal; clearing
/// again with no active filter is a no-op (no delta).
#[test]
fn clear_filter_unhides_and_noops_when_absent() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["a", "b"]);

    // No filter yet -> clear is a pure no-op.
    let noop = state.handle_command(TrameCommand::ClearFilter {
        sheet_id: SheetId("s0".to_owned()),
    });
    assert!(noop.display.is_empty());
    assert!(noop.update.is_none());

    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(0),
        kind: FilterKind::Values(vec!["a".to_owned()]),
    });
    assert!(is_filtered(&mut state, 1, 0), "b hidden");

    let cleared = state.handle_command(TrameCommand::ClearFilter {
        sheet_id: SheetId("s0".to_owned()),
    });
    assert!(matches!(cleared.display[0], TrameEvent::FilterCleared));
    assert!(cleared.update.is_some(), "clear broadcasts the removal");
    assert!(!is_filtered(&mut state, 1, 0), "row visible again");
}

/// The hidden set is keyed by stable `row_uid`, so it follows its row across a
/// sort. The hidden row tracks the data, and no recompute is needed.
#[test]
fn shared_filter_hidden_follows_sort() {
    let mut state = TrameState::new();
    // Rows: A1=3, A2=1, A3=2.
    seed_col_a(&mut state, &["3", "1", "2"]);
    // Hide the row whose value is exactly 1 (the middle row).
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(0),
        kind: FilterKind::Condition {
            op: FilterOp::Ne,
            value: "1".to_owned(),
        },
    });
    assert!(is_filtered(&mut state, 1, 0), "row holding 1 hidden");

    // Sort ascending: order becomes [1, 2, 3]; the hidden row (value 1) moves
    // to the top and its hidden flag must travel with it.
    state.handle_command(TrameCommand::SortRange {
        sheet_id: SheetId("s0".to_owned()),
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(2), Col::new(0)),
        ),
        sort_col: ColCount::new(0),
        order: SortOrder::Ascending,
        header: HeaderRow::Absent,
    });
    assert_eq!(read_cell(&mut state, 0, 0), "1", "smallest sorted to top");
    assert!(
        is_filtered(&mut state, 0, 0),
        "hidden flag followed the row"
    );
    assert!(!is_filtered(&mut state, 1, 0));
    assert!(!is_filtered(&mut state, 2, 0));
}

/// Editing the filtered column recomputes the hidden set (the stored
/// definition drives the recompute): a row that newly passes is unhidden.
#[test]
fn shared_filter_recomputes_on_edit() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["5"]);
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(0),
        kind: FilterKind::Condition {
            op: FilterOp::Gt,
            value: "10".to_owned(),
        },
    });
    assert!(is_filtered(&mut state, 0, 0), "5 is not > 10");

    // Raise the value above the threshold: the row must reappear.
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "20".to_owned(),
    });
    assert!(
        !is_filtered(&mut state, 0, 0),
        "20 > 10, row unhidden after edit"
    );
}

/// A peer receives the filter as `filter_hidden_{sheet}` ops and applies them
/// directly on import (no recompute); the stored condition also round-trips.
#[test]
fn shared_filter_applies_on_remote_import() {
    let mut author = TrameState::new();
    seed_col_a(&mut author, &["10", "20", "30"]);
    author.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(0),
        kind: FilterKind::Condition {
            op: FilterOp::Gte,
            value: "20".to_owned(),
        },
    });
    let snapshot = author.export_snapshot();

    let mut peer = TrameState::new();
    peer.apply_remote(snapshot.as_bytes());
    assert!(is_filtered(&mut peer, 0, 0), "10 hidden on the peer");
    assert!(!is_filtered(&mut peer, 1, 0));
    // The stored condition survived: an edit on the peer recomputes it.
    peer.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "25".to_owned(),
    });
    assert!(
        !is_filtered(&mut peer, 0, 0),
        "25 >= 20 after edit -> visible"
    );
}

/// Concurrent different filters converge: Loro LWW picks a single hidden set,
/// identical on both peers (`trame-shared-filter`, deterministic behaviour).
#[test]
fn concurrent_filters_converge_lww() {
    // Shared base so both peers address the same row uids.
    let mut base = TrameState::new();
    seed_col_a(&mut base, &["1", "2", "3"]);
    let base_snap = base.export_snapshot();

    let mut a = TrameState::new();
    a.import_snapshot(&base_snap)
        .expect("a snapshot this build wrote reads back at the current schema");
    let mut b = TrameState::new();
    b.import_snapshot(&base_snap)
        .expect("a snapshot this build wrote reads back at the current schema");

    a.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(0),
        kind: FilterKind::Condition {
            op: FilterOp::Gt,
            value: "1".to_owned(),
        },
    });
    b.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(0),
        kind: FilterKind::Condition {
            op: FilterOp::Lt,
            value: "3".to_owned(),
        },
    });

    // Exchange deltas (full snapshots merge fine).
    let a_snap = a.export_snapshot();
    let b_snap = b.export_snapshot();
    a.apply_remote(b_snap.as_bytes());
    b.apply_remote(a_snap.as_bytes());

    assert_eq!(
        a.hidden_rows, b.hidden_rows,
        "both peers converge to one hidden set"
    );
}

/// A concurrent race elects one user's whole definition, never a blend of one
/// peer's operator and the other's operand (`TRAME-FEAT-025`). The definition is
/// one register, so the winner is `gt/1` or `lt/3`, never `gt/3`.
#[test]
fn concurrent_filters_elect_a_whole_definition() {
    let mut base = TrameState::new();
    seed_col_a(&mut base, &["1", "2", "3"]);
    let base_snap = base.export_snapshot();

    let mut a = TrameState::new();
    a.import_snapshot(&base_snap).expect("a reads the base");
    let mut b = TrameState::new();
    b.import_snapshot(&base_snap).expect("b reads the base");

    let gt = FilterKind::Condition {
        op: FilterOp::Gt,
        value: "1".to_owned(),
    };
    let lt = FilterKind::Condition {
        op: FilterOp::Lt,
        value: "3".to_owned(),
    };
    a.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(0),
        kind: gt.clone(),
    });
    b.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(0),
        kind: lt.clone(),
    });

    let a_snap = a.export_snapshot();
    let b_snap = b.export_snapshot();
    a.apply_remote(b_snap.as_bytes());
    b.apply_remote(a_snap.as_bytes());

    let (_, a_kind) = a.load_active_filter().expect("a keeps a filter");
    let (_, b_kind) = b.load_active_filter().expect("b keeps a filter");
    assert!(
        a_kind == gt || a_kind == lt,
        "the winner must be one user's whole filter, got {a_kind:?}"
    );
    assert_eq!(a_kind, b_kind, "both peers elect the same whole definition");
}

/// A values filter reads back whole from the register. A reload keeps the same
/// hidden rows, and an edit re-tests against the real list (`TRAME-FEAT-004`,
/// `TRAME-FEAT-031`).
#[test]
fn filter_by_values_survives_a_reload() {
    let mut author = TrameState::new();
    seed_col_a(&mut author, &["a", "b", "c"]);
    author.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(0),
        kind: FilterKind::Values(vec!["a".to_owned(), "c".to_owned()]),
    });
    assert!(is_filtered(&mut author, 1, 0), "b hidden");

    let snapshot = author.export_snapshot();
    let mut peer = TrameState::new();
    peer.import_snapshot(&snapshot).expect("reload");
    assert!(is_filtered(&mut peer, 1, 0), "b still hidden after reload");

    // An edit re-tests against the stored list, which reads back whole.
    peer.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "c".to_owned(),
    });
    assert!(
        !is_filtered(&mut peer, 1, 0),
        "b's new value c is in the list"
    );
}

/// A filter is a plain CRDT mutation, so Ctrl+Z removes it (`trame-undo`).
#[test]
fn shared_filter_is_undoable() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["a", "b"]);
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(0),
        kind: FilterKind::Values(vec!["a".to_owned()]),
    });
    assert!(is_filtered(&mut state, 1, 0), "b hidden by the filter");

    state.handle_command(TrameCommand::Undo);
    assert!(!is_filtered(&mut state, 1, 0), "undo removed the filter");
}

/// Apply on a non-active sheet, or a virtual column, writes nothing and says
/// so (`TRAME-FEAT-037`); a clear with nothing to clear is a pure no-op.
#[test]
fn shared_filter_noop_guards() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["x"]);

    // Wrong sheet.
    let wrong = state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("nope".to_owned()),
        col: Col::new(0),
        kind: FilterKind::Values(vec!["x".to_owned()]),
    });
    assert!(matches!(&wrong.display[..], [TrameEvent::Error { .. }]));
    assert!(wrong.update.is_none());
    let wrong_clear = state.handle_command(TrameCommand::ClearFilter {
        sheet_id: SheetId("nope".to_owned()),
    });
    assert!(wrong_clear.display.is_empty() && wrong_clear.update.is_none());

    // Virtual column (never materialized): nothing to filter.
    let virtual_col = state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: SheetId("s0".to_owned()),
        col: Col::new(50),
        kind: FilterKind::Values(vec!["x".to_owned()]),
    });
    assert!(matches!(
        &virtual_col.display[..],
        [TrameEvent::Error { .. }]
    ));
    assert!(virtual_col.update.is_none());
}

/// Column A holds `values`, column B its row index; a filter on A hides the
/// rows whose value is not `keep`.
fn filtered_sheet(values: &[&str], keep: &str) -> TrameState {
    let mut state = TrameState::new();
    seed_col_a(&mut state, values);
    for row in 0..values.len() as u32 {
        state.handle_command(TrameCommand::SetCellText {
            row: Row::new(row),
            col: Col::new(1),
            text: format!("{row}"),
        });
    }
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: default_sheet(),
        col: Col::new(0),
        kind: FilterKind::Values(vec![keep.to_owned()]),
    });
    state
}

/// A filter never outlives the column it tests (`TRAME-FEAT-030`): deleting
/// that column drops the definition and the hidden rows, and says so.
#[test]
fn filter_goes_with_its_column() {
    let mut state = filtered_sheet(&["x", "y", "x"], "x");
    assert_eq!(state.hidden_rows.len(), 1);
    let out = state.handle_command(TrameCommand::DeleteColumn { at: Col::new(0) });
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, TrameEvent::FilterCleared))
    );
    assert!(state.load_active_filter().is_none());
    assert!(state.hidden_rows.is_empty());
    let def = state
        .doc
        .get_map(SheetContainer::of(SheetRoot::Filters, &default_sheet()).as_str());
    assert_eq!(def.len(), 0, "no definition left in the document");
}

/// The status bar totals what a formula over the same cells totals
/// (`TRAME-FEAT-029`): a hidden row still counts, as it does in `SUM`.
#[test]
fn status_bar_total_matches_sum_under_a_filter() {
    let mut state = filtered_sheet(&["x", "y", "x"], "x");
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(2),
        col: Col::new(1),
        anchor_row: Row::new(0),
        anchor_col: Col::new(1),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    let out = state.handle_command(TrameCommand::RequestSelectionStats);
    let [TrameEvent::SelectionStats { sum, count, .. }] = &out.display[..] else {
        panic!("expected SelectionStats")
    };
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(3),
        text: "=SUM(B1:B3)".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 3), "3");
    assert_eq!(*sum, 3.0);
    assert_eq!(*count, 3);
}

/// An edit outside the filtered column writes nothing to the hidden set
/// (`TRAME-FEAT-036`): the edit is one operation, whatever the rows hidden.
#[test]
fn edit_outside_the_filtered_column_leaves_the_hidden_set_alone() {
    let mut state = filtered_sheet(&["x", "y", "y", "y", "x"], "x");
    assert_eq!(state.hidden_rows.len(), 3);
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "first".to_owned(),
    });
    let before = state.doc.len_ops();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "second".to_owned(),
    });
    assert_eq!(state.doc.len_ops() - before, 1, "the cell write alone");
    assert_eq!(state.hidden_rows.len(), 3);
}

/// The popover lists the values in the order the column reads them: numbers
/// numerically, before text (`TRAME-FEAT-041`).
#[test]
fn filter_values_are_ordered_as_the_column_reads() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["10", "9", "b", "100", "9", "a"]);
    let out = state.handle_command(TrameCommand::RequestActiveColumnValues);
    let [TrameEvent::ColumnValues { values, .. }] = &out.display[..] else {
        panic!("expected ColumnValues")
    };
    assert_eq!(values, &vec!["9", "10", "100", "a", "b"]);
}

/// Unit-test every `condition_pass` operator and both the numeric and the
/// text comparison paths (full branch coverage of the predicate).
#[test]
fn condition_pass_covers_every_operator() {
    use super::super::condition_pass;
    use crate::formula::types::CellValue;
    let num = CellValue::Number(10.0);
    let txt = CellValue::Text("hello".to_owned());
    let empty = CellValue::Empty;

    // Numeric path.
    assert!(condition_pass(FilterOp::Eq, &num, "10"));
    assert!(!condition_pass(FilterOp::Eq, &num, "11"));
    assert!(condition_pass(FilterOp::Ne, &num, "11"));
    assert!(condition_pass(FilterOp::Gt, &num, "5"));
    assert!(condition_pass(FilterOp::Lt, &num, "20"));
    assert!(condition_pass(FilterOp::Gte, &num, "10"));
    assert!(condition_pass(FilterOp::Lte, &num, "10"));

    // Text path (operand not numeric, or value not a number).
    assert!(condition_pass(FilterOp::Eq, &txt, "hello"));
    assert!(
        condition_pass(FilterOp::Gt, &txt, "a"),
        "lexicographic compare"
    );
    assert!(condition_pass(FilterOp::Contains, &txt, "ell"));
    assert!(condition_pass(FilterOp::NotContains, &txt, "xyz"));
    assert!(condition_pass(FilterOp::StartsWith, &txt, "he"));
    assert!(!condition_pass(FilterOp::IsEmpty, &txt, ""));
    assert!(condition_pass(FilterOp::IsNotEmpty, &txt, ""));
    assert!(condition_pass(FilterOp::IsEmpty, &empty, ""));
    assert!(!condition_pass(FilterOp::IsNotEmpty, &empty, ""));
}

/// `filter_pass` for the values mode, and the operator string parse
/// (the legacy spelling the migration reads back).
#[test]
fn filter_pass_values_and_op_string_roundtrip() {
    use super::super::{filter_pass, op_from_str};
    use crate::formula::types::CellValue;
    let kind = FilterKind::Values(vec!["keep".to_owned()]);
    assert!(filter_pass(&kind, &CellValue::Text("keep".to_owned())));
    assert!(!filter_pass(&kind, &CellValue::Text("drop".to_owned())));

    let spellings: &[(FilterOp, &str)] = &[
        (FilterOp::Eq, "eq"),
        (FilterOp::Ne, "ne"),
        (FilterOp::Gt, "gt"),
        (FilterOp::Lt, "lt"),
        (FilterOp::Gte, "gte"),
        (FilterOp::Lte, "lte"),
        (FilterOp::Contains, "contains"),
        (FilterOp::NotContains, "not_contains"),
        (FilterOp::StartsWith, "starts_with"),
        (FilterOp::IsEmpty, "is_empty"),
        (FilterOp::IsNotEmpty, "is_not_empty"),
    ];
    for &(op, spelling) in spellings {
        assert_eq!(op_from_str(spelling), Some(op), "parses {spelling:?}");
    }
    assert_eq!(op_from_str("garbage"), None);
}

/// A pasted formula's relative references reindex by the paste offset, and a
/// plain literal is copied verbatim (`trame-clipboard`). Copy A1:B1 (A1=10,
/// B1==A1*2) and paste with the top-left landing on A3: B3 must store the
/// reindexed `=A3*2` and compute `20`.
#[test]
fn paste_reindexes_relative_formula() {
    use crate::{ClipboardCell, ClipboardPayload};

    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "10".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=A1*2".to_owned(),
    });

    // Source range A1:B1, pasted with top-left at A3 (row 2): dr = +2.
    let payload = ClipboardPayload {
        src_row: Row::new(0),
        src_col: Col::new(0),
        rows: RowCount::new(1),
        cols: ColCount::new(2),
        cells: vec![
            ClipboardCell {
                dr: RowCount::new(0),
                dc: ColCount::new(0),
                raw: "10".to_owned(),
                ..Default::default()
            },
            ClipboardCell {
                dr: RowCount::new(0),
                dc: ColCount::new(1),
                raw: "=A1*2".to_owned(),
                ..Default::default()
            },
        ],
    };
    state.handle_command(TrameCommand::PasteRange {
        target_row: Row::new(2),
        target_col: Col::new(0),
        payload,
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(3),
        col_start: Col::new(0),
        col_end: Col::new(1),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!("expected ViewportData");
    };
    let a3 = cells
        .iter()
        .find(|c| c.row == Row::new(2) && c.col == Col::new(0))
        .unwrap();
    let b3 = cells
        .iter()
        .find(|c| c.row == Row::new(2) && c.col == Col::new(1))
        .unwrap();
    assert_eq!(a3.text, "10", "literal copied verbatim");
    assert_eq!(
        b3.raw, "=A3*2",
        "relative ref reindexed by the paste offset"
    );
    assert_eq!(
        b3.text, "20",
        "reindexed formula recomputes against the pasted literal"
    );
}

/// Autofill in `Auto` mode extrapolates a numeric arithmetic series down a
/// column (`trame-autofill`): A1=1, A2=2 dragged to A5 yields 3, 4, 5.
#[test]
fn fill_auto_extrapolates_numeric_series() {
    use crate::FillMode;
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "2".to_owned(),
    });

    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(1), Col::new(0)),
        ),
        dest: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(4), Col::new(0)),
        ),
        mode: FillMode::Auto,
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(4),
        col_start: Col::new(0),
        col_end: Col::new(0),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let at = |r: u32| {
        cells
            .iter()
            .find(|c| c.row == Row::new(r) && c.col == Col::new(0))
            .unwrap()
            .text
            .clone()
    };
    assert_eq!((at(2), at(3), at(4)), ("3".into(), "4".into(), "5".into()));
}

/// Autofill in `Auto` mode copies (tiles) a non-numeric source and reindexes
/// a copied formula's relative references by its fill offset.
#[test]
fn fill_auto_copies_text_and_reindexes_formula() {
    use crate::FillMode;
    let mut state = TrameState::new();
    // B1 = A1*2; A column holds the multiplicands so the reindex is visible.
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "10".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "20".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=A1*2".to_owned(),
    });

    // Fill B1 down to B2: the single source is not a series -> Copy + reindex.
    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(1)),
            crate::CellPos::new(Row::new(0), Col::new(1)),
        ),
        dest: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(1)),
            crate::CellPos::new(Row::new(1), Col::new(1)),
        ),
        mode: FillMode::Auto,
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(1),
        col_start: Col::new(0),
        col_end: Col::new(1),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let b2 = cells
        .iter()
        .find(|c| c.row == Row::new(1) && c.col == Col::new(1))
        .unwrap();
    assert_eq!(b2.raw, "=A2*2", "relative ref reindexed by the fill offset");
    assert_eq!(b2.text, "40", "reindexed formula recomputes against A2");
}

/// `Series` mode on a single numeric cell increments by 1 (Excel's Ctrl+drag
/// of one number): A1=5 filled to A4 yields 6, 7, 8.
#[test]
fn fill_series_single_cell_increments_by_one() {
    use crate::FillMode;
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "5".to_owned(),
    });

    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(0), Col::new(0)),
        ),
        dest: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(3), Col::new(0)),
        ),
        mode: FillMode::Series,
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(3),
        col_start: Col::new(0),
        col_end: Col::new(0),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let at = |r: u32| {
        cells
            .iter()
            .find(|c| c.row == Row::new(r) && c.col == Col::new(0))
            .unwrap()
            .text
            .clone()
    };
    assert_eq!((at(1), at(2), at(3)), ("6".into(), "7".into(), "8".into()));
}

/// `Copy` mode tiles a multi-cell numeric source verbatim, as Excel's
/// Ctrl+drag of a selection does. A1=1, A2=2 filled to A6 repeats 1, 2, 1, 2
/// -- no series.
#[test]
fn fill_copy_multi_cell_tiles_verbatim() {
    use crate::FillMode;
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "2".to_owned(),
    });

    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(1), Col::new(0)),
        ),
        dest: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(5), Col::new(0)),
        ),
        mode: FillMode::Copy,
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(5),
        col_start: Col::new(0),
        col_end: Col::new(0),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let at = |r: u32| {
        cells
            .iter()
            .find(|c| c.row == Row::new(r) && c.col == Col::new(0))
            .unwrap()
            .text
            .clone()
    };
    assert_eq!(
        (at(2), at(3), at(4), at(5)),
        ("1".into(), "2".into(), "1".into(), "2".into()),
    );
}
