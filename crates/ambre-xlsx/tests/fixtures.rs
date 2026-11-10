//! The codec on real files: workbooks Excel and LibreOffice wrote, read,
//! written back and read again.
//!
//! The files of `calamine/` come from the test corpus of the `calamine`
//! crate, under its MIT licence. `fixtures/THIRD_PARTY.md` names each one and
//! its upstream commit. Excel wrote most of them, LibreOffice
//! the two date files. `generated/generate.sh` writes the files of
//! `generated/`: LibreOffice's export of `trame.fods`, and LibreOffice's save
//! of [`sample`] after this codec wrote it.

#![expect(
    clippy::panic,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_core::index::Pos;
use ambre_core::{
    FileBytes, FontFamily, FontSize, Payload, Rgb, Slant, Underline, Validated, Weight, WordBreak,
};
use ambre_xlsx::cellref::{CellRange, CellRef};
use ambre_xlsx::formula::Formula;
use ambre_xlsx::model::{
    BorderStyle, Cell, CellStyle, ColumnWidth, Content, ErrorValue, HAlign, NumberFormat,
    RowHeight, Sheet, SheetName, StyleId, VAlign, Value, Visibility, Workbook,
};
use ambre_xlsx::report::{Outcome, XlsxConstruct};
use ambre_xlsx::{Imported, XlsxError};

fn fixture(path: &str) -> Payload<FileBytes> {
    let full = format!("{}/tests/fixtures/{path}", env!("CARGO_MANIFEST_DIR"));
    Payload::new(std::fs::read(&full).unwrap_or_else(|e| panic!("{full}: {e}")))
}

fn read(path: &str) -> Imported {
    ambre_xlsx::read(&fixture(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn at(text: &str) -> CellRef {
    CellRef::parse(text).unwrap_or_else(|| panic!("{text} is a cell"))
}

fn cell<'a>(sheet: &'a Sheet, text: &str) -> &'a Cell {
    let at = at(text);
    sheet
        .cells
        .iter()
        .find(|c| c.at == at)
        .unwrap_or_else(|| panic!("{text} holds a cell"))
}

fn value(sheet: &Sheet, text: &str) -> Value {
    match &cell(sheet, text).content {
        Content::Value(v) => v.clone(),
        Content::Formula { cached, .. } | Content::TrameFormula { cached, .. } => cached.clone(),
    }
}

/// The Trame text and the Excel text of a formula cell.
fn formula(sheet: &Sheet, text: &str) -> (Option<String>, String) {
    match &cell(sheet, text).content {
        Content::Formula { formula, .. } => (
            formula.ambre().map(str::to_owned),
            formula.excel().to_owned(),
        ),
        Content::Value(_) | Content::TrameFormula { .. } => {
            panic!("{text} holds a formula the file wrote")
        }
    }
}

fn style(imported: &Imported, sheet: &Sheet, text: &str) -> CellStyle {
    imported.workbook.style(cell(sheet, text).style)
}

fn sheet<'a>(imported: &'a Imported, name: &str) -> &'a Sheet {
    imported
        .workbook
        .sheets
        .iter()
        .find(|s| s.name.as_str() == name)
        .unwrap_or_else(|| panic!("the workbook holds {name}"))
}

fn count(imported: &Imported, construct: XlsxConstruct, outcome: Outcome) -> u32 {
    imported.report.count(construct, outcome).get()
}

/// Write `workbook`, then read the file back.
fn round_trip(workbook: &Workbook) -> Imported {
    let exported = ambre_xlsx::write(workbook).unwrap_or_else(|e| panic!("the write: {e}"));
    ambre_xlsx::read(&exported.bytes).unwrap_or_else(|e| panic!("the read back: {e}"))
}

#[test]
fn a_workbook_reads_its_values_dates_texts_and_names() {
    let book = read("generated/trame.xlsx");
    let data = sheet(&book, "Data");
    // A date stays its serial, under a date format.
    assert_eq!(value(data, "A4"), Value::Number(45_352.0));
    assert!(matches!(
        style(&book, data, "A4").format,
        NumberFormat::Date | NumberFormat::Custom(_)
    ));
    assert_eq!(
        value(data, "B4"),
        Value::Text("Pen & <ink> \"q\"".to_owned())
    );
    // The named range is no construct of Trame's model: it is reported.
    assert_eq!(
        count(&book, XlsxConstruct::DefinedName, Outcome::Dropped),
        1
    );
}

