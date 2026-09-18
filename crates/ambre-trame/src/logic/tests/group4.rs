// ambre-trame/src/logic/tests/group4.rs
// TrameState unit tests (group 4/7). Helpers via `use super::*`;
// logic-module items are reached with `super::super::` paths.

use super::*;

/// Formula references follow the data through a sort (the `trame-formula-engine`
/// guarantee). Each `=A{n}*10` binds to its row's `Uid` at edit time. The sort
/// reorders rows by `mov`, and the uid-anchored bytecode is preserved (no
/// reseed/re-parse). So every formula keeps reading the *same physical* A cell
/// it referenced -- now at a new grid position.
#[test]
fn sort_range_formulas_follow_data() {
    let mut state = TrameState::new();
    for (r, a) in ["2", "1"].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: a.to_string(),
        });
        // B = A of the same row, times 10 (row0 -> =A1*10 -> 20, row1 -> =A2*10 -> 10).
        let cell = format!("=A{}*10", r + 1);
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 1,
            text: cell,
        });
    }
    state.handle_command(TrameCommand::SortRange {
        sheet_id: "s0".to_owned(),
        row_start: 0,
        row_end: 1,
        col_start: 0,
        col_end: 1,
        sort_col: 0,
        ascending: true,
        has_header: false,
    });
    // Data sorts ascending: A = [1, 2].
    assert_eq!(read_col(&mut state, 0, 1, 1), vec!["1", "2"]);
    // References follow the data. The `=A2*10` cell, bound to the row whose
    // A was 1, is now at pos0 and still reads 1 -> 10. The `=A1*10` cell,
    // bound to the row whose A was 2, is now at pos1 and reads 2 -> 20.
    assert_eq!(read_col(&mut state, 1, 1, 2), vec!["10", "20"]);
}

/// A range is a positional *block*, not a set of specific rows. Sorting the
/// rows inside `SUM(A1:A3)` reorders the same three values, so the block (and
/// its total) is unchanged. That is the Excel/Sheets behavior, and the only
/// well-defined one: uid-bound endpoints would scatter and invert the range.
/// Contrast `sort_range_formulas_follow_data`, where single-cell refs *do*
/// follow their row.
#[test]
fn sort_range_sum_block_is_stable_under_internal_sort() {
    let mut state = TrameState::new();
    for (r, a) in ["3", "1", "2"].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: a.to_string(),
        });
    }
    // A4 lives below the sorted block and sums the whole block, plus a
    // sub-block A1:A2 whose endpoints would invert under a naive uid binding.
    state.handle_command(TrameCommand::SetCellText {
        row: 3,
        col: 0,
        text: "=SUM(A1:A3)".into(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 4,
        col: 0,
        text: "=SUM(A1:A2)".into(),
    });
    assert_eq!(read_col(&mut state, 0, 4, 1)[3], "6");
    assert_eq!(read_col(&mut state, 0, 4, 1)[4], "4"); // 3 + 1

    state.handle_command(TrameCommand::SortRange {
        sheet_id: "s0".to_owned(),
        row_start: 0,
        row_end: 2,
        col_start: 0,
        col_end: 0,
        sort_col: 0,
        ascending: true,
        has_header: false,
    });
    // Data sorts to [1, 2, 3]; the SUM cells below are untouched.
    assert_eq!(read_col(&mut state, 0, 2, 2), vec!["1", "2", "3"]);
    // Whole block A1:A3 still sums all three -> 6 (no #REF!, no collapse).
    assert_eq!(read_col(&mut state, 0, 4, 3)[3], "6");
    // Sub-block A1:A2 is positions 0..1 -> 1 + 2 = 3.
    assert_eq!(read_col(&mut state, 0, 4, 3)[4], "3");
}

/// A sort targeting a non-active sheet, or an already-sorted range, makes no
/// CRDT mutation.
#[test]
fn sort_range_noops() {
    let mut state = TrameState::new();
    for (r, a) in ["1", "2"].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: a.to_string(),
        });
    }
    // Wrong sheet: ignored.
    let other = state.handle_command(TrameCommand::SortRange {
        sheet_id: "s9".to_owned(),
        row_start: 0,
        row_end: 1,
        col_start: 0,
        col_end: 0,
        sort_col: 0,
        ascending: true,
        has_header: false,
    });
    assert!(other.update.is_none());
    assert!(other.display.is_empty());
    // Already ascending: acknowledged but no delta.
    let sorted = state.handle_command(TrameCommand::SortRange {
        sheet_id: "s0".to_owned(),
        row_start: 0,
        row_end: 1,
        col_start: 0,
        col_end: 0,
        sort_col: 0,
        ascending: true,
        has_header: false,
    });
    assert!(sorted.update.is_none(), "no reorder, no broadcast");
    assert!(
        sorted
            .display
            .iter()
            .any(|e| matches!(e, TrameEvent::SortComplete))
    );
}

