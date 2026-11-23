// ambre-trame/src/logic/tests/group11.rs
// TrameState unit tests (group 11): merges, lines, sizes and sheets held by
// what they leave alone (`Trame sheet structure held`).

use super::*;
use crate::logic::schema::{
    COL_WIDTH, MATERIALIZE_ROWS, MAX_COL_WIDTH, MAX_ROW_HEIGHT, MIN_COL_WIDTH, MIN_ROW_HEIGHT,
    ROW_HEIGHT, SheetContainer, SheetRoot, default_sheet,
};
use crate::{FillMode, SelectionKind};
use ambre_core::Rgb;
use ambre_core::id::{Id, Sheet};
use ambre_core::index::Pos;
use loro::LoroValue;

fn set(state: &mut TrameState, row: u32, col: u32, text: &str) {
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(row),
        col: Pos::<Col>::new(col),
        text: text.to_owned(),
    });
    state.handle_command(TrameCommand::CommitEdit);
}

fn select(state: &mut TrameState, row: u32, col: u32, row_end: u32, col_end: u32) {
    state.handle_command(TrameCommand::SetActiveCell {
        row: Pos::<Row>::new(row),
        col: Pos::<Col>::new(col),
        anchor_row: Pos::<Row>::new(row_end),
        anchor_col: Pos::<Col>::new(col_end),
        extra: vec![],
        kind: SelectionKind::Cells,
    });
}

fn merge(state: &mut TrameState, row: u32, col: u32, row_end: u32, col_end: u32) -> TrameOutcome {
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)),
            crate::CellPos::new(Pos::<Row>::new(row_end), Pos::<Col>::new(col_end)),
        ),
    })
}

fn range(row: u32, col: u32, row_end: u32, col_end: u32) -> crate::CellRect {
    crate::CellRect::spanning(
        crate::CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)),
        crate::CellPos::new(Pos::<Row>::new(row_end), Pos::<Col>::new(col_end)),
    )
}

fn fill(state: &mut TrameState, src: crate::CellRect, dest: crate::CellRect) {
    let sheet_id = state.active_sheet.clone();
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
fn container(state: &TrameState, name: SheetContainer) -> LoroValue {
    state.doc.get_map(name.as_str()).get_value()
}

/// The widths and heights of the first `n` columns and rows.
fn metrics(state: &mut TrameState, n: u32) -> (Vec<f32>, Vec<f32>) {
    let out = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_core::Seq::nth(3),
        row_start: Pos::<Row>::new(0),
        row_end: Pos::<Row>::new(n - 1),
        col_start: Pos::<Col>::new(0),
        col_end: Pos::<Col>::new(n - 1),
    });
    let Some(TrameEvent::ViewportData {
        col_widths,
        row_heights,
        ..
    }) = out.display.into_iter().next()
    else {
        panic!("expected ViewportData");
    };
    (
        col_widths.iter().map(|w| w.get()).collect(),
        row_heights.iter().map(|h| h.get()).collect(),
    )
}

/// Two peers on one workbook, each with its own identity.
fn two_peers(base: &TrameState) -> (TrameState, TrameState) {
    let snapshot = base.export_snapshot();
    let mut a = TrameState::with_peer(ambre_core::PeerId::new(11));
    a.import_snapshot(&snapshot).expect("the snapshot reads");
    let mut b = TrameState::with_peer(ambre_core::PeerId::new(22));
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
    assert_eq!(read_display(&mut state, 1, 1).merge_span, Some(span(2, 2)));
    for r in 0..4 {
        for c in 0..4 {
            if (1..=2).contains(&r) && (1..=2).contains(&c) {
                continue;
            }
            let cell = read_display(&mut state, r, c);
            assert_eq!(cell.text, format!("{r}{c}"), "({r},{c}) changed");
            assert_eq!(
                cell.is_merged,
                crate::message::Merge::Single,
                "({r},{c}) joined the merge"
            );
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
    let a1 = b
        .index
        .key_at(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)))
        .expect("A1 is materialized");
    b.doc
        .get_map(SheetContainer::of(SheetRoot::Merges, &default_sheet()).as_str())
        .insert(a1.as_str(), a1.as_str())
        .expect("the entry");
    b.doc.commit();
    let update = b.flush_delta().expect("the entry is a delta");
    a.apply_remote(&update);
    let a1 = read_display(&mut a, 0, 0);
    assert_eq!(
        (a1.merge_span, a1.is_merged),
        (None, crate::message::Merge::Single)
    );
    assert!(a.merges.is_empty() && a.covered.is_empty());
}

