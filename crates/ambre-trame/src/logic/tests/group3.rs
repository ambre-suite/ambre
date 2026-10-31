// ambre-trame/src/logic/tests/group3.rs
// TrameState unit tests (group 3/7). Helpers via `use super::*`;
// logic-module items are reached with `super::super::` paths.

use super::*;
use ambre_bus::{SheetId, SheetTab};

/// `UnmergeActive` over a selection spanning several merges splits every one of
/// them, not just the merge under the active cell. Regression: only the last
/// selected merge was freed.
#[test]
fn unmerge_active_splits_every_merge_in_selection() {
    let mut state = TrameState::new();
    // Build two separate merges: A1:B1 and A3:B3.
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(0), Col::new(1)),
        ),
    });
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(2), Col::new(0)),
            crate::CellPos::new(Row::new(2), Col::new(1)),
        ),
    });
    assert_eq!(read_display(&mut state, 0, 0).merge_span, Some(span(1, 2)));
    assert_eq!(read_display(&mut state, 2, 0).merge_span, Some(span(1, 2)));

    // Select A1:B3, covering both merges, then unmerge.
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(0),
        col: Col::new(0),
        anchor_row: Row::new(2),
        anchor_col: Col::new(1),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    let out = state.handle_command(TrameCommand::UnmergeActive);

    assert_eq!(
        read_display(&mut state, 0, 0).merge_span,
        None,
        "first merge split"
    );
    assert_eq!(
        read_display(&mut state, 2, 0).merge_span,
        None,
        "second merge split"
    );

    // The post-unmerge selection covers every freed cell.
    let range = out.display.iter().find_map(|e| match e {
        TrameEvent::SelectRange { rect } => Some(rect_tuple(*rect)),
        _ => None,
    });
    assert_eq!(
        range,
        Some((0, 0, 2, 1)),
        "selection covers all freed cells"
    );
}

/// `UnmergeActive` over a selection that only clips the edge of a merge still
/// grows the post-unmerge selection to the freed block. The merge extends past
/// the drag rectangle, so the exposed cells must end up selected. In the
/// reported bug the selection was smaller than the unmerged cells.
#[test]
fn unmerge_selection_grows_to_cover_straddling_merge() {
    let mut state = TrameState::new();
    // A merge B2:C3 (rows 1..2, cols 1..2).
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(1), Col::new(1)),
            crate::CellPos::new(Row::new(2), Col::new(2)),
        ),
    });
    // Select only A1:B2 -- it clips the merge's top-left corner.
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(0),
        col: Col::new(0),
        anchor_row: Row::new(1),
        anchor_col: Col::new(1),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    let out = state.handle_command(TrameCommand::UnmergeActive);

    assert_eq!(
        read_display(&mut state, 1, 1).merge_span,
        None,
        "merge split"
    );
    let range = out.display.iter().find_map(|e| match e {
        TrameEvent::SelectRange { rect } => Some(rect_tuple(*rect)),
        _ => None,
    });
    // A1 corner (0,0) unioned with the full B2:C3 block -> A1:C3.
    assert_eq!(range, Some((0, 0, 2, 2)), "selection grew over the merge");
}

/// Two vertical merges A1:A2 and B1:B2, selected by their masters only (row 0),
/// then unmerged. The post-unmerge selection must grow down to row 1 to cover
/// every freed cell. In the exact case reported, only A1/B1 stayed selected.
#[test]
fn unmerge_two_vertical_merges_selected_by_masters() {
    let mut state = TrameState::new();
    // A1:A2 (rows 0..1, col 0).
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(1), Col::new(0)),
        ),
    });
    // B1:B2 (rows 0..1, col 1).
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(1)),
            crate::CellPos::new(Row::new(1), Col::new(1)),
        ),
    });
    // Selection spans the two masters only: A1:B1 (row 0).
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(0),
        col: Col::new(1),
        anchor_row: Row::new(0),
        anchor_col: Col::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    let out = state.handle_command(TrameCommand::UnmergeActive);

    let range = out.display.iter().find_map(|e| match e {
        TrameEvent::SelectRange { rect } => Some(rect_tuple(*rect)),
        _ => None,
    });
    assert_eq!(range, Some((0, 0, 1, 1)), "selection grew to cover A1:B2");
}

/// `MergeSelection` over a lone single-cell selection is a silent no-op (a
/// single cell is not a merge), producing no CRDT delta.
#[test]
fn merge_selection_single_cell_is_noop() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(2),
        col: Col::new(2),
        anchor_row: Row::new(2),
        anchor_col: Col::new(2),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    let evts = state.handle_command(TrameCommand::MergeSelection);
    assert!(evts.update.is_none(), "no CRDT mutation");
    assert_eq!(read_display(&mut state, 2, 2).merge_span, None);
}

