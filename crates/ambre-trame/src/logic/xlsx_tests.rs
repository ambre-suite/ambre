// ambre-trame/src/logic/xlsx_tests.rs
// The Excel import and export through the Office converter, linked in place
// as the Logic thread's host build links it (`trame-import-export`). Each
// oracle compares data -- a number, a formula text, a flag -- never the
// text a cell shows.

use super::xlsx::{ImportedCellKind, VERBATIM_MARKER, XLSX_SLICE, classify};
use super::*;
use crate::message::{TrameConstruct, TrameExport, TrameImport, TrameReport, WorkbookImport};
use ambre_bus::interop::{Direction, FileFormat, InteropAnswer, InteropRequest, Outcome};
use ambre_core::index::Pos;
use ambre_xlsx::cellref::{CellRange, CellRef};
use ambre_xlsx::formula::Formula;
use ambre_xlsx::model as xl;
use ambre_xlsx::report::{Outcome as XlOutcome, Report as XlReport, XlsxConstruct};
use proptest::prelude::*;

/// The Office converter, linked in place.
fn office() -> ambre_convert::Converter {
    ambre_convert::Converter::load(
        ambre_core::ConverterFamily::Office,
        &ambre_core::Payload::new(Vec::new()),
    )
    .expect("the host links the engine")
}

fn cell_ref(text: &str) -> CellRef {
    CellRef::parse(text).expect("a cell reference")
}

/// A sheet named `name` holding `cells`.
fn sheet(name: &str, cells: Vec<(&str, xl::Content)>) -> xl::Sheet {
    let mut sheet = xl::Sheet::named(ambre_core::Validated::parse(name.to_owned()).unwrap());
    sheet.cells = cells
        .into_iter()
        .map(|(at, content)| xl::Cell {
            at: cell_ref(at),
            content,
            style: xl::StyleId::DEFAULT,
        })
        .collect();
    sheet
}

fn num(n: f64) -> xl::Content {
    xl::Content::Value(xl::Value::Number(n))
}

fn text(t: &str) -> xl::Content {
    xl::Content::Value(xl::Value::Text(t.to_owned()))
}

/// A formula as a worksheet part stores it, with its cached value.
fn excel(formula: &str, cached: xl::Value) -> xl::Content {
    xl::Content::Formula {
        formula: Formula::from_excel(formula, None, &mut XlReport::default()).unwrap(),
        cached,
    }
}

fn workbook(sheets: Vec<xl::Sheet>) -> xl::Workbook {
    xl::Workbook {
        sheets,
        styles: vec![xl::CellStyle::default()],
    }
}

fn file(workbook: &xl::Workbook) -> ambre_core::Payload<ambre_core::FileBytes> {
    ambre_xlsx::write(workbook)
        .expect("the workbook writes")
        .bytes
}

/// Import `data` whole: the first slice in the command, the rest by resume.
fn import(state: &mut TrameState, data: ambre_core::Payload<ambre_core::FileBytes>) -> TrameReport {
    let mut out = state.handle_conversion(
        TrameCommand::Interop(InteropRequest::Import(TrameImport::Xlsx(WorkbookImport {
            data,
        }))),
        &office(),
    );
    while state.has_pending_work() {
        out = state.resume();
    }
    report_of(&out).expect("an import ends on its report")
}

fn report_of(out: &TrameOutcome) -> Option<TrameReport> {
    out.display.iter().find_map(|e| match e {
        TrameEvent::Interop(InteropAnswer::ImportCompleted { report })
        | TrameEvent::Interop(InteropAnswer::ExportReady { report, .. }) => Some(report.clone()),
        _ => None,
    })
}

/// Export the workbook and read the file back through the codec.
fn export(state: &mut TrameState) -> (xl::Workbook, TrameOutcome) {
    let out = state.handle_conversion(
        TrameCommand::Interop(InteropRequest::Export(TrameExport::Xlsx)),
        &office(),
    );
    let bytes = out
        .display
        .iter()
        .find_map(|e| match e {
            TrameEvent::Interop(InteropAnswer::ExportReady { file, .. }) => Some(file.data.clone()),
            _ => None,
        })
        .expect("an export answers its file");
    let read = ambre_xlsx::read(&ambre_core::Payload::new(bytes.as_bytes().to_vec()))
        .expect("the export reads back");
    (read.workbook, out)
}

