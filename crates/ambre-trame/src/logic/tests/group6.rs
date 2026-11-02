// ambre-trame/src/logic/tests/group6.rs
// TrameState unit tests (group 6/7). Helpers via `use super::*`;
// logic-module items are reached with `super::super::` paths.

use super::*;
use ambre_core::id::{Id, Sheet};
use ambre_core::index::Pos;

/// Autofill of a merged-cell source fills block by block and re-merges every
/// tile (`trame-autofill`). A 2x1 merge at A1:A2 holding "x", dragged to A6,
/// yields merged blocks A3:A4 and A5:A6 each carrying "x". The in-between
/// covered cells stay blank and unmerged.
#[test]
fn fill_merged_source_tiles_and_remerges_blocks() {
    use crate::FillMode;
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        text: "x".to_owned(),
    });
    // Merge A1:A2 (1 col x 2 rows), master = A1.
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            crate::CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(0)),
        ),
    });

    // Drag the handle down: source is the whole merge, dest snapped to A6.
    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: crate::CellRect::spanning(
            crate::CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            crate::CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(0)),
        ),
        dest: crate::CellRect::spanning(
            crate::CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            crate::CellPos::new(Pos::<Row>::new(5), Pos::<Col>::new(0)),
        ),
        mode: FillMode::Auto,
    });

    // Each block's master carries the value and a fresh 2x1 span; the masters
    // (A3, A5) re-merge while the covered rows (A4, A6) stay blank.
    let a3 = read_display(&mut state, 2, 0);
    assert_eq!(a3.merge_span, Some(span(2, 1)), "A3 master spans 2 rows");
    assert_eq!(a3.text, "x");
    let a5 = read_display(&mut state, 4, 0);
    assert_eq!(a5.merge_span, Some(span(2, 1)), "A5 master spans 2 rows");
    assert_eq!(a5.text, "x");
    let a4 = read_display(&mut state, 3, 0);
    assert_eq!(
        a4.is_merged,
        ambre_bus::Merge::Merged,
        "A4 is covered by the A3 block"
    );
    assert_eq!(a4.merge_span, None);
    assert_eq!(a4.text, "");
}

/// Ctrl-drag of a merged numeric source grows by +1 per block (Series), so the
/// merge span and styling survive while each tile master increments. Mirrors the
/// Ctrl path the UI resolves to `FillMode::Series` for a one-block source.
#[test]
fn fill_merged_series_increments_per_block() {
    use crate::FillMode;
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        text: "1".to_owned(),
    });
    // Merge A1:A2 (1 col x 2 rows), master = A1 holding the number 1.
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            crate::CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(0)),
        ),
    });

    // Ctrl-drag down to A6: Series, so each block master grows by +1.
    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: crate::CellRect::spanning(
            crate::CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            crate::CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(0)),
        ),
        dest: crate::CellRect::spanning(
            crate::CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            crate::CellPos::new(Pos::<Row>::new(5), Pos::<Col>::new(0)),
        ),
        mode: FillMode::Series,
    });

    let a3 = read_display(&mut state, 2, 0);
    assert_eq!(a3.merge_span, Some(span(2, 1)), "A3 master spans 2 rows");
    assert_eq!(a3.text, "2", "second block increments to 2");
    let a5 = read_display(&mut state, 4, 0);
    assert_eq!(a5.merge_span, Some(span(2, 1)), "A5 master spans 2 rows");
    assert_eq!(a5.text, "3", "third block increments to 3");
}