/// A concurrent edit that lands a value in a covered cell is masked, not
/// destroyed. The value is hidden while the merge is active and reappears on
/// unmerge (`trame-cell-merges`, data is never destroyed).
#[test]
fn concurrent_value_under_merge_is_masked_then_restored() {
    // Shared base so both peers address rows 0/1 by the *same* stable ids
    // (concurrent dense-list inserts would otherwise create distinct rows).
    // Seeding then clearing A2 materializes rows 0 and 1 without leaving a
    // value behind.
    let mut base = TrameState::new();
    base.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "seed".to_owned(),
    });
    base.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: String::new(),
    });
    let base_snap = base.export_snapshot();

    let mut a = TrameState::new();
    a.import_snapshot(&base_snap)
        .expect("a snapshot this build wrote reads back at the current schema");
    let mut b = TrameState::new();
    b.import_snapshot(&base_snap)
        .expect("a snapshot this build wrote reads back at the current schema");

    // Peer A merges A1:A2 (both empty on its side).
    a.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(1), Col::new(0)),
        ),
    });
    let merge_delta = a.export_snapshot();

    // Peer B concurrently writes A2, then receives A's merge.
    b.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "concurrent".to_owned(),
    });
    let conflicts = b
        .import_snapshot(&merge_delta)
        .expect("the snapshot imports");
    assert!(
        conflicts
            .iter()
            .any(|e| matches!(e, TrameEvent::MergeConflictDetected { .. })),
        "the covered non-empty cell is reported as a conflict"
    );
    // While merged, A2 is masked (blank) but the data still exists.
    assert_eq!(read_display(&mut b, 1, 0).text, "");

    // Unmerge restores the masked value.
    b.handle_command(TrameCommand::UnmergeCells {
        row: Row::new(0),
        col: Col::new(0),
    });
    assert_eq!(
        read_display(&mut b, 1, 0).text,
        "concurrent",
        "masked value reappears"
    );
}

/// Two concurrent merges that overlap resolve to one block. The survivor is
/// the merge whose master key sorts first, not the last writer and not the
/// local one. The winner is read from the keys, so the assertion holds the
/// rule and not the positions this run happened to mint (`TRAME-144`).
#[test]
fn overlapping_merges_resolve_to_the_lowest_master_key() {
    // Shared base so both peers address rows and columns 0..2 by the *same*
    // stable ids. Seeding then clearing C3 materializes them without leaving a
    // value behind.
    let mut base = TrameState::new();
    base.handle_command(TrameCommand::SetCellText {
        row: Row::new(2),
        col: Col::new(2),
        text: "seed".to_owned(),
    });
    base.handle_command(TrameCommand::SetCellText {
        row: Row::new(2),
        col: Col::new(2),
        text: String::new(),
    });
    let base_snap = base.export_snapshot();

    let mut a = TrameState::new();
    a.import_snapshot(&base_snap)
        .expect("a snapshot this build wrote reads back at the current schema");
    let mut b = TrameState::new();
    b.import_snapshot(&base_snap)
        .expect("a snapshot this build wrote reads back at the current schema");

    // A merges A1:B2, B merges B2:C3 concurrently: they overlap at B2.
    a.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(1), Col::new(1)),
        ),
    });
    b.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(1), Col::new(1)),
            crate::CellPos::new(Row::new(2), Col::new(2)),
        ),
    });
    let (delta_a, delta_b) = (a.export_snapshot(), b.export_snapshot());
    a.import_snapshot(&delta_b).expect("the snapshot imports");
    b.import_snapshot(&delta_a).expect("the snapshot imports");

    let master_key = |state: &TrameState, row: u32, col: u32| {
        let (r, c) = state
            .index
            .resolve(at(row, col))
            .expect("materialized by the base");
        crate::model::CellKey::of(r, c)
    };
    let expected = if master_key(&a, 0, 0) < master_key(&a, 1, 1) {
        (at(0, 0), at(1, 1))
    } else {
        (at(1, 1), at(2, 2))
    };

    assert_eq!(a.merges.len(), 1, "the overlap leaves one block, not two");
    assert_eq!(a.merges, b.merges, "both peers reach the same layout");
    assert_eq!(
        a.merges.get(&expected.0).copied(),
        Some(expected.1),
        "the surviving block is the one whose master key sorts first"
    );
    // The formula feed reads the same resolution as the grid: one precedence,
    // one answer, on the active sheet as on any other.
    let sheet = a.active_sheet.clone();
    assert_eq!(
        a.masked_cells_for(&sheet, &a.index),
        a.masked_cells,
        "the engine masks exactly what the display masks"
    );
}

/// A merge follows its cells across a structural insert: inserting a row
/// above the merge shifts the master span down with it.
#[test]
fn merge_follows_structural_insert() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "x".to_owned(),
    });
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(1), Col::new(0)),
            crate::CellPos::new(Row::new(1), Col::new(1)),
        ),
    });
    assert_eq!(read_display(&mut state, 1, 0).merge_span, Some(span(1, 2)));

    // Insert a row at the top: the merge shifts from row 1 to row 2.
    state.handle_command(TrameCommand::InsertRow { at: Row::new(0) });
    assert_eq!(
        read_display(&mut state, 1, 0).merge_span,
        None,
        "old position freed"
    );
    assert_eq!(
        read_display(&mut state, 2, 0).merge_span,
        Some(span(1, 2)),
        "merge shifted down"
    );
}