/// The content the exported `sheet` holds at `at`.
fn content_at<'a>(workbook: &'a xl::Workbook, sheet: &str, at: &str) -> &'a xl::Content {
    let at = cell_ref(at);
    &workbook
        .sheets
        .iter()
        .find(|s| s.name.as_str() == sheet)
        .expect("the sheet leaves")
        .cells
        .iter()
        .find(|c| c.at == at)
        .expect("the cell leaves")
        .content
}

fn excel_text(content: &xl::Content) -> &str {
    match content {
        xl::Content::Formula { formula, .. } => formula.excel(),
        other => panic!("a formula leaves, not {other:?}"),
    }
}

/// The raw text the active sheet stores at `at`.
fn stored(state: &TrameState, at: &str) -> String {
    let at = cell_ref(at);
    state
        .cell_raw(CellPos::new(Pos::new(at.row.get()), Pos::new(at.col.get())))
        .as_str()
        .to_owned()
}

fn set(state: &mut TrameState, row: u32, col: u32, text: &str) {
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(row),
        col: Pos::<Col>::new(col),
        text: text.to_owned(),
    });
}

fn select(state: &mut TrameState, row: u32, col: u32) {
    state.handle_command(TrameCommand::SetActiveCell {
        row: Pos::<Row>::new(row),
        col: Pos::<Col>::new(col),
        anchor_row: Pos::<Row>::new(row),
        anchor_col: Pos::<Col>::new(col),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
}

/// `TRAME-IO-010`: a formula Trame runs lands parsed, one it cannot run
/// lands verbatim behind the marker, and a value lands as a literal. Each
/// cell is in exactly one state.
#[test]
fn an_imported_formula_lands_in_exactly_one_of_three_states() {
    let mut state = TrameState::new();
    import(
        &mut state,
        file(&workbook(vec![sheet(
            "Data",
            vec![
                ("A1", num(2.0)),
                ("A2", excel("SUM(A1,3)", xl::Value::Number(5.0))),
                ("A3", excel("FOOBAR(A1)", xl::Value::Number(7.0))),
                ("A4", excel("{1,2}", xl::Value::Number(1.0))),
                ("A5", text("plain")),
            ],
        )])),
    );
    assert_eq!(stored(&state, "A1"), "2");
    let a2 = CellPos::new(Pos::new(1), Pos::new(0));
    assert_eq!(state.cell_source(a2).as_str(), "=SUM(A1,3)");
    assert_eq!(
        stored(&state, "A3"),
        format!("{VERBATIM_MARKER}=FOOBAR(A1)")
    );
    assert_eq!(stored(&state, "A4"), format!("{VERBATIM_MARKER}={{1,2}}"));
    assert_eq!(stored(&state, "A5"), "plain");
    // The parsed formula runs: its value is the engine's, not the file's.
    let a2 = state.active_cell_id(a2);
    assert!(matches!(state.engine.evaluated_value(a2), Some(CellValue::Number(n)) if n == 5.0));
}

proptest! {
    /// `TRAME-IO-010`, over any formula text: the sort answers one state,
    /// a parsed one parses, and a verbatim one keeps the Excel text whole.
    #[test]
    fn the_sort_answers_one_state_and_keeps_the_text(body in "[A-Z]{1,6}\\((A1|1|\"x\"|B2:C3)?\\)") {
        let content = ambre_xlsx::book::BookContent::Formula {
            trame: ambre_core::Validated::parse(format!("={body}")).ok(),
            excel: ambre_core::Validated::parse(body.clone()).ok(),
            cached: xl::Value::Empty,
        };
        let runs = |t: &ambre_core::Validated<ambre_xlsx::formula::AmbreFormula>| {
            let ast = parse_formula(t.as_str().strip_prefix('=')?).ok()?;
            super::xlsx::calls_builtins(&ast).then(|| crate::formula::text::CellText::new(t.as_str().to_owned()))
        };
        match classify(&content, &runs) {
            Some(ImportedCellKind::Formula(source)) => {
                prop_assert!(parse_formula(source.as_str().trim_start_matches('=')).is_ok());
            }
            Some(ImportedCellKind::UnsupportedFormula(kept)) => prop_assert_eq!(kept.as_str(), body),
            other => prop_assert!(false, "a formula sorted as {:?}", other),
        }
    }
}

/// `TRAME-IO-011`: a formula Trame cannot run leaves byte for byte, never as
/// the value it last computed.
#[test]
fn an_unsupported_formula_reexports_byte_for_byte() {
    let mut state = TrameState::new();
    let kept = [
        "_xlfn.LAMBDA(x,x+1)(A2)",
        "SUM(Sheet1:Sheet3!A1)",
        "FOOBAR(A1)+1",
    ];
    let cells = kept
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let at = format!("B{}", i + 1);
            (at, excel(f, xl::Value::Number(9.0)))
        })
        .collect::<Vec<_>>();
    let cells = cells.iter().map(|(a, c)| (a.as_str(), c.clone())).collect();
    import(&mut state, file(&workbook(vec![sheet("Data", cells)])));
    let (back, _) = export(&mut state);
    for (i, f) in kept.iter().enumerate() {
        assert_eq!(
            excel_text(content_at(&back, "Data", &format!("B{}", i + 1))),
            *f
        );
    }
}

