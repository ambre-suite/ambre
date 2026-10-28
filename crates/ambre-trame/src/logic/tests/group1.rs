// ambre-trame/src/logic/tests/group1.rs
// TrameState unit tests (group 1/7). Helpers via `use super::*`;
// logic-module items are reached with `super::super::` paths.

use super::*;
use crate::logic::schema::{Root, SheetContainer, SheetRoot};

/// Bold a cell, then move the selection elsewhere: the formatting must stay
/// pinned to the cell it was applied to, not follow the cursor. This is the
/// regression the per-cell model fixes.
#[test]
fn bold_stays_on_the_cell_not_the_selection() {
    let mut state = TrameState::new();

    // Select A1 (0,0) and bold it via the single style event.
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(0),
        col: Col::new(0),
        anchor_row: Row::new(0),
        anchor_col: Col::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    let evts = state.handle_command(TrameCommand::SetCellStyle {
        patch: bold_patch(),
    });
    assert!(matches!(
        evts.display[0],
        TrameEvent::StyleApplied {
            bold: ambre_bus::Weight::Bold,
            ..
        }
    ));

    // Move the selection to B2 (1,1): the active cell is not bold, so the
    // toolbar reports false -- A1 keeps its bold.
    let evts = state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(1),
        col: Col::new(1),
        anchor_row: Row::new(1),
        anchor_col: Col::new(1),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    assert!(matches!(
        evts.display[0],
        TrameEvent::StyleApplied {
            bold: ambre_bus::Weight::Regular,
            ..
        }
    ));

    // A viewport covering both cells: only A1 carries the bold run.
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(1),
        col_start: Col::new(0),
        col_end: Col::new(1),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!("expected ViewportData");
    };
    let a1 = cells
        .iter()
        .find(|c| c.row == Row::new(0) && c.col == Col::new(0))
        .unwrap();
    let b2 = cells
        .iter()
        .find(|c| c.row == Row::new(1) && c.col == Col::new(1))
        .unwrap();
    assert_eq!(a1.bold, ambre_bus::Weight::Bold, "A1 must stay bold");
    assert_eq!(
        b2.bold,
        ambre_bus::Weight::Regular,
        "B2 must not become bold"
    );
}

/// Bold and italic survive a snapshot round-trip (the F5 reload path):
/// formatting must be persisted in the CRDT, not only in memory.
#[test]
fn formatting_survives_snapshot_roundtrip() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(0),
        col: Col::new(0),
        anchor_row: Row::new(0),
        anchor_col: Col::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    state.handle_command(TrameCommand::SetCellStyle {
        patch: crate::StylePatch {
            bold: Some(ambre_bus::Weight::Bold),
            italic: Some(ambre_bus::Slant::Italic),
            ..Default::default()
        },
    });

    // Simulate a reload: fresh state restored from the persisted snapshot.
    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored
        .import_snapshot(&snapshot)
        .expect("a snapshot this build wrote reads back at the current schema");

    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(0),
        col_start: Col::new(0),
        col_end: Col::new(0),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!("expected ViewportData");
    };
    let a1 = cells
        .iter()
        .find(|c| c.row == Row::new(0) && c.col == Col::new(0))
        .unwrap();
    assert_eq!(
        a1.bold,
        ambre_bus::Weight::Bold,
        "A1 must stay bold after reload"
    );
    assert_eq!(
        a1.italic,
        ambre_bus::Slant::Italic,
        "A1 must stay italic after reload"
    );
}

