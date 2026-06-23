// ambre-trame/src/logic/tests.rs
// Unit tests for `TrameState` (moved verbatim out of logic.rs).

use super::{TrameOutcome, TrameState, col_to_alpha};
use crate::model::Uid;
use crate::{TrameCommand, TrameEvent};

/// A bold patch over `bold: Some(true)`.
fn bold_patch(on: bool) -> crate::StylePatch {
    crate::StylePatch {
        bold: Some(on),
        ..Default::default()
    }
}

/// Bold a cell, then move the selection elsewhere: the formatting must stay
/// pinned to the cell it was applied to, not follow the cursor. This is the
/// regression the per-cell model fixes.
#[test]
fn bold_stays_on_the_cell_not_the_selection() {
    let mut state = TrameState::new();

    // Select A1 (0,0) and bold it via the single style channel.
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 0,
        anchor_col: 0,
    });
    let evts = state.handle_command(TrameCommand::SetCellStyle {
        patch: bold_patch(true),
    });
    assert!(matches!(
        evts.display[0],
        TrameEvent::StyleApplied { bold: true, .. }
    ));

    // Move the selection to B2 (1,1): the active cell is not bold, so the
    // toolbar reports false -- A1 keeps its bold.
    let evts = state.handle_command(TrameCommand::SetActiveCell {
        row: 1,
        col: 1,
        anchor_row: 1,
        anchor_col: 1,
    });
    assert!(matches!(
        evts.display[0],
        TrameEvent::StyleApplied { bold: false, .. }
    ));

    // A viewport covering both cells: only A1 carries the bold run.
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 1,
        col_start: 0,
        col_end: 1,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!("expected ViewportData");
    };
    let a1 = cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
    let b2 = cells.iter().find(|c| c.row == 1 && c.col == 1).unwrap();
    assert!(a1.bold, "A1 must stay bold");
    assert!(!b2.bold, "B2 must not become bold");
}

/// Bold and italic survive a snapshot round-trip (the F5 reload path):
/// formatting must be persisted in the CRDT, not only in memory.
#[test]
fn formatting_survives_snapshot_roundtrip() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 0,
        anchor_col: 0,
    });
    state.handle_command(TrameCommand::SetCellStyle {
        patch: crate::StylePatch {
            bold: Some(true),
            italic: Some(true),
            ..Default::default()
        },
    });

    // Simulate a reload: fresh state restored from the persisted snapshot.
    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored.import_snapshot(&snapshot);

    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 0,
        col_start: 0,
        col_end: 0,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!("expected ViewportData");
    };
    let a1 = cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
    assert!(a1.bold, "A1 must stay bold after reload");
    assert!(a1.italic, "A1 must stay italic after reload");
}

/// `SetCellStyle` applies a [`StylePatch`] over the selection, persists it in
/// the CRDT (survives a snapshot round-trip), reformats numeric display text
/// per the number format, and reports the active cell's resolved style for the
/// toolbar (`-ui.md` sec.5bis).
#[test]
fn set_cell_style_round_trips_and_formats_numbers() {
    use crate::{CellFormat, HAlign, StylePatch};

    let mut state = TrameState::new();
    // A numeric value at A1, selection collapsed on it.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "3.14159".to_owned(),
    });
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 0,
        anchor_col: 0,
    });

    // Apply Percent with 1 decimal + right alignment + underline.
    let out = state.handle_command(TrameCommand::SetCellStyle {
        patch: StylePatch {
            format: Some(CellFormat::Percent),
            decimals: Some(Some(1)),
            h_align: Some(HAlign::Right),
            underline: Some(true),
            ..Default::default()
        },
    });
    // The toolbar event reflects the resolved style.
    let style_ev = out
        .display
        .iter()
        .find_map(|e| match e {
            TrameEvent::StyleApplied { style, .. } => Some(style.clone()),
            _ => None,
        })
        .expect("StyleApplied emitted");
    assert_eq!(style_ev.format, CellFormat::Percent);
    assert_eq!(style_ev.h_align, HAlign::Right);
    assert!(style_ev.underline);

    // Reload from a snapshot: the style must persist in the CRDT.
    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored.import_snapshot(&snapshot);
    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 0,
        col_start: 0,
        col_end: 0,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!("expected ViewportData");
    };
    let a1 = cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
    assert!(a1.underline, "underline must survive reload");
    assert_eq!(a1.h_align, HAlign::Right, "alignment must survive reload");
    // 3.14159 -> Percent, 1 decimal -> "314.2%".
    assert_eq!(a1.text, "314.2%");
}

/// An edited cell stores its text in the CRDT and overrides the generated
/// label; clearing it falls back to blank.
#[test]
fn edited_text_overrides_generated_label() {
    let mut state = TrameState::new();

    // A1 starts blank (virtual cell).
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 0,
        col_start: 0,
        col_end: 0,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    assert_eq!(cells[0].text, "");

    // Edit A1, then read it back: the stored text wins.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "hello".to_owned(),
    });
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 2,
        row_start: 0,
        row_end: 0,
        col_start: 0,
        col_end: 0,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    assert_eq!(cells[0].text, "hello");

    // Clearing it restores the blank cell.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: String::new(),
    });
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 3,
        row_start: 0,
        row_end: 0,
        col_start: 0,
        col_end: 0,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    assert_eq!(cells[0].text, "");
}

/// A value written far down the grid is keyed by its row/column stable ids,
/// survives a snapshot round-trip, and resolves at the same grid position in
/// a fresh state -- proving the LID-keyed persistence + `SheetIndex` rebuild.
#[test]
fn far_cell_survives_snapshot_roundtrip() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 1000,
        col: 3,
        text: "deep".to_owned(),
    });

    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored.import_snapshot(&snapshot);

    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 1000,
        row_end: 1000,
        col_start: 3,
        col_end: 3,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    assert_eq!(cells[0].text, "deep");
    // An untouched far cell is blank (virtual space).
    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: 2,
        row_start: 2000,
        row_end: 2000,
        col_start: 0,
        col_end: 0,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    assert_eq!(cells[0].text, "");
}

/// A formula referencing another cell still evaluates after the value and the
/// formula are stored under stable id keys and reloaded.
#[test]
fn formula_evaluates_through_stable_keys() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "=21".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=A1*2".to_owned(),
    });

    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored.import_snapshot(&snapshot);

    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 0,
        col_start: 0,
        col_end: 1,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let b1 = cells.iter().find(|c| c.row == 0 && c.col == 1).unwrap();
    assert_eq!(b1.text, "42", "B1 = A1*2 must evaluate after reload");
}