/// An edit emits a CRDT delta through the `CommandSink`. Absorbing the
/// outcome captures exactly one `SyncCommand::SendUpdate`, the seam the
/// production controller drives the Sync thread through.
#[test]
fn edit_emits_send_update_through_sink() {
    use crate::test_support::MockSyncHandle;
    use ambre_bus::{DocumentId, SyncCommand};

    let mut state = TrameState::new();
    let sync = MockSyncHandle::new();
    let doc = DocumentId::new();

    let outcome = state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "hi".to_owned(),
    });
    sync.absorb(doc, &outcome);

    let sent = sync.drain();
    assert_eq!(sent.len(), 1, "one SendUpdate per mutating command");
    assert!(matches!(
        &sent[0],
        SyncCommand::SendUpdate { document_id, payload, .. }
            if *document_id == doc && !payload.is_empty()
    ));
}

/// Full two-peer collaboration loop driven through `MockSyncHandle`. Peer A
/// edits, the mock captures and relays the delta to peer B, and B converges
/// on A's value. A controller test with no network and no hand-shuffled
/// snapshots.
#[test]
fn two_peers_converge_via_mock_sync() {
    use crate::test_support::MockSyncHandle;
    use ambre_bus::DocumentId;

    let doc = DocumentId::new();
    let mut a = TrameState::new();
    let mut b = TrameState::new();
    let sync = MockSyncHandle::new();

    // A writes A1; the mock absorbs the emitted delta.
    let outcome = a.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "shared".to_owned(),
    });
    sync.absorb(doc, &outcome);

    // B has not seen the edit yet.
    assert_eq!(read_cell(&mut b, 0, 0), "");

    // Relay A's pending updates to B; B converges.
    let events = sync.deliver_updates_to(&mut b);
    assert!(events.iter().any(|e| matches!(e, TrameEvent::DataReset)));
    assert!(sync.is_empty(), "buffer drained after delivery");
    assert_eq!(
        read_cell(&mut b, 0, 0),
        "shared",
        "B converges on A's value"
    );
}

/// Holds `SYNC-101` for Trame: a remote delta answers display events alone,
/// and the next local delta carries none of the remote peer's operations.
#[test]
fn a_remote_change_is_never_re_broadcast() {
    let mut a = TrameState::new();
    let mut b = TrameState::new();
    let set = |text: &str| TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: text.to_owned(),
    };
    // B's own genesis goes out with its first edit, before any import.
    b.handle_command(set("b"));
    let from_a = a.handle_command(set("a")).update.expect("a delta");

    let events = b.apply_remote(&from_a);
    assert!(events.iter().any(|e| matches!(e, TrameEvent::DataReset)));
    assert!(b.flush_delta().is_none(), "the import owes nothing");

    let from_b = b
        .handle_command(TrameCommand::SetCellText {
            row: Row::new(1),
            col: Col::new(0),
            text: "next".to_owned(),
        })
        .update
        .expect("a delta");
    let meta = loro::LoroDoc::decode_import_blob_meta(from_b.as_bytes(), false).expect("a delta");
    let a_peer = a.doc.peer_id();
    assert_eq!(
        meta.partial_end_vv.get(&a_peer),
        meta.partial_start_vv.get(&a_peer),
        "the delta re-exports none of A's operations"
    );
}

/// Two peers sharing a cross-sheet formula converge when one renames the
/// target sheet. The reference is bound to the stable `sheet_id`, the same
/// id on both peers via sync. Both keep computing and reconstruct identical
/// formula-bar text (`trame-sheets`).
#[test]
fn cross_sheet_reference_converges_across_peers_on_rename() {
    let mut a = TrameState::new();

    // A builds the workbook: Sheet2!A1 = 5, Sheet1!B1 = Sheet2!A1 * 2.
    a.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet2"),
    });
    let s2 = second_sheet_id(&a);
    a.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    a.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "5".to_owned(),
    });
    a.handle_command(TrameCommand::ActivateSheet {
        sheet_id: SheetId("s0".to_owned()),
    });
    a.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=Sheet2!A1*2".to_owned(),
    });

    // B joins by loading A's snapshot (the real "open shared doc" path, so
    // both peers share the same op history) and converges on the value.
    let mut b = TrameState::new();
    b.import_snapshot(&a.export_snapshot())
        .expect("a snapshot this build wrote reads back at the current schema");
    assert_eq!(
        read_cell(&mut b, 0, 1),
        "10",
        "B resolves the shared sheet_id"
    );
    assert_eq!(read_raw(&mut b, 0, 1), "=Sheet2!A1*2");

    // B renames the target sheet; A merges B's state. A snapshot carries
    // B's full history, including the implicit-s0 op each peer mints in
    // `new()`. An incremental delta would leave that op as a dangling
    // dep.
    b.handle_command(TrameCommand::RenameSheet {
        sheet_id: s2.clone(),
        name: crate::test_support::sheet("Data"),
    });
    a.import_snapshot(&b.export_snapshot())
        .expect("the snapshot imports");

    // Both peers keep computing and reconstruct the same new label.
    assert_eq!(read_cell(&mut a, 0, 1), "10");
    assert_eq!(read_cell(&mut b, 0, 1), "10");
    assert_eq!(read_raw(&mut a, 0, 1), "=Data!A1*2");
    assert_eq!(read_raw(&mut b, 0, 1), "=Data!A1*2");
}