/// The parameterless `SortSelectionAsc` intent (DOM context menu, `trame-sort`)
/// resolves against the tracked selection and active sheet: same outcome as an
/// explicit `SortRange` over `selection.bounds()`.
#[test]
fn sort_selection_resolves_against_tracked_selection() {
    let mut state = TrameState::new();
    for (r, (a, b)) in [("3", "c"), ("1", "a"), ("2", "b")].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: a.to_string(),
        });
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 1,
            text: b.to_string(),
        });
    }
    // The UI reports the selection rectangle A1:B3 via SetActiveCell.
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 2,
        anchor_col: 1,
    });
    let evts = state.handle_command(TrameCommand::SortSelectionAsc);
    assert!(
        evts.display
            .iter()
            .any(|e| matches!(e, TrameEvent::SortComplete))
    );
    assert!(evts.update.is_some(), "a real reorder broadcasts a delta");
    assert_eq!(read_col(&mut state, 0, 2, 1), vec!["1", "2", "3"]);
    assert_eq!(
        read_col(&mut state, 1, 2, 2),
        vec!["a", "b", "c"],
        "row B follows A"
    );
}

/// The whole-sheet `SortSheetAsc` intent (whole-column selection, `trame-sort`)
/// sorts the full used range by the active cell's column. Every row's other
/// columns stay aligned, even the rows below the values reported by a partial
/// selection.
#[test]
fn sort_sheet_sorts_used_range_by_active_column() {
    let mut state = TrameState::new();
    // Sort key in column B; column A is an aligned label that must follow.
    for (r, (a, b)) in [("x", "3"), ("y", "1"), ("z", "2")].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: a.to_string(),
        });
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 1,
            text: b.to_string(),
        });
    }
    // The active cell sits in column B (a whole-column selection of B).
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 1,
        anchor_row: 2,
        anchor_col: 1,
    });
    let evts = state.handle_command(TrameCommand::SortSheetAsc);
    assert!(
        evts.display
            .iter()
            .any(|e| matches!(e, TrameEvent::SortComplete))
    );
    assert!(evts.update.is_some());
    // B sorts ascending; A follows its row across the whole used range.
    assert_eq!(read_col(&mut state, 1, 2, 1), vec!["1", "2", "3"]);
    assert_eq!(
        read_col(&mut state, 0, 2, 2),
        vec!["y", "z", "x"],
        "col A stays aligned"
    );
}

/// Multi-column sort (`trame-sort`): rows are ordered lexicographically by
/// the keys in decreasing priority -- primary col 0 ascending, then secondary
/// col 1 descending breaks ties. Whole rows travel by uid.
#[test]
fn sort_range_multi_lexicographic_with_per_key_direction() {
    let mut state = TrameState::new();
    // (A, B): ties on A=1 are broken by B descending; A=2 is a lone group.
    for (r, (a, b)) in [("1", "a"), ("2", "z"), ("1", "c"), ("1", "b")]
        .iter()
        .enumerate()
    {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: a.to_string(),
        });
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 1,
            text: b.to_string(),
        });
    }
    let evts = state.handle_command(TrameCommand::SortRangeMulti {
        sheet_id: "s0".to_owned(),
        row_start: 0,
        row_end: 3,
        col_start: 0,
        col_end: 1,
        sort_keys: vec![
            SortKey {
                col: 0,
                ascending: true,
            },
            SortKey {
                col: 1,
                ascending: false,
            },
        ],
        has_header: false,
    });
    assert!(
        evts.display
            .iter()
            .any(|e| matches!(e, TrameEvent::SortComplete))
    );
    assert!(evts.update.is_some(), "a real reorder broadcasts a delta");
    // A ascending: the three A=1 rows first, then A=2.
    assert_eq!(read_col(&mut state, 0, 3, 1), vec!["1", "1", "1", "2"]);
    // Within A=1, B descending: c, b, a. The A=2 row keeps its B ("z").
    assert_eq!(read_col(&mut state, 1, 3, 2), vec!["c", "b", "a", "z"]);
}

