// ambre-trame/src/logic/tests/group11.rs
// TrameState unit tests (group 11): merges, lines, sizes and sheets held by
// what they leave alone (`Trame sheet structure held`).

use super::*;
use crate::logic::schema::{
    DEFAULT_SHEET, MATERIALIZE_CAP, MIN_COL_WIDTH, cells_map_name, cellstyle_map_name,
    merges_map_name,
};
use crate::model::cell_key;
use crate::{FillMode, GridRange, SelectionKind};
use ambre_core::Rgb;
use loro::LoroValue;

fn set(state: &mut TrameState, row: u32, col: u32, text: &str) {
    state.handle_command(TrameCommand::SetCellText {
        row,
        col,
        text: text.to_owned(),
    });
    state.handle_command(TrameCommand::CommitEdit);
}

fn select(state: &mut TrameState, row: u32, col: u32, row_end: u32, col_end: u32) {
    state.handle_command(TrameCommand::SetActiveCell {
        row,
        col,
        anchor_row: row_end,
        anchor_col: col_end,
        extra: vec![],
        kind: SelectionKind::Cells,
    });
}

fn merge(state: &mut TrameState, row: u32, col: u32, row_end: u32, col_end: u32) -> TrameOutcome {
    state.handle_command(TrameCommand::MergeCells {
        row,
        col,
        row_end,
        col_end,
    })
}

fn range(row: u32, col: u32, row_end: u32, col_end: u32) -> GridRange {
    GridRange {
        row,
        col,
        row_end,
        col_end,
    }
}

fn fill(state: &mut TrameState, src: GridRange, dest: GridRange) {
    let sheet_id = SheetId(state.active_sheet.clone());
    state.handle_command(TrameCommand::FillRange {
        sheet_id,
        src,
        dest,
        mode: FillMode::Auto,
    });
}

/// The number of operations this peer has written into the document.
fn own_ops(state: &TrameState) -> i32 {
    let peer = state.doc.peer_id();
    state.doc.oplog_vv().get(&peer).copied().unwrap_or(0)
}

/// The value of one named container, for a before-and-after comparison.
fn container(state: &TrameState, name: String) -> LoroValue {
    state.doc.get_map(name).get_value()
}

/// The widths and heights of the first `n` columns and rows.
fn metrics(state: &mut TrameState, n: u32) -> (Vec<f32>, Vec<f32>) {
    let out = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(3),
        row_start: 0,
        row_end: n - 1,
        col_start: 0,
        col_end: n - 1,
    });
    let Some(TrameEvent::ViewportData {
        col_widths,
        row_heights,
        ..
    }) = out.display.into_iter().next()
    else {
        panic!("expected ViewportData");
    };
    (col_widths, row_heights)
}

/// Two peers on one workbook, each with its own identity.
fn two_peers(base: &TrameState) -> (TrameState, TrameState) {
    let snapshot = base.export_snapshot();
    let mut a = TrameState::with_peer(ambre_core::PeerId(11));
    a.import_snapshot(&snapshot).expect("the snapshot reads");
    let mut b = TrameState::with_peer(ambre_core::PeerId(22));
    b.import_snapshot(&snapshot).expect("the snapshot reads");
    (a, b)
}

/// A merge changes no cell outside its rectangle (`TRAME-131`).
#[test]
fn a_merge_leaves_its_neighbours() {
    let mut state = TrameState::new();
    for r in 0..4 {
        for c in 0..4 {
            let inside = (1..=2).contains(&r) && (1..=2).contains(&c);
            if !inside {
                set(&mut state, r, c, &format!("{r}{c}"));
            }
        }
    }
    set(&mut state, 1, 1, "m");
    merge(&mut state, 1, 1, 2, 2);
    assert_eq!(read_display(&mut state, 1, 1).merge_span, Some((2, 2)));
    for r in 0..4 {
        for c in 0..4 {
            if (1..=2).contains(&r) && (1..=2).contains(&c) {
                continue;
            }
            let cell = read_display(&mut state, r, c);
            assert_eq!(cell.text, format!("{r}{c}"), "({r},{c}) changed");
            assert!(!cell.is_merged, "({r},{c}) joined the merge");
        }
    }
}