/// A defined name resolves inside a formula, recomputes when its target
/// changes, survives a snapshot round-trip, and breaks when deleted.
#[test]
fn named_range_defines_resolves_and_deletes() {
    let mut state = TrameState::new();
    // A1=1, A2=2, A3=3.
    for r in 0..3 {
        state.handle_command(TrameCommand::SetCellText {
            row: r,
            col: 0,
            text: format!("{}", r + 1),
        });
    }
    // Define TOTAL = A1:A3, then C1 = SUM(TOTAL).
    state.handle_command(TrameCommand::DefineName {
        name: "TOTAL".to_owned(),
        reference: "A1:A3".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 2,
        text: "=SUM(TOTAL)".to_owned(),
    });

    let read_c1 = |state: &mut TrameState, version: u64| {
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version,
            row_start: 0,
            row_end: 0,
            col_start: 2,
            col_end: 2,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
            panic!()
        };
        cells[0].text.clone()
    };

    assert_eq!(read_c1(&mut state, 1), "6", "SUM(TOTAL) = 1+2+3");

    // Editing a target cell recomputes the name's consumer.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "10".to_owned(),
    });
    assert_eq!(read_c1(&mut state, 2), "15", "SUM(TOTAL) tracks A1 edit");

    // The name survives a snapshot round-trip.
    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored.import_snapshot(&snapshot);
    assert_eq!(
        read_c1(&mut restored, 3),
        "15",
        "name persists across reload"
    );

    // Deleting the name stops it resolving: its consumer no longer computes
    // the sum (the engine drops the now-unresolvable formula, so the cell
    // falls back to its raw text rather than the stale 15).
    restored.handle_command(TrameCommand::DeleteName {
        name: "TOTAL".to_owned(),
    });
    let after = read_c1(&mut restored, 4);
    assert_ne!(after, "15", "deleted name must no longer resolve");
    assert_eq!(
        after, "=SUM(TOTAL)",
        "unresolved formula falls back to raw text"
    );
}

/// End-to-end of the reported bug: plain numeric literals typed into A1/B1
/// must feed `=SUM(A1:B1)` (display "3"), and the formula cell's `raw` must
/// carry the formula source (so the in-cell editor reopens on the formula,
/// not its result).
#[test]
fn sum_over_plain_literals_and_formula_raw_in_viewport() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 2,
        text: "=SUM(A1:B1)".to_owned(),
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 0,
        col_start: 0,
        col_end: 2,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };

    let a1 = cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
    assert_eq!(a1.text, "1");
    assert_eq!(a1.raw, "1", "plain cell raw == display");

    let c1 = cells.iter().find(|c| c.row == 0 && c.col == 2).unwrap();
    assert_eq!(c1.text, "3", "SUM must see plain literals A1=1, B1=2");
    assert_eq!(c1.raw, "=SUM(A1:B1)", "editor must reopen on the formula");
}

/// The same scenario after a reseed (the path a remote echo / snapshot
/// restore takes): plain literals must still feed the formula so the
/// refreshed viewport shows "3", not the optimistic formula text.
#[test]
fn sum_over_plain_literals_survives_reseed() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 2,
        text: "=SUM(A1:B1)".to_owned(),
    });

    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored.import_snapshot(&snapshot);

    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 0,
        col_start: 0,
        col_end: 2,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let c1 = cells.iter().find(|c| c.row == 0 && c.col == 2).unwrap();
    assert_eq!(c1.text, "3", "SUM must re-evaluate after reseed");
    assert_eq!(c1.raw, "=SUM(A1:B1)");
}

/// Two sheets keep their cells isolated (distinct `cells_{sheet}` containers)
/// and `sheet_name_to_id` resolves the default sheet's name.
#[test]
fn multi_sheet_isolation_and_name_lookup() {
    let mut state = TrameState::new();
    assert_eq!(state.sheet_name_to_id("Sheet1").as_deref(), Some("s0"));
    assert_eq!(state.sheet_name_to_id("Nope"), None);

    // Write to the default sheet, then to a second sheet's containers
    // directly (the add-sheet command lands in a later step).
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "on_s0".to_owned(),
    });
    let _ = state.doc.get_map(super::SHEETS_MAP).insert("s1", "Sheet2");
    let other = state.doc.get_movable_list(super::rows_list_name("s1"));
    other
        .push(
            Uid {
                peer: 1,
                counter: 0,
            }
            .to_key_part()
            .as_str(),
        )
        .unwrap();
    state.doc.commit();

    // `cells_s1` is empty; `cells_s0` holds the value -- containers are isolated.
    assert!(matches!(
        state.doc.get_map(super::cells_map_name("s0")).get_value(),
        loro::LoroValue::Map(ref m) if m.len() == 1
    ));
    assert!(matches!(
        state.doc.get_map(super::cells_map_name("s1")).get_value(),
        loro::LoroValue::Map(ref m) if m.is_empty()
    ));
    assert_eq!(state.sheet_name_to_id("Sheet2").as_deref(), Some("s1"));
}

/// Read the text at a single grid cell through a viewport request.
fn read_cell(state: &mut TrameState, row: u32, col: u32) -> String {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 999,
        row_start: row,
        row_end: row,
        col_start: col,
        col_end: col,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    cells[0].text.clone()
}

/// Read the formula-bar `raw` text at a single grid cell through a viewport
/// request (the reconstructed display form, not the stored canonical form).
fn read_raw(state: &mut TrameState, row: u32, col: u32) -> String {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 999,
        row_start: row,
        row_end: row,
        col_start: col,
        col_end: col,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    cells[0].raw.clone()
}

/// A cross-sheet formula is stored bound to the stable `sheet_id` (`#id`
/// form), so a later rename keeps it computing and its formula-bar text
/// follows the new name; a reorder is inert; deleting the target sheet turns
/// it into `#REF!` (spec section 2.D / section 12).
#[test]
fn cross_sheet_reference_survives_rename_and_breaks_on_delete() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: "Sheet2".to_owned(),
    });
    let s2 = second_sheet_id(&state);

    // Sheet2!A1 = 5.
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "5".to_owned(),
    });

    // Sheet1!B1 = Sheet2!A1 * 2.
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: "s0".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=Sheet2!A1*2".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 1), "10");
    // Stored canonical form is id-bound; the formula bar shows the name.
    assert_eq!(
        state.cell_raw(0, 1),
        format!("=#{s2}!A1*2"),
        "v is sheet_id-bound"
    );
    assert_eq!(
        read_raw(&mut state, 0, 1),
        "=Sheet2!A1*2",
        "bar shows display name"
    );

    // Rename the target: the reference keeps computing and the bar follows.
    state.handle_command(TrameCommand::RenameSheet {
        sheet_id: s2.clone(),
        name: "Data".to_owned(),
    });
    assert_eq!(
        read_cell(&mut state, 0, 1),
        "10",
        "rename preserves the reference"
    );
    assert_eq!(
        read_raw(&mut state, 0, 1),
        "=Data!A1*2",
        "bar reflects the new name"
    );

    // Reorder: pure presentation, value unchanged.
    state.handle_command(TrameCommand::MoveSheet {
        sheet_id: s2.clone(),
        to_index: 0,
    });
    assert_eq!(read_cell(&mut state, 0, 1), "10", "reorder is inert");

    // Delete the target sheet: the cross-sheet reference becomes #REF!.
    state.handle_command(TrameCommand::DeleteSheet { sheet_id: s2 });
    assert_eq!(
        read_cell(&mut state, 0, 1),
        "#REF!",
        "deleted target -> #REF!"
    );
}