/// `SetCellStyle` applies a [`StylePatch`] over the selection and persists it
/// in the CRDT, which survives a snapshot round-trip. It reformats numeric
/// display text per the number format, and reports the active cell's resolved
/// style for the toolbar (`trame-cell-styles`).
#[test]
fn set_cell_style_round_trips_and_formats_numbers() {
    use crate::{CellFormat, HAlign, StylePatch};

    let mut state = TrameState::new();
    // A numeric value at A1, selection collapsed on it.
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "3.14159".to_owned(),
    });
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(0),
        col: Col::new(0),
        anchor_row: Row::new(0),
        anchor_col: Col::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });

    // Apply Percent with 1 decimal + right alignment + underline.
    let out = state.handle_command(TrameCommand::SetCellStyle {
        patch: StylePatch {
            format: Some(CellFormat::Percent),
            decimals: Some(crate::Decimals::new(1)),
            h_align: Some(HAlign::Right),
            underline: Some(ambre_bus::Underline::Underlined),
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
    assert_eq!(style_ev.underline, ambre_bus::Underline::Underlined);

    // Reload from a snapshot: the style must persist in the CRDT.
    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored
        .import_snapshot(&snapshot)
        .expect("a snapshot this build wrote reads back at the current schema");
    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(0),
        col_start: Col::new(0),
        col_end: Col::new(0),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!("expected ViewportData");
    };
    let a1 = cells
        .iter()
        .find(|c| c.row == Row::new(0) && c.col == Col::new(0))
        .unwrap();
    assert_eq!(
        a1.underline,
        ambre_bus::Underline::Underlined,
        "underline must survive reload"
    );
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
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(0),
        col_start: Col::new(0),
        col_end: Col::new(0),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    assert_eq!(cells[0].text, "");

    // Edit A1, then read it back: the stored text wins.
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "hello".to_owned(),
    });
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(2),
        row_start: Row::new(0),
        row_end: Row::new(0),
        col_start: Col::new(0),
        col_end: Col::new(0),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    assert_eq!(cells[0].text, "hello");

    // Clearing it restores the blank cell.
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: String::new(),
    });
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(3),
        row_start: Row::new(0),
        row_end: Row::new(0),
        col_start: Col::new(0),
        col_end: Col::new(0),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    assert_eq!(cells[0].text, "");
}

/// A value written far down the grid is keyed by its row/column stable ids.
/// It survives a snapshot round-trip and resolves at the same grid position
/// in a fresh state. That proves the LID-keyed persistence + `SheetIndex`
/// rebuild.
#[test]
fn far_cell_survives_snapshot_roundtrip() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(1000),
        col: Col::new(3),
        text: "deep".to_owned(),
    });

    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored
        .import_snapshot(&snapshot)
        .expect("a snapshot this build wrote reads back at the current schema");

    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(1000),
        row_end: Row::new(1000),
        col_start: Col::new(3),
        col_end: Col::new(3),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    assert_eq!(cells[0].text, "deep");
    // An untouched far cell is blank (virtual space).
    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(2),
        row_start: Row::new(2000),
        row_end: Row::new(2000),
        col_start: Col::new(0),
        col_end: Col::new(0),
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
        row: Row::new(0),
        col: Col::new(0),
        text: "=21".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=A1*2".to_owned(),
    });

    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored
        .import_snapshot(&snapshot)
        .expect("a snapshot this build wrote reads back at the current schema");

    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(0),
        col_start: Col::new(0),
        col_end: Col::new(1),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let b1 = cells
        .iter()
        .find(|c| c.row == Row::new(0) && c.col == Col::new(1))
        .unwrap();
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
            row: Row::new(r),
            col: Col::new(0),
            text: format!("{}", r + 1),
        });
    }
    // Define TOTAL = A1:A3, then C1 = SUM(TOTAL).
    state.handle_command(TrameCommand::DefineName {
        name: crate::test_support::name("TOTAL"),
        reference: "A1:A3".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(2),
        text: "=SUM(TOTAL)".to_owned(),
    });

    let read_c1 = |state: &mut TrameState, version: u64| {
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: ambre_bus::ViewportVersion(version),
            row_start: Row::new(0),
            row_end: Row::new(0),
            col_start: Col::new(2),
            col_end: Col::new(2),
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
            panic!()
        };
        cells[0].text.clone()
    };

    assert_eq!(read_c1(&mut state, 1), "6", "SUM(TOTAL) = 1+2+3");

    // Editing a target cell recomputes the name's consumer.
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "10".to_owned(),
    });
    assert_eq!(read_c1(&mut state, 2), "15", "SUM(TOTAL) tracks A1 edit");

    // The name survives a snapshot round-trip.
    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored
        .import_snapshot(&snapshot)
        .expect("a snapshot this build wrote reads back at the current schema");
    assert_eq!(
        read_c1(&mut restored, 3),
        "15",
        "name persists across reload"
    );

    // Deleting the name stops it resolving: its consumer shows an error where
    // the total stood, never the stale 15 nor its own source (`TRAME-065`).
    restored.handle_command(TrameCommand::DeleteName {
        name: crate::test_support::name("TOTAL"),
    });
    assert_eq!(read_c1(&mut restored, 4), "#NAME?");

    // Defining it again brings the total back.
    restored.handle_command(TrameCommand::DefineName {
        name: crate::test_support::name("TOTAL"),
        reference: "A1:A3".to_owned(),
    });
    assert_eq!(read_c1(&mut restored, 5), "15");
}