#[test]
fn every_error_value_reads_and_a_formula_trame_cannot_spell_stays_excels() {
    let book = read("calamine/errors.xlsx");
    let s = sheet(&book, "Feuil1");
    let errors = [
        ("A1", ErrorValue::DivisionByZero),
        ("A2", ErrorValue::Name),
        ("A3", ErrorValue::Value),
        ("A4", ErrorValue::Null),
        ("A5", ErrorValue::Ref),
        ("A6", ErrorValue::Num),
        ("A7", ErrorValue::NotAvailable),
    ];
    for (cell, error) in errors {
        assert_eq!(value(s, cell), Value::Error(error), "{cell}");
    }
    // The intersection operator and the link to another workbook stay
    // Excel's, byte for byte (`TRAME-IO-011`).
    assert_eq!(formula(s, "A4"), (None, "A1:B1 D1:D9".to_owned()));
    assert_eq!(formula(s, "A5"), (None, "[1]Feuil8!A1".to_owned()));
    assert_eq!(
        count(&book, XlsxConstruct::ForeignFormula, Outcome::Approximated),
        2
    );
    assert_eq!(
        count(&book, XlsxConstruct::ExternalLink, Outcome::Dropped),
        1
    );
}

#[test]
fn a_shared_formula_expands_into_each_cell_it_covers() {
    let simple = read("calamine/shared_formula_simple.xlsx");
    let s = &simple.workbook.sheets[0];
    assert_eq!(formula(s, "B3").0.as_deref(), Some("=A3*2"));
    assert_eq!(
        count(&simple, XlsxConstruct::SharedFormula, Outcome::Carried),
        2
    );

    let named = read("calamine/shared_formula_function_names.xlsx");
    let s = &named.workbook.sheets[0];
    assert_eq!(formula(s, "B10").0.as_deref(), Some("=LOG10(ABS(A10)+1)"));

    // The master cell may come after the cells that name it.
    let reversed = read("calamine/shared_formula_reversed.xlsx");
    for s in &reversed.workbook.sheets {
        assert_eq!(formula(s, "B1").0.as_deref(), Some("=A1"));
        assert_eq!(formula(s, "B5").0.as_deref(), Some("=A5"));
    }

    // An anchored part keeps its place while the shared block moves, then
    // drops its anchor in Trame's dialect.
    let anchored = read("calamine/absolute_shared_formula.xlsx");
    let s = &anchored.workbook.sheets[0];
    assert_eq!(formula(s, "D3"), (Some("=D1".to_owned()), "D$1".to_owned()));
    assert_eq!(
        count(
            &anchored,
            XlsxConstruct::AbsoluteReference,
            Outcome::Approximated
        ),
        15
    );
}

#[test]
fn excel_styles_read_as_trame_cell_styles() {
    let book = read("calamine/styles.xlsx");
    let s = &book.workbook.sheets[0];
    assert_eq!(style(&book, s, "A1").weight, Weight::Bold);
    assert_eq!(style(&book, s, "A2").slant, Slant::Italic);
    assert_eq!(style(&book, s, "A3").underline, Underline::Underlined);
    assert_eq!(
        style(&book, s, "A4").strikethrough,
        ambre_core::Strikethrough::Struck
    );
    assert_eq!(style(&book, s, "A5").fg, Some(Rgb::from_bits(0xFF0000)));
    assert!(style(&book, s, "A6").bg.is_some());
    assert_eq!(style(&book, s, "A7").h_align, HAlign::Center);
    assert_eq!(style(&book, s, "A8").v_align, VAlign::Middle);
    assert_eq!(style(&book, s, "A9").wrap, WordBreak::Anywhere);
    // A cell that names no alignment sits at the bottom, as Excel draws it.
    assert_eq!(style(&book, s, "A1").v_align, VAlign::Bottom);
    // The default font's colour stays the theme's text colour.
    assert_eq!(style(&book, s, "A1").fg, None);
    assert_eq!(s.column_widths.len(), 1);

    let borders = read("calamine/borders.xlsx");
    let s = &borders.workbook.sheets[0];
    let edge = style(&borders, s, "A2").borders.top;
    assert_eq!(
        edge.map(|b| (b.style, b.color)),
        Some((BorderStyle::Solid, Some(Rgb::from_bits(0xFF0000))))
    );
    assert_eq!(
        style(&borders, s, "D2").borders.top.map(|b| b.style),
        Some(BorderStyle::Dashed)
    );
}