/// Undo reverts the last local edit; redo reapplies it. The `UndoManager`
/// records one item per command (one commit), so a single undo restores the
/// cell to its pre-edit (empty) label and a single redo brings the value back.
#[test]
fn undo_then_redo_round_trips_a_cell_edit() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "hello".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 0), "hello");

    let evts = state.handle_command(TrameCommand::Undo);
    assert!(matches!(evts.display[0], TrameEvent::DataReset));
    // Undo produces inverse ops to broadcast, and reports the new stacks.
    assert!(evts.update.is_some(), "undo broadcasts its inverse ops");
    assert!(matches!(
        evts.display.last(),
        Some(TrameEvent::UndoState {
            can_undo: false,
            can_redo: true
        })
    ));
    assert_eq!(read_cell(&mut state, 0, 0), "", "value gone, cell blank");

    let evts = state.handle_command(TrameCommand::Redo);
    assert!(matches!(
        evts.display.last(),
        Some(TrameEvent::UndoState {
            can_undo: true,
            can_redo: false
        })
    ));
    assert_eq!(
        read_cell(&mut state, 0, 0),
        "hello",
        "redo restores the value"
    );
}

/// A multi-cell batch (Ctrl+Enter fill / Delete over a range) is a SINGLE
/// undo item: every write shares one undo group, so one `Undo` restores all
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
/// `LoroDoc` twice (the cell write + the filter recompute), yet the edit must
/// stay a single undo item (`-ui.md` §6.C). `SetCellText` opens an undo group
/// that `CommitEdit` seals on cell exit, so both commits collapse into one
/// step: a single `Undo` restores the pre-edit value and the prior hidden set.
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

/// Every mutating command refreshes the Undo/Redo availability so the chrome
/// enables the Undo button right after the first edit -- without waiting for
/// the user to trigger an undo. A non-mutating command emits no `UndoState`.
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

/// Undo is local-only (features spec §4): `UndoManager` tracks only this peer's
/// commits, never imported remote ops. Peer B writes its own cell, receives a
/// remote edit from A, then undoes -- only B's own edit is reverted; A's remote
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
    a.import_snapshot(&base_snap);
    let mut b = TrameState::new();
    b.import_snapshot(&base_snap);

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

/// Undoing an `InsertRow` must rebuild the `SheetIndex` so positions resolve
/// against the restored row order: the row that had shifted down returns to its
/// original position and the inserted blank row is gone.
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

/// The undo stack lives for the open-document session only (features spec §4:
/// "no undo stack persistence between sessions"). Reopening a document from its
/// persisted snapshot starts with a fresh, empty `UndoManager`: a prior edit is
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
    reopened.import_snapshot(&snapshot);

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
    restored.import_snapshot(&snapshot);
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

/// References are positional and never rewritten (§12): after deleting the
/// row that held a formula's source, the formula reads whatever now sits at
/// that grid position -- here an emptied cell. (Full id-based `#REF!` on
/// delete is a separate broken-references story.)
#[test]
fn formula_reference_is_positional_after_delete() {
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

    // Delete row 1 (the source A2). B1 stays at row 0; A2 is now empty, so
    // the positional reference reads an empty cell.
    state.handle_command(TrameCommand::DeleteRow { at: 1 });
    assert_eq!(read_cell(&mut state, 0, 1), "", "positional ref now empty");
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

/// Read the full DisplayCell at a single grid position.
fn read_display(state: &mut TrameState, row: u32, col: u32) -> crate::DisplayCell {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 5000,
        row_start: row,
        row_end: row,
        col_start: col,
        col_end: col,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    cells[0].clone()
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
    restored.import_snapshot(&snapshot);
    assert_eq!(read_display(&mut restored, 0, 0).merge_span, Some((2, 2)));
}

/// A merge is rejected locally when a covered cell is non-empty, and no CRDT
/// delta is produced (Excel/Sheets behavior, §6).
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

/// The id of the first non-default sheet in the current tab order.
fn second_sheet_id(state: &TrameState) -> String {
    state
        .sheet_order_ids()
        .into_iter()
        .find(|id| id != super::DEFAULT_SHEET)
        .expect("a second sheet")
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

/// Read column `col` over rows `[0..=row_end]` as display strings.
fn read_col(state: &mut TrameState, col: u32, row_end: u32, version: u64) -> Vec<String> {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version,
        row_start: 0,
        row_end,
        col_start: 0,
        col_end: col,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    (0..=row_end)
        .map(|r| {
            cells
                .iter()
                .find(|c| c.row == r && c.col == col)
                .map(|c| c.text.clone())
                .unwrap_or_default()
        })
        .collect()
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

/// Formula references follow the data through a sort (the section 12
/// guarantee). Each `=A{n}*10` binds to its row's `Uid` at edit time; the sort
/// reorders rows by `mov` and the uid-anchored bytecode is preserved (no
/// reseed/re-parse), so every formula keeps reading the *same physical* A cell
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
    // References follow the data: the `=A2*10` cell (bound to the row whose
    // A was 1) is now at pos0 and still reads 1 -> 10; the `=A1*10` cell
    // (bound to the row whose A was 2) is now at pos1 and reads 2 -> 20.
    assert_eq!(read_col(&mut state, 1, 1, 2), vec!["10", "20"]);
}

/// A range is a positional *block*, not a set of specific rows: sorting the
/// rows inside `SUM(A1:A3)` reorders the same three values, so the block (and
/// its total) is unchanged -- the Excel/Sheets behavior, and the only
/// well-defined one (uid-bound endpoints would scatter and invert the range).
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

/// The parameterless `SortSelectionAsc` intent (DOM context menu, sec.5.C)
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

/// The whole-sheet `SortSheetAsc` intent (whole-column selection, sec.5.C)
/// sorts the full used range by the active cell's column, keeping every row's
/// other columns aligned -- even rows below the values reported by a partial
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

#[test]
fn col_alpha_basics() {
    assert_eq!(col_to_alpha(0), "A");
    assert_eq!(col_to_alpha(25), "Z");
    assert_eq!(col_to_alpha(26), "AA");
    assert_eq!(col_to_alpha(51), "AZ");
    assert_eq!(col_to_alpha(52), "BA");
    assert_eq!(col_to_alpha(701), "ZZ");
}

// -------------------------------------------------------------------------
// Shared filter (features spec §1.A)
// -------------------------------------------------------------------------

use ambre_bus::{FilterKind, FilterOp};

/// Whether grid `row` is hidden by the active filter (read through a viewport).
fn is_filtered(state: &mut TrameState, row: u32, col: u32) -> bool {
    read_display(state, row, col).is_filtered
}

/// Seed column A (col 0) with one value per row, top to bottom.
fn seed_col_a(state: &mut TrameState, values: &[&str]) {
    for (r, v) in values.iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: r as u32,
            col: 0,
            text: (*v).to_owned(),
        });
    }
}

/// `RequestActiveColumnValues` returns the active column's distinct visible
/// values (deduped, sorted, empties dropped) for the popover's "by values"
/// list (`-ui.md` sec.5.D).
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
/// (`-ui.md` sec.13bis): `count` tallies non-empty cells, the numeric ones feed
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
/// active cell, mirroring the sort intents (`-ui.md` sec.5.D).
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

