// ambre-trame/src/logic/tests/group13.rs
// TrameState unit tests (group 13): the data tools and the formats. The sort,
// the shared filter, the cell styles and the number formats. Helpers via
// `use super::*`.

use super::*;
use crate::logic::schema::{SheetContainer, SheetRoot};
use crate::{BorderEdit, BorderPreset, BorderStyle, CellFormat, StylePatch};
use ambre_core::Rgb;
use loro::LoroValue;

const COL_1: Col = Col::new(1);
const ROWS_1: RowCount = RowCount::new(1);

/// Select the rectangle `(r1, c1)`-`(r2, c2)`, the active cell at its far corner.
fn select(state: &mut TrameState, r1: u32, c1: u32, r2: u32, c2: u32) {
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(r2),
        col: Col::new(c2),
        anchor_row: Row::new(r1),
        anchor_col: Col::new(c1),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
}

fn set(state: &mut TrameState, row: u32, col: u32, text: &str) {
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(row),
        col: Col::new(col),
        text: text.to_owned(),
    });
}

fn style(state: &mut TrameState, patch: StylePatch) -> TrameOutcome {
    state.handle_command(TrameCommand::SetCellStyle { patch })
}

/// The deep value of one map of the default sheet.
fn deep(state: &TrameState, root: SheetRoot) -> LoroValue {
    state
        .doc
        .get_map(SheetContainer::of(root, &default_sheet()).as_str())
        .get_deep_value()
}

fn cells(state: &TrameState) -> LoroValue {
    deep(state, SheetRoot::Cells)
}

fn style_entries(state: &TrameState) -> usize {
    match deep(state, SheetRoot::CellStyle) {
        LoroValue::Map(m) => m.len(),
        other => panic!("the style container: {other:?}"),
    }
}

fn sort(state: &mut TrameState, rows: (u32, u32), col: u32, ascending: bool) -> TrameOutcome {
    state.handle_command(TrameCommand::SortRange {
        sheet_id: default_sheet(),
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(rows.0), Col::new(0)),
            crate::CellPos::new(Row::new(rows.1), Col::new(1)),
        ),
        sort_col: ColCount::new(col),
        order: if ascending {
            SortOrder::Ascending
        } else {
            SortOrder::Descending
        },
        header: HeaderRow::Absent,
    })
}

fn filter(state: &mut TrameState, col: u32, kind: FilterKind) -> TrameOutcome {
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: default_sheet(),
        col: Col::new(col),
        kind,
    })
}

fn greater_than(value: &str) -> FilterKind {
    FilterKind::Condition {
        op: FilterOp::Gt,
        value: value.to_owned(),
    }
}

/// A patch that changes no attribute writes nothing: no line, no delta, no
/// undo item. The undo after it takes back the edit before it (`TRAME-189`).
#[test]
fn an_empty_style_patch_writes_nothing() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "kept");
    select(&mut state, 0, 0, 40, 6);
    let before = state.doc.len_ops();
    let out = style(&mut state, StylePatch::default());
    assert!(out.update.is_none());
    assert_eq!(state.doc.len_ops(), before);
    state.handle_command(TrameCommand::Undo);
    assert_eq!(read_cell(&mut state, 0, 0), "");
}

/// A cell returned to the default style leaves no entry in the style
/// container (`TRAME-096`).
#[test]
fn a_cell_back_to_the_default_style_leaves_no_entry() {
    let mut state = TrameState::new();
    select(&mut state, 0, 0, 0, 0);
    style(
        &mut state,
        StylePatch {
            bold: Some(true),
            bg: Some(Some(Rgb::from_bits(0x00ff_0000))),
            ..Default::default()
        },
    );
    assert_eq!(style_entries(&state), 1);
    style(
        &mut state,
        StylePatch {
            bold: Some(false),
            bg: Some(None),
            ..Default::default()
        },
    );
    assert_eq!(style_entries(&state), 0);
}