/// The first sheet a session adds keeps its `_00000000` id. Once the session
/// has minted its last sheet counter, it refuses the next sheet. The sheet
/// that holds the last id keeps its name, which a repeated id would rename
/// (`TRAME-243`).
#[test]
fn a_spent_sheet_counter_refuses_the_next_sheet_and_overwrites_nothing() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("First"),
    });
    let first = state.all_sheet_ids();
    assert!(first.iter().any(|id| id.0.ends_with("_00000000")));
    state.sheet_counter = Some(ambre_core::IdCounter::new(u32::MAX));
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Last"),
    });
    assert_eq!(state.sheet_counter, None);
    let tabs = |state: &mut TrameState| {
        let out = state.handle_command(TrameCommand::CreateSheet {
            name: crate::test_support::sheet("Late"),
        });
        let refused = out.display.iter().any(
            |e| matches!(e, TrameEvent::Error { message } if message.contains("no fresh sheet id")),
        );
        (refused, state.all_sheet_ids().len())
    };
    assert_eq!(
        tabs(&mut state),
        (true, 3),
        "the sheet past the last id is refused"
    );
    let vv = state.doc.oplog_vv();
    state.doc.commit();
    assert_eq!(state.doc.oplog_vv(), vv, "the refusal wrote nothing");
    let last = state.sheet_list_event();
    let TrameEvent::SheetList { sheets, .. } = &last else {
        panic!("expected SheetList")
    };
    assert!(
        sheets
            .iter()
            .any(|t| t.id.0.ends_with("_ffffffff") && t.name.as_str() == "Last"),
        "the last id keeps its sheet"
    );
}

/// Creating a sheet mints a stable id, lists it after the default in the tab
/// strip, and keeps its cells isolated from the default sheet.
#[test]
fn create_sheet_lists_and_isolates() {
    let mut state = TrameState::new();
    let evts = state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Data"),
    });
    let TrameEvent::SheetList { sheets, active } = &evts.display[0] else {
        panic!("expected SheetList")
    };
    assert_eq!(active.0, "s0", "active sheet unchanged by create");
    assert_eq!(sheets.len(), 2);
    assert_eq!(
        sheets[0],
        SheetTab {
            id: SheetId("s0".to_owned()),
            name: crate::test_support::sheet("Sheet1")
        },
        "default tab first"
    );
    assert_eq!(sheets[1].name.as_str(), "Data");

    let s1 = sheets[1].id.0.clone();
    // Write on the default sheet, switch to the new one: it is blank.
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "on_s0".to_owned(),
    });
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: SheetId(s1.clone()),
    });
    assert_eq!(read_cell(&mut state, 0, 0), "", "new sheet starts empty");
    // Write here, switch back: each sheet keeps its own value.
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "on_s1".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 0), "on_s1");
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: SheetId("s0".to_owned()),
    });
    assert_eq!(read_cell(&mut state, 0, 0), "on_s0");
}

/// The first `ScrollViewport` of a session rides a `SheetList` back, so
/// the UI tab strip hydrates on load with no separate fetch command.
/// Subsequent scrolls carry only the viewport.
#[test]
fn first_scroll_viewport_hydrates_sheet_list_once() {
    let mut state = TrameState::new();
    let scroll = || TrameCommand::ScrollViewport {
        version: ambre_core::Generation::nth(1),
        row_start: Row::new(0),
        row_end: Row::new(4),
        col_start: Col::new(0),
        col_end: Col::new(4),
    };

    let first = state.handle_command(scroll());
    assert!(
        matches!(first.display.first(), Some(TrameEvent::ViewportData { .. })),
        "viewport data leads the response",
    );
    let sheet_list = first
        .display
        .iter()
        .find(|e| matches!(e, TrameEvent::SheetList { .. }));
    let Some(TrameEvent::SheetList { sheets, active }) = sheet_list else {
        panic!("first ScrollViewport must emit a SheetList");
    };
    assert_eq!(active.0, "s0");
    assert_eq!(
        sheets,
        &vec![SheetTab {
            id: SheetId("s0".to_owned()),
            name: crate::test_support::sheet("Sheet1")
        }]
    );

    // The second scroll carries the viewport only -- no repeated SheetList.
    let second = state.handle_command(scroll());
    assert!(
        !second
            .display
            .iter()
            .any(|e| matches!(e, TrameEvent::SheetList { .. })),
        "SheetList is emitted only once per session",
    );
}

/// A cross-sheet formula on the active sheet evaluates a cell living on
/// another sheet -- the end-to-end path unlocked by the lifecycle commands.
#[test]
fn cross_sheet_formula_evaluates_end_to_end() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet2"),
    });
    let s2 = second_sheet_id(&state);

    // Put 5 into Sheet2!A1.
    state.handle_command(TrameCommand::ActivateSheet { sheet_id: s2 });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "5".to_owned(),
    });

    // Back on Sheet1, B1 references it.
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: SheetId("s0".to_owned()),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=Sheet2!A1*2".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 1), "10", "B1 = Sheet2!A1 * 2");
}