/// Hidden rows are excluded from the viewport but still counted by `=SUM`
/// (features spec §8): the filter is a display concern, not a data concern.
#[test]
fn shared_filter_does_not_affect_sum() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["1", "2", "3"]);
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=SUM(A1:A3)".to_owned(),
    });

    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: "s0".to_owned(),
        col: 0,
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
        sheet_id: "s0".to_owned(),
    });
    assert!(noop.display.is_empty());
    assert!(noop.update.is_none());

    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: "s0".to_owned(),
        col: 0,
        kind: FilterKind::Values(vec!["a".to_owned()]),
    });
    assert!(is_filtered(&mut state, 1, 0), "b hidden");

    let cleared = state.handle_command(TrameCommand::ClearFilter {
        sheet_id: "s0".to_owned(),
    });
    assert!(matches!(cleared.display[0], TrameEvent::FilterCleared));
    assert!(cleared.update.is_some(), "clear broadcasts the removal");
    assert!(!is_filtered(&mut state, 1, 0), "row visible again");
}

/// The hidden set is keyed by stable `row_uid`, so it follows its row across a
/// sort -- the hidden row tracks the data, no recompute needed.
#[test]
fn shared_filter_hidden_follows_sort() {
    let mut state = TrameState::new();
    // Rows: A1=3, A2=1, A3=2.
    seed_col_a(&mut state, &["3", "1", "2"]);
    // Hide the row whose value is exactly 1 (the middle row).
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: "s0".to_owned(),
        col: 0,
        kind: FilterKind::Condition {
            op: FilterOp::Ne,
            value: "1".to_owned(),
        },
    });
    assert!(is_filtered(&mut state, 1, 0), "row holding 1 hidden");

    // Sort ascending: order becomes [1, 2, 3]; the hidden row (value 1) moves
    // to the top and its hidden flag must travel with it.
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
        sheet_id: "s0".to_owned(),
        col: 0,
        kind: FilterKind::Condition {
            op: FilterOp::Gt,
            value: "10".to_owned(),
        },
    });
    assert!(is_filtered(&mut state, 0, 0), "5 is not > 10");

    // Raise the value above the threshold: the row must reappear.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
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
        sheet_id: "s0".to_owned(),
        col: 0,
        kind: FilterKind::Condition {
            op: FilterOp::Gte,
            value: "20".to_owned(),
        },
    });
    let snapshot = author.export_snapshot();

    let mut peer = TrameState::new();
    peer.apply_remote(&snapshot);
    assert!(is_filtered(&mut peer, 0, 0), "10 hidden on the peer");
    assert!(!is_filtered(&mut peer, 1, 0));
    // The stored condition survived: an edit on the peer recomputes it.
    peer.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "25".to_owned(),
    });
    assert!(
        !is_filtered(&mut peer, 0, 0),
        "25 >= 20 after edit -> visible"
    );
}

/// Concurrent different filters converge: Loro LWW picks a single hidden set,
/// identical on both peers (features spec §1.A "deterministic behavior").
#[test]
fn concurrent_filters_converge_lww() {
    // Shared base so both peers address the same row uids.
    let mut base = TrameState::new();
    seed_col_a(&mut base, &["1", "2", "3"]);
    let base_snap = base.export_snapshot();

    let mut a = TrameState::new();
    a.import_snapshot(&base_snap);
    let mut b = TrameState::new();
    b.import_snapshot(&base_snap);

    a.handle_command(TrameCommand::ApplyFilter {
        sheet_id: "s0".to_owned(),
        col: 0,
        kind: FilterKind::Condition {
            op: FilterOp::Gt,
            value: "1".to_owned(),
        },
    });
    b.handle_command(TrameCommand::ApplyFilter {
        sheet_id: "s0".to_owned(),
        col: 0,
        kind: FilterKind::Condition {
            op: FilterOp::Lt,
            value: "3".to_owned(),
        },
    });

    // Exchange deltas (full snapshots merge fine).
    let a_snap = a.export_snapshot();
    let b_snap = b.export_snapshot();
    a.apply_remote(&b_snap);
    b.apply_remote(&a_snap);

    assert_eq!(
        a.hidden_rows, b.hidden_rows,
        "both peers converge to one hidden set"
    );
}

/// A filter is a plain CRDT mutation, so Ctrl+Z removes it (features spec §4).
#[test]
fn shared_filter_is_undoable() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["a", "b"]);
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: "s0".to_owned(),
        col: 0,
        kind: FilterKind::Values(vec!["a".to_owned()]),
    });
    assert!(is_filtered(&mut state, 1, 0), "b hidden by the filter");

    state.handle_command(TrameCommand::Undo);
    assert!(!is_filtered(&mut state, 1, 0), "undo removed the filter");
}

/// Apply/clear on a non-active sheet, or a virtual column, is a pure no-op.
#[test]
fn shared_filter_noop_guards() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["x"]);

    // Wrong sheet.
    let wrong = state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: "nope".to_owned(),
        col: 0,
        kind: FilterKind::Values(vec!["x".to_owned()]),
    });
    assert!(wrong.display.is_empty() && wrong.update.is_none());
    let wrong_clear = state.handle_command(TrameCommand::ClearFilter {
        sheet_id: "nope".to_owned(),
    });
    assert!(wrong_clear.display.is_empty() && wrong_clear.update.is_none());

    // Virtual column (never materialized): nothing to filter.
    let virtual_col = state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: "s0".to_owned(),
        col: 50,
        kind: FilterKind::Values(vec!["x".to_owned()]),
    });
    assert!(virtual_col.display.is_empty() && virtual_col.update.is_none());
}