/// Dragging a stack of two equal merged blocks tiles the whole pattern. The
/// fill re-merges every destination block and, under `Auto`, the two numeric
/// masters form a series that keeps progressing across the new blocks.
#[test]
fn fill_multi_merged_source_continues_pattern() {
    use crate::FillMode;
    let mut state = TrameState::new();
    // A1 = 1, A3 = 2; merge A1:A2 and A3:A4 into two 2x1 blocks.
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(2),
        col: Pos::<Col>::new(0),
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            crate::CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(0)),
        ),
    });
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Pos::<Row>::new(2), Pos::<Col>::new(0)),
            crate::CellPos::new(Pos::<Row>::new(3), Pos::<Col>::new(0)),
        ),
    });

    // Drag the two-block source (A1:A4) down to A8 -> two more blocks.
    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: crate::CellRect::spanning(
            crate::CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            crate::CellPos::new(Pos::<Row>::new(3), Pos::<Col>::new(0)),
        ),
        dest: crate::CellRect::spanning(
            crate::CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            crate::CellPos::new(Pos::<Row>::new(7), Pos::<Col>::new(0)),
        ),
        mode: FillMode::Auto,
    });

    // A5 (block 3) and A7 (block 4) re-merge and continue the 1,2,... series.
    let a5 = read_display(&mut state, 4, 0);
    assert_eq!(a5.merge_span, Some(span(2, 1)), "A5 master spans 2 rows");
    assert_eq!(a5.text, "3");
    let a7 = read_display(&mut state, 6, 0);
    assert_eq!(a7.merge_span, Some(span(2, 1)), "A7 master spans 2 rows");
    assert_eq!(a7.text, "4");
    // Covered rows of the new blocks stay blank.
    let a6 = read_display(&mut state, 5, 0);
    assert_eq!(a6.is_merged, ambre_bus::Merge::Merged);
    assert_eq!(a6.text, "");
}

/// FindNext scans displayed values, folds case by default, and picks the
/// match after the active cell (wrapping to the top).
#[test]
fn find_next_matches_and_wraps() {
    let mut state = TrameState::new();
    seed_find(&mut state);

    // Active at A1 (0,0): the three "apple" cells (case-folded) match; the
    // focused one is the first strictly after the active cell -> A2.
    state.handle_command(TrameCommand::SetActiveCell {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        anchor_row: Pos::<Row>::new(0),
        anchor_col: Pos::<Col>::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    let out = state.handle_command(TrameCommand::FindNext {
        query: find_query("apple".to_owned(), crate::FindOptions::default()),
    });
    let (matches, active) = find_result(&out);
    assert_eq!(matches, vec![(0, 0), (1, 0), (3, 0)]);
    assert_eq!(active, Some(1), "next after A1 is A2");
    // No CRDT delta for a read-only find.
    assert!(out.update.is_none());

    // Active at the last match A4 (3,0): the next wraps back to A1.
    state.handle_command(TrameCommand::SetActiveCell {
        row: Pos::<Row>::new(3),
        col: Pos::<Col>::new(0),
        anchor_row: Pos::<Row>::new(3),
        anchor_col: Pos::<Col>::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    let out = state.handle_command(TrameCommand::FindNext {
        query: find_query("apple".to_owned(), crate::FindOptions::default()),
    });
    assert_eq!(find_result(&out).1, Some(0), "wraps to the first match");
}

/// The case and the extent narrow the match set.
#[test]
fn find_next_honors_case_and_whole_cell() {
    let mut state = TrameState::new();
    seed_find(&mut state);
    state.handle_command(TrameCommand::SetActiveCell {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        anchor_row: Pos::<Row>::new(0),
        anchor_col: Pos::<Col>::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });

    // Case-sensitive "apple": only the exact-case A1 substring matches.
    let out = state.handle_command(TrameCommand::FindNext {
        query: find_query(
            "apple".to_owned(),
            crate::FindOptions {
                case: crate::CaseMatch::Sensitive,
                ..Default::default()
            },
        ),
    });
    assert_eq!(find_result(&out).0, vec![(0, 0)]);

    // Whole-cell, case-insensitive "apple": A1 and A4 are exactly "apple"
    // (folded); "Apple pie" is not.
    let out = state.handle_command(TrameCommand::FindNext {
        query: find_query(
            "apple".to_owned(),
            crate::FindOptions {
                extent: crate::MatchExtent::WholeCell,
                ..Default::default()
            },
        ),
    });
    assert_eq!(find_result(&out).0, vec![(0, 0), (3, 0)]);
}

/// `FIND-002`: the walk covers the whole sheet, not the rows the view asked
/// for. A match far past the requested window is reported and elected.
#[test]
fn a_search_reads_past_the_requested_window() {
    let mut state = TrameState::new();
    for (row, col) in [(0, 0), (900, 60)] {
        state.handle_command(TrameCommand::SetCellText {
            row: Pos::<Row>::new(row),
            col: Pos::<Col>::new(col),
            text: "needle".to_owned(),
        });
    }
    state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_core::Generation::nth(1),
        row_start: Pos::<Row>::new(0),
        row_end: Pos::<Row>::new(30),
        col_start: Pos::<Col>::new(0),
        col_end: Pos::<Col>::new(10),
    });
    let out = state.handle_command(TrameCommand::FindNext {
        query: find_query("needle".to_owned(), crate::FindOptions::default()),
    });
    assert_eq!(find_result(&out), (vec![(0, 0), (900, 60)], Some(1)));
}

/// `FIND-014`: a replace-all rewrites the matched text and nothing else.
/// The text around a match reads back as it was. So does a cell the query
/// never matched, and a match on a sheet out of scope.
#[test]
fn a_replace_all_leaves_every_unmatched_text_alone() {
    let mut state = TrameState::new();
    seed_find(&mut state); // A1 "apple", A2 "Apple pie", A3 "banana", A4 "APPLE"
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(1),
        text: "grape".to_owned(),
    });
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet2"),
    });
    let s2 = second_sheet_id(&state);
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        text: "apple".to_owned(),
    });
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: Id::<Sheet>::from_text("s0".to_owned()),
    });

    let out = state.handle_command(TrameCommand::ReplaceAll {
        query: find_query("apple".to_owned(), crate::FindOptions::default()),
        replacement: "fig".to_owned(),
    });
    assert_eq!(replaced_count(&out), 3);
    assert_eq!(read_cell(&mut state, 1, 0), "fig pie");
    assert_eq!(read_cell(&mut state, 2, 0), "banana");
    assert_eq!(read_cell(&mut state, 0, 1), "grape");
    assert_eq!(read_cell(&mut state, 5, 5), "");
    state.handle_command(TrameCommand::ActivateSheet { sheet_id: s2 });
    assert_eq!(read_cell(&mut state, 0, 0), "apple", "out of scope");
}