/// An entry of one cell is no merge, from this peer or from another
/// (`TRAME-128`).
#[test]
fn a_one_cell_merge_from_a_peer_is_no_merge() {
    let mut base = TrameState::new();
    set(&mut base, 0, 0, "a");
    set(&mut base, 1, 0, "b");
    let (mut a, mut b) = two_peers(&base);
    // The peer writes the entry the local command refuses to write.
    let (rl, cl) = b.index.resolve(0, 0).expect("A1 is materialized");
    b.doc
        .get_map(merges_map_name(TEST_SHEET))
        .insert(&cell_key(rl, cl), cell_key(rl, cl).as_str())
        .expect("the entry");
    b.doc.commit();
    let update = b.flush_delta().expect("the entry is a delta");
    a.apply_remote(&update);
    let a1 = read_display(&mut a, 0, 0);
    assert_eq!((a1.merge_span, a1.is_merged), (None, false));
    assert!(a.merges.is_empty() && a.covered.is_empty());
}

/// A merge whose corner lies past the materialization cap writes no entry
/// and sends nothing (`TRAME-148`).
#[test]
fn a_merge_past_the_cap_writes_no_entry() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "a");
    let before = own_ops(&state);
    let out = merge(&mut state, 0, 0, MATERIALIZE_CAP as u32 + 5, 0);
    assert!(out.update.is_none(), "the refused merge sent a delta");
    assert_eq!(own_ops(&state), before);
    assert!(
        matches!(container(&state, merges_map_name(TEST_SHEET)), LoroValue::Map(m) if m.is_empty())
    );
    assert_eq!(read_display(&mut state, 0, 0).merge_span, None);
}

/// A value a conflicting merge masked rejoins the formulas once the merge
/// is split (`TRAME-146`). The detection of that conflict writes nothing
/// (`TRAME-031`).
#[test]
fn an_unmerge_returns_a_masked_value_to_the_formulas() {
    let mut base = TrameState::new();
    set(&mut base, 0, 0, "1");
    set(&mut base, 0, 2, "=A2*2");
    // Both peers must share row 2, so the base materializes it.
    set(&mut base, 1, 1, "pad");
    let (mut a, mut b) = two_peers(&base);
    // A merges A1:A2 while B types into A2: the merged peer masks B's value.
    let merged = merge(&mut a, 0, 0, 1, 0).update.expect("the merge");
    let typed = b
        .handle_command(TrameCommand::SetCellText {
            row: 1,
            col: 0,
            text: "5".to_owned(),
        })
        .update
        .expect("the typed value");
    a.apply_remote(&typed);
    let own = own_ops(&b);
    let events = b.apply_remote(&merged);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TrameEvent::MergeConflictDetected { .. })),
        "no conflict detected"
    );
    assert_eq!(own_ops(&b), own, "the detection wrote to the document");
    assert!(b.flush_delta().is_none(), "the detection owes a delta");
    assert_eq!(read_cell(&mut b, 0, 2), "0", "the masked value computed");

    b.handle_command(TrameCommand::UnmergeCells { row: 0, col: 0 });
    assert_eq!(read_cell(&mut b, 1, 0), "5");
    assert_eq!(read_cell(&mut b, 0, 2), "10", "the freed value is ignored");
}

/// An unmerge over a selection that crosses no merge leaves the document
/// as it was (`TRAME-137`).
#[test]
fn an_unmerge_crossing_no_merge_writes_nothing() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "a");
    merge(&mut state, 5, 5, 6, 6);
    select(&mut state, 0, 0, 2, 2);
    let before = own_ops(&state);
    let out = state.handle_command(TrameCommand::UnmergeActive);
    assert!(out.update.is_none() && out.display.is_empty());
    assert_eq!(own_ops(&state), before);
    assert_eq!(read_display(&mut state, 5, 5).merge_span, Some((2, 2)));
}

