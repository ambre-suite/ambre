// ambre-trame/src/logic/tests/group3.rs
// TrameState unit tests (group 3/7). Helpers via `use super::*`;
// logic-module items are reached with `super::super::` paths.

use super::*;

/// `UnmergeActive` over a selection spanning several merges splits every one of
/// them, not just the merge under the active cell (regression: only the last
/// selected merge was freed).
#[test]
fn unmerge_active_splits_every_merge_in_selection() {
    let mut state = TrameState::new();
    // Build two separate merges: A1:B1 and A3:B3.
    state.handle_command(TrameCommand::MergeCells {
        row: 0,
        col: 0,
        row_end: 0,
        col_end: 1,
    });
    state.handle_command(TrameCommand::MergeCells {
        row: 2,
        col: 0,
        row_end: 2,
        col_end: 1,
    });
    assert_eq!(read_display(&mut state, 0, 0).merge_span, Some((2, 1)));
    assert_eq!(read_display(&mut state, 2, 0).merge_span, Some((2, 1)));

    // Select A1:B3, covering both merges, then unmerge.
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 2,
        anchor_col: 1,
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
        TrameEvent::SelectRange { r1, c1, r2, c2 } => Some((*r1, *c1, *r2, *c2)),
        _ => None,
    });
    assert_eq!(
        range,
        Some((0, 0, 2, 1)),
        "selection covers all freed cells"
    );
}

/// `UnmergeActive` over a selection that only clips the edge of a merge still
/// grows the post-unmerge selection to the freed block: the merge extends past
/// the drag rectangle, so the exposed cells must end up selected (the reported
/// bug -- the selection was smaller than the unmerged cells).
#[test]
fn unmerge_selection_grows_to_cover_straddling_merge() {
    let mut state = TrameState::new();
    // A merge B2:C3 (rows 1..2, cols 1..2).
    state.handle_command(TrameCommand::MergeCells {
        row: 1,
        col: 1,
        row_end: 2,
        col_end: 2,
    });
    // Select only A1:B2 -- it clips the merge's top-left corner.
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 1,
        anchor_col: 1,
    });
    let out = state.handle_command(TrameCommand::UnmergeActive);

    assert_eq!(
        read_display(&mut state, 1, 1).merge_span,
        None,
        "merge split"
    );
    let range = out.display.iter().find_map(|e| match e {
        TrameEvent::SelectRange { r1, c1, r2, c2 } => Some((*r1, *c1, *r2, *c2)),
        _ => None,
    });
    // A1 corner (0,0) unioned with the full B2:C3 block -> A1:C3.
    assert_eq!(range, Some((0, 0, 2, 2)), "selection grew over the merge");
}

/// Two vertical merges A1:A2 and B1:B2, selected by their masters only (row 0),
/// then unmerged: the post-unmerge selection must grow down to row 1 to cover
/// every freed cell (the exact case reported: only A1/B1 stayed selected).
#[test]
fn unmerge_two_vertical_merges_selected_by_masters() {
    let mut state = TrameState::new();
    // A1:A2 (rows 0..1, col 0).
    state.handle_command(TrameCommand::MergeCells {
        row: 0,
        col: 0,
        row_end: 1,
        col_end: 0,
    });
    // B1:B2 (rows 0..1, col 1).
    state.handle_command(TrameCommand::MergeCells {
        row: 0,
        col: 1,
        row_end: 1,
        col_end: 1,
    });
    // Selection spans the two masters only: A1:B1 (row 0).
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 1,
        anchor_row: 0,
        anchor_col: 0,
    });
    let out = state.handle_command(TrameCommand::UnmergeActive);

    let range = out.display.iter().find_map(|e| match e {
        TrameEvent::SelectRange { r1, c1, r2, c2 } => Some((*r1, *c1, *r2, *c2)),
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
        row: 2,
        col: 2,
        anchor_row: 2,
        anchor_col: 2,
    });
    let evts = state.handle_command(TrameCommand::MergeSelection);
    assert!(evts.update.is_none(), "no CRDT mutation");
    assert_eq!(read_display(&mut state, 2, 2).merge_span, None);
}

