// ambre-trame/src/logic/tests/group12.rs
// TrameState unit tests (group 12): what a sheet computes and how it leaves.
// The function library through the engine, the named ranges, and the CSV and
// native file paths. Helpers via `use super::*`.

use super::*;
use crate::formula::types::CellValue;
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
        data: ambre_core::Payload::<ambre_core::FileBytes>::new(data.to_vec()),
        delimiter: None,
        header: HeaderRow::Absent,
        target_sheet_id,
        start_row: Row::new(0),
        start_col: Col::new(0),
    })
}

/// Text that Rust's number parse reads, but as a number that is not finite.
const NOT_FINITE: [&str; 8] = [
    "NaN",
    "nan",
    "inf",
    "-INF",
    "Infinity",
    "+infinity",
    "1e999",
    "-1e999",
];

/// What `ISNUMBER` answers for each of the first `rows` cells of column A,
/// read through formulas in column C.
fn numbers_in_col_a(state: &mut TrameState, rows: u32) -> Vec<String> {
    (0..rows)
        .map(|r| {
            state.handle_command(TrameCommand::SetCellText {
                row: Row::new(r),
                col: Col::new(2),
                text: format!("=ISNUMBER(A{})", r + 1),
            });
            read_cell(state, r, 2)
        })
        .collect()
}

/// The answer of [`numbers_in_col_a`] when no cell holds a number.
fn no_numbers(rows: usize) -> Vec<String> {
    vec![CellValue::Boolean(false).to_display(); rows]
}

/// A typed spelling of a number that is not finite stays text, as in Excel
/// (`TRAME-245`).
#[test]
fn a_typed_number_that_is_not_finite_stays_text() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &NOT_FINITE);
    assert_eq!(numbers_in_col_a(&mut state, 8), no_numbers(8));
    assert_eq!(read_cell(&mut state, 2, 0), "inf");
}

/// A pasted spelling of a number that is not finite stays text (`TRAME-245`).
#[test]
fn a_pasted_number_that_is_not_finite_stays_text() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::PasteText {
        text: NOT_FINITE.join("\n"),
    });
    assert_eq!(read_cell(&mut state, 7, 0), "-1e999");
    assert_eq!(numbers_in_col_a(&mut state, 8), no_numbers(8));
}

/// An imported spelling of a number that is not finite stays text
/// (`TRAME-245`).
#[test]
fn an_imported_number_that_is_not_finite_stays_text() {
    let mut state = TrameState::new();
    import(&mut state, NOT_FINITE.join("\n").as_bytes(), None);
    assert_eq!(read_cell(&mut state, 7, 0), "-1e999");
    assert_eq!(numbers_in_col_a(&mut state, 8), no_numbers(8));
}

/// A fill copies a spelling of a number that is not finite as text
/// (`TRAME-245`).
#[test]
fn a_filled_number_that_is_not_finite_stays_text() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["NaN", "1e308", "1.7e308"]);
    let cell = |row| crate::CellPos::new(Row::new(row), Col::new(0));
    let sheet_id = state.active_sheet.clone();
    state.handle_command(TrameCommand::FillRange {
        sheet_id: sheet_id.clone(),
        src: crate::CellRect::spanning(cell(0), cell(0)),
        dest: crate::CellRect::spanning(cell(0), cell(1)),
        mode: crate::FillMode::Copy,
    });
    assert_eq!(read_cell(&mut state, 1, 0), "NaN");
    assert_eq!(numbers_in_col_a(&mut state, 2), no_numbers(2));
}

/// A series stops at its last finite value: the cells past it keep what
/// they held, and the fill says so. The series wrote the text `inf` there
/// before (`TRAME-UI-180`).
#[test]
fn a_series_past_the_largest_number_stops_there() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["1e308", "1.7e308", "", "kept"]);
    let cell = |row| crate::CellPos::new(Row::new(row), Col::new(0));
    let sheet_id = state.active_sheet.clone();
    let out = state.handle_command(TrameCommand::FillRange {
        sheet_id,
        src: crate::CellRect::spanning(cell(0), cell(1)),
        dest: crate::CellRect::spanning(cell(0), cell(3)),
        mode: crate::FillMode::Series,
    });
    assert_eq!(read_cell(&mut state, 2, 0), "");
    assert_eq!(read_cell(&mut state, 3, 0), "kept");
    assert!(
        out.display.iter().any(
            |e| matches!(e, TrameEvent::Error { message } if message.contains("largest number"))
        ),
        "the fill says where it stopped"
    );
}

/// A series from one number steps by one, even below the spacing of the
/// floats. So 1e16 two terms on is 1e16 + 2. A step taken as
/// `(start + 1) - start` read zero there, and the number repeated.
#[test]
fn a_series_from_one_large_number_keeps_its_step() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["10000000000000000"]);
    let cell = |row| crate::CellPos::new(Row::new(row), Col::new(0));
    let sheet_id = state.active_sheet.clone();
    state.handle_command(TrameCommand::FillRange {
        sheet_id,
        src: crate::CellRect::spanning(cell(0), cell(0)),
        dest: crate::CellRect::spanning(cell(0), cell(2)),
        mode: crate::FillMode::Series,
    });
    assert_eq!(read_cell(&mut state, 2, 0), "10000000000000002");
}