/// A merged fill empties the covered cells of each tile and leaves the
/// source blocks as they were (`TRAME-147`, `TRAME-142`).
#[test]
fn a_merged_fill_clears_its_tiles_and_keeps_its_source() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "x");
    merge(&mut state, 0, 0, 1, 0);
    set(&mut state, 3, 0, "under");
    set(&mut state, 5, 0, "under");
    fill(&mut state, range(0, 0, 1, 0), range(0, 0, 5, 0));
    for row in [3, 5] {
        let cell = read_display(&mut state, row, 0);
        assert_eq!(cell.raw, "", "row {row} kept a value under a tile");
        assert!(cell.is_merged);
    }
    assert!(state.masked_cells.is_empty(), "a tile masks a value");
    let a1 = read_display(&mut state, 0, 0);
    assert_eq!((a1.text.as_str(), a1.merge_span), ("x", Some((1, 2))));
    let a2 = read_display(&mut state, 1, 0);
    assert_eq!((a2.raw.as_str(), a2.is_merged), ("", true));
}

/// Each tile master takes the style of the source master, and a formula
/// shifts by the tile offset (`TRAME-149`, `TRAME-150`).
#[test]
fn a_merged_fill_carries_the_style_and_shifts_the_formula() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "=B1");
    merge(&mut state, 0, 0, 1, 0);
    select(&mut state, 0, 0, 0, 0);
    state.handle_command(TrameCommand::SetCellStyle {
        patch: crate::StylePatch {
            bold: Some(true),
            bg: Some(Some(Rgb::from_bits(0x00ff_0000))),
            ..Default::default()
        },
    });
    fill(&mut state, range(0, 0, 1, 0), range(0, 0, 5, 0));
    for (row, formula) in [(2, "=B3"), (4, "=B5")] {
        let cell = read_display(&mut state, row, 0);
        assert_eq!(cell.raw, formula, "tile at row {row}");
        assert!(cell.bold, "tile at row {row} lost the bold");
        assert_eq!(
            cell.bg,
            Some(Rgb::from_bits(0x00ff_0000)),
            "tile at row {row}"
        );
    }
}

/// A source of unequal blocks fills cell by cell and merges nothing
/// (`TRAME-143`).
#[test]
fn a_source_of_unequal_blocks_fills_cell_by_cell() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "a");
    set(&mut state, 2, 0, "b");
    merge(&mut state, 0, 0, 1, 0);
    merge(&mut state, 2, 0, 4, 0);
    fill(&mut state, range(0, 0, 4, 0), range(0, 0, 9, 0));
    let column: Vec<String> = (5..10).map(|r| read_cell(&mut state, r, 0)).collect();
    assert_eq!(column, ["a", "", "b", "", ""]);
    for row in 5..10 {
        assert!(
            !read_display(&mut state, row, 0).is_merged,
            "row {row} merged"
        );
    }
}

/// Moving the selection writes nothing (`TRAME-UI-115`).
#[test]
fn a_selection_move_writes_nothing() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "a");
    merge(&mut state, 2, 2, 3, 3);
    let before = own_ops(&state);
    let moves = [
        (1, 1, 1, 1, SelectionKind::Cells),
        (2, 2, 3, 3, SelectionKind::Cells),
        (0, 0, 9, 9, SelectionKind::Rows),
        (500, 3, 500, 3, SelectionKind::Cols),
    ];
    for (row, col, anchor_row, anchor_col, kind) in moves {
        let out = state.handle_command(TrameCommand::SetActiveCell {
            row,
            col,
            anchor_row,
            anchor_col,
            extra: vec![],
            kind,
        });
        assert!(out.update.is_none(), "a selection move sent a delta");
    }
    assert_eq!(own_ops(&state), before);
}

