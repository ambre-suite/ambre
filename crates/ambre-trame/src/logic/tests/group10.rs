// ambre-trame/src/logic/tests/group10.rs
// TrameState unit tests (group 10): the fill, the paste and the replace held
// by what they refuse (`Trame cell editing held`).

use super::*;
use crate::FillMode;

fn set(state: &mut TrameState, row: u32, col: u32, text: &str) {
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(row),
        col: Col::new(col),
        text: text.to_owned(),
    });
    // Seal the edit as its own undo item, as the UI's cell exit does.
    state.handle_command(TrameCommand::CommitEdit);
}

fn range(row: u32, col: u32, row_end: u32, col_end: u32) -> crate::CellRect {
    crate::CellRect::spanning(
        crate::CellPos::new(Row::new(row), Col::new(col)),
        crate::CellPos::new(Row::new(row_end), Col::new(col_end)),
    )
}

fn fill(state: &mut TrameState, src: crate::CellRect, dest: crate::CellRect) -> TrameOutcome {
    let sheet_id = state.active_sheet.clone();
    state.handle_command(TrameCommand::FillRange {
        sheet_id,
        src,
        dest,
        mode: FillMode::Auto,
    })
}

fn merge(state: &mut TrameState, row: u32, col: u32, row_end: u32, col_end: u32) {
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(row), Col::new(col)),
            crate::CellPos::new(Row::new(row_end), Col::new(col_end)),
        ),
    });
}

/// A text source with no numeric pattern repeats verbatim on the plain path,
/// one value after the other along the drag (`TRAME-UI-068`).
#[test]
fn a_text_source_repeats_verbatim() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "a");
    set(&mut state, 1, 0, "b");
    fill(&mut state, range(0, 0, 1, 0), range(0, 0, 4, 0));
    let column: Vec<String> = (0..5).map(|r| read_cell(&mut state, r, 0)).collect();
    assert_eq!(column, ["a", "b", "a", "b", "a"]);
}

/// A merged fill writes every line up to the drag edge (`TRAME-UI-134`). A
/// block the edge cuts is written to the edge. An upward fill stays on the
/// source's block grid, so it never lands on the source.
#[test]
fn a_merged_fill_writes_up_to_the_drag_edge() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "x");
    merge(&mut state, 0, 0, 1, 0);
    // Down three rows: one whole block (A3:A4) and a cut one (A5).
    fill(&mut state, range(0, 0, 1, 0), range(0, 0, 4, 0));
    let a3 = read_display(&mut state, 2, 0);
    assert_eq!((a3.text.as_str(), a3.merge_span), ("x", Some(span(2, 1))));
    let a5 = read_display(&mut state, 4, 0);
    assert_eq!(a5.text, "x", "the cut block is written");
    assert_eq!(
        a5.merge_span, None,
        "a block cut to one cell is a plain cell"
    );

    // Up from a block at A5:A6 to A2: a whole block (A3:A4) and a cut one (A2).
    let mut state = TrameState::new();
    set(&mut state, 4, 1, "y");
    merge(&mut state, 4, 1, 5, 1);
    fill(&mut state, range(4, 1, 5, 1), range(1, 1, 5, 1));
    let b5 = read_display(&mut state, 4, 1);
    assert_eq!((b5.text.as_str(), b5.merge_span), ("y", Some(span(2, 1))));
    let b3 = read_display(&mut state, 2, 1);
    assert_eq!((b3.text.as_str(), b3.merge_span), ("y", Some(span(2, 1))));
    assert_eq!(read_cell(&mut state, 1, 1), "y", "the cut block is written");
    assert_eq!(
        read_cell(&mut state, 0, 1),
        "",
        "nothing above the drag edge"
    );
}

/// The tile walk alone: every line between the source and the edge belongs to
/// one tile, and no tile overlaps the source (`TRAME-UI-134`).
#[test]
fn the_merged_tiles_cover_the_destination_outside_the_source() {
    use super::super::fill::merged_tiles as tiles;
    for (s1, s2, d1, d2, b) in [
        (4, 5, 1, 5, 2),
        (0, 1, 0, 6, 2),
        (3, 5, 0, 10, 3),
        (2, 2, 0, 7, 1),
    ] {
        let (r1, r2, rd1, rd2) = (Row::new(s1), Row::new(s2), Row::new(d1), Row::new(d2));
        let mut covered: Vec<u32> = tiles(r1, r2, rd1, rd2, RowCount::new(b))
            .iter()
            .flat_map(|t| t.first.get()..t.first.get() + t.len.get())
            .collect();
        covered.sort_unstable();
        let expected: Vec<u32> = (d1..=d2).filter(|l| *l < s1 || *l > s2).collect();
        assert_eq!(
            covered, expected,
            "source {s1}..={s2}, drag {d1}..={d2}, block {b}"
        );
    }
}

/// A whole fill is one undo item: one undo clears every filled cell and
/// leaves the source as it was typed (`TRAME-UI-073`).
#[test]
fn a_fill_is_one_undo_item() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "1");
    set(&mut state, 1, 0, "2");
    let before = state.undo_manager.undo_count();
    fill(&mut state, range(0, 0, 1, 0), range(0, 0, 5, 0));
    state.handle_command(TrameCommand::CommitEdit);
    assert_eq!(read_cell(&mut state, 5, 0), "6");
    assert_eq!(state.undo_manager.undo_count(), before + 1);

    state.handle_command(TrameCommand::Undo);
    let column: Vec<String> = (0..6).map(|r| read_cell(&mut state, r, 0)).collect();
    assert_eq!(column, ["1", "2", "", "", "", ""]);
}