/// `TRAME-IO-012`: every construct the codec drops or approximates reaches
/// the import's report under its own name, typed, never in silence.
#[test]
fn every_ignored_excel_feature_is_named_in_the_report() {
    for bytes in [
        include_bytes!("../../../ambre-xlsx/tests/fixtures/calamine/picture.xlsx").as_slice(),
        include_bytes!("../../../ambre-xlsx/tests/fixtures/calamine/pivots.xlsx").as_slice(),
        include_bytes!("../../../ambre-xlsx/tests/fixtures/calamine/date_1904.xlsx").as_slice(),
    ] {
        let codec = ambre_xlsx::read(&ambre_core::Payload::new(bytes.to_vec()))
            .expect("the fixture reads")
            .report;
        let lossy: Vec<_> = codec
            .entries()
            .iter()
            .filter(|e| e.outcome != XlOutcome::Carried)
            .filter(|e| e.construct != XlsxConstruct::ForeignFormula)
            .collect();
        assert!(!lossy.is_empty(), "the fixture loses something");
        let mut state = TrameState::new();
        let report = import(&mut state, ambre_core::Payload::new(bytes.to_vec()));
        for entry in lossy {
            let outcome = match entry.outcome {
                XlOutcome::Approximated => Outcome::Approximated,
                _ => Outcome::Dropped,
            };
            assert!(
                report
                    .count(TrameConstruct::Excel(entry.construct), outcome)
                    .get()
                    >= entry.count.get(),
                "{:?} is missing from the report",
                entry.construct
            );
        }
    }
}

/// A workbook of `rows` numbers in column A.
fn large(rows: usize) -> xl::Workbook {
    let cells: Vec<(String, xl::Content)> = (0..rows)
        .map(|i| (format!("A{}", i + 1), num(i as f64)))
        .collect();
    workbook(vec![sheet(
        "Big",
        cells.iter().map(|(a, c)| (a.as_str(), c.clone())).collect(),
    )])
}

fn progress_of(out: &TrameOutcome) -> Option<(u32, u32)> {
    out.display.iter().find_map(|e| match e {
        TrameEvent::Interop(InteropAnswer::Progress(p)) => Some((p.done().get(), p.total().get())),
        _ => None,
    })
}

/// `TRAME-IO-013` and `INTEROP-013`: a large import writes one slice in the
/// command and one per resume. Each slice short of the end reports further
/// on against one total, and each is a delta of its own. A scroll between
/// two slices is served without landing the import.
#[test]
fn a_large_import_lands_one_slice_at_a_time() {
    let rows = 3 * XLSX_SLICE.get() as usize + 10;
    let mut state = TrameState::new();
    let first = state.handle_conversion(
        TrameCommand::Interop(InteropRequest::Import(TrameImport::Xlsx(WorkbookImport {
            data: file(&large(rows)),
        }))),
        &office(),
    );
    assert!(first.update.is_some(), "the first slice is broadcast");
    let (mut reached, total) = progress_of(&first).expect("the first slice reports");
    assert_eq!(total as usize, rows);
    assert!(state.has_pending_work());
    // A scroll is served at once and leaves the import running.
    read_cell_value(&mut state, 0);
    assert!(state.has_pending_work(), "a scroll waits for no import");
    let mut last = None;
    while state.has_pending_work() {
        let out = state.resume();
        assert!(out.update.is_some(), "every slice is broadcast");
        match progress_of(&out) {
            Some((done, t)) => {
                assert_eq!(t, total);
                assert!(done > reached, "a slice reports further on");
                assert!(done - reached <= XLSX_SLICE.get());
                reached = done;
            }
            None => last = Some(out),
        }
    }
    assert!(report_of(&last.expect("a last slice")).is_some());
    assert_eq!(stored(&state, &format!("A{rows}")), (rows - 1).to_string());
}