/// Each preset over a 3x3 selection reaches only the edges it names
/// (`TRAME-099`). The edge count tells the presets apart on its own.
#[test]
fn a_border_preset_reaches_only_its_edges() {
    // [top, right, bottom, left] of the cell at (r, c), for each preset.
    type Edges = fn(u32, u32) -> [bool; 4];
    let cases: [(BorderPreset, usize, Edges); 10] = [
        (BorderPreset::None, 0, |_, _| [false; 4]),
        (BorderPreset::All, 36, |_, _| [true; 4]),
        (BorderPreset::Outline, 12, |r, c| {
            [r == 0, c == 2, r == 2, c == 0]
        }),
        (BorderPreset::Inner, 24, |r, c| [r > 0, c < 2, r < 2, c > 0]),
        (BorderPreset::InnerHorizontal, 12, |r, _| {
            [r > 0, false, r < 2, false]
        }),
        (BorderPreset::InnerVertical, 12, |_, c| {
            [false, c < 2, false, c > 0]
        }),
        (BorderPreset::Top, 3, |r, _| [r == 0, false, false, false]),
        (BorderPreset::Bottom, 3, |r, _| {
            [false, false, r == 2, false]
        }),
        (BorderPreset::Left, 3, |_, c| [false, false, false, c == 0]),
        (BorderPreset::Right, 3, |_, c| [false, c == 2, false, false]),
    ];
    for (preset, count, expected) in cases {
        let mut state = TrameState::new();
        select(&mut state, 0, 0, 2, 2);
        style(
            &mut state,
            StylePatch {
                border: Some(BorderEdit {
                    preset,
                    style: BorderStyle::Solid,
                    color: None,
                }),
                ..Default::default()
            },
        );
        let mut seen = 0;
        for r in 0..3 {
            for c in 0..3 {
                let b = state
                    .styles
                    .get(&at(r, c))
                    .map(|s| s.borders)
                    .unwrap_or_default();
                let got = [
                    b.top.is_some(),
                    b.right.is_some(),
                    b.bottom.is_some(),
                    b.left.is_some(),
                ];
                assert_eq!(got, expected(r, c), "{preset:?} at ({r}, {c})");
                seen += got.iter().filter(|&&e| e).count();
            }
        }
        assert_eq!(seen, count, "{preset:?}");
    }
}

/// A restyle across a merge styles the master and leaves every covered cell
/// without a style of its own (`TRAME-098`).
#[test]
fn a_covered_cell_takes_no_style() {
    let mut state = TrameState::new();
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::new(0), Col::new(0)),
            crate::CellPos::new(Row::new(1), Col::new(1)),
        ),
    });
    select(&mut state, 0, 0, 2, 2);
    style(
        &mut state,
        StylePatch {
            bg: Some(Some(Rgb::from_bits(0x0000_ff00))),
            border: Some(BorderEdit {
                preset: BorderPreset::All,
                style: BorderStyle::Thick,
                color: None,
            }),
            ..Default::default()
        },
    );
    assert!(state.styles.contains_key(&at(0, 0)), "the master is styled");
    for covered in [at(0, 1), at(1, 0), at(1, 1)] {
        assert!(!state.styles.contains_key(&covered), "{covered:?}");
    }
    // Nine cells, three covered: six entries reach the document.
    assert_eq!(style_entries(&state), 6);
}

/// Styling a selection leaves every value in it as it was: the text, the
/// formula and what they compute (`TRAME-191`).
#[test]
fn styling_keeps_every_value() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "12");
    set(&mut state, 0, 1, "=A1*2");
    set(&mut state, 0, 2, "word");
    let before = cells(&state);
    select(&mut state, 0, 0, 0, 2);
    style(
        &mut state,
        StylePatch {
            bold: Some(true),
            format: Some(CellFormat::Number),
            decimals: Some(Some(0)),
            ..Default::default()
        },
    );
    assert_eq!(cells(&state), before);
    assert_eq!(read_raw(&mut state, 0, 1), "=A1*2");
    assert_eq!(read_cell(&mut state, 0, 1), "24");
    assert_eq!(read_cell(&mut state, 0, 2), "word");
}

