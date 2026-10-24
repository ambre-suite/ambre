// ambre-trame/src/logic/tests/group12.rs
// TrameState unit tests (group 12): what a sheet computes and how it leaves.
// The function library through the engine, the named ranges, and the CSV and
// native file paths. Helpers via `use super::*`.

use super::*;
use crate::logic::schema::{SheetContainer, SheetRoot};
use loro::LoroValue;

/// A sheet with one plain edit, so an undo has something to take back that
/// is not the gesture under test.
fn with_one_edit() -> TrameState {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "kept".to_owned(),
    });
    state
}

fn import(state: &mut TrameState, data: &[u8], target_sheet_id: Option<SheetId>) -> TrameOutcome {
    state.handle_command(TrameCommand::ImportCsv {
        data: ambre_core::FileBytes::new(data.to_vec()),
        delimiter: None,
        header: HeaderRow::Absent,
        target_sheet_id,
        start_row: Row::new(0),
        start_col: Col::new(0),
    })
}

/// A function the library does not know reads as `#N/A`, never as a blank
/// cell (`TRAME-049`).
#[test]
fn an_unknown_function_reads_as_an_error() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["1", "2"]);
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=SUMM(A1:A2)".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 1), "#N/A");
}

/// A function name is matched whatever its case: `sum` is `SUM` (`TRAME-196`).
#[test]
fn a_function_name_is_read_whatever_its_case() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["1", "2", "4"]);
    for (col, text) in [(1, "=sum(A1:A3)"), (2, "=Sum(A1:A3)"), (3, "=SUM(A1:A3)")] {
        state.handle_command(TrameCommand::SetCellText {
            row: Row::new(0),
            col: Col::new(col),
            text: text.to_owned(),
        });
        assert_eq!(read_cell(&mut state, 0, col), "7", "{text}");
    }
}

/// A recomputation runs on the clock the engine reads for it: `TODAY()`
/// answers the current day, not the 1899-12-30 a missing clock would give
/// (`TRAME-198`).
#[test]
fn today_reads_the_clock_of_the_recomputation() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "=TODAY()".to_owned(),
    });
    let shown: f64 = read_cell(&mut state, 0, 0).parse().expect("a serial");
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs_f64();
    let today = (secs / 86_400.0 + 25_569.0).floor();
    assert!(
        (shown - today).abs() <= 1.0,
        "{shown} is not today ({today})"
    );
}

/// A paste or a fill moves the cell references of a formula and leaves its
/// names alone (`TRAME-219`).
#[test]
fn a_paste_and_a_fill_leave_a_name_in_place() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["1", "2", "3"]);
    state.handle_command(TrameCommand::DefineName {
        name: crate::test_support::name("TOTAL"),
        reference: "A1:A3".to_owned(),
    });
    let formula = |raw: &str| crate::ClipboardCell {
        dr: RowCount::new(0),
        dc: ColCount::new(0),
        raw: raw.to_owned(),
        bold: false,
        italic: false,
        style: crate::CellStyle::default(),
    };
    state.handle_command(TrameCommand::PasteRange {
        target_row: Row::new(2),
        target_col: Col::new(1),
        payload: crate::ClipboardPayload {
            src_row: Row::new(0),
            src_col: Col::new(0),
            rows: RowCount::new(1),
            cols: ColCount::new(1),
            cells: vec![formula("=SUM(TOTAL)+A1")],
        },
    });
    assert_eq!(read_raw(&mut state, 2, 1), "=SUM(TOTAL)+B3");
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(2),
        text: "=SUM(TOTAL)*A1".to_owned(),
    });
    state.handle_command(TrameCommand::FillRange {
        sheet_id: crate::logic::default_sheet(),
        src: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(2)),
            crate::CellPos::new(Row::new(0), Col::new(2)),
        ),
        dest: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(2)),
            crate::CellPos::new(Row::new(1), Col::new(2)),
        ),
        mode: crate::FillMode::Auto,
    });
    assert_eq!(read_raw(&mut state, 1, 2), "=SUM(TOTAL)*A2");
    assert_eq!(read_cell(&mut state, 1, 2), "12");
}

/// Removing a name nobody defined writes nothing and says nothing
/// (`TRAME-156`).
#[test]
fn removing_an_unknown_name_changes_nothing() {
    let mut state = with_one_edit();
    let before = state.doc.len_ops();
    let out = state.handle_command(TrameCommand::DeleteName {
        name: crate::test_support::name("NOWHERE"),
    });
    assert!(out.display.is_empty() && out.update.is_none());
    assert_eq!(state.doc.len_ops(), before);
}

/// An import of many cells is one undo item: one undo takes the sheet and
/// every cell back, and leaves the edit before it (`TRAME-IO-025`).
#[test]
fn an_import_is_one_undo_item() {
    let mut state = with_one_edit();
    let csv: String = (0..40).map(|r| format!("{r},a{r},b{r},c{r}\n")).collect();
    let (sheet_id, rows, cols) = import_result(&import(&mut state, csv.as_bytes(), None));
    assert_eq!((rows, cols), (40, 4));
    assert_eq!(state.sheet_order_ids().len(), 2);
    state.handle_command(TrameCommand::Undo);
    assert!(
        !state.sheet_order_ids().contains(&sheet_id),
        "the sheet left"
    );
    assert_eq!(
        state.active_sheet,
        crate::logic::default_sheet(),
        "the reader left it too"
    );
    assert_eq!(read_cell(&mut state, 0, 0), "kept", "the edit before stays");
}