/// Unit-test every `condition_pass` operator and both the numeric and the
/// text comparison paths (full branch coverage of the predicate).
#[test]
fn condition_pass_covers_every_operator() {
    use super::condition_pass;
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

/// `filter_pass` for the values mode, and the operator string round-trip.
#[test]
fn filter_pass_values_and_op_string_roundtrip() {
    use super::{filter_pass, op_from_str, op_to_str};
    use crate::formula::types::CellValue;
    let kind = FilterKind::Values(vec!["keep".to_owned()]);
    assert!(filter_pass(&kind, &CellValue::Text("keep".to_owned())));
    assert!(!filter_pass(&kind, &CellValue::Text("drop".to_owned())));

    for op in [
        FilterOp::Eq,
        FilterOp::Ne,
        FilterOp::Gt,
        FilterOp::Lt,
        FilterOp::Gte,
        FilterOp::Lte,
        FilterOp::Contains,
        FilterOp::NotContains,
        FilterOp::StartsWith,
        FilterOp::IsEmpty,
        FilterOp::IsNotEmpty,
    ] {
        assert_eq!(op_from_str(op_to_str(op)), Some(op), "round-trips {op:?}");
    }
    assert_eq!(op_from_str("garbage"), None);
}

/// A pasted formula's relative references reindex by the paste offset, and a
/// plain literal is copied verbatim (`-ui.md` sec.5.E). Copy A1:B1 (A1=10,
/// B1==A1*2) and paste with the top-left landing on A3: B3 must store the
/// reindexed `=A3*2` and compute `20`.
#[test]
fn paste_reindexes_relative_formula() {
    use crate::{ClipboardCell, ClipboardPayload};

    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "10".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=A1*2".to_owned(),
    });

    // Source range A1:B1, pasted with top-left at A3 (row 2): dr = +2.
    let payload = ClipboardPayload {
        src_row: 0,
        src_col: 0,
        rows: 1,
        cols: 2,
        cells: vec![
            ClipboardCell {
                dr: 0,
                dc: 0,
                raw: "10".to_owned(),
                ..Default::default()
            },
            ClipboardCell {
                dr: 0,
                dc: 1,
                raw: "=A1*2".to_owned(),
                ..Default::default()
            },
        ],
    };
    state.handle_command(TrameCommand::PasteRange {
        target_row: 2,
        target_col: 0,
        payload,
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 3,
        col_start: 0,
        col_end: 1,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!("expected ViewportData");
    };
    let a3 = cells.iter().find(|c| c.row == 2 && c.col == 0).unwrap();
    let b3 = cells.iter().find(|c| c.row == 2 && c.col == 1).unwrap();
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
/// column (`-ui.md` sec.5.F): A1=1, A2=2 dragged to A5 yields 3, 4, 5.
#[test]
fn fill_auto_extrapolates_numeric_series() {
    use crate::{FillMode, GridRange};
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "2".to_owned(),
    });

    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: GridRange {
            row: 0,
            col: 0,
            row_end: 1,
            col_end: 0,
        },
        dest: GridRange {
            row: 0,
            col: 0,
            row_end: 4,
            col_end: 0,
        },
        mode: FillMode::Auto,
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 4,
        col_start: 0,
        col_end: 0,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let at = |r: u32| {
        cells
            .iter()
            .find(|c| c.row == r && c.col == 0)
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
    use crate::{FillMode, GridRange};
    let mut state = TrameState::new();
    // B1 = A1*2; A column holds the multiplicands so the reindex is visible.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "10".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "20".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=A1*2".to_owned(),
    });

    // Fill B1 down to B2: the single source is not a series -> Copy + reindex.
    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: GridRange {
            row: 0,
            col: 1,
            row_end: 0,
            col_end: 1,
        },
        dest: GridRange {
            row: 0,
            col: 1,
            row_end: 1,
            col_end: 1,
        },
        mode: FillMode::Auto,
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 1,
        col_start: 0,
        col_end: 1,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let b2 = cells.iter().find(|c| c.row == 1 && c.col == 1).unwrap();
    assert_eq!(b2.raw, "=A2*2", "relative ref reindexed by the fill offset");
    assert_eq!(b2.text, "40", "reindexed formula recomputes against A2");
}

/// `Series` mode on a single numeric cell increments by 1 (Excel's Ctrl+drag
/// of one number): A1=5 filled to A4 yields 6, 7, 8.
#[test]
fn fill_series_single_cell_increments_by_one() {
    use crate::{FillMode, GridRange};
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "5".to_owned(),
    });

    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: GridRange {
            row: 0,
            col: 0,
            row_end: 0,
            col_end: 0,
        },
        dest: GridRange {
            row: 0,
            col: 0,
            row_end: 3,
            col_end: 0,
        },
        mode: FillMode::Series,
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 3,
        col_start: 0,
        col_end: 0,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let at = |r: u32| {
        cells
            .iter()
            .find(|c| c.row == r && c.col == 0)
            .unwrap()
            .text
            .clone()
    };
    assert_eq!((at(1), at(2), at(3)), ("6".into(), "7".into(), "8".into()));
}

/// `Copy` mode tiles a multi-cell numeric source verbatim (Excel's Ctrl+drag
/// of a selection): A1=1, A2=2 filled to A6 repeats 1, 2, 1, 2 -- no series.
#[test]
fn fill_copy_multi_cell_tiles_verbatim() {
    use crate::{FillMode, GridRange};
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "2".to_owned(),
    });

    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: GridRange {
            row: 0,
            col: 0,
            row_end: 1,
            col_end: 0,
        },
        dest: GridRange {
            row: 0,
            col: 0,
            row_end: 5,
            col_end: 0,
        },
        mode: FillMode::Copy,
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 5,
        col_start: 0,
        col_end: 0,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let at = |r: u32| {
        cells
            .iter()
            .find(|c| c.row == r && c.col == 0)
            .unwrap()
            .text
            .clone()
    };
    assert_eq!(
        (at(2), at(3), at(4), at(5)),
        ("1".into(), "2".into(), "1".into(), "2".into()),
    );
}

/// Autofill of a merged-cell source fills block by block and re-merges every
/// tile (sec.5.F). A 2x1 merge at A1:A2 holding "x", dragged down to A6, yields
/// merged blocks A3:A4 and A5:A6 each carrying "x"; the in-between covered cells
/// stay blank and unmerged.
#[test]
fn fill_merged_source_tiles_and_remerges_blocks() {
    use crate::{FillMode, GridRange};
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "x".to_owned(),
    });
    // Merge A1:A2 (1 col x 2 rows), master = A1.
    state.handle_command(TrameCommand::MergeCells {
        row: 0,
        col: 0,
        row_end: 1,
        col_end: 0,
    });

    // Drag the handle down: source is the whole merge, dest snapped to A6.
    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: GridRange {
            row: 0,
            col: 0,
            row_end: 1,
            col_end: 0,
        },
        dest: GridRange {
            row: 0,
            col: 0,
            row_end: 5,
            col_end: 0,
        },
        mode: FillMode::Auto,
    });

    // Each block's master carries the value and a fresh 2x1 span; the masters
    // (A3, A5) re-merge while the covered rows (A4, A6) stay blank.
    let a3 = read_display(&mut state, 2, 0);
    assert_eq!(a3.merge_span, Some((1, 2)), "A3 master spans 2 rows");
    assert_eq!(a3.text, "x");
    let a5 = read_display(&mut state, 4, 0);
    assert_eq!(a5.merge_span, Some((1, 2)), "A5 master spans 2 rows");
    assert_eq!(a5.text, "x");
    let a4 = read_display(&mut state, 3, 0);
    assert!(a4.is_merged, "A4 is covered by the A3 block");
    assert_eq!(a4.merge_span, None);
    assert_eq!(a4.text, "");
}

/// Ctrl-drag of a merged numeric source grows by +1 per block (Series), so the
/// merge span and styling survive while each tile master increments. Mirrors the
/// Ctrl path the UI resolves to `FillMode::Series` for a one-block source.
#[test]
fn fill_merged_series_increments_per_block() {
    use crate::{FillMode, GridRange};
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "1".to_owned(),
    });
    // Merge A1:A2 (1 col x 2 rows), master = A1 holding the number 1.
    state.handle_command(TrameCommand::MergeCells {
        row: 0,
        col: 0,
        row_end: 1,
        col_end: 0,
    });

    // Ctrl-drag down to A6: Series, so each block master grows by +1.
    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: GridRange {
            row: 0,
            col: 0,
            row_end: 1,
            col_end: 0,
        },
        dest: GridRange {
            row: 0,
            col: 0,
            row_end: 5,
            col_end: 0,
        },
        mode: FillMode::Series,
    });

    let a3 = read_display(&mut state, 2, 0);
    assert_eq!(a3.merge_span, Some((1, 2)), "A3 master spans 2 rows");
    assert_eq!(a3.text, "2", "second block increments to 2");
    let a5 = read_display(&mut state, 4, 0);
    assert_eq!(a5.merge_span, Some((1, 2)), "A5 master spans 2 rows");
    assert_eq!(a5.text, "3", "third block increments to 3");
}