/// A line released on its own slot writes nothing (`TRAME-173`).
#[test]
fn a_line_released_on_its_slot_writes_nothing() {
    let mut state = TrameState::new();
    for r in 0..3 {
        set(&mut state, r, r, "v");
    }
    let before = own_ops(&state);
    for cmd in [
        TrameCommand::MoveRow { from: 1, to: 1 },
        TrameCommand::MoveColumn { from: 2, to: 2 },
    ] {
        let out = state.handle_command(cmd);
        assert!(out.update.is_none() && out.display.is_empty());
    }
    assert_eq!(own_ops(&state), before);
}

/// The height, the width and the hidden mark of a line follow it when the
/// lines move (`TRAME-170`). A width stays with its column through an
/// insert, a delete and a move (`TRAME-UI-037`).
#[test]
fn a_line_keeps_its_size_and_its_hidden_mark_when_it_moves() {
    let mut state = TrameState::new();
    for (r, v) in ["a", "b", "c", "d"].iter().enumerate() {
        set(&mut state, r as u32, 0, v);
    }
    let sheet_id = SheetId(state.active_sheet.clone());
    state.handle_command(TrameCommand::ResizeRow {
        version: ambre_bus::ViewportVersion(1),
        sheet_id: sheet_id.clone(),
        row: 1,
        height_px: 40.0,
    });
    state.handle_command(TrameCommand::ResizeCol {
        version: ambre_bus::ViewportVersion(1),
        sheet_id: sheet_id.clone(),
        col: 2,
        width_px: 150.0,
    });
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id,
        col: 0,
        kind: FilterKind::Values(vec!["a".to_owned(), "c".to_owned(), "d".to_owned()]),
    });
    assert!(is_filtered(&mut state, 1, 0));
    let (_, heights) = metrics(&mut state, 6);
    let default_height = heights[0];

    // Row B moves to the bottom: its height and its hidden mark go with it.
    state.handle_command(TrameCommand::MoveRow { from: 1, to: 3 });
    assert_eq!(read_cell(&mut state, 3, 0), "b");
    let (_, heights) = metrics(&mut state, 6);
    assert_eq!((heights[1], heights[3]), (default_height, 40.0));
    assert!(is_filtered(&mut state, 3, 0) && !is_filtered(&mut state, 1, 0));

    // Column C is pushed right, moved, then pulled back by a delete.
    let (widths, _) = metrics(&mut state, 6);
    let default_width = widths[0];
    state.handle_command(TrameCommand::InsertColumn { at: 0 });
    let (widths, _) = metrics(&mut state, 6);
    assert_eq!((widths[2], widths[3]), (default_width, 150.0));
    state.handle_command(TrameCommand::MoveColumn { from: 3, to: 1 });
    let (widths, _) = metrics(&mut state, 6);
    assert_eq!((widths[1], widths[3]), (150.0, default_width));
    state.handle_command(TrameCommand::DeleteColumn { at: 0 });
    let (widths, _) = metrics(&mut state, 6);
    assert_eq!((widths[0], widths[1]), (150.0, default_width));
}

/// A width never drops below the minimum, wherever the pointer went
/// (`TRAME-UI-126`).
#[test]
fn a_column_is_never_narrower_than_its_minimum() {
    let mut state = TrameState::new();
    let sheet_id = SheetId(state.active_sheet.clone());
    for (col, width) in [(0, 3.0), (1, 0.0), (2, -40.0)] {
        state.handle_command(TrameCommand::ResizeCol {
            version: ambre_bus::ViewportVersion(1),
            sheet_id: sheet_id.clone(),
            col,
            width_px: width,
        });
    }
    let (widths, _) = metrics(&mut state, 3);
    assert_eq!(widths, [MIN_COL_WIDTH; 3]);
}

/// A write ten thousand lines past the used range is refused with no
/// delta, and the line before that limit is written (`TRAME-089`).
#[test]
fn a_write_past_the_line_cap_is_refused() {
    let mut state = TrameState::new();
    let cap = MATERIALIZE_CAP as u32;
    // The document's own setup leaves first, so only the write is measured.
    state.flush_delta();
    let before = own_ops(&state);
    let out = state.handle_command(TrameCommand::SetCellText {
        row: cap,
        col: 0,
        text: "far".to_owned(),
    });
    assert!(out.update.is_none(), "the refused write sent a delta");
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, TrameEvent::Error { .. }))
    );
    assert_eq!(own_ops(&state), before);
    set(&mut state, cap - 1, 0, "near");
    assert_eq!(read_cell(&mut state, cap - 1, 0), "near");
}