#[test]
fn merged_ranges_read_as_they_were_drawn() {
    let book = read("calamine/merge_cells.xlsx");
    let merges: Vec<String> = book.workbook.sheets[0]
        .merges
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(merges, ["A1:B1", "A2:A4", "B2:D4"]);
    assert_eq!(count(&book, XlsxConstruct::Merge, Outcome::Carried), 3);
}

#[test]
fn a_hidden_sheet_stays_hidden_and_a_chart_sheet_is_reported() {
    // `TRAME-IO-015`: a hidden sheet is kept, hidden, never dropped.
    let book = read("calamine/any_sheets.xlsx");
    let names: Vec<(&str, Visibility)> = book
        .workbook
        .sheets
        .iter()
        .map(|s| (s.name.as_str(), s.visibility))
        .collect();
    assert_eq!(
        names,
        [
            ("Visible", Visibility::Visible),
            ("Hidden", Visibility::Hidden),
            ("VeryHidden", Visibility::Hidden)
        ]
    );
    assert_eq!(
        count(&book, XlsxConstruct::HiddenSheet, Outcome::Carried),
        1
    );
    assert_eq!(
        count(&book, XlsxConstruct::VeryHiddenSheet, Outcome::Approximated),
        1
    );
    assert_eq!(count(&book, XlsxConstruct::Chartsheet, Outcome::Dropped), 1);
}

#[test]
fn every_construct_trame_lacks_is_reported_never_dropped_in_silence() {
    // `TRAME-IO-012`, the codec's half: each ignored feature is an entry.
    let cases: [(&str, XlsxConstruct, u32); 5] = [
        ("calamine/pivots.xlsx", XlsxConstruct::PivotTable, 5),
        ("calamine/picture.xlsx", XlsxConstruct::Picture, 2),
        ("calamine/table-multiple.xlsx", XlsxConstruct::Table, 4),
        (
            "calamine/rich_text_shared_strings.xlsx",
            XlsxConstruct::RichText,
            2,
        ),
        ("calamine/pivots.xlsx", XlsxConstruct::TextOrientation, 1),
    ];
    for (path, construct, n) in cases {
        assert_eq!(count(&read(path), construct, Outcome::Dropped), n, "{path}");
    }
    // The rich text keeps its words.
    let rich = read("calamine/rich_text_shared_strings.xlsx");
    assert_eq!(
        value(&rich.workbook.sheets[0], "A2"),
        Value::Text("test2 other".to_owned())
    );
}

#[test]
fn an_escaped_carriage_return_and_a_strict_package_read() {
    let book = read("calamine/has_x000D_.xlsx");
    // The part holds `ABC_x000D_` then a literal CR LF. XML reads a literal
    // CR LF as one LF, so the escape alone keeps its CR.
    assert_eq!(
        value(&book.workbook.sheets[0], "A1"),
        Value::Text("ABC\r\nDEF".to_owned())
    );
    // The strict namespace names every relationship under another URI.
    let strict = read("calamine/strict_iso_paths.xlsx");
    assert_eq!(strict.workbook.sheets[0].cells.len(), 1368);
}

#[test]
fn a_password_protected_workbook_is_refused_with_its_reason() {
    assert_eq!(
        ambre_xlsx::read(&fixture("calamine/password_protected.xlsx")).err(),
        Some(XlsxError::CompoundFile)
    );
}