/// A masked cell (covered by a merge, non-empty -> concurrent conflict) is
/// excluded from computation on *every* sheet, not only the displayed one
/// (`trame-cell-merges`, `TRAME-020`).
/// Regression for the view-dependent masking bug. The feed masked local index
/// 0 alone -- the active sheet -- so a cross-sheet reference to a masked cell
/// elsewhere leaked the hidden value. The formula result then depended on the
/// tab in front of the user.
#[test]
fn masked_cell_excluded_across_sheets_regardless_of_active_tab() {
    // Shared base: Sheet2 exists with rows 0/1 materialized under stable ids
    // (seed+clear A2), a same-sheet reader `Sheet2!C1 = =A2`, and a cross-sheet
    // reader `Sheet1!B1 = =Sheet2!A2`. Both peers inherit identical ids.
    let mut base = TrameState::new();
    base.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet2"),
    });
    let s2 = second_sheet_id(&base);
    base.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    base.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "seed".to_owned(),
    });
    base.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: String::new(),
    });
    // Same-sheet reader on Sheet2 (read while Sheet2 is the active tab).
    base.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(2),
        text: "=A2".to_owned(),
    });
    // Cross-sheet reader on Sheet1 (read while Sheet1 is the active tab, so the
    // masked cell lives on a *non-active* sheet).
    base.handle_command(TrameCommand::ActivateSheet {
        sheet_id: SheetId("s0".to_owned()),
    });
    base.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=Sheet2!A2".to_owned(),
    });
    let base_snap = base.export_snapshot();

    let mut a = TrameState::new();
    a.import_snapshot(&base_snap)
        .expect("a snapshot this build wrote reads back at the current schema");
    let mut b = TrameState::new();
    b.import_snapshot(&base_snap)
        .expect("a snapshot this build wrote reads back at the current schema");

    // Peer A merges Sheet2!A1:A2 (both empty on its side).
    a.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    a.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(1), Col::new(0)),
        ),
    });
    let merge_delta = a.export_snapshot();

    // Peer B concurrently writes Sheet2!A2, then receives A's merge -> A2 is
    // masked (covered + non-empty), and stays the active tab.
    b.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    b.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "77".to_owned(),
    });
    let conflicts = b
        .import_snapshot(&merge_delta)
        .expect("the snapshot imports");
    assert!(
        conflicts
            .iter()
            .any(|e| matches!(e, TrameEvent::MergeConflictDetected { .. })),
        "the covered non-empty cell is reported as a conflict"
    );

    // Sheet2 is active: the same-sheet reader sees the masked cell excluded.
    let target_active = read_cell(&mut b, 0, 2);

    // Switch to Sheet1: the cross-sheet reader now targets a *non-active* sheet.
    b.handle_command(TrameCommand::ActivateSheet {
        sheet_id: SheetId("s0".to_owned()),
    });
    let target_inactive = read_cell(&mut b, 0, 1);

    assert_ne!(
        target_inactive, "77",
        "the cross-sheet reader must not leak the masked value when its target sheet is inactive"
    );
    assert_eq!(
        target_inactive, target_active,
        "a masked cell reads identically whether or not its sheet is the active tab (TRAME-020)"
    );
}

/// Renaming changes only the display label; the stable id (and the cells
/// keyed under it) are untouched.
#[test]
fn rename_sheet_keeps_id_and_cells() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Data"),
    });
    let s1 = second_sheet_id(&state);
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s1.clone(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "v".to_owned(),
    });

    let evts = state.handle_command(TrameCommand::RenameSheet {
        sheet_id: s1.clone(),
        name: crate::test_support::sheet("Renamed"),
    });
    let TrameEvent::SheetList { sheets, .. } = &evts.display[0] else {
        panic!()
    };
    assert!(
        sheets
            .iter()
            .any(|t| t.id == s1 && t.name.as_str() == "Renamed")
    );
    // The value survives the rename (same id, same container).
    assert_eq!(read_cell(&mut state, 0, 0), "v");
}

/// Moving a sheet reorders the tab strip without touching its content.
#[test]
fn move_sheet_reorders_tabs() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("B"),
    });
    let b = second_sheet_id(&state);
    // Order is [s0, B]; move B to the front.
    let evts = state.handle_command(TrameCommand::MoveSheet {
        sheet_id: b.clone(),
        to_index: crate::SheetTabPos::ZERO,
    });
    let TrameEvent::SheetList { sheets, .. } = &evts.display[0] else {
        panic!()
    };
    assert_eq!(sheets[0].id, b, "B moved to the front");
    assert_eq!(sheets[1].id.0, "s0");
}

/// Deleting the last remaining sheet is rejected; the workbook keeps >= 1.
#[test]
fn delete_last_sheet_rejected() {
    let mut state = TrameState::new();
    let evts = state.handle_command(TrameCommand::DeleteSheet {
        sheet_id: SheetId("s0".to_owned()),
    });
    assert!(matches!(evts.display[0], TrameEvent::Error { .. }));
    assert!(evts.update.is_none());
}

/// Deleting the active sheet falls back to a neighbour and refreshes the
/// tab strip; the deleted sheet is gone from the list.
#[test]
fn delete_active_sheet_falls_back() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("B"),
    });
    let b = second_sheet_id(&state);
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: b.clone(),
    });

    let evts = state.handle_command(TrameCommand::DeleteSheet {
        sheet_id: b.clone(),
    });
    let list = evts
        .display
        .iter()
        .find_map(|e| match e {
            TrameEvent::SheetList { sheets, active } => Some((sheets.clone(), active.clone())),
            _ => None,
        })
        .expect("SheetList emitted");
    assert_eq!(list.1.0, "s0", "active fell back to the default sheet");
    assert!(!list.0.iter().any(|t| t.id == b), "deleted sheet gone");
}