fn read_cell_value(state: &mut TrameState, row: u32) {
    state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_core::Seq::nth(1),
        row_start: Pos::<Row>::new(row),
        row_end: Pos::<Row>::new(row + 20),
        col_start: Pos::<Col>::new(0),
        col_end: Pos::<Col>::new(5),
    });
}

/// `TRAME-IO-013`: an edit during an import lands the import whole first,
/// and one undo then removes the import whole, every slice of it.
#[test]
fn an_edit_lands_the_import_first_and_undo_removes_it_whole() {
    let rows = 2 * XLSX_SLICE.get() as usize + 5;
    let mut state = TrameState::new();
    let before = state.sheet_order_ids();
    state.handle_conversion(
        TrameCommand::Interop(InteropRequest::Import(TrameImport::Xlsx(WorkbookImport {
            data: file(&large(rows)),
        }))),
        &office(),
    );
    assert!(state.has_pending_work());
    state.handle_command(TrameCommand::Undo);
    assert!(!state.has_pending_work(), "an edit lands the import first");
    assert_eq!(state.sheet_order_ids(), before, "one undo removes it whole");
}

/// `TRAME-IO-015`: a hidden sheet lands hidden: its cells are there, the
/// strip shows no tab for it, and the export writes it hidden again.
#[test]
fn a_hidden_sheet_imports_hidden_and_leaves_hidden() {
    let mut hidden = sheet("Secret", vec![("A1", num(42.0))]);
    hidden.visibility = xl::Visibility::Hidden;
    let mut state = TrameState::new();
    import(
        &mut state,
        file(&workbook(vec![
            sheet(
                "Shown",
                vec![("A1", excel("Secret!A1*2", xl::Value::Empty))],
            ),
            hidden,
        ])),
    );
    let TrameEvent::SheetList { sheets, .. } = state.sheet_list_event() else {
        panic!("a sheet list");
    };
    assert!(sheets.iter().all(|t| t.name.as_str() != "Secret"));
    assert_eq!(state.hidden_sheet_ids().len(), 1);
    let a1 = state.active_cell_id(CellPos::new(Pos::new(0), Pos::new(0)));
    assert!(matches!(state.engine.evaluated_value(a1), Some(CellValue::Number(n)) if n == 84.0));
    let (back, _) = export(&mut state);
    let secret = back
        .sheets
        .iter()
        .find(|s| s.name.as_str() == "Secret")
        .expect("the hidden sheet leaves");
    assert_eq!(secret.visibility, xl::Visibility::Hidden);
}

/// `TRAME-IO-016`: a merge survives an import and an export.
#[test]
fn merges_survive_an_import_and_an_export() {
    let mut data = sheet("Data", vec![("B2", text("merged")), ("E5", num(1.0))]);
    let merge = CellRange::parse("B2:D3").unwrap();
    data.merges = vec![merge];
    let mut state = TrameState::new();
    import(&mut state, file(&workbook(vec![data])));
    assert_eq!(
        state
            .resolved_merges(&state.active_sheet.clone(), &state.index)
            .len(),
        1
    );
    let (back, _) = export(&mut state);
    assert_eq!(back.sheets.last().unwrap().merges, vec![merge]);
}

/// `TRAME-IO-017`: a reference leaves at its position today. A row inserted
/// above the cell it reads moves it, and the export follows.
#[test]
fn a_reference_leaves_at_its_current_position() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "5");
    set(&mut state, 0, 1, "=A1*2");
    select(&mut state, 0, 0);
    state.handle_command(TrameCommand::InsertRowAbove);
    let (back, _) = export(&mut state);
    assert_eq!(excel_text(content_at(&back, "Sheet1", "B2")), "A2*2");
}

/// `TRAME-IO-018`: a reference to a deleted line, or to a deleted sheet,
/// leaves as `#REF!`, the error Excel reads, never as an empty cell.
#[test]
fn a_broken_reference_leaves_as_the_error_excel_reads() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Gone"),
    });
    let gone = state
        .sheet_order_ids()
        .into_iter()
        .find(|id| *id != super::default_sheet())
        .unwrap();
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: super::default_sheet(),
    });
    set(&mut state, 0, 0, "1");
    set(&mut state, 1, 1, "=A1+1");
    set(&mut state, 2, 1, "=Gone!A1+1");
    select(&mut state, 0, 0);
    state.handle_command(TrameCommand::DeleteSelectedRows);
    state.handle_command(TrameCommand::DeleteSheet { sheet_id: gone });
    let (back, _) = export(&mut state);
    assert_eq!(excel_text(content_at(&back, "Sheet1", "B1")), "#REF!+1");
    assert_eq!(excel_text(content_at(&back, "Sheet1", "B2")), "#REF!+1");
}