#[test]
fn a_libreoffice_workbook_reads_its_values_formulas_styles_and_layout() {
    let book = read("generated/trame.xlsx");
    let data = sheet(&book, "Data");
    assert_eq!(value(data, "B1"), Value::Number(1.5));
    assert_eq!(formula(data, "D1").0.as_deref(), Some("=SUM(B1:C1)"));
    assert_eq!(
        formula(data, "D2").0.as_deref(),
        Some("=IF(B1>1,\"big\",\"small\")")
    );
    assert_eq!(value(data, "D2"), Value::Text("big".to_owned()));
    assert_eq!(formula(data, "D3").0.as_deref(), Some("=Hidden!A1*2"));
    assert_eq!(value(data, "C3"), Value::Boolean(true));
    let title = style(&book, data, "A1");
    assert_eq!((title.weight, title.slant), (Weight::Bold, Slant::Italic));
    assert_eq!(title.fg, Some(Rgb::from_bits(0x0000FF)));
    assert_eq!(title.bg, Some(Rgb::from_bits(0xFFFF00)));
    assert_eq!(title.h_align, HAlign::Center);
    // The FODS names a face it never declares, so LibreOffice wrote the
    // workbook's default font, which reads as no family of the cell's own.
    assert_eq!(title.font, None::<FontFamily>);
    assert_eq!(title.size, FontSize::new(14.0));
    assert_eq!(
        title.borders.bottom.and_then(|b| b.color),
        Some(Rgb::from_bits(0xFF0000))
    );
    assert!(matches!(style(&book, data, "C1").format, NumberFormat::Number(d) if d.get() == 2));
    assert!(matches!(style(&book, data, "C2").format, NumberFormat::Percent(d) if d.get() == 0));
    assert_eq!(style(&book, data, "C3").wrap, WordBreak::Anywhere);
    assert_eq!(
        data.merges
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["A2:B3"]
    );
    assert_eq!(
        data.column_widths.first().map(|c| c.first),
        Some(Pos::new(0))
    );
    assert_eq!(data.row_heights.first().map(|r| r.height.get()), Some(48.0));
    assert_eq!(sheet(&book, "Hidden").visibility, Visibility::Hidden);
}

#[test]
fn the_1904_date_system_reads_on_the_1900_calendar() {
    let modern = read("calamine/date.xlsx");
    let old = read("calamine/date_1904.xlsx");
    let (a, b) = (&modern.workbook.sheets[0], &old.workbook.sheets[0]);
    for cell in ["A1", "A2", "A3"] {
        assert_eq!(value(a, cell), value(b, cell), "{cell}");
    }
    assert_eq!(
        count(&old, XlsxConstruct::DateSystem, Outcome::Approximated),
        1
    );
}

#[test]
fn every_fixture_survives_a_write_and_a_read_unchanged() {
    // `TRAME-IO-019`, the codec's half: what a read produced, a write and a
    // read give back.
    for path in [
        "calamine/errors.xlsx",
        "calamine/styles.xlsx",
        "calamine/borders.xlsx",
        "calamine/merge_cells.xlsx",
        "calamine/any_sheets.xlsx",
        "calamine/pivots.xlsx",
        "calamine/shared_formula_dragged.xlsx",
        "calamine/absolute_shared_formula.xlsx",
        "calamine/strict_iso_paths.xlsx",
        "calamine/has_x000D_.xlsx",
        "generated/trame.xlsx",
        "calamine/date_1904.xlsx",
    ] {
        let first = read(path);
        let second = round_trip(&first.workbook);
        assert_eq!(second.workbook, first.workbook, "{path}");
    }
}

#[test]
fn a_foreign_formula_is_written_back_byte_for_byte() {
    // `TRAME-IO-011`, the codec's half.
    let book = read("calamine/errors.xlsx");
    let back = round_trip(&book.workbook);
    let s = sheet(&back, "Feuil1");
    assert_eq!(formula(s, "A4"), (None, "A1:B1 D1:D9".to_owned()));
    assert_eq!(formula(s, "A5"), (None, "[1]Feuil8!A1".to_owned()));
}

#[test]
fn one_workbook_writes_one_sequence_of_bytes() {
    let workbook = sample();
    let a = ambre_xlsx::write(&workbook).map(|e| e.bytes.into_bytes());
    let b = ambre_xlsx::write(&workbook).map(|e| e.bytes.into_bytes());
    assert!(a.is_ok());
    assert_eq!(a, b);
}

#[test]
fn a_workbook_without_a_sheet_or_with_two_sheets_of_one_name_writes_nothing() {
    assert_eq!(
        ambre_xlsx::write(&Workbook::default()).err(),
        Some(XlsxError::NoSheet)
    );
    let mut twins = sample();
    let mut copy = twins.sheets[0].clone();
    copy.name = name("DATA");
    twins.sheets.push(copy);
    assert!(matches!(
        ambre_xlsx::write(&twins),
        Err(XlsxError::DuplicateSheetName(_))
    ));
}