/// A merge whose corner lies past the materialization cap writes no entry
/// and sends nothing (`TRAME-148`).
#[test]
fn a_merge_past_the_cap_writes_no_entry() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "a");
    let before = own_ops(&state);
    let out = merge(&mut state, 0, 0, MATERIALIZE_ROWS.get() + 5, 0);
    assert!(out.update.is_none(), "the refused merge sent a delta");
    assert_eq!(own_ops(&state), before);
    assert!(
        matches!(container(&state, SheetContainer::of(SheetRoot::Merges, &default_sheet())), LoroValue::Map(m) if m.is_empty())
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
            row: Pos::<Row>::new(1),
            col: Pos::<Col>::new(0),
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

    b.handle_command(TrameCommand::UnmergeCells {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
    });
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
    assert_eq!(read_display(&mut state, 5, 5).merge_span, Some(span(2, 2)));
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
        assert_eq!(cell.is_merged, crate::message::Merge::Merged);
    }
    assert!(state.masked_cells.is_empty(), "a tile masks a value");
    let a1 = read_display(&mut state, 0, 0);
    assert_eq!((a1.text.as_str(), a1.merge_span), ("x", Some(span(2, 1))));
    let a2 = read_display(&mut state, 1, 0);
    assert_eq!(
        (a2.raw.as_str(), a2.is_merged),
        ("", crate::message::Merge::Merged)
    );
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
            bold: Some(ambre_core::Weight::Bold),
            bg: Some(Some(Rgb::from_bits(0x00ff_0000))),
            ..Default::default()
        },
    });
    fill(&mut state, range(0, 0, 1, 0), range(0, 0, 5, 0));
    for (row, formula) in [(2, "=B3"), (4, "=B5")] {
        let cell = read_display(&mut state, row, 0);
        assert_eq!(cell.raw, formula, "tile at row {row}");
        assert_eq!(
            cell.bold,
            ambre_core::Weight::Bold,
            "tile at row {row} lost the bold"
        );
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
        assert_eq!(
            read_display(&mut state, row, 0).is_merged,
            crate::message::Merge::Single,
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
            row: Pos::<Row>::new(row),
            col: Pos::<Col>::new(col),
            anchor_row: Pos::<Row>::new(anchor_row),
            anchor_col: Pos::<Col>::new(anchor_col),
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
        TrameCommand::MoveRow {
            from: Pos::<Row>::new(1),
            to: Pos::<Row>::new(1),
        },
        TrameCommand::MoveColumn {
            from: Pos::<Col>::new(2),
            to: Pos::<Col>::new(2),
        },
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
    let sheet_id = state.active_sheet.clone();
    state.handle_command(TrameCommand::ResizeRow {
        version: ambre_core::Seq::nth(1),
        sheet_id: sheet_id.clone(),
        row: Pos::<Row>::new(1),
        height: ambre_core::Px::<ambre_core::Doc>::new(40.0),
    });
    state.handle_command(TrameCommand::ResizeCol {
        version: ambre_core::Seq::nth(1),
        sheet_id: sheet_id.clone(),
        col: Pos::<Col>::new(2),
        width: ambre_core::Px::<ambre_core::Doc>::new(150.0),
    });
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id,
        col: Pos::<Col>::new(0),
        kind: FilterKind::Values(vec!["a".to_owned(), "c".to_owned(), "d".to_owned()]),
    });
    assert!(is_filtered(&mut state, 1, 0));
    let (_, heights) = metrics(&mut state, 6);
    let default_height = heights[0];

    // Row B moves to the bottom: its height and its hidden mark go with it.
    state.handle_command(TrameCommand::MoveRow {
        from: Pos::<Row>::new(1),
        to: Pos::<Row>::new(3),
    });
    assert_eq!(read_cell(&mut state, 3, 0), "b");
    let (_, heights) = metrics(&mut state, 6);
    assert_eq!((heights[1], heights[3]), (default_height, 40.0));
    assert!(is_filtered(&mut state, 3, 0) && !is_filtered(&mut state, 1, 0));

    // Column C is pushed right, moved, then pulled back by a delete.
    let (widths, _) = metrics(&mut state, 6);
    let default_width = widths[0];
    state.handle_command(TrameCommand::InsertColumn {
        at: Pos::<Col>::new(0),
    });
    let (widths, _) = metrics(&mut state, 6);
    assert_eq!((widths[2], widths[3]), (default_width, 150.0));
    state.handle_command(TrameCommand::MoveColumn {
        from: Pos::<Col>::new(3),
        to: Pos::<Col>::new(1),
    });
    let (widths, _) = metrics(&mut state, 6);
    assert_eq!((widths[1], widths[3]), (150.0, default_width));
    state.handle_command(TrameCommand::DeleteColumn {
        at: Pos::<Col>::new(0),
    });
    let (widths, _) = metrics(&mut state, 6);
    assert_eq!((widths[0], widths[1]), (150.0, default_width));
}