/// Styling a selection moves neither the cursor nor the selection
/// (`TRAME-192`).
#[test]
fn styling_keeps_the_selection() {
    let mut state = TrameState::new();
    select(&mut state, 1, 1, 3, 4);
    let (active, bounds) = (state.active, state.selection_bounds());
    style(&mut state, bold_patch(true));
    style(
        &mut state,
        StylePatch {
            border: Some(BorderEdit {
                preset: BorderPreset::Outline,
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    assert_eq!(state.active, active);
    assert_eq!(state.selection_bounds(), bounds);
    assert_eq!(state.selected_cells().len(), 12);
}

/// A word under a number format shows unchanged (`TRAME-103`).
#[test]
fn a_word_ignores_its_number_format() {
    for format in [
        CellFormat::Number,
        CellFormat::Percent,
        CellFormat::Currency,
        CellFormat::Scientific,
        CellFormat::Fraction,
    ] {
        let mut state = TrameState::new();
        set(&mut state, 0, 0, "rain");
        select(&mut state, 0, 0, 0, 0);
        style(
            &mut state,
            StylePatch {
                format: Some(format.clone()),
                ..Default::default()
            },
        );
        assert_eq!(read_cell(&mut state, 0, 0), "rain", "{format:?}");
    }
}

/// A step of the decimal count changes how a value reads, never the value.
/// The engine holds the same result, the document the same text
/// (`TRAME-158`).
#[test]
fn a_decimal_step_keeps_the_computed_value() {
    let mut state = TrameState::new();
    set(&mut state, 0, 0, "=1/3");
    select(&mut state, 0, 0, 0, 0);
    style(
        &mut state,
        StylePatch {
            format: Some(CellFormat::Number),
            decimals: Some(Some(2)),
            ..Default::default()
        },
    );
    assert_eq!(read_cell(&mut state, 0, 0), "0.33");
    let id = state.active_cell_id(CellPos::new(Row::new(0), Col::new(0)));
    let value = format!("{:?}", state.engine.evaluated_value(id));
    let before = cells(&state);
    style(
        &mut state,
        StylePatch {
            decimals_step: Some(2),
            ..Default::default()
        },
    );
    assert_eq!(read_cell(&mut state, 0, 0), "0.3333");
    assert_eq!(format!("{:?}", state.engine.evaluated_value(id)), value);
    assert_eq!(cells(&state), before);
}

/// A filter over a column of formulas tests what they compute: their text,
/// `=2*3`, is no number at all (`TRAME-FEAT-034`).
#[test]
fn a_filter_reads_the_computed_value() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["=2*3", "=1+1", "=4*2"]);
    filter(&mut state, 0, greater_than("5"));
    assert!(!is_filtered(&mut state, 0, 0), "6 shows");
    assert!(is_filtered(&mut state, 1, 0), "2 hides");
    assert!(!is_filtered(&mut state, 2, 0), "8 shows");
    filter(&mut state, 0, FilterKind::Values(vec!["8".to_owned()]));
    assert!(is_filtered(&mut state, 0, 0), "6 hides");
    assert!(!is_filtered(&mut state, 2, 0), "8 shows");
}

/// A second filter replaces the first: the rows the first hid show again
/// (`TRAME-FEAT-032`).
#[test]
fn a_second_filter_replaces_the_first() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["1", "9", "9"]);
    set(&mut state, 0, 1, "9");
    set(&mut state, 1, 1, "1");
    set(&mut state, 2, 1, "9");
    filter(&mut state, 0, greater_than("5"));
    assert!(is_filtered(&mut state, 0, 0));
    let out = filter(&mut state, 1, greater_than("5"));
    assert!(!is_filtered(&mut state, 0, 0), "the first filter is gone");
    assert!(is_filtered(&mut state, 1, 0));
    assert!(!is_filtered(&mut state, 2, 0));
    assert!(out.display.iter().any(|e| matches!(
        e,
        TrameEvent::FilterApplied {
            hidden_row_count: ROWS_1,
            col: COL_1
        }
    )));
}

/// Applying a filter and clearing it leave every cell of the sheet as it was,
/// in every column (`TRAME-FEAT-035`, `TRAME-FEAT-039`).
#[test]
fn a_filter_and_its_clear_keep_every_cell() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["1", "5", "9"]);
    set(&mut state, 0, 1, "a");
    set(&mut state, 1, 2, "=SUM(A1:A3)");
    set(&mut state, 2, 3, "=SUBTOTAL(9,A1:A3)");
    let before = cells(&state);
    let total = read_cell(&mut state, 1, 2);
    filter(&mut state, 0, greater_than("4"));
    assert!(is_filtered(&mut state, 0, 0));
    assert_eq!(cells(&state), before, "the apply");
    assert_eq!(read_cell(&mut state, 1, 2), total);
    state.handle_command(TrameCommand::ClearFilter {
        sheet_id: default_sheet(),
    });
    assert!(!is_filtered(&mut state, 0, 0));
    assert_eq!(cells(&state), before, "the clear");
    assert_eq!(read_cell(&mut state, 1, 2), total);
}