/// A fill writes inside the rectangle the drag covered and nowhere else, and
/// a destination that does not hold the source writes nothing
/// (`TRAME-UI-071`).
#[test]
fn a_fill_writes_only_inside_the_drag() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "a");
    set(&mut state, 5, 0, "keep");
    set(&mut state, 2, 1, "keep");
    fill(&mut state, range(0, 0, 0, 0), range(0, 0, 3, 0));
    assert_eq!(read_cell(&mut state, 3, 0), "a");
    assert_eq!(read_cell(&mut state, 4, 0), "", "below the drag edge");
    assert_eq!(read_cell(&mut state, 5, 0), "keep");
    assert_eq!(read_cell(&mut state, 2, 1), "keep", "beside the drag");

    let out = fill(&mut state, range(0, 0, 0, 0), range(6, 0, 8, 0));
    assert!(out.update.is_none(), "a malformed drag sent a delta");
    assert_eq!(read_cell(&mut state, 6, 0), "");
}

/// A drag that ends where it started writes nothing (`TRAME-UI-072`).
#[test]
fn a_drag_back_to_its_start_writes_nothing() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "1");
    set(&mut state, 1, 0, "2");
    let before = state.undo_manager.undo_count();
    let out = fill(&mut state, range(0, 0, 1, 0), range(0, 0, 1, 0));
    assert!(out.update.is_none());
    assert_eq!(state.undo_manager.undo_count(), before);
}

/// A fill that names a sheet other than the active one touches nothing
/// (`TRAME-UI-074`).
#[test]
fn a_fill_on_another_sheet_touches_nothing() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "a");
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet2"),
    });
    let other = second_sheet_id(&state);
    let out = state.handle_command(TrameCommand::FillRange {
        sheet_id: other.clone(),
        src: range(0, 0, 0, 0),
        dest: range(0, 0, 3, 0),
        mode: FillMode::Auto,
    });
    assert!(out.update.is_none(), "the fill wrote somewhere");
    assert_eq!(read_cell(&mut state, 1, 0), "");
    state.handle_command(TrameCommand::ActivateSheet { sheet_id: other });
    assert_eq!(read_cell(&mut state, 1, 0), "");
}

/// A paste of any size is one undo item (`TRAME-UI-063`).
#[test]
fn a_paste_is_one_undo_item() {
    let mut state = TrameState::new();
    for r in 0..3 {
        for c in 0..3 {
            set(&mut state, r, c, &format!("{r}{c}"));
        }
    }
    select(&mut state, 0, 0, 2, 2);
    let payload = copied(&mut state);
    let before = state.undo_manager.undo_count();
    state.handle_command(TrameCommand::PasteRange {
        target_row: Row::new(10),
        target_col: Col::new(0),
        payload,
    });
    state.handle_command(TrameCommand::CommitEdit);
    assert_eq!(read_cell(&mut state, 12, 2), "22");
    assert_eq!(state.undo_manager.undo_count(), before + 1);
    state.handle_command(TrameCommand::Undo);
    for r in 10..13 {
        for c in 0..3 {
            assert_eq!(
                read_cell(&mut state, r, c),
                "",
                "({r},{c}) survived the undo"
            );
        }
    }
}

/// A blank cell of the copied range leaves the cell it lands on as it was
/// (`TRAME-UI-062`).
#[test]
fn a_blank_source_cell_leaves_its_destination() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "a");
    set(&mut state, 0, 2, "c");
    set(&mut state, 5, 1, "kept");
    select(&mut state, 0, 0, 0, 2);
    let payload = copied(&mut state);
    state.handle_command(TrameCommand::PasteRange {
        target_row: Row::new(5),
        target_col: Col::new(0),
        payload,
    });
    let row: Vec<String> = (0..3).map(|c| read_cell(&mut state, 5, c)).collect();
    assert_eq!(row, ["a", "kept", "c"]);
}

/// A replace across every sheet leaves the user on the sheet they were
/// looking at, and announces no other (`TRAME-UI-138`).
#[test]
fn a_workbook_replace_keeps_the_active_sheet() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "foo");
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet2"),
    });
    let other = second_sheet_id(&state);
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: other.clone(),
    });
    set(&mut state, 0, 0, "foo");
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: SheetId("s0".to_owned()),
    });

    let out = state.handle_command(TrameCommand::ReplaceAll {
        query: "foo".to_owned(),
        replacement: "bar".to_owned(),
        options: crate::FindOptions {
            sheets: crate::SheetScope::All,
            ..Default::default()
        },
    });
    assert_eq!(state.active_sheet, default_sheet());
    assert!(
        !out.display.iter().any(|e| matches!(
            e,
            TrameEvent::SheetList { active, .. } if active.0 != "s0"
        )),
        "the replace moved the user to another sheet"
    );
    assert_eq!(read_cell(&mut state, 0, 0), "bar");
}

fn select(state: &mut TrameState, row: u32, col: u32, row_end: u32, col_end: u32) {
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(row),
        col: Col::new(col),
        anchor_row: Row::new(row_end),
        anchor_col: Col::new(col_end),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
}

fn copied(state: &mut TrameState) -> crate::ClipboardPayload {
    let out = state.handle_command(TrameCommand::CopyRange);
    let Some(TrameEvent::Clipboard(payload)) = out.display.into_iter().next() else {
        panic!("expected a Clipboard event");
    };
    payload
}