/// A file with no record writes nothing: no sheet, no delta, no undo item.
/// The undo that follows takes back the edit before it (`TRAME-IO-028`).
#[test]
fn a_file_with_no_record_leaves_the_document_alone() {
    for data in [&b""[..], b"\n\n", b"name,age\n"] {
        let mut state = with_one_edit();
        let before = state.doc.len_ops();
        let out = state.handle_command(TrameCommand::ImportCsv {
            data: ambre_core::FileBytes::new(data.to_vec()),
            delimiter: None,
            header: HeaderRow::Present,
            target_sheet_id: None,
            start_row: Row::new(0),
            start_col: Col::new(0),
        });
        assert!(out.update.is_none(), "{data:?}");
        assert_eq!(state.sheet_order_ids().len(), 1, "{data:?}");
        assert_eq!(state.doc.len_ops(), before, "{data:?}");
        state.handle_command(TrameCommand::Undo);
        assert_eq!(read_cell(&mut state, 0, 0), "", "{data:?}");
    }
}

/// An empty field writes nothing: a second import into the same sheet leaves
/// the value under its hole (`TRAME-IO-038`).
#[test]
fn an_empty_field_keeps_the_cell_under_it() {
    let mut state = TrameState::new();
    let (sheet_id, _, _) = import_result(&import(&mut state, b"1,2,3\n", None));
    import_result(&import(&mut state, b"7,,9\n", Some(sheet_id)));
    assert_eq!(read_col(&mut state, 2, 0, 70), vec!["9"]);
    assert_eq!(read_cell(&mut state, 0, 0), "7");
    assert_eq!(
        read_cell(&mut state, 0, 1),
        "2",
        "the hole kept the old value"
    );
}

/// The three separators detection knows, and one it does not. A separator
/// inside a quoted field does not count. A pipe lands in columns only when
/// the caller names it (`TRAME-IO-035`).
#[test]
fn a_separator_outside_the_three_is_named_by_the_caller() {
    let mut state = TrameState::new();
    import_result(&import(&mut state, b"a\tb\tc\n1\t2\t3\n", None));
    assert_eq!(read_cell(&mut state, 1, 2), "3", "a tab is detected");
    // Commas inside the quotes outnumber the semicolons outside them.
    import_result(&import(&mut state, b"\"x,y,z,w\";b\n1;2\n", None));
    assert_eq!(read_cell(&mut state, 0, 0), "x,y,z,w");
    assert_eq!(read_cell(&mut state, 1, 1), "2");
    // A pipe is not guessed: the line lands whole.
    import_result(&import(&mut state, b"a|b\n", None));
    assert_eq!(read_cell(&mut state, 0, 0), "a|b");
    let out = state.handle_command(TrameCommand::ImportCsv {
        data: ambre_core::FileBytes::new(b"a|b\n".to_vec()),
        delimiter: crate::CsvDelimiter::new(b'|').ok(),
        header: HeaderRow::Absent,
        target_sheet_id: None,
        start_row: Row::new(0),
        start_col: Col::new(0),
    });
    import_result(&out);
    assert_eq!(read_cell(&mut state, 0, 1), "b");
}

/// A CSV export writes no delta and adds no operation (`TRAME-IO-026`).
#[test]
fn a_csv_export_writes_nothing() {
    let mut state = with_one_edit();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "=LEN(A1)".to_owned(),
    });
    let before = state.doc.len_ops();
    let version = state.doc.oplog_vv();
    for cmd in [
        TrameCommand::ExportActiveSheetCsv,
        TrameCommand::ExportCsv {
            sheet_id: crate::logic::default_sheet(),
            delimiter: Some(crate::CsvDelimiter::SEMICOLON),
            range: None,
        },
    ] {
        let out = state.handle_command(cmd);
        assert!(!export_bytes(&out).is_empty());
        assert!(out.update.is_none() && out.ephemeral.is_none());
    }
    assert_eq!(state.doc.len_ops(), before);
    assert_eq!(state.doc.oplog_vv(), version);
}

/// A native save reads the document and leaves it as it was (`TRAME-IO-043`).
#[test]
fn a_save_leaves_the_document_alone() {
    let state = with_one_edit();
    let before = state.doc.len_ops();
    let version = state.doc.oplog_vv();
    let snapshot = state.export_snapshot();
    let file = state.export_ambre_plaintext(ambre_core::DocumentId::from_bytes([7; 16]));
    assert!(file.is_some_and(|f| !f.is_empty()));
    assert_eq!(state.doc.len_ops(), before);
    assert_eq!(state.doc.oplog_vv(), version);
    assert_eq!(state.export_snapshot(), snapshot);
}

/// A computed value stays out of the document: the cell holds its raw text,
/// and the result appears nowhere in the CRDT (`TRAME-010`).
#[test]
fn a_computed_value_stays_out_of_the_document() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "=1111*2+2020".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(1),
        text: "=A1&\"x\"".to_owned(),
    });
    assert_eq!(read_cell(&mut state, 0, 0), "4242");
    assert_eq!(read_cell(&mut state, 0, 1), "4242x");
    let LoroValue::Map(cells) = state
        .doc
        .get_map(SheetContainer::of(SheetRoot::Cells, &crate::logic::default_sheet()).as_str())
        .get_deep_value()
    else {
        panic!("the cells map");
    };
    let mut raw: Vec<String> = cells
        .values()
        .map(|v| match v {
            LoroValue::String(s) => s.to_string(),
            other => panic!("a cell holds more than its text: {other:?}"),
        })
        .collect();
    raw.sort();
    // The reference is stored by identity (`TRAME-226`), so only its tail is
    // the typed text.
    assert_eq!(raw[0], "=1111*2+2020");
    assert!(
        raw[1].starts_with("={") && raw[1].ends_with("}&\"x\""),
        "{raw:?}"
    );
    let whole = format!("{:?}", state.doc.get_deep_value());
    assert!(
        !whole.contains("4242"),
        "a result reached the CRDT: {whole}"
    );
}