/// Ascending sort reorders whole rows by the key column; cells in other
/// columns of the same row travel with it (stable-id keyed).
#[test]
fn sort_range_ascending_moves_whole_rows() {
    let mut state = TrameState::new();
    // A: 3,1,2  B: "c","a","b" (paired with A so we can check rows move whole)
    for (r, (a, b)) in [("3", "c"), ("1", "a"), ("2", "b")].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: Row::new(r as u32),
            col: Col::new(0),
            text: a.to_string(),
        });
        state.handle_command(TrameCommand::SetCellText {
            row: Row::new(r as u32),
            col: Col::new(1),
            text: b.to_string(),
        });
    }
    let evts = state.handle_command(TrameCommand::SortRange {
        sheet_id: SheetId("s0".to_owned()),
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(2), Col::new(1)),
        ),
        sort_col: ColCount::new(0),
        order: SortOrder::Ascending,
        header: HeaderRow::Absent,
    });
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

/// Descending sort and the header pin: the first row stays put.
#[test]
fn sort_range_descending_keeps_header() {
    let mut state = TrameState::new();
    for (r, a) in ["Header", "1", "3", "2"].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: Row::new(r as u32),
            col: Col::new(0),
            text: a.to_string(),
        });
    }
    state.handle_command(TrameCommand::SortRange {
        sheet_id: SheetId("s0".to_owned()),
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(3), Col::new(0)),
        ),
        sort_col: ColCount::new(0),
        order: SortOrder::Descending,
        header: HeaderRow::Present,
    });
    assert_eq!(read_col(&mut state, 0, 3, 1), vec!["Header", "3", "2", "1"]);
}

/// The tab label of `sheet_id`, as the strip shows it.
fn tab_name(state: &TrameState, sheet_id: &SheetId) -> String {
    match state.sheet_list_event() {
        TrameEvent::SheetList { sheets, .. } => sheets
            .into_iter()
            .find(|t| t.id == *sheet_id)
            .map(|t| t.name.to_string())
            .expect("the sheet is listed"),
        other => panic!("expected SheetList, got {other:?}"),
    }
}

/// Every label the strip shows, in order.
fn tab_names(state: &TrameState) -> Vec<String> {
    match state.sheet_list_event() {
        TrameEvent::SheetList { sheets, .. } => {
            sheets.into_iter().map(|t| t.name.to_string()).collect()
        }
        other => panic!("expected SheetList, got {other:?}"),
    }
}

/// The strip names a new sheet after its tab count. So add, add, delete the
/// first added, add asks for a taken name (`TRAME-078`). The new sheet takes
/// the first free variant instead.
#[test]
fn created_sheet_never_repeats_a_name() {
    let mut state = TrameState::new();
    for name in ["Sheet2", "Sheet3"] {
        state.handle_command(TrameCommand::CreateSheet {
            name: crate::test_support::sheet(name),
        });
    }
    let first_added = second_sheet_id(&state);
    state.handle_command(TrameCommand::DeleteSheet {
        sheet_id: first_added,
    });
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet3"),
    });
    assert_eq!(tab_names(&state), vec!["Sheet1", "Sheet3", "Sheet3 (2)"]);
}

/// A rename to a name another sheet carries is refused and said (`TRAME-078`).
#[test]
fn rename_to_a_taken_name_is_refused() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Data"),
    });
    let data = second_sheet_id(&state);
    let evts = state.handle_command(TrameCommand::RenameSheet {
        sheet_id: data.clone(),
        name: crate::test_support::sheet("Sheet1"),
    });
    assert!(matches!(&evts.display[..], [TrameEvent::Error { .. }]));
    assert!(evts.update.is_none());
    assert_eq!(tab_name(&state, &data), "Data");
}

/// Two peers give two sheets one name concurrently (`TRAME-078`). The merge
/// stores both, and every peer reads the second one with a suffix.
#[test]
fn concurrent_duplicate_names_read_apart() {
    let mut a = TrameState::new();
    a.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Two"),
    });
    let mut b = TrameState::new();
    b.import_snapshot(&a.export_snapshot())
        .expect("a snapshot this build wrote reads back at the current schema");
    let two = second_sheet_id(&a);
    a.handle_command(TrameCommand::RenameSheet {
        sheet_id: two.clone(),
        name: crate::test_support::sheet("Budget"),
    });
    b.handle_command(TrameCommand::RenameSheet {
        sheet_id: SheetId("s0".to_owned()),
        name: crate::test_support::sheet("Budget"),
    });
    let (sa, sb) = (a.export_snapshot(), b.export_snapshot());
    a.import_snapshot(&sb).expect("the snapshot imports");
    b.import_snapshot(&sa).expect("the snapshot imports");
    assert_eq!(tab_names(&a), tab_names(&b));
    let names = tab_names(&a);
    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1], "two tabs read apart: {names:?}");
    assert!(names.contains(&"Budget".to_owned()));
}

/// A create asking for a name another sheet carries in another case takes
/// the first free variant: case does not tell two tabs apart (`TRAME-078`).
#[test]
fn created_sheet_never_repeats_a_name_whatever_its_case() {
    let mut state = TrameState::new();
    for name in ["Data", "DATA"] {
        state.handle_command(TrameCommand::CreateSheet {
            name: crate::test_support::sheet(name),
        });
    }
    assert_eq!(tab_names(&state), vec!["Sheet1", "Data", "DATA (2)"]);
}