/// Dragging a stack of two equal merged blocks tiles the whole pattern: the
/// fill re-merges every destination block and, under `Auto`, the two numeric
/// masters form a series that keeps progressing across the new blocks.
#[test]
fn fill_multi_merged_source_continues_pattern() {
    use crate::{FillMode, GridRange};
    let mut state = TrameState::new();
    // A1 = 1, A3 = 2; merge A1:A2 and A3:A4 into two 2x1 blocks.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 2,
        col: 0,
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::MergeCells {
        row: 0,
        col: 0,
        row_end: 1,
        col_end: 0,
    });
    state.handle_command(TrameCommand::MergeCells {
        row: 2,
        col: 0,
        row_end: 3,
        col_end: 0,
    });

    // Drag the two-block source (A1:A4) down to A8 -> two more blocks.
    state.handle_command(TrameCommand::FillRange {
        sheet_id: state.active_sheet.clone(),
        src: GridRange {
            row: 0,
            col: 0,
            row_end: 3,
            col_end: 0,
        },
        dest: GridRange {
            row: 0,
            col: 0,
            row_end: 7,
            col_end: 0,
        },
        mode: FillMode::Auto,
    });

    // A5 (block 3) and A7 (block 4) re-merge and continue the 1,2,... series.
    let a5 = read_display(&mut state, 4, 0);
    assert_eq!(a5.merge_span, Some((1, 2)), "A5 master spans 2 rows");
    assert_eq!(a5.text, "3");
    let a7 = read_display(&mut state, 6, 0);
    assert_eq!(a7.merge_span, Some((1, 2)), "A7 master spans 2 rows");
    assert_eq!(a7.text, "4");
    // Covered rows of the new blocks stay blank.
    let a6 = read_display(&mut state, 5, 0);
    assert!(a6.is_merged);
    assert_eq!(a6.text, "");
}

// -- Find & Replace (sec.5.G) ---------------------------------------------

/// Seed a few cells on the active sheet for the find/replace tests.
fn seed_find(state: &mut TrameState) {
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "apple".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "Apple pie".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 2,
        col: 0,
        text: "banana".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 3,
        col: 0,
        text: "APPLE".to_owned(),
    });
}

fn find_result(out: &TrameOutcome) -> (Vec<(u32, u32)>, Option<usize>) {
    out.display
        .iter()
        .find_map(|e| match e {
            TrameEvent::FindResult { matches, active } => Some((matches.clone(), *active)),
            _ => None,
        })
        .expect("a FindResult")
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
        row: 0,
        col: 0,
        anchor_row: 0,
        anchor_col: 0,
    });
    let out = state.handle_command(TrameCommand::FindNext {
        query: "apple".to_owned(),
        match_case: false,
        whole_cell: false,
    });
    let (matches, active) = find_result(&out);
    assert_eq!(matches, vec![(0, 0), (1, 0), (3, 0)]);
    assert_eq!(active, Some(1), "next after A1 is A2");
    // No CRDT delta for a read-only find.
    assert!(out.update.is_none());

    // Active at the last match A4 (3,0): the next wraps back to A1.
    state.handle_command(TrameCommand::SetActiveCell {
        row: 3,
        col: 0,
        anchor_row: 3,
        anchor_col: 0,
    });
    let out = state.handle_command(TrameCommand::FindNext {
        query: "apple".to_owned(),
        match_case: false,
        whole_cell: false,
    });
    assert_eq!(find_result(&out).1, Some(0), "wraps to the first match");
}

/// match_case and whole_cell narrow the match set.
#[test]
fn find_next_honors_case_and_whole_cell() {
    let mut state = TrameState::new();
    seed_find(&mut state);
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 0,
        anchor_col: 0,
    });

    // Case-sensitive "apple": only the exact-case A1 substring matches.
    let out = state.handle_command(TrameCommand::FindNext {
        query: "apple".to_owned(),
        match_case: true,
        whole_cell: false,
    });
    assert_eq!(find_result(&out).0, vec![(0, 0)]);

    // Whole-cell, case-insensitive "apple": A1 and A4 are exactly "apple"
    // (folded); "Apple pie" is not.
    let out = state.handle_command(TrameCommand::FindNext {
        query: "apple".to_owned(),
        match_case: false,
        whole_cell: true,
    });
    assert_eq!(find_result(&out).0, vec![(0, 0), (3, 0)]);
}

fn replaced_count(out: &TrameOutcome) -> u32 {
    out.display
        .iter()
        .find_map(|e| match e {
            TrameEvent::ReplaceDone { replaced } => Some(*replaced),
            _ => None,
        })
        .expect("a ReplaceDone")
}

/// ReplaceAll rewrites the raw text of every match, reports the count, and is
/// a single undo group (one Undo restores all cells).
#[test]
fn replace_all_rewrites_and_undoes_as_one_group() {
    let mut state = TrameState::new();
    seed_find(&mut state);

    let out = state.handle_command(TrameCommand::ReplaceAll {
        query: "apple".to_owned(),
        replacement: "cherry".to_owned(),
        match_case: false,
        whole_cell: false,
        in_formulas: false,
        all_sheets: false,
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

/// A formula-source cell is rewritten only when `in_formulas` is set; matching
/// still tests the displayed value.
#[test]
fn replace_all_gates_formula_cells() {
    let mut state = TrameState::new();
    // A1 = 2, A2 = "=A1" (displays "2"). Replace displayed "2" with "9".
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "=A1".to_owned(),
    });

    // Without in_formulas: the literal A1 is rewritten, the formula A2 is left.
    let out = state.handle_command(TrameCommand::ReplaceAll {
        query: "2".to_owned(),
        replacement: "9".to_owned(),
        match_case: false,
        whole_cell: false,
        in_formulas: false,
        all_sheets: false,
    });
    assert_eq!(replaced_count(&out), 1);
    assert_eq!(read_raw(&mut state, 0, 0), "9");
    assert_eq!(
        read_raw(&mut state, 1, 0),
        "=A1",
        "formula source untouched"
    );

    // With in_formulas: a formula cell matches on its raw text. "=A1"
    // contains "A1" -> rewrite the formula source to "=B2".
    let out = state.handle_command(TrameCommand::ReplaceAll {
        query: "A1".to_owned(),
        replacement: "B2".to_owned(),
        match_case: false,
        whole_cell: false,
        in_formulas: true,
        all_sheets: false,
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
        row: 0,
        col: 0,
        anchor_row: 0,
        anchor_col: 0,
    });
    let out = state.handle_command(TrameCommand::ReplaceOne {
        query: "apple".to_owned(),
        replacement: "cherry".to_owned(),
        match_case: false,
        whole_cell: false,
        in_formulas: false,
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
        row: 2,
        col: 0,
        anchor_row: 2,
        anchor_col: 0,
    });
    let out = state.handle_command(TrameCommand::ReplaceOne {
        query: "apple".to_owned(),
        replacement: "cherry".to_owned(),
        match_case: false,
        whole_cell: false,
        in_formulas: false,
    });
    assert_eq!(replaced_count(&out), 0);
    assert_eq!(find_result(&out).1, Some(1), "advances to A4 after A3");
}

/// `all_sheets` widens the rewrite to every sheet; a single Undo reverts both.
#[test]
fn replace_all_spans_sheets() {
    let mut state = TrameState::new();
    // Sheet1!A1 = "foo".
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "foo".to_owned(),
    });
    // Sheet2!A1 = "foo".
    state.handle_command(TrameCommand::CreateSheet {
        name: "Sheet2".to_owned(),
    });
    let s2 = second_sheet_id(&state);
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "foo".to_owned(),
    });
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: "s0".to_owned(),
    });

    let out = state.handle_command(TrameCommand::ReplaceAll {
        query: "foo".to_owned(),
        replacement: "bar".to_owned(),
        match_case: false,
        whole_cell: false,
        in_formulas: false,
        all_sheets: true,
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
        sheet_id: "s0".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 0), "foo");
}