/// A concurrent edit that lands a value in a covered cell is masked, not
/// destroyed: the value is hidden while the merge is active and reappears on
/// unmerge (§6, data is never destroyed).
#[test]
fn concurrent_value_under_merge_is_masked_then_restored() {
    // Shared base so both peers address rows 0/1 by the *same* stable ids
    // (concurrent dense-list inserts would otherwise create distinct rows).
    // Seeding then clearing A2 materializes rows 0 and 1 without leaving a
    // value behind.
    let mut base = TrameState::new();
    base.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "seed".to_owned(),
    });
    base.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: String::new(),
    });
    let base_snap = base.export_snapshot();

    let mut a = TrameState::new();
    a.import_snapshot(&base_snap);
    let mut b = TrameState::new();
    b.import_snapshot(&base_snap);

    // Peer A merges A1:A2 (both empty on its side).
    a.handle_command(TrameCommand::MergeCells {
        row: 0,
        col: 0,
        row_end: 1,
        col_end: 0,
    });
    let merge_delta = a.export_snapshot();

    // Peer B concurrently writes A2, then receives A's merge.
    b.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "concurrent".to_owned(),
    });
    let conflicts = b.apply_remote(&merge_delta);
    assert!(
        conflicts
            .iter()
            .any(|e| matches!(e, TrameEvent::MergeConflictDetected { .. })),
        "the covered non-empty cell is reported as a conflict"
    );
    // While merged, A2 is masked (blank) but the data still exists.
    assert_eq!(read_display(&mut b, 1, 0).text, "");

    // Unmerge restores the masked value.
    b.handle_command(TrameCommand::UnmergeCells { row: 0, col: 0 });
    assert_eq!(
        read_display(&mut b, 1, 0).text,
        "concurrent",
        "masked value reappears"
    );
}

/// A merge follows its cells across a structural insert: inserting a row
/// above the merge shifts the master span down with it.
#[test]
fn merge_follows_structural_insert() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "x".to_owned(),
    });
    state.handle_command(TrameCommand::MergeCells {
        row: 1,
        col: 0,
        row_end: 1,
        col_end: 1,
    });
    assert_eq!(read_display(&mut state, 1, 0).merge_span, Some((2, 1)));

    // Insert a row at the top: the merge shifts from row 1 to row 2.
    state.handle_command(TrameCommand::InsertRow { at: 0 });
    assert_eq!(
        read_display(&mut state, 1, 0).merge_span,
        None,
        "old position freed"
    );
    assert_eq!(
        read_display(&mut state, 2, 0).merge_span,
        Some((2, 1)),
        "merge shifted down"
    );
}

/// An edit emits a CRDT delta through the `CommandSink`: absorbing the
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
        row: 0,
        col: 0,
        text: "hi".to_owned(),
    });
    sync.absorb(doc, &outcome);

    let sent = sync.drain();
    assert_eq!(sent.len(), 1, "one SendUpdate per mutating command");
    assert!(matches!(
        &sent[0],
        SyncCommand::SendUpdate { document_id, payload }
            if *document_id == doc && !payload.is_empty()
    ));
}

/// Full two-peer collaboration loop driven through `MockSyncHandle`: peer A
/// edits, the mock captures and relays the delta to peer B, and B converges
/// on A's value -- a controller test with no network and no hand-shuffled
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
        row: 0,
        col: 0,
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

/// Two peers sharing a cross-sheet formula converge when one renames the
/// target sheet: the reference is bound to the stable `sheet_id` (same id on
/// both peers via sync), so both keep computing and reconstruct identical
/// formula-bar text (spec section 2.D).
#[test]
fn cross_sheet_reference_converges_across_peers_on_rename() {
    let mut a = TrameState::new();

    // A builds the workbook: Sheet2!A1 = 5, Sheet1!B1 = Sheet2!A1 * 2.
    a.handle_command(TrameCommand::CreateSheet {
        name: "Sheet2".to_owned(),
    });
    let s2 = second_sheet_id(&a);
    a.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    a.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "5".to_owned(),
    });
    a.handle_command(TrameCommand::ActivateSheet {
        sheet_id: "s0".to_owned(),
    });
    a.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=Sheet2!A1*2".to_owned(),
    });

    // B joins by loading A's snapshot (the real "open shared doc" path, so
    // both peers share the same op history) and converges on the value.
    let mut b = TrameState::new();
    b.import_snapshot(&a.export_snapshot());
    assert_eq!(
        read_cell(&mut b, 0, 1),
        "10",
        "B resolves the shared sheet_id"
    );
    assert_eq!(read_raw(&mut b, 0, 1), "=Sheet2!A1*2");

    // B renames the target sheet; A merges B's state (a snapshot carries
    // B's full history, including the implicit-s0 op each peer mints in
    // `new()`, which an incremental delta would leave as a dangling dep).
    b.handle_command(TrameCommand::RenameSheet {
        sheet_id: s2.clone(),
        name: "Data".to_owned(),
    });
    a.apply_remote(&b.export_snapshot());

    // Both peers keep computing and reconstruct the same new label.
    assert_eq!(read_cell(&mut a, 0, 1), "10");
    assert_eq!(read_cell(&mut b, 0, 1), "10");
    assert_eq!(read_raw(&mut a, 0, 1), "=Data!A1*2");
    assert_eq!(read_raw(&mut b, 0, 1), "=Data!A1*2");
}

