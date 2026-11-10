//! Properties of the codec on the model side. Any workbook Trame can hand
//! the writer reads back unchanged. No byte sequence makes the reader panic.

#![expect(
    clippy::panic,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_core::index::Pos;
use ambre_core::{
    Bounded, FileBytes, FontFamily, FontSize, Payload, Px, Rgb, Slant, Strikethrough, Underline,
    Validated, Weight, WordBreak,
};
use ambre_xlsx::cellref::{CellRange, CellRef};
use ambre_xlsx::formula::Formula;
use ambre_xlsx::model::{
    BorderStyle, Borders, Cell, CellBorder, CellStyle, ColumnWidth, Content, ErrorValue,
    FormatCode, HAlign, NumberFormat, RowHeight, Sheet, SheetName, StyleId, VAlign, Value,
    Visibility, Workbook,
};
use proptest::prelude::*;

fn rgb() -> impl Strategy<Value = Rgb> {
    (0u32..=0xFF_FFFF).prop_map(Rgb::from_bits)
}

fn border() -> impl Strategy<Value = Option<CellBorder>> {
    let style = prop::sample::select(vec![
        BorderStyle::Solid,
        BorderStyle::Dashed,
        BorderStyle::Dotted,
        BorderStyle::Medium,
        BorderStyle::Thick,
        BorderStyle::Double,
    ]);
    prop::option::of(
        (style, prop::option::of(rgb())).prop_map(|(style, color)| CellBorder { style, color }),
    )
}

fn number_format() -> impl Strategy<Value = NumberFormat> {
    let d = (0u8..=15).prop_map(|n| Bounded::new(n).unwrap_or(Bounded::MIN));
    prop_oneof![
        Just(NumberFormat::General),
        d.clone().prop_map(NumberFormat::Number),
        d.clone().prop_map(NumberFormat::Percent),
        d.prop_map(NumberFormat::Scientific),
        Just(NumberFormat::Fraction),
        Just(NumberFormat::Date),
        Just(NumberFormat::Time),
        Just(NumberFormat::Text),
        prop::sample::select(vec![
            "#,##0.00",
            "0.0\" kg\"",
            "[Red]0;-0",
            "d-mmm-yy hh:mm"
        ])
        .prop_map(|c| NumberFormat::Custom(
            Validated::<FormatCode>::parse(c.to_owned()).unwrap_or_else(|_| panic!("{c}"))
        )),
    ]
}

/// A style Trame can hold. A font or a size equal to the workbook default
/// reads back as no font or size of its own, so neither is drawn.
fn style() -> impl Strategy<Value = CellStyle> {
    (
        (
            prop::sample::select(vec![Weight::Regular, Weight::Bold]),
            prop::sample::select(vec![Slant::Upright, Slant::Italic]),
            prop::sample::select(vec![Underline::Plain, Underline::Underlined]),
            prop::sample::select(vec![Strikethrough::Plain, Strikethrough::Struck]),
            prop::sample::select(vec![
                HAlign::Auto,
                HAlign::Left,
                HAlign::Center,
                HAlign::Right,
            ]),
            prop::sample::select(vec![VAlign::Top, VAlign::Middle, VAlign::Bottom]),
            prop::sample::select(vec![WordBreak::Whole, WordBreak::Anywhere]),
        ),
        (
            prop::option::of(rgb()),
            prop::option::of(rgb()),
            prop::option::of(
                prop::sample::select(vec!["Arial", "Georgia", "Serif", "Monospace"])
                    .prop_map(FontFamily::from_label),
            ),
            prop::option::of(
                (6u16..=72)
                    .prop_filter("the default size", |s| *s != 11)
                    .prop_map(FontSize::from_whole_points),
            ),
            number_format(),
            (border(), border(), border(), border()),
        ),
    )
        .prop_map(
            |(
                (weight, slant, underline, strikethrough, h_align, v_align, wrap),
                (fg, bg, font, size, format, (top, right, bottom, left)),
            )| CellStyle {
                weight,
                slant,
                underline,
                strikethrough,
                h_align,
                v_align,
                wrap,
                fg,
                bg,
                font,
                size,
                format,
                borders: Borders {
                    top,
                    right,
                    bottom,
                    left,
                },
            },
        )
}

fn value() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<f64>()
            .prop_filter("a finite number", |n| n.is_finite())
            .prop_map(Value::Number),
        "[ -~\\r\\n\\t\u{e9}\u{263a}_]{1,24}".prop_map(Value::Text),
        any::<bool>().prop_map(Value::Boolean),
        prop::sample::select(ErrorValue::ALL.to_vec()).prop_map(Value::Error),
    ]
}