#[test]
fn a_write_libreoffice_saved_again_reads_back_the_same_workbook() {
    // LibreOffice opened the file this codec wrote for `sample`, and saved it
    // in its own words: the values, formulas and layout survive both tools.
    let book = read("generated/ambre_resaved.xlsx");
    let ours = sample();
    assert_eq!(book.workbook.sheets.len(), ours.sheets.len());
    for (theirs, mine) in book.workbook.sheets.iter().zip(&ours.sheets) {
        assert_eq!(theirs.name, mine.name);
        assert_eq!(theirs.visibility, mine.visibility);
        assert_eq!(theirs.merges, mine.merges);
        for c in &mine.cells {
            let back = cell(theirs, &c.at.to_string());
            match (&c.content, &back.content) {
                (Content::Value(a), Content::Value(b)) => assert_eq!(a, b, "{}", c.at),
                // LibreOffice saves a typed boolean as the call `TRUE()`.
                (Content::Value(a @ Value::Boolean(_)), Content::Formula { cached, .. }) => {
                    assert_eq!(a, cached, "{}", c.at);
                }
                (Content::Formula { formula: a, .. }, Content::Formula { formula: b, .. }) => {
                    assert_eq!(a.ambre(), b.ambre(), "{}", c.at);
                }
                (a, b) => panic!("{}: {a:?} came back as {b:?}", c.at),
            }
            let (mine_style, their_style) = (ours.style(c.style), book.workbook.style(back.style));
            assert_eq!(
                (
                    mine_style.weight,
                    mine_style.bg,
                    mine_style.h_align,
                    &mine_style.format
                ),
                (
                    their_style.weight,
                    their_style.bg,
                    their_style.h_align,
                    &their_style.format
                ),
                "{}",
                c.at
            );
        }
        for (a, b) in theirs.column_widths.iter().zip(&mine.column_widths) {
            assert!((a.width.get() - b.width.get()).abs() <= 1.0, "{a:?} {b:?}");
        }
    }
}

/// Write the file LibreOffice resaves into `ambre_resaved.xlsx`, when
/// `AMBRE_XLSX_SAMPLE` names where. A run without it writes nothing.
#[test]
fn the_sample_writes_where_asked() {
    if let Ok(path) = std::env::var("AMBRE_XLSX_SAMPLE") {
        let exported = ambre_xlsx::write(&sample()).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&path, exported.bytes.as_bytes()).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
}

fn name(text: &str) -> Validated<SheetName> {
    Validated::<SheetName>::parse(text.to_owned()).unwrap_or_else(|e| panic!("{text}: {e}"))
}

/// A workbook as Trame would hand it to the writer: two sheets, one hidden,
/// values of every kind, formulas, styles, a merge and a layout.
fn sample() -> Workbook {
    let bold = CellStyle {
        weight: Weight::Bold,
        bg: Some(Rgb::from_bits(0xFFEE88)),
        h_align: HAlign::Center,
        ..CellStyle::default()
    };
    let money = CellStyle {
        format: NumberFormat::Number(
            ambre_core::Bounded::new(2).unwrap_or(ambre_core::Bounded::MIN),
        ),
        ..CellStyle::default()
    };
    let put = |text: &str, content: Content, style: usize| Cell {
        at: at(text),
        content,
        style: StyleId::at(Pos::from_usize(style)),
    };
    let f = |text: &str, cached: Value| Content::Formula {
        formula: Formula::from_ambre(text).unwrap_or_else(|e| panic!("{text}: {e:?}")),
        cached,
    };
    let mut data = Sheet::named(name("Data"));
    data.cells = vec![
        put("A1", Content::Value(Value::Text("Item".to_owned())), 1),
        put("B1", Content::Value(Value::Text("Price".to_owned())), 1),
        put("A2", Content::Value(Value::Text("Pen & ink".to_owned())), 0),
        put("B2", Content::Value(Value::Number(1.25)), 2),
        put("A3", Content::Value(Value::Text("Paper".to_owned())), 0),
        put("B3", Content::Value(Value::Number(4.5)), 2),
        put("A4", Content::Value(Value::Boolean(true)), 0),
        put("B4", f("=SUM(B2:B3)", Value::Number(5.75)), 2),
        put(
            "C4",
            f("=IF(B4>5,\"high\",\"low\")", Value::Text("high".to_owned())),
            0,
        ),
        put(
            "D4",
            f("=CONCAT(A2,A3)", Value::Text("Pen & inkPaper".to_owned())),
            0,
        ),
        put("A5", f("=Notes!A1*2", Value::Number(84.0)), 0),
    ];
    data.merges = vec![CellRange::spanning(at("C1"), at("D1"))];
    data.column_widths = vec![ColumnWidth {
        first: Pos::new(0),
        last: Pos::new(0),
        width: ambre_core::Px::new(120.0),
    }];
    data.row_heights = vec![RowHeight {
        row: Pos::new(0),
        height: ambre_core::Px::new(32.0),
    }];
    let mut notes = Sheet::named(name("Notes"));
    notes.visibility = Visibility::Hidden;
    notes.cells = vec![put("A1", Content::Value(Value::Number(42.0)), 0)];
    Workbook {
        sheets: vec![data, notes],
        styles: vec![CellStyle::default(), bold, money],
    }
}