/// Two sheets beside the default one, each holding a value, a formula and
/// a bold cell. Returns their ids, the default first.
fn three_sheets(state: &mut TrameState) -> Vec<String> {
    for name in ["Two", "Three"] {
        state.handle_command(TrameCommand::CreateSheet {
            name: name.to_owned(),
        });
    }
    let ids = state.sheet_order_ids();
    for id in &ids {
        state.handle_command(TrameCommand::ActivateSheet {
            sheet_id: SheetId(id.clone()),
        });
        set(state, 0, 0, "4");
        set(state, 0, 1, "=A1*2");
        select(state, 0, 0, 0, 0);
        state.handle_command(TrameCommand::SetCellStyle {
            patch: bold_patch(true),
        });
    }
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: SheetId(ids[0].clone()),
    });
    ids
}

/// The cells and the styles of every sheet in `ids`.
fn contents(state: &TrameState, ids: &[String]) -> Vec<(LoroValue, LoroValue)> {
    ids.iter()
        .map(|id| {
            (
                container(state, cells_map_name(id)),
                container(state, cellstyle_map_name(id)),
            )
        })
        .collect()
}

/// A rename leaves the sheet at its slot in the strip (`TRAME-161`).
#[test]
fn a_rename_keeps_the_tab_order() {
    let mut state = TrameState::new();
    let ids = three_sheets(&mut state);
    for (id, name) in [(&ids[0], "Zulu"), (&ids[2], "Alpha")] {
        state.handle_command(TrameCommand::RenameSheet {
            sheet_id: SheetId(id.clone()),
            name: name.to_owned(),
        });
    }
    assert_eq!(state.sheet_order_ids(), ids);
}

/// A tab reorder changes no cell, style or formula (`TRAME-162`).
#[test]
fn a_tab_reorder_changes_no_content() {
    let mut state = TrameState::new();
    let ids = three_sheets(&mut state);
    let before = contents(&state, &ids);
    state.handle_command(TrameCommand::MoveSheet {
        sheet_id: SheetId(ids[2].clone()),
        to_index: 0,
    });
    assert_eq!(state.sheet_order_ids()[0], ids[2]);
    assert_eq!(contents(&state, &ids), before);
    assert_eq!(read_cell(&mut state, 0, 1), "8");
}

/// A sheet delete leaves the other sheets' data as it was (`TRAME-164`).
#[test]
fn a_sheet_delete_leaves_the_other_sheets() {
    let mut state = TrameState::new();
    let ids = three_sheets(&mut state);
    let kept = [ids[0].clone(), ids[2].clone()];
    let before = contents(&state, &kept);
    state.handle_command(TrameCommand::DeleteSheet {
        sheet_id: SheetId(ids[1].clone()),
    });
    assert_eq!(state.sheet_order_ids(), kept);
    assert_eq!(contents(&state, &kept), before);
}

/// A tab switch writes nothing and so sends nothing (`TRAME-165`).
#[test]
fn a_tab_switch_writes_nothing() {
    let mut state = TrameState::new();
    let ids = three_sheets(&mut state);
    state.flush_delta();
    let before = own_ops(&state);
    for id in ids.iter().rev() {
        let out = state.handle_command(TrameCommand::ActivateSheet {
            sheet_id: SheetId(id.clone()),
        });
        assert!(out.update.is_none(), "the switch to {id} sent a delta");
    }
    assert_eq!(own_ops(&state), before);
}