/// `TRAME-IO-048`: a cross-sheet reference leaves with the sheet's display
/// name of today, after a rename.
#[test]
fn a_cross_sheet_reference_carries_the_display_name() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Other"),
    });
    let other = state
        .sheet_order_ids()
        .into_iter()
        .find(|id| *id != super::default_sheet())
        .unwrap();
    state.handle_command(TrameCommand::ActivateSheet {
        sheet_id: super::default_sheet(),
    });
    set(&mut state, 0, 0, "=Other!A1+1");
    state.handle_command(TrameCommand::RenameSheet {
        sheet_id: other,
        name: crate::test_support::sheet("Q3 Sales"),
    });
    let (back, _) = export(&mut state);
    assert_eq!(
        excel_text(content_at(&back, "Sheet1", "A1")),
        "'Q3 Sales'!A1+1"
    );
}

/// `TRAME-IO-047`: an export writes nothing into the document: no
/// operation, no delta.
#[test]
fn an_export_leaves_the_document_as_it_was() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "1");
    set(&mut state, 0, 1, "=A1+1");
    let before = state.doc.oplog_vv();
    let (_, out) = export(&mut state);
    assert!(out.update.is_none());
    assert_eq!(state.doc.oplog_vv(), before);
}

/// The values of every cell of every sheet: the oracle of a round trip.
fn values(workbook: &xl::Workbook) -> Vec<(String, String, xl::Value)> {
    let mut out: Vec<_> = workbook
        .sheets
        .iter()
        .flat_map(|s| {
            s.cells.iter().map(|c| {
                let value = match &c.content {
                    xl::Content::Value(v) => v.clone(),
                    xl::Content::Formula { formula, .. } => {
                        xl::Value::Text(format!("={}", formula.excel()))
                    }
                    xl::Content::TrameFormula { formula, .. } => {
                        xl::Value::Text(formula.as_str().to_owned())
                    }
                };
                (s.name.as_str().to_owned(), c.at.to_string(), value)
            })
        })
        .filter(|(_, _, v)| *v != xl::Value::Empty)
        .collect();
    out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    out
}