/// A1 of the active sheet, as displayed.
fn read_a1(state: &mut TrameState, version: u64) -> String {
    read_cell_v(state, 0, 0, version)
}

/// The display text of `(row, col)` on the active sheet.
fn read_cell_v(state: &mut TrameState, row: u32, col: u32, version: u64) -> String {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(version),
        row_start: Row::new(row),
        row_end: Row::new(row),
        col_start: Col::new(col),
        col_end: Col::new(col),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    cells[0].text.clone()
}

/// An unqualified definition binds to the sheet it was defined on, so every
/// reader totals the same cells whatever tab it looks at (`TRAME-062`).
#[test]
fn unqualified_name_resolves_the_same_for_every_reader() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Other"),
    });
    let other = second_sheet_id(&state);
    // Sheet1!A1:A2 holds 1 and 2; Other!A1:A2 holds 10 and 20.
    for (sheet, base) in [(default_sheet(), 1), (other.clone(), 10)] {
        state.handle_command(TrameCommand::ActivateSheet { sheet_id: sheet });
        for row in 0..2 {
            state.handle_command(TrameCommand::SetCellText {
                row: Row::new(row),
                col: Col::new(0),
                text: format!("{}", base * (row + 1)),
            });
        }
    }
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: SheetId("s0".to_owned()),
    });
    state.handle_command(TrameCommand::DefineName {
        name: crate::test_support::name("TOTAL"),
        reference: "A1:A2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=SUM(TOTAL)".to_owned(),
    });
    assert_eq!(read_cell_v(&mut state, 0, 1, 1), "3");
    // A formula on the other sheet reads the same name, and the same cells.
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: other.clone(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=SUM(TOTAL)".to_owned(),
    });
    assert_eq!(read_cell_v(&mut state, 0, 1, 2), "3");
}

/// A name reaching another sheet by its display name survives that sheet
/// being renamed (`TRAME-067`).
#[test]
fn name_on_another_sheet_survives_its_rename() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Data"),
    });
    let data = second_sheet_id(&state);
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: data.clone(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "7".to_owned(),
    });
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: SheetId("s0".to_owned()),
    });
    state.handle_command(TrameCommand::DefineName {
        name: crate::test_support::name("RATE"),
        reference: "Data!A1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "=RATE*2".to_owned(),
    });
    assert_eq!(read_a1(&mut state, 1), "14");
    state.handle_command(TrameCommand::RenameSheet {
        sheet_id: data,
        name: crate::test_support::sheet("Inputs"),
    });
    assert_eq!(
        read_a1(&mut state, 2),
        "14",
        "the rename leaves RATE resolving"
    );
}

/// A definition that is not a reference, or names a sheet that does not exist,
/// is refused when it is made, and writes nothing (`TRAME-069`).
#[test]
fn definition_that_is_not_a_reference_is_refused() {
    let mut state = TrameState::new();
    for reference in ["A1+1", "SUM(A1:A3)", "Nowhere!A1", "not a ref", ""] {
        let evts = state.handle_command(TrameCommand::DefineName {
            name: crate::test_support::name("TOTAL"),
            reference: reference.to_owned(),
        });
        assert!(
            matches!(&evts.display[..], [TrameEvent::Error { .. }]),
            "{reference:?} is refused"
        );
        assert!(evts.update.is_none(), "{reference:?} writes nothing");
    }
}

/// A name no formula can read is refused when it is made (`TRAME-155`). A
/// leading digit, a space or an empty text is not a name at all. A cell
/// reference or a boolean has a name's shape, but parses as something else.
#[test]
fn name_a_formula_cannot_read_is_refused() {
    for text in ["1TOTAL", "MY TOTAL", ""] {
        assert!(
            crate::DefinedName::new(text).is_err(),
            "{text:?} is no name"
        );
    }
    let mut state = TrameState::new();
    for name in ["A1", "TRUE"] {
        let evts = state.handle_command(TrameCommand::DefineName {
            name: crate::test_support::name(name),
            reference: "B1:B3".to_owned(),
        });
        assert!(
            matches!(&evts.display[..], [TrameEvent::Error { .. }]),
            "{name:?} is refused"
        );
        assert!(evts.update.is_none(), "{name:?} writes nothing");
    }
    let evts = state.handle_command(TrameCommand::DefineName {
        name: crate::test_support::name("_TOTAL_2"),
        reference: "B1:B3".to_owned(),
    });
    assert!(evts.update.is_some(), "a readable name is stored");
}