/// ReplaceAll rewrites the raw text of every match, reports the count, and is
/// a single undo group (one Undo restores all cells).
#[test]
fn replace_all_rewrites_and_undoes_as_one_group() {
    let mut state = TrameState::new();
    seed_find(&mut state);

    let out = state.handle_command(TrameCommand::ReplaceAll {
        query: find_query("apple".to_owned(), crate::FindOptions::default()),
        replacement: "cherry".to_owned(),
    });
    assert_eq!(replaced_count(&out), 3);
    assert_eq!(read_cell(&mut state, 0, 0), "cherry");
    assert_eq!(read_cell(&mut state, 1, 0), "cherry pie");
    assert_eq!(read_cell(&mut state, 3, 0), "cherry"); // case-insensitive substring
    assert_eq!(read_cell(&mut state, 2, 0), "banana"); // untouched

    // One Undo restores every replaced cell at once.
    state.handle_command(TrameCommand::Undo);
    assert_eq!(read_cell(&mut state, 0, 0), "apple");
    assert_eq!(read_cell(&mut state, 1, 0), "Apple pie");
    assert_eq!(read_cell(&mut state, 3, 0), "APPLE");
}

/// A formula-source cell is rewritten only when the scope is `Formulas`; matching
/// still tests the displayed value.
#[test]
fn replace_all_gates_formula_cells() {
    let mut state = TrameState::new();
    // A1 = 2, A2 = "=A1" (displays "2"). Replace displayed "2" with "9".
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(1),
        col: Pos::<Col>::new(0),
        text: "=A1".to_owned(),
    });

    // Values scope: the literal A1 is rewritten, the formula A2 is left.
    let out = state.handle_command(TrameCommand::ReplaceAll {
        query: find_query("2".to_owned(), crate::FindOptions::default()),
        replacement: "9".to_owned(),
    });
    assert_eq!(replaced_count(&out), 1);
    assert_eq!(read_raw(&mut state, 0, 0), "9");
    assert_eq!(
        read_raw(&mut state, 1, 0),
        "=A1",
        "formula source untouched"
    );

    // Formulas scope: a formula cell matches on its raw text. "=A1"
    // contains "A1" -> rewrite the formula source to "=B2".
    let out = state.handle_command(TrameCommand::ReplaceAll {
        query: find_query(
            "A1".to_owned(),
            crate::FindOptions {
                scope: crate::SearchScope::Formulas,
                ..Default::default()
            },
        ),
        replacement: "B2".to_owned(),
    });
    assert_eq!(replaced_count(&out), 1);
    assert_eq!(read_raw(&mut state, 1, 0), "=B2");
}