/// The canonical dump of an import: the report, then each sheet with its
/// layout and its cells, then the styles. A copied file keeps its dump
/// beside it, as `<file>.expected`.
fn dump(imported: &Imported) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for e in imported.report.entries() {
        let _ = writeln!(
            out,
            "report {:?} {:?} {}",
            e.construct,
            e.outcome,
            e.count.get()
        );
    }
    for s in &imported.workbook.sheets {
        let merges: Vec<String> = s.merges.iter().map(ToString::to_string).collect();
        let _ = writeln!(
            out,
            "sheet {:?} {:?} merges {merges:?}",
            s.name.as_str(),
            s.visibility
        );
        for c in &s.column_widths {
            let _ = writeln!(
                out,
                "  width {}..={} {}",
                c.first.get(),
                c.last.get(),
                c.width.get()
            );
        }
        for r in &s.row_heights {
            let _ = writeln!(out, "  height {} {}", r.row.get(), r.height.get());
        }
        for c in &s.cells {
            let content = match &c.content {
                Content::Value(v) => format!("{v:?}"),
                Content::Formula { formula, cached } => format!(
                    "formula {:?} trame {:?} array {:?} cached {cached:?}",
                    formula.excel(),
                    formula.ambre(),
                    formula.array().map(|a| a.to_string())
                ),
                Content::TrameFormula { formula, cached } => {
                    format!("trame {:?} cached {cached:?}", formula.as_str())
                }
            };
            let _ = writeln!(out, "  {} s{} {content}", c.at, c.style.slot().get());
        }
    }
    for (i, style) in imported.workbook.styles.iter().enumerate() {
        let format = match &style.format {
            NumberFormat::Custom(code) => format!("Custom({:?})", code.as_str()),
            other => format!("{other:?}"),
        };
        let _ = writeln!(
            out,
            "style {i} {:?} {:?} {:?} {:?} {:?} {:?} {:?} fg {:?} bg {:?} font {:?} size {:?} {format} {:?}",
            style.weight,
            style.slant,
            style.underline,
            style.strikethrough,
            style.h_align,
            style.v_align,
            style.wrap,
            style.fg,
            style.bg,
            style.font,
            style.size,
            style.borders
        );
    }
    out
}

#[test]
fn every_copied_file_reads_as_its_expected_dump() {
    // `AMBRE_XLSX_BLESS=1` rewrites the dumps. Accepting one is a human act:
    // read its diff before the commit.
    let dir = format!("{}/tests/fixtures/calamine", env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "xlsx"))
        .collect();
    files.sort();
    assert!(files.len() >= 18);
    for path in files {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let text = match ambre_xlsx::read(&Payload::new(bytes)) {
            Ok(imported) => dump(&imported),
            Err(e) => format!("error {e}\n"),
        };
        let expected = path.with_extension("xlsx.expected");
        if std::env::var_os("AMBRE_XLSX_BLESS").is_some() {
            std::fs::write(&expected, &text).unwrap_or_else(|e| panic!("{e}"));
        }
        let golden = std::fs::read_to_string(&expected)
            .unwrap_or_else(|e| panic!("{}: {e}", expected.display()));
        assert_eq!(text, golden, "{}", path.display());
    }
}