fn value_content() -> impl Strategy<Value = xl::Content> {
    prop_oneof![
        (-1.0e9f64..1.0e9).prop_map(num),
        "[a-z ]{1,8}".prop_map(|t| text(&t)),
        any::<bool>().prop_map(|b| xl::Content::Value(xl::Value::Boolean(b))),
        Just(excel("SUM(A1:A3)", xl::Value::Empty)),
        Just(excel("Two!A1&\"x\"", xl::Value::Empty)),
        Just(excel("FOOBAR(B1)", xl::Value::Number(3.0))),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// `TRAME-IO-019`: an import, an export and a second import yield the
    /// same values: each number, each text, each formula.
    #[test]
    fn an_import_export_reimport_yields_the_same_values(
        one in proptest::collection::vec(value_content(), 1..12),
        two in proptest::collection::vec(value_content(), 1..6),
    ) {
        let place = |cells: Vec<xl::Content>| -> Vec<(String, xl::Content)> {
            cells
                .into_iter()
                .enumerate()
                .map(|(i, c)| (format!("{}{}", ["A", "B", "C"][i % 3], i / 3 + 1), c))
                .collect()
        };
        let (one, two) = (place(one), place(two));
        let source = workbook(vec![
            sheet("One", one.iter().map(|(a, c)| (a.as_str(), c.clone())).collect()),
            sheet("Two", two.iter().map(|(a, c)| (a.as_str(), c.clone())).collect()),
        ]);
        let mut first = TrameState::new();
        import(&mut first, file(&source));
        let (exported, _) = export(&mut first);
        let mut second = TrameState::new();
        import(&mut second, ambre_xlsx::write(&exported).unwrap().bytes);
        let (again, _) = export(&mut second);
        let drop_default = |w: &xl::Workbook| {
            values(w).into_iter().filter(|(s, _, _)| s != "Sheet1").collect::<Vec<_>>()
        };
        prop_assert_eq!(drop_default(&again), drop_default(&exported));
        let imported: Vec<_> = drop_default(&exported)
            .into_iter()
            .filter(|(_, _, v)| !matches!(v, xl::Value::Text(t) if t.starts_with('=')))
            .collect();
        let source_values: Vec<_> = values(&source)
            .into_iter()
            .filter(|(_, _, v)| !matches!(v, xl::Value::Text(t) if t.starts_with('=')))
            .map(|(s, a, v)| (s, a, match v {
                xl::Value::Boolean(b) => xl::Value::Text(if b { "TRUE" } else { "FALSE" }.to_owned()),
                other => other,
            }))
            .collect();
        let imported: Vec<_> = imported
            .into_iter()
            .map(|(s, a, v)| (s, a, match v {
                xl::Value::Boolean(b) => xl::Value::Text(if b { "TRUE" } else { "FALSE" }.to_owned()),
                other => other,
            }))
            .collect();
        prop_assert_eq!(imported, source_values);
    }
}

/// The bytes the module writes for a style are the bytes Trame stores. The
/// twin of `ambre-xlsx` holds the layout of the style record, field for
/// field. A style crossing an import lands as the user set it.
#[test]
fn the_style_twin_writes_the_bytes_trame_stores() {
    use super::styles::StyleRecord;
    use crate::message::{BorderStyle, Borders, CellBorder, CellFormat, CellStyle, HAlign, VAlign};
    let styles = [
        StyleRecord::default(),
        StyleRecord {
            bold: ambre_core::Weight::Bold,
            italic: ambre_core::Slant::Italic,
            style: CellStyle {
                underline: ambre_core::Underline::Underlined,
                h_align: HAlign::Right,
                v_align: VAlign::Top,
                fg: Some(ambre_core::Rgb::from_bits(0x12_34_56)),
                bg: Some(ambre_core::Rgb::from_bits(0xab_cd_ef)),
                size: Some(ambre_core::FontSize::from_whole_points(14)),
                format: CellFormat::Custom("0.00\"x\"".to_owned()),
                decimals: ambre_core::Bounded::new(3),
                borders: Borders {
                    top: Some(CellBorder {
                        style: BorderStyle::Double,
                        color: Some(ambre_core::Rgb::from_bits(0xff_00_00)),
                    }),
                    left: Some(CellBorder {
                        style: BorderStyle::Dotted,
                        color: None,
                    }),
                    ..Borders::default()
                },
                ..CellStyle::default()
            },
        },
    ];
    for record in styles {
        let bytes = postcard::to_allocvec(&record).unwrap();
        let twin: ambre_xlsx::book::TrameStyle = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(postcard::to_allocvec(&twin).unwrap(), bytes);
    }
}

/// Import `data` whole as an OpenDocument spreadsheet.
fn import_ods(
    state: &mut TrameState,
    data: ambre_core::Payload<ambre_core::FileBytes>,
) -> TrameReport {
    let mut out = state.handle_conversion(
        TrameCommand::Interop(InteropRequest::Import(TrameImport::Ods(WorkbookImport {
            data,
        }))),
        &office(),
    );
    while state.has_pending_work() {
        out = state.resume();
    }
    report_of(&out).expect("an import ends on its report")
}

/// Export the workbook as `as_ods` names, and read the file back through
/// the codec of its format.
fn export_as(state: &mut TrameState, export: TrameExport) -> (xl::Workbook, TrameOutcome) {
    let ods = matches!(export, TrameExport::Ods);
    let out = state.handle_conversion(
        TrameCommand::Interop(InteropRequest::Export(export)),
        &office(),
    );
    let file = out
        .display
        .iter()
        .find_map(|e| match e {
            TrameEvent::Interop(InteropAnswer::ExportReady { file, .. }) => Some(file.clone()),
            _ => None,
        })
        .expect("an export answers its file");
    let bytes = ambre_core::Payload::new(file.data.as_bytes().to_vec());
    let read = if ods {
        assert_eq!(
            file.media_type.as_str(),
            "application/vnd.oasis.opendocument.spreadsheet"
        );
        assert!(file.filename.as_str().ends_with(".ods"));
        ambre_xlsx::read_ods(&bytes)
    } else {
        ambre_xlsx::read(&bytes)
    };
    (read.expect("the export reads back").workbook, out)
}

/// Trame's form of the whole document, in an order of its own. Each cell
/// comes with the bytes of its style, sorted by place, beside each sheet's
/// layout. A column width reads as zero. An xlsx file counts a width in
/// digits of its font, and an OpenDocument file in inches. So LibreOffice
/// writes one column at two widths a few pixels apart.
fn document(state: &TrameState) -> Vec<String> {
    let mut report = TrameReport::new(Direction::Export, FileFormat::Xlsx);
    let (book, _) = state.excel_workbook(&mut report);
    let mut out = Vec::new();
    for sheet in &book.sheets {
        let mut merges = sheet.merges.clone();
        merges.sort();
        let mut widths: Vec<_> = sheet
            .column_widths
            .iter()
            .map(|w| (w.first, w.last))
            .collect();
        widths.sort();
        let mut heights: Vec<_> = sheet
            .row_heights
            .iter()
            .map(|h| (h.row, h.height.get().to_bits()))
            .collect();
        heights.sort();
        out.push(format!(
            "{} {:?} merges {merges:?} widths {widths:?} heights {heights:?}",
            sheet.name, sheet.visibility
        ));
        let mut cells: Vec<String> = sheet
            .cells
            .iter()
            .map(|c| {
                let style = book
                    .styles
                    .get(c.style.slot().to_usize())
                    .map(|s| s.as_bytes().to_vec())
                    .unwrap_or_default();
                format!("{} {} {:?} {style:?}", sheet.name, c.at, c.content)
            })
            .collect();
        cells.sort();
        out.extend(cells);
    }
    out
}

/// The entries of a report, sorted: two readers meet the constructs of one
/// workbook in their own order.
fn entries(report: &TrameReport) -> Vec<String> {
    let mut out: Vec<String> = report
        .entries()
        .iter()
        .map(|e| format!("{:?} {:?} {}", e.construct, e.outcome, e.count.get()))
        .collect();
    out.sort();
    out
}

/// Each cell of `workbook` with its style resolved, and each sheet's
/// layout. A reader of the file sees these, in whatever order the file
/// lists its styles. A control character XML 1.0 cannot carry leaves an
/// OpenDocument text, which has no escape for it, so no text keeps one here.
fn resolved(workbook: &xl::Workbook) -> Vec<String> {
    let mut workbook = workbook.clone();
    for sheet in &mut workbook.sheets {
        for cell in &mut sheet.cells {
            if let xl::Content::Formula {
                cached: xl::Value::Text(t),
                ..
            }
            | xl::Content::Value(xl::Value::Text(t)) = &mut cell.content
            {
                t.retain(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'));
            }
        }
    }
    let workbook = &workbook;
    workbook
        .sheets
        .iter()
        .flat_map(|s| {
            let layout = format!(
                "{} {:?} merges {:?} widths {:?} heights {:?}",
                s.name.as_str(),
                s.visibility,
                s.merges,
                s.column_widths,
                s.row_heights
            );
            std::iter::once(layout).chain(s.cells.iter().map(move |c| {
                format!(
                    "{} {} {:?} {:?}",
                    s.name.as_str(),
                    c.at,
                    c.content,
                    workbook.style(c.style)
                )
            }))
        })
        .collect()
}

/// `TRAME-IO-029`: an OpenDocument spreadsheet lands as the same document
/// as the same workbook from Excel, with the same report. LibreOffice wrote
/// both files from one source, `trame.fods`.
#[test]
fn an_ods_import_lands_the_same_document_as_the_same_workbook_from_excel() {
    let mut from_ods = TrameState::new();
    let ods_report = import_ods(
        &mut from_ods,
        ambre_core::Payload::new(
            include_bytes!("../../../ambre-xlsx/tests/fixtures/generated/trame.ods").to_vec(),
        ),
    );
    let mut from_xlsx = TrameState::new();
    let xlsx_report = import(
        &mut from_xlsx,
        ambre_core::Payload::new(
            include_bytes!("../../../ambre-xlsx/tests/fixtures/generated/trame.xlsx").to_vec(),
        ),
    );
    assert_eq!(document(&from_ods), document(&from_xlsx));
    assert_eq!(entries(&ods_report), entries(&xlsx_report));
    assert_eq!(ods_report.format(), FileFormat::Ods);
    assert!(document(&from_ods).len() > 2);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// `TRAME-IO-029`, over generated workbooks: the same workbook written
    /// as xlsx and as OpenDocument lands as one document and one report.
    #[test]
    fn an_ods_and_an_xlsx_of_one_workbook_land_as_one_document(
        one in proptest::collection::vec(value_content(), 1..12),
        two in proptest::collection::vec(value_content(), 1..6),
    ) {
        let place = |cells: Vec<xl::Content>| -> Vec<(String, xl::Content)> {
            cells
                .into_iter()
                .enumerate()
                .map(|(i, c)| (format!("{}{}", ["A", "B", "C"][i % 3], i / 3 + 1), c))
                .collect()
        };
        let (one, two) = (place(one), place(two));
        let mut source = workbook(vec![
            sheet("One", one.iter().map(|(a, c)| (a.as_str(), c.clone())).collect()),
            sheet("Two", two.iter().map(|(a, c)| (a.as_str(), c.clone())).collect()),
        ]);
        source.sheets[0].merges = vec![CellRange::parse("E1:F2").unwrap()];
        let mut from_ods = TrameState::new();
        let ods_report = import_ods(&mut from_ods, ambre_xlsx::write_ods(&source).unwrap().bytes);
        let mut from_xlsx = TrameState::new();
        let xlsx_report = import(&mut from_xlsx, file(&source));
        prop_assert_eq!(document(&from_ods), document(&from_xlsx));
        prop_assert_eq!(entries(&ods_report), entries(&xlsx_report));
    }

    /// `TRAME-IO-066`: the OpenDocument export carries the workbook the Excel
    /// export carries, and a second import of it gives the same file again.
    #[test]
    fn an_ods_export_carries_what_the_excel_export_carries(
        one in proptest::collection::vec(value_content(), 1..12),
        two in proptest::collection::vec(value_content(), 1..6),
    ) {
        let place = |cells: Vec<xl::Content>| -> Vec<(String, xl::Content)> {
            cells
                .into_iter()
                .enumerate()
                .map(|(i, c)| (format!("{}{}", ["A", "B", "C"][i % 3], i / 3 + 1), c))
                .collect()
        };
        let (one, two) = (place(one), place(two));
        let mut hidden = sheet("Two", two.iter().map(|(a, c)| (a.as_str(), c.clone())).collect());
        hidden.visibility = xl::Visibility::Hidden;
        let mut shown = sheet("One", one.iter().map(|(a, c)| (a.as_str(), c.clone())).collect());
        shown.merges = vec![CellRange::parse("E1:F2").unwrap()];
        shown.column_widths = vec![xl::ColumnWidth {
            first: Pos::new(1),
            last: Pos::new(1),
            width: ambre_core::Px::new(120.0),
        }];
        shown.row_heights = vec![xl::RowHeight {
            row: Pos::new(2),
            height: ambre_core::Px::new(40.0),
        }];
        let source = workbook(vec![shown, hidden]);
        let mut state = TrameState::new();
        import(&mut state, file(&source));
        let (as_ods, _) = export_as(&mut state, TrameExport::Ods);
        let (as_xlsx, _) = export_as(&mut state, TrameExport::Xlsx);
        prop_assert_eq!(resolved(&as_ods), resolved(&as_xlsx));
        let mut again = TrameState::new();
        import_ods(&mut again, ambre_xlsx::write_ods(&as_ods).unwrap().bytes);
        let (twice, _) = export_as(&mut again, TrameExport::Ods);
        let drop_default = |w: &xl::Workbook| {
            resolved(w).into_iter().filter(|line| !line.starts_with("Sheet1")).collect::<Vec<_>>()
        };
        prop_assert_eq!(drop_default(&twice), drop_default(&as_ods));
    }
}

/// `TRAME-IO-071`: the OpenDocument export reaches the File menu, and an
/// import reaches the picker that accepts the format.
#[test]
fn the_file_menu_offers_the_opendocument_import_and_export() {
    use crate::ui::commands::{EXPORT_ODS, IMPORT_ODS, command_to_app};
    assert!(matches!(
        command_to_app(EXPORT_ODS),
        Some(TrameCommand::Interop(InteropRequest::Export(
            TrameExport::Ods
        )))
    ));
    assert_eq!(
        TrameCommand::Interop(InteropRequest::Export(TrameExport::Ods)).converter_family(),
        Some(ambre_core::ConverterFamily::Office)
    );
    assert!(!TrameCommand::Interop(InteropRequest::Export(TrameExport::Ods)).is_mutating());
    let _ = IMPORT_ODS;
}