/// ReplaceOne rewrites only the active cell's match and advances the
/// FindResult to the next hit; a non-matching active cell just advances.
#[test]
fn replace_one_replaces_active_and_advances() {
    let mut state = TrameState::new();
    seed_find(&mut state); // A1 "apple", A2 "Apple pie", A3 "banana", A4 "APPLE"

    // Active on A1 (a match): replace just this cell, advance to A2.
    state.handle_command(TrameCommand::SetActiveCell {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        anchor_row: Pos::<Row>::new(0),
        anchor_col: Pos::<Col>::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    let out = state.handle_command(TrameCommand::ReplaceOne {
        query: find_query("apple".to_owned(), crate::FindOptions::default()),
        replacement: "cherry".to_owned(),
    });
    assert_eq!(replaced_count(&out), 1);
    assert_eq!(read_cell(&mut state, 0, 0), "cherry");
    assert_eq!(read_cell(&mut state, 1, 0), "Apple pie", "A2 untouched");
    // The reply advances to the next remaining match (A2 at index 0 of the
    // recomputed set).
    let (matches, active) = find_result(&out);
    assert_eq!(matches, vec![(1, 0), (3, 0)]);
    assert_eq!(active, Some(0));

    // Active on a non-match (A3): nothing replaced, but it advances.
    state.handle_command(TrameCommand::SetActiveCell {
        row: Pos::<Row>::new(2),
        col: Pos::<Col>::new(0),
        anchor_row: Pos::<Row>::new(2),
        anchor_col: Pos::<Col>::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    let out = state.handle_command(TrameCommand::ReplaceOne {
        query: find_query("apple".to_owned(), crate::FindOptions::default()),
        replacement: "cherry".to_owned(),
    });
    assert_eq!(replaced_count(&out), 0);
    assert_eq!(find_result(&out).1, Some(1), "advances to A4 after A3");
}

/// `SheetScope::All` widens the rewrite to every sheet; a single Undo reverts both.
#[test]
fn replace_all_spans_sheets() {
    let mut state = TrameState::new();
    // Sheet1!A1 = "foo".
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        text: "foo".to_owned(),
    });
    // Sheet2!A1 = "foo".
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet2"),
    });
    let s2 = second_sheet_id(&state);
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        text: "foo".to_owned(),
    });
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: Id::<Sheet>::from_text("s0".to_owned()),
    });

    let out = state.handle_command(TrameCommand::ReplaceAll {
        query: find_query(
            "foo".to_owned(),
            crate::FindOptions {
                sheets: crate::SheetScope::All,
                ..Default::default()
            },
        ),
        replacement: "bar".to_owned(),
    });
    assert_eq!(replaced_count(&out), 2);
    assert_eq!(read_cell(&mut state, 0, 0), "bar");
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    assert_eq!(read_cell(&mut state, 0, 0), "bar", "other sheet rewritten");

    // One Undo reverts the whole cross-sheet replace.
    state.handle_command(TrameCommand::Undo);
    assert_eq!(read_cell(&mut state, 0, 0), "foo");
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: Id::<Sheet>::from_text("s0".to_owned()),
    });
    assert_eq!(read_cell(&mut state, 0, 0), "foo");
}