// -------------------------------------------------------------------------
// CSV interop (Trame interop MVP, `ambre-spec-trame-interop.md` §4)
// -------------------------------------------------------------------------

/// Pull the `ImportComplete` event out of an outcome.
fn import_result(out: &TrameOutcome) -> (String, u32, u32) {
    out.display
        .iter()
        .find_map(|e| match e {
            TrameEvent::ImportComplete {
                sheet_id,
                rows,
                cols,
            } => Some((sheet_id.clone(), *rows, *cols)),
            _ => None,
        })
        .expect("expected ImportComplete")
}

/// Pull the exported file bytes out of an outcome.
fn export_bytes(out: &TrameOutcome) -> Vec<u8> {
    out.display
        .iter()
        .find_map(|e| match e {
            TrameEvent::ExportComplete { data, .. } => Some(data.clone()),
            _ => None,
        })
        .expect("expected ExportComplete")
}

/// A CSV import lands its values into a freshly created sheet, and the
/// reported dimensions match the parsed grid.
#[test]
fn csv_import_creates_sheet_and_cells() {
    let mut state = TrameState::new();
    let csv = b"a,b,c\n1,2,3\n".to_vec();
    let out = state.handle_command(TrameCommand::ImportCsv {
        data: csv,
        delimiter: None,
        has_header: false,
        target_sheet_id: None,
        start_row: 0,
        start_col: 0,
    });
    let (sheet_id, rows, cols) = import_result(&out);
    assert_eq!((rows, cols), (2, 3));
    // The import activates the new sheet, so read_cell reads it.
    assert_eq!(state.active_sheet, sheet_id);
    assert_eq!(read_cell(&mut state, 0, 0), "a");
    assert_eq!(read_cell(&mut state, 1, 2), "3");
}

/// `has_header` skips the first record.
#[test]
fn csv_import_skips_header() {
    let mut state = TrameState::new();
    let out = state.handle_command(TrameCommand::ImportCsv {
        data: b"name,age\nalice,30\n".to_vec(),
        delimiter: None,
        has_header: true,
        target_sheet_id: None,
        start_row: 0,
        start_col: 0,
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
        data: b"x;y;z\n1;2;3\n".to_vec(),
        delimiter: None,
        has_header: false,
        target_sheet_id: None,
        start_row: 0,
        start_col: 0,
    });
    assert_eq!(read_cell(&mut state, 0, 1), "y");
    assert_eq!(read_cell(&mut state, 1, 2), "3");
}

/// Import then export reproduces the same grid (values round-trip).
#[test]
fn csv_import_export_roundtrip() {
    let mut state = TrameState::new();
    let out = state.handle_command(TrameCommand::ImportCsv {
        data: b"a,b\nc,d\n".to_vec(),
        delimiter: None,
        has_header: false,
        target_sheet_id: None,
        start_row: 0,
        start_col: 0,
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

/// Malformed CSV input must never panic the Logic thread (interop §10 DoD):
/// ragged rows, embedded quotes/newlines, mixed delimiters, lone quotes and
/// random bytes all parse to *some* graceful result -- no `unwrap` blow-up.
#[test]
fn csv_import_malformed_never_panics() {
    // Hand-picked adversarial inputs covering the documented failure shapes.
    let cases: &[&[u8]] = &[
        b"",                                                // empty file
        b"\n\n\n",                                          // only blank lines
        b"a,b,c\n1,2\n3\n4,5,6,7,8",                        // ragged rows (varying field counts)
        b"\"unterminated,quote\nnext,row",                  // lone opening quote spanning lines
        b"a,\"b\nc\",d\n",         // embedded newline inside a quoted field
        b"a,\"b\"\"q\"\"\",c\n",   // embedded escaped quotes
        b"x;y,z\tw|v\n1;2,3\t4|5", // mixed delimiters on one line
        b",,,\n,,,\n",             // all-empty fields
        &[0xff, 0xfe, 0x00, 0x01, b'\n', 0x80, b',', 0x7f], // invalid UTF-8 / control bytes
    ];
    for (i, data) in cases.iter().enumerate() {
        let mut state = TrameState::new();
        // Each variant: auto-detect and forced delimiters, with/without header.
        for delimiter in [None, Some(b','), Some(b';')] {
            for has_header in [false, true] {
                // Must return without panicking; the result is unspecified but
                // well-formed (an ImportComplete or an Error event).
                let _ = state.handle_command(TrameCommand::ImportCsv {
                    data: data.to_vec(),
                    delimiter,
                    has_header,
                    target_sheet_id: None,
                    start_row: 0,
                    start_col: 0,
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
/// with broad coverage of byte soup (interop §10 robustness DoD).
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
            data,
            delimiter: None,
            has_header: next() % 2 == 0,
            target_sheet_id: None,
            start_row: 0,
            start_col: 0,
        });
    }
}

/// Export is bounded to the actually-used range: columns materialized by a
/// scroll or a since-cleared edit (here a far cell written then deleted) must
/// not pad the output with trailing empty columns.
#[test]
fn csv_export_trims_empty_trailing_columns() {
    let mut state = TrameState::new();
    // Materialize out to column R (17) by writing then clearing a far cell.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 17,
        text: "x".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 17,
        text: String::new(),
    });
    // Real content only in columns A..C.
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 2,
        text: "3".to_owned(),
    });

    let out = state.handle_command(TrameCommand::ExportActiveSheetCsv);
    let bytes = export_bytes(&out);
    assert_eq!(String::from_utf8(bytes).unwrap(), "1,2,3\n");
}

/// Reproduce the real flow: import a CSV whose rows carry trailing empty
/// columns, then export the active sheet -- the output must NOT carry the
/// padding columns back.
#[test]
fn csv_import_then_export_drops_trailing_empties() {
    let mut state = TrameState::new();
    let csv = b"12,13,14,15,16,17,,,,,,,,,,,,\n12,13,14,15,16,17,,,,,,,,,,,,\n".to_vec();
    let out = state.handle_command(TrameCommand::ImportCsv {
        data: csv,
        delimiter: None,
        has_header: false,
        target_sheet_id: None,
        start_row: 0,
        start_col: 0,
    });
    let _ = import_result(&out);

    let out = state.handle_command(TrameCommand::ExportActiveSheetCsv);
    let text = String::from_utf8(export_bytes(&out)).unwrap();
    assert_eq!(text, "12,13,14,15,16,17\n12,13,14,15,16,17\n");
}

/// The user's real scenario: type 6 columns x 10 rows (A1:F10), then export.
/// The output must be exactly 6 columns wide, no trailing empties.
#[test]
fn csv_export_typed_6x10_has_no_trailing_columns() {
    let mut state = TrameState::new();
    for row in 0..10u32 {
        for col in 0..6u32 {
            state.handle_command(TrameCommand::SetCellText {
                row,
                col,
                text: (14 + col).to_string(),
            });
        }
    }
    let out = state.handle_command(TrameCommand::ExportActiveSheetCsv);
    let text = String::from_utf8(export_bytes(&out)).unwrap();
    for line in text.lines() {
        assert_eq!(
            line, "14,15,16,17,18,19",
            "line had trailing empties: {line:?}"
        );
    }
}

/// Export emits formula results, not the formula source.
#[test]
fn csv_export_evaluates_formulas() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 1,
        text: "=A1*3".to_owned(),
    });
    let out = state.handle_command(TrameCommand::ExportCsv {
        sheet_id: "s0".to_owned(),
        delimiter: None,
        range: None,
    });
    let bytes = export_bytes(&out);
    assert_eq!(String::from_utf8(bytes).unwrap(), "2,6\n");
}