/// A step past the largest number passes it at the first filled cell. The
/// series stops there and says so, and no cell reads `inf` (`TRAME-UI-180`).
#[test]
fn a_step_past_the_largest_number_stops_at_once() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["-1.7e308", "1.7e308", "kept"]);
    let cell = |row| crate::CellPos::new(Row::new(row), Col::new(0));
    let sheet_id = state.active_sheet.clone();
    let out = state.handle_command(TrameCommand::FillRange {
        sheet_id,
        src: crate::CellRect::spanning(cell(0), cell(1)),
        dest: crate::CellRect::spanning(cell(0), cell(2)),
        mode: crate::FillMode::Series,
    });
    assert_eq!(read_cell(&mut state, 2, 0), "kept");
    assert!(
        out.display.iter().any(
            |e| matches!(e, TrameEvent::Error { message } if message.contains("largest number"))
        ),
        "the fill says where it stopped"
    );
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
        bold: ambre_bus::Weight::Regular,
        italic: ambre_bus::Slant::Upright,
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

/// An import into a new sheet, with the sheet counter spent, says so and
/// writes nothing: no sheet, no cell, no undo item. The undo that follows
/// takes back the edit before it (`TRAME-243`).
#[test]
fn an_import_with_a_spent_sheet_counter_writes_nothing() {
    let mut state = with_one_edit();
    state.sheet_counter = None;
    let undo_before = state.undo_manager.undo_count();
    let vv_before = state.doc.oplog_vv();
    let out = import(&mut state, b"1,2\n3,4\n", None);
    assert!(
        out.display.iter().any(
            |e| matches!(e, TrameEvent::Error { message } if message.contains("no fresh sheet id"))
        ),
        "the refusal says so"
    );
    assert!(out.update.is_none());
    state.doc.commit();
    assert_eq!(state.doc.oplog_vv(), vv_before, "nothing left pending");
    assert_eq!(state.undo_manager.undo_count(), undo_before);
    assert_eq!(state.sheet_order_ids().len(), 1);
    assert_eq!(state.active_sheet, crate::logic::default_sheet());
    state.handle_command(TrameCommand::Undo);
    assert_eq!(read_cell(&mut state, 0, 0), "", "the edit before is undone");
}

/// A workbook whose cell map holds `key`, a key no build writes, opened.
fn open_with_cell_key(key: &str) -> Vec<TrameEvent> {
    let source = TrameState::new();
    source
        .doc
        .get_map(SheetContainer::of(SheetRoot::Cells, &crate::logic::default_sheet()).as_str())
        .insert(key, "shadow")
        .expect("a crafted key");
    source.doc.commit();
    let mut opened = TrameState::new();
    opened
        .import_snapshot(&source.export_snapshot())
        .expect("the workbook still opens")
}

/// A cell key in a spelling no build writes is refused, and the refusal is
/// said. A crafted peer's short, signed or upper-case uid never reads in
/// silence (`TRAME-248`).
#[test]
fn a_cell_key_in_a_non_canonical_spelling_is_refused_and_said() {
    let canonical = "00000000000000ab_00000002";
    for key in [
        "ab_2:ab_3".to_owned(),
        format!("+{canonical}:{canonical}"),
        format!("{}:{canonical}", canonical.to_uppercase()),
    ] {
        let events = open_with_cell_key(&key);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, TrameEvent::UndecodableRecords)),
            "{key}"
        );
    }
}

/// A non-canonical key in each map keyed by a cell or a line is refused and
/// said. The style, merge, width, height and filter readers each meet one
/// (`TRAME-248`).
#[test]
fn a_non_canonical_key_in_every_keyed_map_is_said() {
    let canonical = "00000000000000ab_00000002";
    let cell_key = format!("{}:{canonical}", canonical.to_uppercase());
    let line_key = canonical.to_uppercase();
    let crafted: [(SheetRoot, &str, LoroValue); 5] = [
        (SheetRoot::CellStyle, &cell_key, LoroValue::from(vec![0u8])),
        (
            SheetRoot::Merges,
            &cell_key,
            LoroValue::from(format!("{canonical}:{canonical}")),
        ),
        (SheetRoot::ColWidths, &line_key, LoroValue::from(80.0)),
        (SheetRoot::RowHeights, &line_key, LoroValue::from(20.0)),
        (SheetRoot::FilterHidden, &line_key, LoroValue::from(true)),
    ];
    for (root, key, value) in crafted {
        let source = TrameState::new();
        source
            .doc
            .get_map(SheetContainer::of(root, &crate::logic::default_sheet()).as_str())
            .insert(key, value)
            .expect("a crafted key");
        source.doc.commit();
        let mut opened = TrameState::new();
        let events = opened
            .import_snapshot(&source.export_snapshot())
            .expect("the workbook still opens");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, TrameEvent::UndecodableRecords)),
            "{root:?}"
        );
    }
}

/// A line element in a spelling no build writes is skipped, and said.
#[test]
fn a_line_element_in_a_non_canonical_spelling_is_refused_and_said() {
    let source = TrameState::new();
    source
        .doc
        .get_movable_list(
            SheetContainer::of(SheetRoot::Rows, &crate::logic::default_sheet()).as_str(),
        )
        .push("ab_2")
        .expect("a crafted line");
    source.doc.commit();
    let mut opened = TrameState::new();
    let events = opened
        .import_snapshot(&source.export_snapshot())
        .expect("the workbook still opens");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TrameEvent::UndecodableRecords))
    );
    assert_eq!(
        opened.index.row_len(),
        RowCount::ZERO,
        "the line is skipped"
    );
}

/// A file with no record writes nothing: no sheet, no delta, no undo item.
/// The undo that follows takes back the edit before it (`TRAME-IO-028`).
#[test]
fn a_file_with_no_record_leaves_the_document_alone() {
    for data in [&b""[..], b"\n\n", b"name,age\n"] {
        let mut state = with_one_edit();
        let before = state.doc.len_ops();
        let out = state.handle_command(TrameCommand::ImportCsv {
            data: ambre_core::Payload::<ambre_core::FileBytes>::new(data.to_vec()),
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
        data: ambre_core::Payload::<ambre_core::FileBytes>::new(b"a|b\n".to_vec()),
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