/// Creating a sheet mints a stable id, lists it after the default in the tab
/// strip, and keeps its cells isolated from the default sheet.
#[test]
fn create_sheet_lists_and_isolates() {
    let mut state = TrameState::new();
    let evts = state.handle_command(TrameCommand::CreateSheet {
        name: "Data".to_owned(),
    });
    let TrameEvent::SheetList { sheets, active } = &evts.display[0] else {
        panic!("expected SheetList")
    };
    assert_eq!(active, "s0", "active sheet unchanged by create");
    assert_eq!(sheets.len(), 2);
    assert_eq!(
        sheets[0],
        ("s0".to_owned(), "Sheet1".to_owned()),
        "default tab first"
    );
    assert_eq!(sheets[1].1, "Data");

    let s1 = sheets[1].0.clone();
    // Write on the default sheet, switch to the new one: it is blank.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "on_s0".to_owned(),
    });
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s1.clone(),
    });
    assert_eq!(read_cell(&mut state, 0, 0), "", "new sheet starts empty");
    // Write here, switch back: each sheet keeps its own value.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "on_s1".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 0), "on_s1");
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: "s0".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 0), "on_s0");
}

/// The first `ScrollViewport` of a session rides a `SheetList` back so the
/// UI tab strip hydrates on load (no separate fetch command); subsequent
/// scrolls carry only the viewport.
#[test]
fn first_scroll_viewport_hydrates_sheet_list_once() {
    let mut state = TrameState::new();
    let scroll = || TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 4,
        col_start: 0,
        col_end: 4,
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
    assert_eq!(active, "s0");
    assert_eq!(sheets, &vec![("s0".to_owned(), "Sheet1".to_owned())]);

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
        name: "Sheet2".to_owned(),
    });
    let s2 = second_sheet_id(&state);

    // Put 5 into Sheet2!A1.
    state.handle_command(TrameCommand::ActivateSheet { sheet_id: s2 });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "5".to_owned(),
    });

    // Back on Sheet1, B1 references it.
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: "s0".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=Sheet2!A1*2".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 1), "10", "B1 = Sheet2!A1 * 2");
}

/// A masked cell (covered by a merge, non-empty -> concurrent conflict) is
/// excluded from computation on *every* sheet, not only the displayed one (§6).
/// Regression for the view-dependent masking bug: masking was applied to the
/// engine feed only at local index 0 (the active sheet), so a cross-sheet
/// reference to a masked cell on a *non-active* sheet leaked the hidden value,
/// making the formula result depend on which tab was shown.
#[test]
fn masked_cell_excluded_across_sheets_regardless_of_active_tab() {
    // Shared base: Sheet2 exists with rows 0/1 materialized under stable ids
    // (seed+clear A2), a same-sheet reader `Sheet2!C1 = =A2`, and a cross-sheet
    // reader `Sheet1!B1 = =Sheet2!A2`. Both peers inherit identical ids.
    let mut base = TrameState::new();
    base.handle_command(TrameCommand::CreateSheet {
        name: "Sheet2".to_owned(),
    });
    let s2 = second_sheet_id(&base);
    base.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    base.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "seed".to_owned(),
    });
    base.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: String::new(),
    });
    // Same-sheet reader on Sheet2 (read while Sheet2 is the active tab).
    base.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 2,
        text: "=A2".to_owned(),
    });
    // Cross-sheet reader on Sheet1 (read while Sheet1 is the active tab, so the
    // masked cell lives on a *non-active* sheet).
    base.handle_command(TrameCommand::ActivateSheet {
        sheet_id: "s0".to_owned(),
    });
    base.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=Sheet2!A2".to_owned(),
    });
    let base_snap = base.export_snapshot();

    let mut a = TrameState::new();
    a.import_snapshot(&base_snap);
    let mut b = TrameState::new();
    b.import_snapshot(&base_snap);

    // Peer A merges Sheet2!A1:A2 (both empty on its side).
    a.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    a.handle_command(TrameCommand::MergeCells {
        row: 0,
        col: 0,
        row_end: 1,
        col_end: 0,
    });
    let merge_delta = a.export_snapshot();

    // Peer B concurrently writes Sheet2!A2, then receives A's merge -> A2 is
    // masked (covered + non-empty), and stays the active tab.
    b.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    b.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "77".to_owned(),
    });
    let conflicts = b.apply_remote(&merge_delta);
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
        sheet_id: "s0".to_owned(),
    });
    let target_inactive = read_cell(&mut b, 0, 1);

    assert_ne!(
        target_inactive, "77",
        "the cross-sheet reader must not leak the masked value when its target sheet is inactive"
    );
    assert_eq!(
        target_inactive, target_active,
        "a masked cell reads identically whether or not its sheet is the active tab (view-independent, §6)"
    );
}