/// The parameterless `SortSelectionMulti` intent (the "Sort range..." dialog,
/// `trame-sort`) resolves its range against the tracked selection, exactly like the
/// single-column `SortSelectionAsc`. Keys are range-relative to the selection's
/// first column.
#[test]
fn sort_selection_multi_resolves_against_tracked_selection() {
    let mut state = TrameState::new();
    for (r, (a, b)) in [("1", "b"), ("1", "a"), ("2", "a")].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: a.to_string(),
        });
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 1,
            text: b.to_string(),
        });
    }
    // Selection A1:B3 reported via SetActiveCell.
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 2,
        anchor_col: 1,
    });
    let evts = state.handle_command(TrameCommand::SortSelectionMulti {
        sort_keys: vec![
            SortKey {
                col: 0,
                ascending: true,
            },
            SortKey {
                col: 1,
                ascending: true,
            },
        ],
        has_header: false,
    });
    assert!(
        evts.display
            .iter()
            .any(|e| matches!(e, TrameEvent::SortComplete))
    );
    assert_eq!(read_col(&mut state, 0, 2, 1), vec!["1", "1", "2"]);
    // Within A=1, B ascending: a then b.
    assert_eq!(read_col(&mut state, 1, 2, 2), vec!["a", "b", "a"]);
}

/// Multi-column sort honours `has_header`: the first range row is pinned and the
/// rows below are ordered by the keys (A ascending, then B ascending).
#[test]
fn sort_range_multi_pins_header_row() {
    let mut state = TrameState::new();
    for (r, (a, b)) in [("Header", "h"), ("2", "b"), ("1", "a"), ("2", "a")]
        .iter()
        .enumerate()
    {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: a.to_string(),
        });
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 1,
            text: b.to_string(),
        });
    }
    state.handle_command(TrameCommand::SortRangeMulti {
        sheet_id: "s0".to_owned(),
        row_start: 0,
        row_end: 3,
        col_start: 0,
        col_end: 1,
        sort_keys: vec![
            SortKey {
                col: 0,
                ascending: true,
            },
            SortKey {
                col: 1,
                ascending: true,
            },
        ],
        has_header: true,
    });
    assert_eq!(read_col(&mut state, 0, 3, 1), vec!["Header", "1", "2", "2"]);
    assert_eq!(read_col(&mut state, 1, 3, 2), vec!["h", "a", "a", "b"]);
}

/// Guards: an empty key list (or one whose every column falls outside the range)
/// is a pure no-op -- no `SortComplete`, no CRDT delta.
#[test]
fn sort_range_multi_no_keys_is_noop() {
    let mut state = TrameState::new();
    for (r, a) in ["3", "1", "2"].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: a.to_string(),
        });
    }
    // No keys at all.
    let empty = state.handle_command(TrameCommand::SortRangeMulti {
        sheet_id: "s0".to_owned(),
        row_start: 0,
        row_end: 2,
        col_start: 0,
        col_end: 0,
        sort_keys: vec![],
        has_header: false,
    });
    assert!(empty.update.is_none());
    assert!(
        !empty
            .display
            .iter()
            .any(|e| matches!(e, TrameEvent::SortComplete))
    );
    // A key whose column is outside [col_start..=col_end] is dropped, leaving no
    // usable key -- same no-op.
    let out = state.handle_command(TrameCommand::SortRangeMulti {
        sheet_id: "s0".to_owned(),
        row_start: 0,
        row_end: 2,
        col_start: 0,
        col_end: 0,
        sort_keys: vec![SortKey {
            col: 5,
            ascending: true,
        }],
        has_header: false,
    });
    assert!(out.update.is_none());
    // The order is untouched.
    assert_eq!(read_col(&mut state, 0, 2, 1), vec!["3", "1", "2"]);
}

/// An already-sorted range acknowledges with `SortComplete` but produces no CRDT
/// delta (the stable sort leaves the uid order untouched).
#[test]
fn sort_range_multi_already_sorted_emits_no_delta() {
    let mut state = TrameState::new();
    for (r, (a, b)) in [("1", "a"), ("1", "b"), ("2", "a")].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: a.to_string(),
        });
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 1,
            text: b.to_string(),
        });
    }
    let evts = state.handle_command(TrameCommand::SortRangeMulti {
        sheet_id: "s0".to_owned(),
        row_start: 0,
        row_end: 2,
        col_start: 0,
        col_end: 1,
        sort_keys: vec![
            SortKey {
                col: 0,
                ascending: true,
            },
            SortKey {
                col: 1,
                ascending: true,
            },
        ],
        has_header: false,
    });
    assert!(
        evts.display
            .iter()
            .any(|e| matches!(e, TrameEvent::SortComplete))
    );
    assert!(
        evts.update.is_none(),
        "an already-sorted range broadcasts no delta"
    );
}

#[test]
fn col_alpha_basics() {
    assert_eq!(col_to_alpha(0), "A");
    assert_eq!(col_to_alpha(25), "Z");
    assert_eq!(col_to_alpha(26), "AA");
    assert_eq!(col_to_alpha(51), "AZ");
    assert_eq!(col_to_alpha(52), "BA");
    assert_eq!(col_to_alpha(701), "ZZ");
}