/// A rename to a name another sheet carries in another case is refused and
/// said (`TRAME-078`). A sheet may still change the case of its own name.
#[test]
fn rename_to_a_name_taken_in_another_case_is_refused() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Data"),
    });
    let data = second_sheet_id(&state);
    let evts = state.handle_command(TrameCommand::RenameSheet {
        sheet_id: data.clone(),
        name: crate::test_support::sheet("sheet1"),
    });
    assert!(matches!(&evts.display[..], [TrameEvent::Error { .. }]));
    assert!(evts.update.is_none());
    assert_eq!(tab_name(&state, &data), "Data");

    let evts = state.handle_command(TrameCommand::RenameSheet {
        sheet_id: data.clone(),
        name: crate::test_support::sheet("DATA"),
    });
    assert!(
        evts.update.is_some(),
        "a sheet changes the case of its own name"
    );
    assert_eq!(tab_name(&state, &data), "DATA");
}

/// Two peers give two sheets names that differ only in case, concurrently
/// (`TRAME-078`). Every peer reads the second one with a suffix.
#[test]
fn concurrent_names_differing_in_case_read_apart() {
    let mut a = TrameState::new();
    a.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Two"),
    });
    let mut b = TrameState::new();
    b.import_snapshot(&a.export_snapshot())
        .expect("a snapshot this build wrote reads back at the current schema");
    let two = second_sheet_id(&a);
    a.handle_command(TrameCommand::RenameSheet {
        sheet_id: two,
        name: crate::test_support::sheet("Budget"),
    });
    b.handle_command(TrameCommand::RenameSheet {
        sheet_id: SheetId("s0".to_owned()),
        name: crate::test_support::sheet("BUDGET"),
    });
    let (sa, sb) = (a.export_snapshot(), b.export_snapshot());
    a.import_snapshot(&sb).expect("the snapshot imports");
    b.import_snapshot(&sa).expect("the snapshot imports");
    assert_eq!(tab_names(&a), tab_names(&b));
    let names: Vec<String> = tab_names(&a).iter().map(|n| n.to_lowercase()).collect();
    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1], "two tabs read apart: {names:?}");
}

/// A stored name the rule refuses reads as the fallback `Sheet<n>`, `n` its
/// tab's position, and the document keeps the name it holds (`TRAME-235`).
/// The fallback is a name like any other. A formula resolves it, it never
/// takes the name another sheet stores, and a rename stores the fix.
#[test]
fn a_stored_name_the_rule_refuses_reads_as_its_fallback() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Data"),
    });
    let s1 = second_sheet_id(&state);
    let stored = "Q1:Q2 [draft] of the year two thousand";
    let sheets = state.doc.get_map(crate::logic::Root::Sheets.as_str());
    let _ = sheets.insert(s1.as_str(), stored);
    state.doc.commit();
    state.reseed_engine();
    assert_eq!(tab_names(&state), vec!["Sheet1", "Sheet2"]);
    let held = |state: &TrameState| match state
        .doc
        .get_map(crate::logic::Root::Sheets.as_str())
        .get(s1.as_str())
    {
        Some(loro::ValueOrContainer::Value(loro::LoroValue::String(s))) => s.to_string(),
        other => panic!("expected the stored name, got {other:?}"),
    };
    assert_eq!(
        held(&state),
        stored,
        "the read leaves the document as it was"
    );

    let evts = state.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "=Sheet2!A1*2".to_owned(),
    });
    assert!(
        !evts
            .display
            .iter()
            .any(|e| matches!(e, TrameEvent::Error { .. })),
        "the formula names the fallback"
    );
    assert_eq!(read_raw(&mut state, 1, 0), "=Sheet2!A1*2");

    let _ = sheets.insert("s0", "Sheet2");
    state.doc.commit();
    state.reseed_engine();
    assert_eq!(
        tab_names(&state),
        vec!["Sheet2", "Sheet2 (2)"],
        "a stored name claims before a fallback"
    );

    state.handle_command(TrameCommand::RenameSheet {
        sheet_id: s1.clone(),
        name: crate::test_support::sheet("Q1 draft"),
    });
    assert_eq!(held(&state), "Q1 draft", "a rename stores the fix");
    assert_eq!(tab_names(&state), vec!["Sheet2", "Q1 draft"]);
}

/// A formula qualifier resolves whatever its case, as two names differing
/// only in case are one name (`TRAME-236`). The formula bar then shows the
/// name as the tab spells it.
#[test]
fn a_qualifier_resolves_whatever_its_case() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Data"),
    });
    let evts = state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "=data!A1*2".to_owned(),
    });
    assert!(
        !evts
            .display
            .iter()
            .any(|e| matches!(e, TrameEvent::Error { .. }))
    );
    assert_eq!(read_raw(&mut state, 0, 0), "=Data!A1*2");
}