/// Renaming changes only the display label; the stable id (and the cells
/// keyed under it) are untouched.
#[test]
fn rename_sheet_keeps_id_and_cells() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: "Data".to_owned(),
    });
    let s1 = second_sheet_id(&state);
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s1.clone(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "v".to_owned(),
    });

    let evts = state.handle_command(TrameCommand::RenameSheet {
        sheet_id: s1.clone(),
        name: "Renamed".to_owned(),
    });
    let TrameEvent::SheetList { sheets, .. } = &evts.display[0] else {
        panic!()
    };
    assert!(
        sheets
            .iter()
            .any(|(id, name)| id == &s1 && name == "Renamed")
    );
    // The value survives the rename (same id, same container).
    assert_eq!(read_cell(&mut state, 0, 0), "v");
}

/// Moving a sheet reorders the tab strip without touching its content.
#[test]
fn move_sheet_reorders_tabs() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: "B".to_owned(),
    });
    let b = second_sheet_id(&state);
    // Order is [s0, B]; move B to the front.
    let evts = state.handle_command(TrameCommand::MoveSheet {
        sheet_id: b.clone(),
        to_index: 0,
    });
    let TrameEvent::SheetList { sheets, .. } = &evts.display[0] else {
        panic!()
    };
    assert_eq!(sheets[0].0, b, "B moved to the front");
    assert_eq!(sheets[1].0, "s0");
}

/// Deleting the last remaining sheet is rejected; the workbook keeps >= 1.
#[test]
fn delete_last_sheet_rejected() {
    let mut state = TrameState::new();
    let evts = state.handle_command(TrameCommand::DeleteSheet {
        sheet_id: "s0".to_owned(),
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
        name: "B".to_owned(),
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
    assert_eq!(list.1, "s0", "active fell back to the default sheet");
    assert!(!list.0.iter().any(|(id, _)| id == &b), "deleted sheet gone");
}

/// Ascending sort reorders whole rows by the key column; cells in other
/// columns of the same row travel with it (stable-id keyed).
#[test]
fn sort_range_ascending_moves_whole_rows() {
    let mut state = TrameState::new();
    // A: 3,1,2  B: "c","a","b" (paired with A so we can check rows move whole)
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
    let evts = state.handle_command(TrameCommand::SortRange {
        sheet_id: "s0".to_owned(),
        row_start: 0,
        row_end: 2,
        col_start: 0,
        col_end: 1,
        sort_col: 0,
        ascending: true,
        has_header: false,
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

/// Descending sort and the `has_header` pin: the first row stays put.
#[test]
fn sort_range_descending_keeps_header() {
    let mut state = TrameState::new();
    for (r, a) in ["Header", "1", "3", "2"].iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: a.to_string(),
        });
    }
    state.handle_command(TrameCommand::SortRange {
        sheet_id: "s0".to_owned(),
        row_start: 0,
        row_end: 3,
        col_start: 0,
        col_end: 0,
        sort_col: 0,
        ascending: false,
        has_header: true,
    });
    assert_eq!(read_col(&mut state, 0, 3, 1), vec!["Header", "3", "2", "1"]);
}