/// A width never drops below the minimum, wherever the pointer went
/// (`TRAME-UI-126`).
#[test]
fn a_column_is_never_narrower_than_its_minimum() {
    let mut state = TrameState::new();
    let sheet_id = state.active_sheet.clone();
    for (col, width) in [(0, 3.0), (1, 0.0), (2, -40.0)] {
        state.handle_command(TrameCommand::ResizeCol {
            version: ambre_core::Seq::nth(1),
            sheet_id: sheet_id.clone(),
            col: Pos::<Col>::new(col),
            width: ambre_core::Px::<ambre_core::Doc>::new(width),
        });
    }
    let (widths, _) = metrics(&mut state, 3);
    assert_eq!(widths, [MIN_COL_WIDTH.get(); 3]);
}

/// A peer writes sizes no resize gives. Each reads clamped into its range,
/// or as the default when it is no finite number (`TRAME-UI-181`).
#[test]
fn a_peer_size_out_of_range_reads_clamped_or_default() {
    let mut peer = TrameState::new();
    let sheet_id = peer.active_sheet.clone();
    for line in 0..6 {
        peer.handle_command(TrameCommand::ResizeCol {
            version: ambre_core::Seq::nth(1),
            sheet_id: sheet_id.clone(),
            col: Pos::<Col>::new(line),
            width: ambre_core::Px::<ambre_core::Doc>::new(100.0),
        });
        peer.handle_command(TrameCommand::ResizeRow {
            version: ambre_core::Seq::nth(1),
            sheet_id: sheet_id.clone(),
            row: Pos::<Row>::new(line),
            height: ambre_core::Px::<ambre_core::Doc>::new(30.0),
        });
    }
    let mut local = TrameState::new();
    local
        .import_snapshot(&peer.export_snapshot())
        .expect("a snapshot this build wrote");
    peer.flush_delta();
    // Past the `f32` range, 1e300 stays a finite `f64` and reads as a bound.
    let hostile = [-1e30, f64::NAN, f64::INFINITY, 1e30, -1e300, 1e300];
    for root in [SheetRoot::ColWidths, SheetRoot::RowHeights] {
        let map = peer
            .doc
            .get_map(SheetContainer::of(root, &sheet_id).as_str());
        let LoroValue::Map(sizes) = map.get_value() else {
            panic!("the sizes map");
        };
        // Each size goes to the line at its position, the order the reading
        // below takes. A derived uid keeps no order of its own.
        let keys: Vec<String> = (0..6)
            .filter_map(|line| match root {
                SheetRoot::ColWidths => peer.index.col_id(Pos::<Col>::new(line)),
                _ => peer.index.row_id(Pos::<Row>::new(line)),
            })
            .map(|uid| uid.to_key_part())
            .filter(|key| sizes.contains_key(key))
            .collect();
        for (key, size) in keys.iter().zip(hostile) {
            map.insert(key, size).expect("a crafted size");
        }
    }
    peer.doc.commit();
    let delta = peer.flush_delta().expect("the crafted sizes");
    local.apply_remote(&delta);
    let (widths, heights) = metrics(&mut local, 6);
    assert_eq!(
        widths,
        [
            MIN_COL_WIDTH,
            COL_WIDTH,
            COL_WIDTH,
            MAX_COL_WIDTH,
            MIN_COL_WIDTH,
            MAX_COL_WIDTH
        ]
        .map(|w| w.get())
    );
    assert_eq!(
        heights,
        [
            MIN_ROW_HEIGHT,
            ROW_HEIGHT,
            ROW_HEIGHT,
            MAX_ROW_HEIGHT,
            MIN_ROW_HEIGHT,
            MAX_ROW_HEIGHT
        ]
        .map(|h| h.get())
    );
}