/// Two peers adding a sheet at once each keep theirs, and both strips read
/// the same order (`TRAME-071`).
#[test]
fn two_peers_adding_a_sheet_keep_both() {
    let (mut a, mut b) = two_peers(&TrameState::new());
    let from_a = a
        .handle_command(TrameCommand::CreateSheet {
            name: "Budget".to_owned(),
        })
        .update
        .expect("the create");
    let from_b = b
        .handle_command(TrameCommand::CreateSheet {
            name: "Budget".to_owned(),
        })
        .update
        .expect("the create");
    let added_a = a.sheet_order_ids()[1].clone();
    let added_b = b.sheet_order_ids()[1].clone();
    assert_ne!(added_a, added_b, "the two peers minted one id");
    a.apply_remote(&from_b);
    b.apply_remote(&from_a);
    assert_eq!(a.sheet_order_ids(), b.sheet_order_ids());
    let order = a.sheet_order_ids();
    assert_eq!(order.len(), 3);
    assert!(order.contains(&added_a) && order.contains(&added_b));
}

/// The tab strip one peer reads: each sheet's id and name, and the active id.
fn strip(state: &TrameState) -> (Vec<(String, String)>, String) {
    match state.sheet_list_event() {
        TrameEvent::SheetList { sheets, active } => (
            sheets.into_iter().map(|t| (t.id.0, t.name)).collect(),
            active.0,
        ),
        other => panic!("not a sheet list: {other:?}"),
    }
}

/// Two peers deleting the last two sheets at once both read one sheet, the
/// default one, and the merge writes nothing (`TRAME-077`). That sheet is
/// the last one, so a delete of it is refused (`TRAME-022`). The first edit
/// on it stores it, so a sheet a peer adds beside it does not hide it.
#[test]
fn two_peers_deleting_the_last_two_sheets_keep_one() {
    let mut base = TrameState::new();
    base.handle_command(TrameCommand::CreateSheet {
        name: "Two".to_owned(),
    });
    let ids = base.sheet_order_ids();
    assert_eq!(ids.len(), 2);
    let (mut a, mut b) = two_peers(&base);
    a.flush_delta();
    b.flush_delta();
    let from_a = a
        .handle_command(TrameCommand::DeleteSheet {
            sheet_id: SheetId(ids[1].clone()),
        })
        .update
        .expect("the delete");
    let from_b = b
        .handle_command(TrameCommand::DeleteSheet {
            sheet_id: SheetId(ids[0].clone()),
        })
        .update
        .expect("the delete");
    let (ops_a, ops_b) = (own_ops(&a), own_ops(&b));
    a.apply_remote(&from_b);
    b.apply_remote(&from_a);
    let one = (
        vec![(DEFAULT_SHEET.to_owned(), "Sheet1".to_owned())],
        DEFAULT_SHEET.to_owned(),
    );
    assert_eq!(strip(&a), one);
    assert_eq!(strip(&b), one);
    // The merge stored nothing on either peer.
    assert_eq!((own_ops(&a), own_ops(&b)), (ops_a, ops_b));
    assert!(a.flush_delta().is_none() && b.flush_delta().is_none());

    // The last sheet stays: its delete is refused and writes nothing.
    let out = a.handle_command(TrameCommand::DeleteSheet {
        sheet_id: SheetId(DEFAULT_SHEET.to_owned()),
    });
    assert!(out.update.is_none());
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, TrameEvent::Error { .. }))
    );

    // A writes on the sheet while B adds one: both keep both.
    let from_a = a
        .handle_command(TrameCommand::SetCellText {
            row: 0,
            col: 0,
            text: "kept".to_owned(),
        })
        .update
        .expect("the write");
    let from_b = b
        .handle_command(TrameCommand::CreateSheet {
            name: "New".to_owned(),
        })
        .update
        .expect("the create");
    a.apply_remote(&from_b);
    b.apply_remote(&from_a);
    assert_eq!(a.sheet_order_ids(), b.sheet_order_ids());
    assert_eq!(a.sheet_order_ids().len(), 2);
    assert_eq!(a.sheet_order_ids()[0], DEFAULT_SHEET);
    assert_eq!(read_cell(&mut b, 0, 0), "kept");
}