fn content() -> impl Strategy<Value = Content> {
    prop_oneof![
        3 => value().prop_map(Content::Value),
        1 => (prop::sample::select(vec!["=A1+1", "=SUM(A1:B9)*2", "=IFS(A1>0,\"pos\",TRUE,\"neg\")", "=CONCAT(\"a\",B2)", "=Sheet1!C3"]), prop_oneof![Just(Value::Empty), value()])
            .prop_map(|(text, cached)| Content::Formula {
                formula: Formula::from_ambre(text).unwrap_or_else(|e| panic!("{text}: {e:?}")),
                cached,
            }),
    ]
}

fn sheet(index: usize, styles: usize) -> impl Strategy<Value = Sheet> {
    let cells = prop::collection::btree_map((0u32..40, 0u32..12), (content(), 0..styles), 0..30);
    let merges = prop::collection::vec((0u32..20, 12u32..20, 1u32..3, 1u32..3), 0..3);
    let widths = prop::collection::btree_set(0u32..30, 0..4);
    let heights = prop::collection::btree_map(0u32..60, 1u32..200, 0..4);
    (cells, merges, widths, heights, any::<bool>()).prop_map(
        move |(cells, merges, widths, heights, hidden)| {
            let mut sheet = Sheet::named(
                Validated::<SheetName>::parse(format!("Sheet {}", index + 1))
                    .unwrap_or_else(|e| panic!("{e}")),
            );
            sheet.visibility = if hidden && index > 0 {
                Visibility::Hidden
            } else {
                Visibility::Visible
            };
            sheet.cells = cells
                .into_iter()
                .map(|((row, col), (content, style))| Cell {
                    at: CellRef {
                        row: Pos::new(row),
                        col: Pos::new(col),
                    },
                    content,
                    style: StyleId::at(Pos::from_usize(style)),
                })
                .collect();
            // Merges sit in disjoint column bands, so two never overlap.
            let mut bands: Vec<_> = merges
                .into_iter()
                .enumerate()
                .map(|(i, (row, col, h, w))| {
                    let col = col + u32::try_from(i).unwrap_or(0) * 4;
                    CellRange::spanning(
                        CellRef {
                            row: Pos::new(row),
                            col: Pos::new(col),
                        },
                        CellRef {
                            row: Pos::new(row + h),
                            col: Pos::new(col + w),
                        },
                    )
                })
                .collect();
            bands.sort();
            sheet.merges = bands;
            sheet.column_widths = widths
                .into_iter()
                .map(|c| ColumnWidth {
                    first: Pos::new(c),
                    last: Pos::new(c),
                    width: Px::new(20.0 + c as f32 * 3.0),
                })
                .collect();
            sheet.row_heights = heights
                .into_iter()
                .map(|(row, px)| RowHeight {
                    row: Pos::new(row),
                    height: Px::new(px as f32),
                })
                .collect();
            sheet
        },
    )
}

/// A workbook of distinct styles, the default first, and one to three
/// sheets, the first visible.
fn workbook() -> impl Strategy<Value = Workbook> {
    prop::collection::vec(style(), 0..6)
        .prop_map(|extra| {
            let mut styles = vec![CellStyle::default()];
            for s in extra {
                if !styles.contains(&s) {
                    styles.push(s);
                }
            }
            styles
        })
        .prop_flat_map(|styles| {
            let n = styles.len();
            (
                Just(styles),
                (1usize..=3).prop_flat_map(move |count| {
                    (0..count).map(|i| sheet(i, n)).collect::<Vec<_>>()
                }),
            )
        })
        .prop_map(|(styles, sheets)| Workbook { sheets, styles })
}

/// The workbook a read gives back for `workbook`: a cell that holds no value
/// and the default style is no cell of a file.
fn as_read(workbook: &Workbook) -> Workbook {
    let mut expected = workbook.clone();
    for sheet in &mut expected.sheets {
        sheet.cells.retain(|c| {
            !(c.content == Content::Value(Value::Empty) && c.style == StyleId::DEFAULT)
        });
    }
    expected
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn a_workbook_trame_can_hold_reads_back_unchanged(workbook in workbook()) {
        let exported = ambre_xlsx::write(&workbook);
        prop_assert!(exported.is_ok());
        let Ok(exported) = exported else { return Ok(()) };
        let back = ambre_xlsx::read(&exported.bytes);
        prop_assert!(back.is_ok(), "{:?}", back.err());
        let Ok(back) = back else { return Ok(()) };
        prop_assert_eq!(back.workbook, as_read(&workbook));
    }

    #[test]
    fn random_bytes_never_panic_the_reader(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = ambre_xlsx::read(&Payload::<FileBytes>::new(bytes));
    }

    #[test]
    fn a_corrupted_workbook_never_panics_the_reader(flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8)) {
        let mut bytes = sample_bytes();
        for (at, byte) in flips {
            let i = at.index(bytes.len());
            if let Some(b) = bytes.get_mut(i) {
                *b = byte;
            }
        }
        let _ = ambre_xlsx::read(&Payload::<FileBytes>::new(bytes));
    }
}