/// End-to-end of the reported bug: plain numeric literals typed into A1/B1
/// must feed `=SUM(A1:B1)` (display "3"). The formula cell's `raw` must
/// carry the formula source, so the in-cell editor reopens on the formula,
/// not its result.
#[test]
fn sum_over_plain_literals_and_formula_raw_in_viewport() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(2),
        text: "=SUM(A1:B1)".to_owned(),
    });

    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(0),
        col_start: Col::new(0),
        col_end: Col::new(2),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };

    let a1 = cells
        .iter()
        .find(|c| c.row == Row::new(0) && c.col == Col::new(0))
        .unwrap();
    assert_eq!(a1.text, "1");
    assert_eq!(a1.raw, "1", "plain cell raw == display");

    let c1 = cells
        .iter()
        .find(|c| c.row == Row::new(0) && c.col == Col::new(2))
        .unwrap();
    assert_eq!(c1.text, "3", "SUM must see plain literals A1=1, B1=2");
    assert_eq!(c1.raw, "=SUM(A1:B1)", "editor must reopen on the formula");
}

/// The same scenario after a reseed, the path a remote echo / snapshot
/// restore takes. Plain literals must still feed the formula, so the
/// refreshed viewport shows "3", not the optimistic formula text.
#[test]
fn sum_over_plain_literals_survives_reseed() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "1".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "2".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(2),
        text: "=SUM(A1:B1)".to_owned(),
    });

    let snapshot = state.export_snapshot();
    let mut restored = TrameState::new();
    restored
        .import_snapshot(&snapshot)
        .expect("a snapshot this build wrote reads back at the current schema");

    let evts = restored.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(0),
        col_start: Col::new(0),
        col_end: Col::new(2),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    let c1 = cells
        .iter()
        .find(|c| c.row == Row::new(0) && c.col == Col::new(2))
        .unwrap();
    assert_eq!(c1.text, "3", "SUM must re-evaluate after reseed");
    assert_eq!(c1.raw, "=SUM(A1:B1)");
}

/// Two sheets keep their cells isolated (distinct `cells_{sheet}` containers)
/// and `sheet_name_to_id` resolves the default sheet's name.
#[test]
fn multi_sheet_isolation_and_name_lookup() {
    let mut state = TrameState::new();
    assert_eq!(
        state.sheet_name_to_id(&ambre_bus::SheetName::stored("Sheet1".to_owned())),
        Some(default_sheet())
    );
    assert_eq!(
        state.sheet_name_to_id(&ambre_bus::SheetName::stored("Nope".to_owned())),
        None
    );

    // Write to the default sheet, then to a second sheet's containers
    // directly (the add-sheet command lands in a later step).
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "on_s0".to_owned(),
    });
    let _ = state
        .doc
        .get_map(Root::Sheets.as_str())
        .insert("s1", "Sheet2");
    let other = state
        .doc
        .get_movable_list(SheetContainer::of(SheetRoot::Rows, &SheetId("s1".to_owned())).as_str());
    other
        .push(
            Uid {
                peer: ambre_core::PeerId(1),
                counter: ambre_core::IdCounter::new(0),
            }
            .to_key_part()
            .as_str(),
        )
        .unwrap();
    state.doc.commit();

    // `cells_s1` is empty; `cells_s0` holds the value -- containers are isolated.
    assert!(matches!(
        state.doc.get_map(SheetContainer::of(SheetRoot::Cells, &default_sheet()).as_str()).get_value(),
        loro::LoroValue::Map(ref m) if m.len() == 1
    ));
    assert!(matches!(
        state.doc.get_map(SheetContainer::of(SheetRoot::Cells, &SheetId("s1".to_owned())).as_str()).get_value(),
        loro::LoroValue::Map(ref m) if m.is_empty()
    ));
    assert_eq!(
        state.sheet_name_to_id(&ambre_bus::SheetName::stored("Sheet2".to_owned())),
        Some(SheetId("s1".to_owned()))
    );
}