/// A sort moves rows and rewrites no cell: the cell container is the same
/// before and after, while the order changes (`TRAME-178`).
#[test]
fn a_sort_rewrites_no_cell() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["3", "1", "2"]);
    set(&mut state, 0, 1, "=A1*10");
    let before = cells(&state);
    sort(&mut state, (0, 2), 0, true);
    assert_eq!(read_col(&mut state, 0, 2, 80), vec!["1", "2", "3"]);
    assert_eq!(read_cell(&mut state, 2, 1), "30", "the formula went along");
    assert_eq!(cells(&state), before);
}

/// Two rows whose keys compare equal keep their order, ascending and
/// descending alike (`TRAME-180`).
#[test]
fn equal_keys_keep_their_order() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["1", "0", "1", "1"]);
    for (row, label) in ["x", "z", "y", "w"].iter().enumerate() {
        set(&mut state, row as u32, 1, label);
    }
    sort(&mut state, (0, 3), 0, true);
    assert_eq!(read_col(&mut state, 1, 3, 81), vec!["z", "x", "y", "w"]);
    sort(&mut state, (0, 3), 0, false);
    assert_eq!(read_col(&mut state, 1, 3, 82), vec!["x", "y", "w", "z"]);
}

/// Values of different types order by the type rank: numbers, then text,
/// then booleans, then errors, then blanks (`TRAME-111`).
#[test]
fn mixed_types_order_by_rank() {
    let mut state = TrameState::new();
    seed_col_a(
        &mut state,
        &["b", "=1=1", "", "2", "=1/0", "a", "1", "=1=2"],
    );
    sort(&mut state, (0, 7), 0, true);
    let got = read_col(&mut state, 0, 7, 83);
    assert_eq!(&got[..4], ["1", "2", "a", "b"]);
    // The case the grid shows a boolean in is not this rule's.
    let booleans: Vec<String> = got[4..6].iter().map(|s| s.to_uppercase()).collect();
    assert_eq!(booleans, ["FALSE", "TRUE"]);
    assert!(got[6].starts_with('#'), "the error comes before the blank");
    assert_eq!(got[7], "");
}

/// A descending sort reverses the whole comparison, so the blank orders
/// first (`TRAME-112`).
#[test]
fn a_descending_sort_puts_the_blank_first() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["2", "", "1", "3"]);
    sort(&mut state, (0, 3), 0, false);
    assert_eq!(read_col(&mut state, 0, 3, 84), vec!["", "3", "2", "1"]);
}

/// A hidden row takes part in the order: the 9 the filter hides goes to the
/// bottom of an ascending sort (`TRAME-114`).
#[test]
fn a_hidden_row_is_ordered_too() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["9", "1", "2"]);
    filter(
        &mut state,
        0,
        FilterKind::Condition {
            op: FilterOp::Lt,
            value: "5".to_owned(),
        },
    );
    assert!(is_filtered(&mut state, 0, 0));
    sort(&mut state, (0, 2), 0, true);
    assert_eq!(read_col(&mut state, 0, 2, 85), vec!["1", "2", "9"]);
    assert!(is_filtered(&mut state, 2, 0), "the hidden row moved down");
}

/// A sort over fewer than two sortable rows writes nothing and is not
/// acknowledged. The cases are one row, and two rows under a header
/// (`TRAME-185`).
#[test]
fn a_sort_of_fewer_than_two_rows_changes_nothing() {
    let mut state = TrameState::new();
    seed_col_a(&mut state, &["9", "1"]);
    let before = state.doc.len_ops();
    for (row_end, header) in [(0, HeaderRow::Absent), (1, HeaderRow::Present)] {
        let out = state.handle_command(TrameCommand::SortRange {
            sheet_id: default_sheet(),
            rect: crate::CellRect::spanning(
                crate::CellPos::new(Row::new(0), Col::new(0)),
                crate::CellPos::new(Row::new(row_end), Col::new(0)),
            ),
            sort_col: ColCount::new(0),
            order: SortOrder::Ascending,
            header,
        });
        assert!(out.update.is_none(), "{row_end} {header:?}");
        assert!(
            !out.display
                .iter()
                .any(|e| matches!(e, TrameEvent::SortComplete)),
            "{row_end} {header:?}"
        );
    }
    assert_eq!(state.doc.len_ops(), before);
    assert_eq!(read_col(&mut state, 0, 1, 86), vec!["9", "1"]);
}