/// A write ten thousand lines past the used range is refused with no
/// delta, and the line before that limit is written (`TRAME-089`).
#[test]
fn a_write_past_the_line_cap_is_refused() {
    let mut state = TrameState::new();
    let cap = MATERIALIZE_ROWS.get();
    // The document's own setup leaves first, so only the write is measured.
    state.flush_delta();
    let before = own_ops(&state);
    let out = state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(cap),
        col: Pos::<Col>::new(0),
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
fn three_sheets(state: &mut TrameState) -> Vec<Id<Sheet>> {
    for name in ["Two", "Three"] {
        state.handle_command(TrameCommand::CreateSheet {
            name: crate::test_support::sheet(name),
        });
    }
    let ids = state.sheet_order_ids();
    for id in &ids {
        state.handle_command(TrameCommand::ActivateSheet {
            sheet_id: id.clone(),
        });
        set(state, 0, 0, "4");
        set(state, 0, 1, "=A1*2");
        select(state, 0, 0, 0, 0);
        state.handle_command(TrameCommand::SetCellStyle {
            patch: bold_patch(),
        });
    }
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: ids[0].clone(),
    });
    ids
}

/// The cells and the styles of every sheet in `ids`.
fn contents(state: &TrameState, ids: &[Id<Sheet>]) -> Vec<(LoroValue, LoroValue)> {
    ids.iter()
        .map(|id| {
            (
                container(state, SheetContainer::of(SheetRoot::Cells, id)),
                container(state, SheetContainer::of(SheetRoot::CellStyle, id)),
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
            sheet_id: id.clone(),
            name: crate::test_support::sheet(name),
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
        sheet_id: ids[2].clone(),
        to_index: Pos::<crate::TabStrip>::ZERO,
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
        sheet_id: ids[1].clone(),
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
            sheet_id: id.clone(),
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
            name: crate::test_support::sheet("Budget"),
        })
        .update
        .expect("the create");
    let from_b = b
        .handle_command(TrameCommand::CreateSheet {
            name: crate::test_support::sheet("Budget"),
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
fn strip(state: &TrameState) -> (Vec<(Id<Sheet>, String)>, Id<Sheet>) {
    match state.sheet_list_event() {
        TrameEvent::SheetList { sheets, active, .. } => (
            sheets
                .into_iter()
                .map(|t| (t.id, t.name.to_string()))
                .collect(),
            active,
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
        name: crate::test_support::sheet("Two"),
    });
    let ids = base.sheet_order_ids();
    assert_eq!(ids.len(), 2);
    let (mut a, mut b) = two_peers(&base);
    a.flush_delta();
    b.flush_delta();
    let from_a = a
        .handle_command(TrameCommand::DeleteSheet {
            sheet_id: ids[1].clone(),
        })
        .update
        .expect("the delete");
    let from_b = b
        .handle_command(TrameCommand::DeleteSheet {
            sheet_id: ids[0].clone(),
        })
        .update
        .expect("the delete");
    let (ops_a, ops_b) = (own_ops(&a), own_ops(&b));
    a.apply_remote(&from_b);
    b.apply_remote(&from_a);
    let one = (
        vec![(default_sheet(), "Sheet1".to_owned())],
        default_sheet(),
    );
    assert_eq!(strip(&a), one);
    assert_eq!(strip(&b), one);
    // The merge stored nothing on either peer.
    assert_eq!((own_ops(&a), own_ops(&b)), (ops_a, ops_b));
    assert!(a.flush_delta().is_none() && b.flush_delta().is_none());

    // The last sheet stays: its delete is refused and writes nothing.
    let out = a.handle_command(TrameCommand::DeleteSheet {
        sheet_id: default_sheet(),
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
            row: Pos::<Row>::new(0),
            col: Pos::<Col>::new(0),
            text: "kept".to_owned(),
        })
        .update
        .expect("the write");
    let from_b = b
        .handle_command(TrameCommand::CreateSheet {
            name: crate::test_support::sheet("New"),
        })
        .update
        .expect("the create");
    a.apply_remote(&from_b);
    b.apply_remote(&from_a);
    assert_eq!(a.sheet_order_ids(), b.sheet_order_ids());
    assert_eq!(a.sheet_order_ids().len(), 2);
    assert_eq!(a.sheet_order_ids()[0], default_sheet());
    assert_eq!(read_cell(&mut b, 0, 0), "kept");
}

/// A peer's merge entry past the area bound is no merge. It lists none of
/// its cells into the derived caches, and a merge within the bound beside it
/// still places (`TRAME-315`, audit `P35`).
#[test]
fn a_merge_entry_past_the_area_bound_lists_no_cell() {
    let mut base = TrameState::new();
    set(&mut base, 0, 0, "a");
    let (mut a, mut b) = two_peers(&base);
    // 600 x 600 = 360 000 cells, past the 262 144 of the bound.
    let far = CellPos::new(Pos::<Row>::new(599), Pos::<Col>::new(701));
    b.materialize(far).expect("within the materialization cap");
    let key = |state: &TrameState, row: u32, col: u32| {
        state
            .index
            .key_at(CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)))
            .expect("materialized")
    };
    let merges = b
        .doc
        .get_map(SheetContainer::of(SheetRoot::Merges, &default_sheet()).as_str());
    merges
        .insert(key(&b, 0, 0).as_str(), key(&b, 599, 599).as_str())
        .expect("the huge entry");
    merges
        .insert(key(&b, 0, 700).as_str(), key(&b, 1, 701).as_str())
        .expect("a merge within the bound");
    b.doc.commit();
    let update = b.flush_delta().expect("the entries are a delta");
    a.apply_remote(&update);
    for state in [&mut a, &mut b] {
        state.rebuild_merges();
        assert_eq!(state.covered.len(), 3, "the huge entry listed its cells");
        assert!(
            state
                .merges
                .contains_key(&CellPos::new(Pos::<Row>::ZERO, Pos::<Col>::new(700))),
            "the merge within the bound placed"
        );
        assert!(
            !state
                .merges
                .contains_key(&CellPos::new(Pos::<Row>::ZERO, Pos::<Col>::ZERO)),
            "the huge entry placed a merge"
        );
    }
}

/// The local merge command writes no entry the bound would skip: it sends
/// nothing, and the sheet holds no merge (`TRAME-315`).
#[test]
fn a_local_merge_past_the_area_bound_writes_no_entry() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "a");
    let before = own_ops(&state);
    let out = merge(&mut state, 0, 0, 599, 599);
    assert!(out.update.is_none(), "the refused merge sent a delta");
    assert_eq!(own_ops(&state), before);
    assert!(
        matches!(container(&state, SheetContainer::of(SheetRoot::Merges, &default_sheet())), LoroValue::Map(m) if m.is_empty())
    );
}

/// Merge entries past the entry bound are skipped in key order, the same on
/// every peer: the sheet places at most that many merges (`TRAME-315`).
#[test]
fn merge_entries_past_the_entry_bound_are_skipped() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "a");
    // Two merges a row, side by side, over 4 200 rows: 8 400 entries.
    state
        .materialize(CellPos::new(Pos::<Row>::new(4_199), Pos::<Col>::new(3)))
        .expect("within the materialization cap");
    let merges = state
        .doc
        .get_map(SheetContainer::of(SheetRoot::Merges, &default_sheet()).as_str());
    for row in 0..4_200 {
        for col in [0, 2] {
            let at = |c: u32| {
                state
                    .index
                    .key_at(CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(c)))
                    .expect("materialized")
            };
            merges
                .insert(at(col).as_str(), at(col + 1).as_str())
                .expect("the entry");
        }
    }
    state.doc.commit();
    state.rebuild_merges();
    let bound = usize::try_from(crate::logic::merges::MERGE_MAX_ENTRIES.get()).expect("fits");
    assert_eq!(state.merges.len(), bound);
}