fn sample_bytes() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/generated/trame.xlsx"
    );
    std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// A package holding `parts`, deflated, beside the content types part the
/// package layer writes.
fn package(parts: &[(&str, &str)]) -> Payload<FileBytes> {
    let mut writer = ambre_xlsx::opc::PackageWriter::new();
    for (name, body) in parts {
        let name = ambre_xlsx::opc::PartName::new(name).unwrap_or_else(|| panic!("{name}"));
        writer.part(name, None, Payload::new(body.as_bytes().to_vec()));
    }
    let bytes = writer.finish().unwrap_or_else(|e| panic!("{e}"));
    Payload::new(bytes.into_bytes())
}

const ROOT_RELS: &str = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>";
const BOOK_RELS: &str = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>";
const BOOK: &str = "<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>";

#[test]
fn a_malformed_package_fails_as_a_typed_error() {
    let cases: Vec<(&str, Payload<FileBytes>)> = vec![
        (
            "no workbook relationship",
            package(&[("xl/workbook.xml", BOOK)]),
        ),
        (
            "a workbook that is no XML",
            package(&[("_rels/.rels", ROOT_RELS), ("xl/workbook.xml", "<workbook")]),
        ),
        (
            "a workbook of no sheet",
            package(&[
                ("_rels/.rels", ROOT_RELS),
                ("xl/workbook.xml", "<workbook/>"),
            ]),
        ),
        (
            "a sheet part missing",
            package(&[
                ("_rels/.rels", ROOT_RELS),
                ("xl/_rels/workbook.xml.rels", BOOK_RELS),
                ("xl/workbook.xml", BOOK),
            ]),
        ),
        (
            "a sheet that is no XML",
            package(&[
                ("_rels/.rels", ROOT_RELS),
                ("xl/_rels/workbook.xml.rels", BOOK_RELS),
                ("xl/workbook.xml", BOOK),
                (
                    "xl/worksheets/sheet1.xml",
                    "<worksheet><sheetData><row><c r=\"A1\"><v>1</c>",
                ),
            ]),
        ),
        (
            "a relationship that climbs out",
            package(&[(
                "_rels/.rels",
                &ROOT_RELS.replace("xl/workbook.xml", "../../../etc/passwd"),
            )]),
        ),
    ];
    for (case, bytes) in cases {
        assert!(ambre_xlsx::read(&bytes).is_err(), "{case}");
    }
}

#[test]
fn a_sheet_of_odd_cells_reads_what_it_can() {
    let sheet = "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>\
        <row><c><v>1</v></c><c t=\"s\"><v>99</v></c><c t=\"b\"><v>maybe</v></c><c t=\"e\"><v>#WHAT</v></c></row>\
        <row r=\"9999999\"><c r=\"ZZZZ1\"><v>2</v></c></row>\
        <row r=\"3\"><c r=\"A3\"><f t=\"shared\" si=\"7\"/><v>3</v></c><c r=\"B3\" s=\"999\"><v>nan</v></c></row>\
        </sheetData><mergeCells><mergeCell ref=\"A1:XFE9\"/><mergeCell ref=\"B2\"/></mergeCells></worksheet>";
    let bytes = package(&[
        ("_rels/.rels", ROOT_RELS),
        ("xl/_rels/workbook.xml.rels", BOOK_RELS),
        ("xl/workbook.xml", BOOK),
        ("xl/worksheets/sheet1.xml", sheet),
    ]);
    let Ok(read) = ambre_xlsx::read(&bytes) else {
        panic!("the sheet reads");
    };
    let cells = &read.workbook.sheets[0].cells;
    // A1 reads. A string index past the list reads as no value. So do a
    // boolean and an error that spell nothing, a cell past the grid and a NaN.
    assert_eq!(
        cells.first().map(|c| (c.at.to_string(), &c.content)),
        Some(("A1".to_owned(), &Content::Value(Value::Number(1.0))))
    );
    // A shared formula whose master is absent keeps its cached value.
    assert!(
        cells
            .iter()
            .any(|c| c.at.to_string() == "A3" && c.content == Content::Value(Value::Number(3.0)))
    );
    assert!(read.workbook.sheets[0].merges.is_empty());
}
