// ambre-trame/src/logic/tests/group1.rs
// TrameState unit tests (group 1/7). Helpers via `use super::*`;
// logic-module items are reached with `super::super::` paths.

use super::*;

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
    let _ = state
        .doc
        .get_map(super::super::SHEETS_MAP)
        .insert("s1", "Sheet2");
    let other = state
        .doc
        .get_movable_list(super::super::rows_list_name("s1"));
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
        state.doc.get_map(super::super::cells_map_name("s0")).get_value(),
        loro::LoroValue::Map(ref m) if m.len() == 1
    ));
    assert!(matches!(
        state.doc.get_map(super::super::cells_map_name("s1")).get_value(),
        loro::LoroValue::Map(ref m) if m.is_empty()
    ));
    assert_eq!(state.sheet_name_to_id("Sheet2").as_deref(), Some("s1"));
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

/// A `SetCellStyle` over a multi-cell selection is a single undo step: one Undo
/// must clear the whole selection, not just the last cell. `persist_cell_style`
/// commits per cell and Loro records one undo item per commit, so the selection
/// loop is wrapped in one `UndoManager` group (`-ui.md` §6.C, `-logic.md` §8).
#[test]
fn selection_style_is_a_single_undo() {
    let mut state = TrameState::new();
    // Selection A1:C1 (row 0, cols 0..2).
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 2,
        anchor_row: 0,
        anchor_col: 0,
    });
    state.handle_command(TrameCommand::SetCellStyle {
        patch: bold_patch(true),
    });
    // One undo.
    state.handle_command(TrameCommand::Undo);
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 1,
        row_start: 0,
        row_end: 0,
        col_start: 0,
        col_end: 2,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!("expected ViewportData");
    };
    for c in 0..=2u32 {
        let cell = cells.iter().find(|x| x.row == 0 && x.col == c).unwrap();
        assert!(
            !cell.bold,
            "col {c} still bold after ONE undo -> style was multi-undo"
        );
    }
}