/// A CSV import lands its values into a freshly created sheet, and the
/// reported dimensions match the parsed grid.
#[test]
fn csv_import_creates_sheet_and_cells() {
    let mut state = TrameState::new();
    let csv = b"a,b,c\n1,2,3\n".to_vec();
    let out = state.handle_command(TrameCommand::ImportCsv {
        data: ambre_core::Payload::<ambre_core::FileBytes>::new(csv),
        delimiter: None,
        header: HeaderRow::Absent,
        target_sheet_id: None,
        start_row: Pos::<Row>::new(0),
        start_col: Pos::<Col>::new(0),
    });
    let (sheet_id, rows, cols) = import_result(&out);
    assert_eq!((rows, cols), (2, 3));
    // The import activates the new sheet, so read_cell reads it.
    assert_eq!(state.active_sheet, sheet_id);
    assert_eq!(read_cell(&mut state, 0, 0), "a");
    assert_eq!(read_cell(&mut state, 1, 2), "3");
}

/// A header row skips the first record.
#[test]
fn csv_import_skips_header() {
    let mut state = TrameState::new();
    let out = state.handle_command(TrameCommand::ImportCsv {
        data: ambre_core::Payload::<ambre_core::FileBytes>::new(b"name,age\nalice,30\n".to_vec()),
        delimiter: None,
        header: HeaderRow::Present,
        target_sheet_id: None,
        start_row: Pos::<Row>::new(0),
        start_col: Pos::<Col>::new(0),
    });
    let (_, rows, _) = import_result(&out);
    assert_eq!(rows, 1);
    assert_eq!(read_cell(&mut state, 0, 0), "alice");
}

/// A semicolon-delimited file is auto-detected.
#[test]
fn csv_delimiter_autodetect() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::ImportCsv {
        data: ambre_core::Payload::<ambre_core::FileBytes>::new(b"x;y;z\n1;2;3\n".to_vec()),
        delimiter: None,
        header: HeaderRow::Absent,
        target_sheet_id: None,
        start_row: Pos::<Row>::new(0),
        start_col: Pos::<Col>::new(0),
    });
    assert_eq!(read_cell(&mut state, 0, 1), "y");
    assert_eq!(read_cell(&mut state, 1, 2), "3");
}

/// Import then export reproduces the same grid (values round-trip).
#[test]
fn csv_import_export_roundtrip() {
    let mut state = TrameState::new();
    let out = state.handle_command(TrameCommand::ImportCsv {
        data: ambre_core::Payload::<ambre_core::FileBytes>::new(b"a,b\nc,d\n".to_vec()),
        delimiter: None,
        header: HeaderRow::Absent,
        target_sheet_id: None,
        start_row: Pos::<Row>::new(0),
        start_col: Pos::<Col>::new(0),
    });
    let (sheet_id, _, _) = import_result(&out);
    let out = state.handle_command(TrameCommand::ExportCsv {
        sheet_id,
        delimiter: None,
        range: None,
    });
    let bytes = export_bytes(&out);
    assert_eq!(String::from_utf8(bytes).unwrap(), "a,b\nc,d\n");
}

/// Malformed CSV input must never panic the Logic thread
/// (`trame-import-export`, and the CSV fuzz belt of `ambre-spec-fixtures.md`).
/// Ragged rows, embedded quotes/newlines, mixed delimiters, lone quotes and
/// random bytes all parse to *some* graceful result, with no `unwrap` blow-up.
#[test]
fn csv_import_malformed_never_panics() {
    // Hand-picked adversarial inputs covering the documented failure shapes.
    let cases: &[&[u8]] = &[
        b"",                                                // empty file
        b"\n\n\n",                                          // blank lines
        b"a,b,c\n1,2\n3\n4,5,6,7,8",                        // ragged rows
        b"\"unterminated,quote\nnext,row",                  // lone opening quote
        b"a,\"b\nc\",d\n",                                  // newline in a quoted field
        b"a,\"b\"\"q\"\"\",c\n",                            // escaped quotes
        b"x;y,z\tw|v\n1;2,3\t4|5",                          // mixed delimiters
        b",,,\n,,,\n",                                      // empty fields
        &[0xff, 0xfe, 0x00, 0x01, b'\n', 0x80, b',', 0x7f], // invalid UTF-8 bytes
    ];
    for (i, data) in cases.iter().enumerate() {
        let mut state = TrameState::new();
        // Each variant: auto-detect and forced delimiters, with/without header.
        for delimiter in [
            None,
            Some(crate::CsvDelimiter::COMMA),
            Some(crate::CsvDelimiter::SEMICOLON),
        ] {
            for header in [HeaderRow::Absent, HeaderRow::Present] {
                // Must return without panicking; the result is unspecified but
                // well-formed (an ImportComplete or an Error event).
                let _ = state.handle_command(TrameCommand::ImportCsv {
                    data: ambre_core::Payload::<ambre_core::FileBytes>::new(data.to_vec()),
                    delimiter,
                    header,
                    target_sheet_id: None,
                    start_row: Pos::<Row>::new(0),
                    start_col: Pos::<Col>::new(0),
                });
            }
        }
        // Sanity: the state survived and still serves a viewport.
        let _ = read_cell(&mut state, 0, 0);
        let _ = i; // index kept for readability when a case trips a debugger
    }
}