/// `RequestActiveColumnValues` returns the active column's distinct visible
/// values (deduped, sorted, empties dropped) for the popover's "by values"
/// list (`trame-shared-filter`).
#[test]
fn active_column_values_are_distinct_sorted_and_visible() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["cherry", "apple", "", "banana", "apple"]);
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 0,
        anchor_col: 0,
    });

    let evts = state.handle_command(TrameCommand::RequestActiveColumnValues);
    match &evts.display[0] {
        TrameEvent::ColumnValues { col, values } => {
            assert_eq!(*col, 0);
            assert_eq!(values, &["apple", "banana", "cherry"]);
        }
        other => panic!("expected ColumnValues, got {other:?}"),
    }
    assert!(evts.update.is_none(), "a values query is read-only");
}

/// `RequestSelectionStats` aggregates the tracked selection for the status bar
/// (`trame-status-bar`): `count` tallies non-empty cells, the numeric ones feed
/// `count_num`/`sum`/`avg`/`min`/`max`.
#[test]
fn selection_stats_aggregate_numeric_cells() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["10", "20", "", "30", "apple"]);
    // Select the whole column A range (0,0)..=(4,0).
    state.handle_command(TrameCommand::SetActiveCell {
        row: 4,
        col: 0,
        anchor_row: 0,
        anchor_col: 0,
    });

    let evts = state.handle_command(TrameCommand::RequestSelectionStats);
    match &evts.display[0] {
        TrameEvent::SelectionStats {
            sum,
            avg,
            count,
            count_num,
            min,
            max,
        } => {
            assert_eq!(*count, 4, "10/20/30/apple are non-empty");
            assert_eq!(*count_num, 3, "only 10/20/30 are numeric");
            assert_eq!(*sum, 60.0);
            assert_eq!(*avg, 20.0);
            assert_eq!(*min, 10.0);
            assert_eq!(*max, 30.0);
        }
        other => panic!("expected SelectionStats, got {other:?}"),
    }
    assert!(evts.update.is_none(), "a stats query is read-only");
}

/// The parameterless filter wrappers resolve the sheet/column from the tracked
/// active cell, mirroring the sort intents (`trame-shared-filter`).
#[test]
fn active_column_filter_wrappers_apply_and_clear() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["10", "20", "30"]);
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 0,
        anchor_col: 0,
    });

    let applied = state.handle_command(TrameCommand::ApplyFilterToActiveColumn {
        kind: FilterKind::Condition {
            op: FilterOp::Gt,
            value: "15".to_owned(),
        },
    });
    assert!(matches!(
        applied.display[0],
        TrameEvent::FilterApplied {
            hidden_row_count: 1,
            col: 0
        }
    ));
    assert!(is_filtered(&mut state, 0, 0), "10 hidden");
    assert!(!is_filtered(&mut state, 1, 0), "20 visible");

    let cleared = state.handle_command(TrameCommand::ClearActiveFilter);
    assert!(matches!(cleared.display[0], TrameEvent::FilterCleared));
    assert!(
        !is_filtered(&mut state, 0, 0),
        "row visible again after clear"
    );
}

/// A "filter by values" keeps only the listed (checked) values visible; the
/// others are flagged `is_filtered`. `FilterApplied` reports the hidden count.
#[test]
fn shared_filter_by_values_hides_nonmatching() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["apple", "banana", "cherry", "apple"]);

    let evts = state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: "s0".to_owned(),
        col: 0,
        kind: FilterKind::Values(vec!["apple".to_owned()]),
    });
    assert!(matches!(
        evts.display[0],
        TrameEvent::FilterApplied {
            hidden_row_count: 2,
            col: 0
        }
    ));
    assert!(
        evts.display
            .iter()
            .any(|e| matches!(e, TrameEvent::DataReset))
    );
    assert!(evts.update.is_some(), "filter broadcasts a CRDT delta");

    assert!(!is_filtered(&mut state, 0, 0), "apple visible");
    assert!(is_filtered(&mut state, 1, 0), "banana hidden");
    assert!(is_filtered(&mut state, 2, 0), "cherry hidden");
    assert!(!is_filtered(&mut state, 3, 0), "apple visible");
}

/// A numeric "greater than" condition hides rows that fail it.
#[test]
fn shared_filter_condition_greater_than() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["10", "20", "30"]);

    let evts = state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: "s0".to_owned(),
        col: 0,
        kind: FilterKind::Condition {
            op: FilterOp::Gt,
            value: "15".to_owned(),
        },
    });
    assert!(matches!(
        evts.display[0],
        TrameEvent::FilterApplied {
            hidden_row_count: 1,
            col: 0
        }
    ));
    assert!(is_filtered(&mut state, 0, 0), "10 is not > 15");
    assert!(!is_filtered(&mut state, 1, 0), "20 > 15");
    assert!(!is_filtered(&mut state, 2, 0), "30 > 15");
}