/// A deleted sheet leaves none of its data in the document (`TRAME-076`):
/// every per-sheet container is empty, the metrics and the filter included.
#[test]
fn deleted_sheet_leaves_no_data() {
    use crate::logic::schema::{SheetContainer, SheetRoot};
    use loro::LoroValue;
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Gone"),
    });
    let gone = second_sheet_id(&state);
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: gone.clone(),
    });
    for (row, text) in ["a", "b"].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: Row::new(row as u32),
            col: Col::new(0),
            text: text.to_string(),
        });
    }
    state.handle_command(TrameCommand::ResizeCol {
        version: ambre_core::Generation::nth(1),
        sheet_id: gone.clone(),
        col: Col::new(0),
        width: ambre_core::DocPx::new(120.0),
    });
    state.handle_command(TrameCommand::ResizeRow {
        version: ambre_core::Generation::nth(2),
        sheet_id: gone.clone(),
        row: Row::new(0),
        height: ambre_core::DocPx::new(40.0),
    });
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: gone.clone(),
        col: Col::new(0),
        kind: ambre_bus::FilterKind::Values(vec!["a".to_owned()]),
    });
    let filled = |state: &TrameState, name: &SheetContainer| !matches!(state.doc.get_map(name.as_str()).get_value(), LoroValue::Map(m) if m.is_empty());
    for name in [
        SheetContainer::of(SheetRoot::ColWidths, &gone),
        SheetContainer::of(SheetRoot::RowHeights, &gone),
        SheetContainer::of(SheetRoot::Filters, &gone),
        SheetContainer::of(SheetRoot::FilterHidden, &gone),
    ] {
        assert!(
            filled(&state, &name),
            "{} was written before the delete",
            name.as_str()
        );
    }

    state.handle_command(TrameCommand::DeleteSheet {
        sheet_id: gone.clone(),
    });
    for name in [
        SheetContainer::of(SheetRoot::Cells, &gone),
        SheetContainer::of(SheetRoot::CellStyle, &gone),
        SheetContainer::of(SheetRoot::Merges, &gone),
        SheetContainer::of(SheetRoot::ColWidths, &gone),
        SheetContainer::of(SheetRoot::RowHeights, &gone),
        SheetContainer::of(SheetRoot::Filters, &gone),
        SheetContainer::of(SheetRoot::FilterHidden, &gone),
    ] {
        assert!(
            !filled(&state, &name),
            "{} outlived its sheet",
            name.as_str()
        );
    }
    for name in [
        SheetContainer::of(SheetRoot::Rows, &gone),
        SheetContainer::of(SheetRoot::Cols, &gone),
    ] {
        assert!(
            state.doc.get_movable_list(name.as_str()).is_empty(),
            "{} outlived its sheet",
            name.as_str()
        );
    }
}

/// A formula qualifying a sheet no tab carries is refused when it is typed
/// (`TRAME-211`): nothing is stored, and the user is told.
#[test]
fn formula_naming_a_missing_sheet_is_refused() {
    let mut state = TrameState::new();
    let evts = state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "=Nowhere!A1*2".to_owned(),
    });
    assert!(
        evts.display
            .iter()
            .any(|e| matches!(e, TrameEvent::Error { message } if message.contains("Nowhere")))
    );
    assert_eq!(read_raw(&mut state, 0, 0), "", "nothing was stored");
    // The same formula lands once the sheet exists, stored by its id.
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Nowhere"),
    });
    let evts = state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "=Nowhere!A1*2".to_owned(),
    });
    assert!(
        !evts
            .display
            .iter()
            .any(|e| matches!(e, TrameEvent::Error { .. }))
    );
    assert_eq!(read_raw(&mut state, 0, 0), "=Nowhere!A1*2");
}

/// A write past the materialization cap is refused and said
/// (`TRAME-UI-164`), never dropped in silence.
#[test]
fn write_beyond_the_cap_is_refused_and_said() {
    let mut state = TrameState::new();
    let row = crate::logic::schema::MATERIALIZE_ROWS.get() + 5;
    let evts = state.handle_command(TrameCommand::SetCellText {
        row: Row::new(row),
        col: Col::new(0),
        text: "far".to_owned(),
    });
    assert!(
        evts.display
            .iter()
            .any(|e| matches!(e, TrameEvent::Error { .. }))
    );
    assert_eq!(read_cell(&mut state, row, 0), "");
}

/// A sheet created while a cross-sheet formula is live leaves the formula
/// reading the sheet it names. The new id sorts before `s0`, so the engine
/// index the formula compiled against for Sheet1 now names the new sheet.
/// Regression: `create_sheet` rebuilt the sheet maps without a reseed
/// (`TRAME-232`).
#[test]
fn a_new_sheet_leaves_a_cross_sheet_formula_on_its_sheet() {
    let mut a = TrameState::new();
    a.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "5".to_owned(),
    });
    a.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet2"),
    });
    let s2 = second_sheet_id(&a);
    a.handle_command(TrameCommand::ActivateSheet { sheet_id: s2 });
    // B1 exists before the formula reads it, so the formula depends on it.
    a.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "0".to_owned(),
    });
    a.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "=Sheet1!A1+B1".to_owned(),
    });
    assert_eq!(read_cell(&mut a, 0, 0), "5");

    a.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet3"),
    });
    // A recompute of A1 reads Sheet1!A1 again, through the compiled index.
    a.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "1".to_owned(),
    });
    assert_eq!(
        read_cell(&mut a, 0, 0),
        "6",
        "Sheet2!A1 still reads Sheet1, not the new sheet"
    );
}