/// Repro: a brand-new document where the author types a value, then autofills a
/// numeric series down. A peer that re-opens the document hydrates from the
/// replayed incremental `SendUpdate` deltas (NOT a full snapshot -- that is what
/// "going back to the home page and reopening" does on the wire). The reopened
/// peer must see the full filled series, not just the seed cell.
#[test]
fn autofill_series_survives_incremental_delta_replay() {
    use ambre_bus::{FillMode, GridRange};

    let mut author = TrameState::new();

    // A1 = 1 (the seed).
    let u1 = author
        .handle_command(TrameCommand::SetCellText {
            row: 0,
            col: 0,
            text: "1".to_owned(),
        })
        .update
        .expect("seed write produces a delta");

    // Autofill A1 down to A4: Auto mode on a single number -> 1,2,3,4.
    let u2 = author
        .handle_command(TrameCommand::FillRange {
            sheet_id: "s0".to_owned(),
            src: GridRange {
                row: 0,
                col: 0,
                row_end: 0,
                col_end: 0,
            },
            dest: GridRange {
                row: 0,
                col: 0,
                row_end: 3,
                col_end: 0,
            },
            mode: FillMode::Series,
        })
        .update
        .expect("fill produces a delta");

    // Author sees the full series locally.
    assert_eq!(read_cell(&mut author, 0, 0), "1");
    assert_eq!(read_cell(&mut author, 1, 0), "2");
    assert_eq!(read_cell(&mut author, 2, 0), "3");
    assert_eq!(read_cell(&mut author, 3, 0), "4");

    // Reopen: a fresh peer replays the broadcast deltas in order.
    let mut reopened = TrameState::new();
    reopened.apply_remote(&u1);
    reopened.apply_remote(&u2);

    assert_eq!(read_cell(&mut reopened, 0, 0), "1", "seed survives reopen");
    assert_eq!(read_cell(&mut reopened, 1, 0), "2", "A2 survives reopen");
    assert_eq!(read_cell(&mut reopened, 2, 0), "3", "A3 survives reopen");
    assert_eq!(read_cell(&mut reopened, 3, 0), "4", "A4 survives reopen");
}

/// Generalized guard against the autofill-class bug: any batched command that
/// formats cells must broadcast a self-contained incremental delta. The
/// convergence proptest exchanges full snapshots, so it cannot catch an
/// incremental delta that drops ops (the failure mode where a per-cell helper
/// flushed mid-batch and discarded the delta). Here a single author formats a
/// 3x3 selection whose rows/columns are NOT yet materialized -- forcing
/// `persist_format` to scaffold lines mid-loop -- then a fresh peer replays only
/// the broadcast delta. Every formatted cell must survive the reopen.
#[test]
fn batched_formatting_broadcasts_complete_delta() {
    let mut author = TrameState::new();

    // Select A1:C3 on an empty sheet: nothing is materialized yet, so the bold
    // pass must create rows 0..2 and columns 0..2 as it goes.
    author.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 2,
        anchor_col: 2,
    });
    let delta = author
        .handle_command(TrameCommand::SetCellStyle {
            patch: bold_patch(true),
        })
        .update
        .expect("formatting a selection produces a delta");

    // Reopen: a fresh peer replays only the broadcast delta.
    let mut reopened = TrameState::new();
    reopened.apply_remote(&delta);

    for row in 0..=2 {
        for col in 0..=2 {
            assert!(
                read_display(&mut reopened, row, col).bold,
                "bold on ({row},{col}) must survive an incremental reopen"
            );
        }
    }
}

/// Repro: edits made AFTER a reopen must broadcast a causally self-contained
/// delta. A reopened session starts from a fresh `LoroDoc` (new random PeerID)
/// whose constructor mints local genesis ops, THEN imports the document history.
/// If the export cursor is advanced past those never-broadcast local ops, a
/// later edit's delta depends on them -- a dangling causal dependency that a
/// third peer (the next reopen) cannot apply, so the edit silently vanishes.
#[test]
fn edit_after_reopen_broadcasts_causally_complete_delta() {
    // Original author creates the doc and writes A1.
    let mut author = TrameState::new();
    let u_a1 = author
        .handle_command(TrameCommand::SetCellText {
            row: 0,
            col: 0,
            text: "1".to_owned(),
        })
        .update
        .expect("A1 write produces a delta");

    // Reopen #1: fresh peer hydrates from the broadcast history, then writes A2.
    let mut reopened = TrameState::new();
    reopened.apply_remote(&u_a1);
    let u_a2 = reopened
        .handle_command(TrameCommand::SetCellText {
            row: 1,
            col: 0,
            text: "1".to_owned(),
        })
        .update
        .expect("A2 write produces a delta");

    // Reopen #2: another fresh peer replays the full broadcast history.
    let mut reopened2 = TrameState::new();
    reopened2.apply_remote(&u_a1);
    reopened2.apply_remote(&u_a2);

    assert_eq!(
        read_cell(&mut reopened2, 0, 0),
        "1",
        "A1 survives reopen #2"
    );
    assert_eq!(
        read_cell(&mut reopened2, 1, 0),
        "1",
        "A2 survives reopen #2"
    );
}

/// A server history replay applied as a batch repaints once: `apply_remote_batch`
/// merges every delta, then emits a single `DataReset` (not one per delta), while
/// reconstructing the full grid. This is the fix for the reopen flicker where a
/// cell present only in the last delta flashed in across N intermediate renders.
#[test]
fn apply_remote_batch_emits_single_datareset_with_full_state() {
    let mut author = TrameState::new();
    let u1 = author
        .handle_command(TrameCommand::SetCellText {
            row: 0,
            col: 0,
            text: "1".to_owned(),
        })
        .update
        .expect("A1 delta");
    let u2 = author
        .handle_command(TrameCommand::SetCellText {
            row: 1,
            col: 0,
            text: "2".to_owned(),
        })
        .update
        .expect("A2 delta");

    let mut reopened = TrameState::new();
    let events = reopened.apply_remote_batch(&[u1, u2]);

    let resets = events
        .iter()
        .filter(|e| matches!(e, TrameEvent::DataReset))
        .count();
    assert_eq!(resets, 1, "the whole replay repaints exactly once");

    assert_eq!(read_cell(&mut reopened, 0, 0), "1");
    assert_eq!(read_cell(&mut reopened, 1, 0), "2");
}