/// Pseudo-random byte fuzz over `ImportCsv`: a deterministic LCG generates a few
/// hundred arbitrary inputs; none may panic. Complements the crafted cases above
/// with broad coverage of byte soup (`trame-import-export`).
#[test]
fn csv_import_random_bytes_never_panics() {
    // Deterministic linear-congruential generator -- no external rng dependency.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    for _ in 0..400 {
        let len = (next() % 64) as usize;
        // Bias toward CSV-significant bytes so structure (delimiters, quotes,
        // newlines) shows up, with occasional arbitrary bytes mixed in.
        let alphabet = b",;\n\"\r\t ab1\xff";
        let data: Vec<u8> = (0..len)
            .map(|_| alphabet[(next() as usize) % alphabet.len()])
            .collect();
        let mut state = TrameState::new();
        let _ = state.handle_command(TrameCommand::ImportCsv {
            data: ambre_core::Payload::<ambre_core::FileBytes>::new(data),
            delimiter: None,
            header: if next() % 2 == 0 {
                HeaderRow::Present
            } else {
                HeaderRow::Absent
            },
            target_sheet_id: None,
            start_row: Pos::<Row>::new(0),
            start_col: Pos::<Col>::new(0),
        });
    }
}

/// Export is bounded to the actually-used range. Columns materialized by a
/// scroll or a since-cleared edit (a far cell written then deleted) must not
/// pad the output with trailing empty columns.
#[test]
fn csv_export_trims_empty_trailing_columns() {
    let mut state = TrameState::new();
    // Materialize out to column R (17) by writing then clearing a far cell.
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(17),
        text: "x".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(17),
        text: String::new(),
    });
    // Real content only in columns A..C.
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(1),
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(2),
        text: "3".to_owned(),
    });

    let out = state.handle_command(TrameCommand::ExportActiveSheetCsv);
    let bytes = export_bytes(&out);
    assert_eq!(String::from_utf8(bytes).unwrap(), "1,2,3\n");
}

/// Reproduce the real flow: import a CSV whose rows carry trailing empty
/// columns, then export the active sheet. The output must NOT carry the
/// padding columns back.
#[test]
fn csv_import_then_export_drops_trailing_empties() {
    let mut state = TrameState::new();
    let csv = b"12,13,14,15,16,17,,,,,,,,,,,,\n12,13,14,15,16,17,,,,,,,,,,,,\n".to_vec();
    let out = state.handle_command(TrameCommand::ImportCsv {
        data: ambre_core::Payload::<ambre_core::FileBytes>::new(csv),
        delimiter: None,
        header: HeaderRow::Absent,
        target_sheet_id: None,
        start_row: Pos::<Row>::new(0),
        start_col: Pos::<Col>::new(0),
    });
    let _ = import_result(&out);

    let out = state.handle_command(TrameCommand::ExportActiveSheetCsv);
    let text = String::from_utf8(export_bytes(&out)).unwrap();
    assert_eq!(text, "12,13,14,15,16,17\n12,13,14,15,16,17\n");
}