/// A cross-sheet formula is stored bound to the stable `sheet_id` (`#id`
/// form). A later rename keeps it computing, and its formula-bar text
/// follows the new name. A reorder is inert; deleting the target sheet turns
/// it into `#REF!` (`trame-sheets`).
#[test]
fn cross_sheet_reference_survives_rename_and_breaks_on_delete() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet2"),
    });
    let s2 = second_sheet_id(&state);

    // Sheet2!A1 = 5.
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: s2.clone(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "5".to_owned(),
    });

    // Sheet1!B1 = Sheet2!A1 * 2.
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: SheetId("s0".to_owned()),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=Sheet2!A1*2".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 1), "10");
    // Stored canonical form is id-bound, its cell named by its row and column
    // (`TRAME-228`); the formula bar shows the name and the position.
    let stored = state
        .cell_raw(CellPos::new(Row::new(0), Col::new(1)))
        .into_string();
    assert!(
        stored.starts_with(&format!("=#{s2}!{{")) && stored.ends_with("}*2"),
        "v is sheet_id-bound: {stored}"
    );
    assert_eq!(
        read_raw(&mut state, 0, 1),
        "=Sheet2!A1*2",
        "bar shows display name"
    );

    // Rename the target: the reference keeps computing and the bar follows.
    state.handle_command(TrameCommand::RenameSheet {
        sheet_id: s2.clone(),
        name: crate::test_support::sheet("Data"),
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
        to_index: crate::SheetTabPos::ZERO,
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
/// records one item per command (one commit). A single undo restores the cell
/// to its pre-edit (empty) label, and a single redo brings the value back.
#[test]
fn undo_then_redo_round_trips_a_cell_edit() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
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
            undo: ambre_core::Availability::Disabled,
            redo: ambre_core::Availability::Enabled
        })
    ));
    assert_eq!(read_cell(&mut state, 0, 0), "", "value gone, cell blank");

    let evts = state.handle_command(TrameCommand::Redo);
    assert!(matches!(
        evts.display.last(),
        Some(TrameEvent::UndoState {
            undo: ambre_core::Availability::Enabled,
            redo: ambre_core::Availability::Disabled
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
/// loop is wrapped in one `UndoManager` group (`trame-undo`).
#[test]
fn selection_style_is_a_single_undo() {
    let mut state = TrameState::new();
    // Selection A1:C1 (row 0, cols 0..2).
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(0),
        col: Col::new(2),
        anchor_row: Row::new(0),
        anchor_col: Col::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    state.handle_command(TrameCommand::SetCellStyle {
        patch: bold_patch(),
    });
    // One undo.
    state.handle_command(TrameCommand::Undo);
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        row_end: Row::new(0),
        col_start: Col::new(0),
        col_end: Col::new(2),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!("expected ViewportData");
    };
    for c in 0..=2u32 {
        let cell = cells
            .iter()
            .find(|x| x.row == Row::new(0) && x.col == Col::new(c))
            .unwrap();
        assert_eq!(
            cell.bold,
            ambre_bus::Weight::Regular,
            "col {c} still bold after ONE undo -> style was multi-undo"
        );
    }
}

use ambre_bus::{CellFormat, StylePatch};

/// Format `A1` holding `value` with `format`, the selection on it.
fn formatted_a1(value: &str, format: CellFormat) -> TrameState {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: value.to_owned(),
    });
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(0),
        col: Col::new(0),
        anchor_row: Row::new(0),
        anchor_col: Col::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    state.handle_command(TrameCommand::SetCellStyle {
        patch: StylePatch {
            format: Some(format),
            ..Default::default()
        },
    });
    state
}

/// A number stays a number whatever its format prints, for the grid and for
/// the toolbar alike (`TRAME-UI-093`, `TRAME-UI-094`, `TRAME-186`). A text
/// stays a text.
#[test]
fn a_formatted_number_aligns_as_a_number_everywhere() {
    for format in [
        CellFormat::Currency,
        CellFormat::Percent,
        CellFormat::Fraction,
        CellFormat::Date,
        CellFormat::Custom("0 \"kg\"".to_owned()),
    ] {
        let mut state = formatted_a1("0.5", format.clone());
        let a1 = read_display(&mut state, 0, 0);
        assert!(
            a1.text.parse::<f64>().is_err(),
            "{format:?} prints {:?}, which is not a bare number",
            a1.text
        );
        assert_eq!(
            a1.numeric,
            ambre_bus::CellValueKind::Number,
            "{format:?}: the grid aligns it as a number"
        );
        let TrameEvent::StyleApplied { numeric, .. } = state.style_state_event() else {
            panic!("expected StyleApplied")
        };
        assert_eq!(
            numeric,
            ambre_bus::CellValueKind::Number,
            "{format:?}: the toolbar agrees with the grid"
        );
    }
    let mut state = formatted_a1("label", CellFormat::Currency);
    assert_eq!(
        read_display(&mut state, 0, 0).numeric,
        ambre_bus::CellValueKind::Text
    );
}

/// The decimal buttons step the count a cell carries (`TRAME-105`). The count
/// starts at the family default and stops at zero.
#[test]
fn decimal_step_adds_one_place_to_the_count() {
    let mut state = formatted_a1("3.14159", CellFormat::Number);
    let step = |state: &mut TrameState, by: i8| {
        state.handle_command(TrameCommand::SetCellStyle {
            patch: StylePatch {
                decimals_step: Some(crate::DecimalsStep::new(by)),
                ..Default::default()
            },
        });
        read_display(state, 0, 0).text
    };
    assert_eq!(
        read_display(&mut state, 0, 0).text,
        "3.14",
        "the Number default is 2"
    );
    assert_eq!(step(&mut state, 1), "3.142");
    assert_eq!(step(&mut state, 1), "3.1416");
    assert_eq!(step(&mut state, -1), "3.142");
    for _ in 0..5 {
        step(&mut state, -1);
    }
    assert_eq!(
        read_display(&mut state, 0, 0).text,
        "3",
        "the count stops at zero"
    );
}

/// A date-formatted cell shows a calendar date, never its serial
/// (`TRAME-107`), and a time-formatted one the time of day.
#[test]
fn date_and_time_cells_show_the_calendar() {
    let mut state = formatted_a1("46294.5", CellFormat::Date);
    assert_eq!(read_display(&mut state, 0, 0).text, "2026-09-29");
    let mut state = formatted_a1("46294.5", CellFormat::Time);
    assert_eq!(read_display(&mut state, 0, 0).text, "12:00:00");
}

/// A cell carrying a custom pattern shows it applied (`TRAME-109`).
#[test]
fn custom_pattern_is_applied() {
    let mut state = formatted_a1("1234.5", CellFormat::Custom("#,##0.00 \"EUR\"".to_owned()));
    assert_eq!(read_display(&mut state, 0, 0).text, "1,234.50 EUR");
    // A text under the pattern stays the text (`TRAME-103`).
    let mut state = formatted_a1("n/a", CellFormat::Custom("0.00".to_owned()));
    assert_eq!(read_display(&mut state, 0, 0).text, "n/a");
}

/// Select `(r1, c1)`..`(r2, c2)`, the active cell at `(r1, c1)`.
fn select(state: &mut TrameState, (r1, c1): (u32, u32), (r2, c2): (u32, u32)) {
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(r1),
        col: Col::new(c1),
        anchor_row: Row::new(r2),
        anchor_col: Col::new(c2),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
}

/// With the cursor on a cell a merge covers, the toolbar reports the style of
/// the block the user sees, which its master carries (`TRAME-188`).
#[test]
fn toolbar_reports_the_block_style_inside_a_merge() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "title".to_owned(),
    });
    select(&mut state, (0, 0), (1, 1));
    state.handle_command(TrameCommand::MergeSelection);
    select(&mut state, (0, 0), (0, 0));
    state.handle_command(TrameCommand::SetCellStyle {
        patch: StylePatch {
            bold: Some(ambre_bus::Weight::Bold),
            format: Some(CellFormat::Currency),
            ..Default::default()
        },
    });
    select(&mut state, (1, 1), (1, 1));
    let TrameEvent::StyleApplied {
        style,
        bold,
        merged,
        ..
    } = state.style_state_event()
    else {
        panic!("expected StyleApplied")
    };
    assert_eq!(merged, ambre_bus::Merge::Merged);
    assert_eq!(bold, ambre_bus::Weight::Bold, "the block is bold");
    assert_eq!(style.format, CellFormat::Currency);
}

/// A formatting gesture on a cell past the materialization cap says so, and
/// draws no style the document did not store (`TRAME-193`).
#[test]
fn style_beyond_the_cap_is_refused_and_not_drawn() {
    let mut state = TrameState::new();
    let row = crate::logic::schema::MATERIALIZE_CAP as u32 + 5;
    select(&mut state, (row, 0), (row, 0));
    let out = state.handle_command(TrameCommand::SetCellStyle {
        patch: StylePatch {
            bold: Some(ambre_bus::Weight::Bold),
            ..Default::default()
        },
    });
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, TrameEvent::Error { .. }))
    );
    assert_eq!(
        read_display(&mut state, row, 0).bold,
        ambre_bus::Weight::Regular,
        "nothing drawn"
    );
    let TrameEvent::StyleApplied { bold, .. } = state.style_state_event() else {
        panic!("expected StyleApplied")
    };
    assert_eq!(
        bold,
        ambre_bus::Weight::Regular,
        "the toolbar reports what the document holds"
    );
}
