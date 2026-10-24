// ambre-trame/src/logic/convergence_tests.rs
// ============================================================================
// CRDT convergence property tests (Epic `ET`). Mandatory for every collaborative
// CRDT state: `ambre-spec-testing.md`.
//
// `proptest` generates interleaved sequences of the `TrameCommand`s that
// mutate the document. They are `SetCellText`, row and column inserts and
// deletes, `SortRange`, merge and `FillRange`. A paste, the batch clear of a
// cut, and a name defined or removed join them.
//
// Each op goes to one of two peers, then the full CRDT history is exchanged
// both ways. The invariant: after the exchange the two peers
// observe an *identical* materialized grid -- same text, same formula results
// (recompute determinism), same merge geometry. This exercises the own-uid cell
// model (`trame-rows-and-columns`, `trame-formula-engine`). References must
// follow data across a concurrent sort, and structural ops on both peers must
// reconcile to one converged order.
//
// The exchange uses full snapshots rather than incremental `SendUpdate` deltas.
// Each peer mints an implicit-`s0` op in `new()` under a local origin. An
// incremental delta would then carry a dangling dependency on the other peer's
// private op, the same reason `cross_sheet_reference_converges_across_peers_on_
// rename` merges snapshots. A snapshot carries the full history, so importing
// it yields the union -- the CRDT convergence guarantee.
// ============================================================================

use ambre_core::Rgb;
use proptest::prelude::*;

use ambre_bus::{
    BorderEdit, BorderPreset, BorderStyle, Borders, CellFormat, CellStyle, ClipboardCell,
    ClipboardPayload, FillMode, FilterKind, FilterOp, HAlign, HeaderRow, SortOrder, StylePatch,
    VAlign,
};

use super::TrameState;
use crate::{CellPos, Col, ColCount, MergeSpan, Row, RowCount};
use crate::{TrameCommand, TrameEvent};

/// Bounded grid the generators address. Kept small so a sort/merge/fill spans a
/// handful of cells and shrinking stays fast, while still forcing overlapping
/// structural edits between the two peers.
const ROWS: u32 = 5;
const COLS: u32 = 4;

/// The default (implicit-first) sheet id every fresh `TrameState` carries.
const SHEET: &str = "s0";

/// One document mutation. Every variant carries explicit geometry, so the
/// generators never depend on a tracked active cell / selection.
#[derive(Debug, Clone)]
enum Op {
    SetText {
        row: u32,
        col: u32,
        text: String,
    },
    InsertRow {
        at: u32,
    },
    DeleteRow {
        at: u32,
    },
    InsertColumn {
        at: u32,
    },
    DeleteColumn {
        at: u32,
    },
    MoveRow {
        from: u32,
        to: u32,
    },
    MoveColumn {
        from: u32,
        to: u32,
    },
    Sort {
        ascending: bool,
    },
    Merge {
        row: u32,
        col: u32,
        row_end: u32,
        col_end: u32,
    },
    Unmerge {
        row: u32,
        col: u32,
    },
    Fill,
    /// Apply a shared filter on column 0 (a plain CRDT mutation). Concurrent
    /// filters on the two peers resolve by Loro LWW. That folds the explicit
    /// `concurrent_filters_converge_lww` case into the property test: the
    /// converged `is_filtered` set is part of the asserted snapshot.
    Filter {
        op: FilterOp,
        value: u32,
    },
    ClearFilter,
    /// Paste a block of texts at a target cell (`TRAME-UI-124`). Blank texts
    /// are omitted from the payload, as a copy omits blank cells.
    Paste {
        row: u32,
        col: u32,
        texts: Vec<String>,
    },
    /// Clear a rectangle in one batch: what a cut sends once its paste lands.
    ClearBatch {
        row: u32,
        col: u32,
        rows: u32,
        cols: u32,
    },
    /// Define the one name the generator knows, `TOTAL`, over a reference.
    /// Two peers defining it at once race for one definition (`TRAME-068`).
    DefineName {
        reference: &'static str,
    },
    DeleteName,
}

impl Op {
    /// Translate to the `TrameCommand` the Logic dispatcher would receive.
    fn into_command(self) -> TrameCommand {
        match self {
            Op::SetText { row, col, text } => TrameCommand::SetCellText {
                row: Row::new(row),
                col: Col::new(col),
                text,
            },
            Op::InsertRow { at } => TrameCommand::InsertRow { at: Row::new(at) },
            Op::DeleteRow { at } => TrameCommand::DeleteRow { at: Row::new(at) },
            Op::InsertColumn { at } => TrameCommand::InsertColumn { at: Col::new(at) },
            Op::DeleteColumn { at } => TrameCommand::DeleteColumn { at: Col::new(at) },
            Op::MoveRow { from, to } => TrameCommand::MoveRow {
                from: Row::new(from),
                to: Row::new(to),
            },
            Op::MoveColumn { from, to } => TrameCommand::MoveColumn {
                from: Col::new(from),
                to: Col::new(to),
            },
            Op::Sort { ascending } => TrameCommand::SortRange {
                sheet_id: ambre_bus::SheetId(SHEET.to_owned()),
                rect: crate::CellRect::spanning(
                    crate::CellPos::new(Row::new(0), Col::new(0)),
                    crate::CellPos::new(Row::new(ROWS - 1), Col::new(COLS - 1)),
                ),
                sort_col: ColCount::new(0),
                order: if ascending {
                    SortOrder::Ascending
                } else {
                    SortOrder::Descending
                },
                header: HeaderRow::Absent,
            },
            Op::Merge {
                row,
                col,
                row_end,
                col_end,
            } => TrameCommand::MergeCells {
                rect: crate::CellRect::spanning(
                    crate::CellPos::new(Row::new(row), Col::new(col)),
                    crate::CellPos::new(Row::new(row_end), Col::new(col_end)),
                ),
            },
            Op::Unmerge { row, col } => TrameCommand::UnmergeCells {
                row: Row::new(row),
                col: Col::new(col),
            },
            Op::Filter { op, value } => TrameCommand::ApplyFilter {
                sheet_id: ambre_bus::SheetId(SHEET.to_owned()),
                col: Col::new(0),
                kind: FilterKind::Condition {
                    op,
                    value: value.to_string(),
                },
            },
            Op::ClearFilter => TrameCommand::ClearFilter {
                sheet_id: ambre_bus::SheetId(SHEET.to_owned()),
            },
            Op::Fill => TrameCommand::FillRange {
                sheet_id: ambre_bus::SheetId(SHEET.to_owned()),
                src: crate::CellRect::spanning(
                    crate::CellPos::new(Row::new(0), Col::new(0)),
                    crate::CellPos::new(Row::new(0), Col::new(0)),
                ),
                dest: crate::CellRect::spanning(
                    crate::CellPos::new(Row::new(0), Col::new(0)),
                    crate::CellPos::new(Row::new(ROWS - 1), Col::new(0)),
                ),
                mode: FillMode::Auto,
            },
            Op::Paste { row, col, texts } => TrameCommand::PasteRange {
                target_row: Row::new(row),
                target_col: Col::new(col),
                payload: ClipboardPayload {
                    src_row: Row::new(0),
                    src_col: Col::new(0),
                    rows: RowCount::new(texts.len() as u32),
                    cols: ColCount::new(1),
                    cells: texts
                        .into_iter()
                        .enumerate()
                        .filter(|(_, raw)| !raw.is_empty())
                        .map(|(dr, raw)| ClipboardCell {
                            dr: RowCount::new(dr as u32),
                            dc: ColCount::new(0),
                            raw,
                            bold: false,
                            italic: false,
                            style: CellStyle::default(),
                        })
                        .collect(),
                },
            },
            Op::ClearBatch {
                row,
                col,
                rows,
                cols,
            } => TrameCommand::SetCellsBatch {
                cells: (row..row + rows)
                    .flat_map(|r| {
                        (col..col + cols)
                            .map(move |c| (CellPos::new(Row::new(r), Col::new(c)), String::new()))
                    })
                    .collect(),
            },
            Op::DefineName { reference } => TrameCommand::DefineName {
                name: crate::test_support::name("TOTAL"),
                reference: reference.to_owned(),
            },
            Op::DeleteName => TrameCommand::DeleteName {
                name: crate::test_support::name("TOTAL"),
            },
        }
    }
}

/// Cell contents the generator picks from: plain numbers, short strings, and a
/// few formulas that reference other window cells. A concurrent sort can then
/// move the data out from under them, the `trame-formula-engine` guarantee
/// under test.
fn text_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        (0i64..20).prop_map(|n| n.to_string()),
        Just("foo".to_owned()),
        Just("bar".to_owned()),
        Just("=A1+B1".to_owned()),
        Just("=SUM(A1:A3)".to_owned()),
        Just("=B2*2".to_owned()),
        Just("=SUM(TOTAL)".to_owned()),
        Just(String::new()),
    ]
}

/// A single op over the bounded grid.
fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        // Weight value edits highest so structural ops act on populated cells.
        4 => (0..ROWS, 0..COLS, text_strategy())
            .prop_map(|(row, col, text)| Op::SetText { row, col, text }),
        1 => (0..ROWS).prop_map(|at| Op::InsertRow { at }),
        1 => (0..ROWS).prop_map(|at| Op::DeleteRow { at }),
        1 => (0..COLS).prop_map(|at| Op::InsertColumn { at }),
        1 => (0..COLS).prop_map(|at| Op::DeleteColumn { at }),
        1 => (0..ROWS, 0..ROWS).prop_map(|(from, to)| Op::MoveRow { from, to }),
        1 => (0..COLS, 0..COLS).prop_map(|(from, to)| Op::MoveColumn { from, to }),
        1 => any::<bool>().prop_map(|ascending| Op::Sort { ascending }),
        1 => (0..ROWS - 1, 0..COLS - 1).prop_map(|(row, col)| Op::Merge {
            row,
            col,
            row_end: row + 1,
            col_end: col + 1,
        }),
        1 => (0..ROWS, 0..COLS).prop_map(|(row, col)| Op::Unmerge { row, col }),
        1 => Just(Op::Fill),
        // A few filter operators over the value window so concurrent filters on
        // the two peers race for the LWW-elected hidden set.
        1 => (
            prop_oneof![
                Just(FilterOp::Gt),
                Just(FilterOp::Lt),
                Just(FilterOp::Gte),
                Just(FilterOp::Ne),
            ],
            0..ROWS,
        )
            .prop_map(|(op, value)| Op::Filter { op, value }),
        1 => Just(Op::ClearFilter),
        1 => (
            0..ROWS,
            0..COLS,
            proptest::collection::vec(text_strategy(), 1..4),
        )
            .prop_map(|(row, col, texts)| Op::Paste { row, col, texts }),
        1 => (0..ROWS, 0..COLS, 1..3u32, 1..3u32).prop_map(|(row, col, rows, cols)| {
            Op::ClearBatch {
                row,
                col,
                rows,
                cols,
            }
        }),
        1 => prop_oneof![Just("A1:A3"), Just("B1:B4"), Just("C2")]
            .prop_map(|reference| Op::DefineName { reference }),
        1 => Just(Op::DeleteName),
    ]
}

/// A program: a sequence of `(peer, op)` steps. `peer == false` -> A, `true` -> B.
fn program_strategy() -> impl Strategy<Value = Vec<(bool, Op)>> {
    proptest::collection::vec((any::<bool>(), op_strategy()), 0..14)
}

/// The materialized grid over the bounded window, as a peer would render it.
/// Captures everything two converged peers must agree on: formula results
/// (`text`), formatting (`bold`), merge geometry, and the shared-filter hidden
/// state (`is_filtered`) elected by LWW.
type Snapshot = Vec<(CellPos, String, bool, bool, Option<MergeSpan>, bool)>;

fn read_window(state: &mut TrameState) -> Snapshot {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(u64::MAX),
        row_start: Row::new(0),
        row_end: Row::new(ROWS - 1),
        col_start: Col::new(0),
        col_end: Col::new(COLS - 1),
    });
    let cells = evts
        .display
        .iter()
        .find_map(|e| match e {
            TrameEvent::ViewportData { cells, .. } => Some(cells),
            _ => None,
        })
        .expect("ScrollViewport yields ViewportData");
    let mut rows: Snapshot = cells
        .iter()
        .map(|c| {
            (
                CellPos::new(c.row, c.col),
                c.text.clone(),
                c.bold,
                c.is_merged,
                c.merge_span,
                c.is_filtered,
            )
        })
        .collect();
    rows.sort_by_key(|(at, ..)| *at);
    rows
}

proptest! {
    // Convergence is the contract; default 256 cases is plenty of interleavings
    // for a 14-step program over a 5x4 grid and keeps the suite fast.
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Two peers applying an interleaved op stream converge to an identical
    /// grid after exchanging full history both ways. The op set holds a paste
    /// and a batch clear over cells the other peer edits (`TRAME-UI-124`).
    #[test]
    fn two_peers_converge_under_interleaved_ops(program in program_strategy()) {
        // Shared base: A seeds it, B imports it, so both share the implicit-s0
        // history before diverging. Any op past the seeded extent simply
        // materializes lazily -- identically on both once history is merged.
        let mut a = TrameState::new();
        for (r, text) in ["10", "5", "20", "1"].iter().enumerate() {
            a.handle_command(TrameCommand::SetCellText {
                row: Row::new(r as u32),
                col: Col::new(0),
                text: (*text).to_owned(),
            });
        }
        let base = a.export_snapshot();
        let mut b = TrameState::new();
        b
            .import_snapshot(&base)
            .expect("a snapshot this build wrote reads back at the current schema");

        // Route each generated op to its peer.
        for (peer, op) in program {
            let target = if peer { &mut b } else { &mut a };
            target.handle_command(op.into_command());
        }

        // Full bidirectional exchange: snapshots taken *before* either import,
        // so each carries only that peer's history; importing both yields the
        // union on both sides.
        let sa = a.export_snapshot();
        let sb = b.export_snapshot();
        a.import_snapshot(&sb)
        .expect("the snapshot imports");
        b.import_snapshot(&sa)
        .expect("the snapshot imports");

        let grid_a = read_window(&mut a);
        let grid_b = read_window(&mut b);
        prop_assert_eq!(
            &grid_a,
            &grid_b,
            "peers diverged after full history exchange"
        );
        // One definition of each name on both peers (`TRAME-068`); the grid
        // above reads it through `=SUM(TOTAL)`.
        let names = |s: &TrameState| s.doc.get_map(crate::logic::Root::NamedRanges.as_str()).get_deep_value();
        prop_assert_eq!(names(&a), names(&b), "the named ranges diverged");

        // Recompute determinism: a fresh peer rebuilt from the converged
        // snapshot renders the same grid (no derived-state path dependence).
        let mut c = TrameState::new();
        c
            .import_snapshot(&a.export_snapshot())
            .expect("a snapshot this build wrote reads back at the current schema");
        prop_assert_eq!(read_window(&mut c), grid_a, "reload diverged from live peer");
    }
}

// ============================================================================
// Concurrent whole-cell style edits (`trame-schema`, `TRAME-016`).
//
// A cell's whole style is one atomic LWW register in `cellstyle_{sheet}`, keyed
// by the cell's stable id pair. An earlier schema scattered it over up to 11
// per-attribute maps, where two peers restyling the same cell raced once per
// container. Nothing forced one writer to win every register.
//
// The invariant under test is the atomicity that replaced it. The converged
// cell must carry a style one of the two peers actually wrote, never a
// per-attribute blend of both.
// ============================================================================

/// The observable style of a cell, as a peer renders it. Number format and
/// decimals are absent on purpose: `DisplayCell` does not surface them, since
/// the Logic thread folds them into `text`. They cannot be asserted here even
/// though the register carries them.
type StyleView = (
    bool,
    bool,
    bool,
    bool,
    HAlign,
    VAlign,
    bool,
    Option<Rgb>,
    Option<Rgb>,
    Option<String>,
    Option<ambre_core::FontSize>,
    Borders,
);

/// Read A1's style through a viewport request, the way the UI thread sees it.
fn read_style(state: &mut TrameState) -> StyleView {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(u64::MAX),
        row_start: Row::new(0),
        row_end: Row::new(0),
        col_start: Col::new(0),
        col_end: Col::new(0),
    });
    let cells = evts
        .display
        .iter()
        .find_map(|e| match e {
            TrameEvent::ViewportData { cells, .. } => Some(cells),
            _ => None,
        })
        .expect("ScrollViewport yields ViewportData");
    let c = cells
        .iter()
        .find(|c| c.row == Row::new(0) && c.col == Col::new(0))
        .expect("A1 is materialized");
    (
        c.bold,
        c.italic,
        c.underline,
        c.strikethrough,
        c.h_align,
        c.v_align,
        c.wrap,
        c.fg,
        c.bg,
        c.font.clone(),
        c.size,
        c.borders,
    )
}

/// Select A1, then rewrite its whole style.
fn apply_style(state: &mut TrameState, patch: StylePatch) {
    state.handle_command(TrameCommand::SetActiveCell {
        row: Row::new(0),
        col: Col::new(0),
        anchor_row: Row::new(0),
        anchor_col: Col::new(0),
        extra: vec![],
        kind: crate::SelectionKind::Cells,
    });
    state.handle_command(TrameCommand::SetCellStyle { patch });
}

/// The style the shared base pins on A1 before the peers diverge. Touches every
/// style container, so both concurrent edits below emit real ops in each of
/// them. A `clear` over an unstyled cell would delete absent keys, a no-op, and
/// the race would never happen.
fn base_style() -> StylePatch {
    StylePatch {
        bold: Some(true),
        underline: Some(true),
        wrap: Some(true),
        h_align: Some(HAlign::Right),
        v_align: Some(VAlign::Bottom),
        fg: Some(Some(Rgb::from_bits(0x00FF_0000))),
        bg: Some(Some(Rgb::from_bits(0x0000_FF00))),
        font: Some(Some(ambre_core::FontFamily::from_label("Arial"))),
        size: Some(Some(ambre_core::FontSize::from_whole_points(18))),
        format: Some(CellFormat::Percent),
        border: Some(BorderEdit {
            preset: BorderPreset::All,
            style: BorderStyle::Solid,
            color: Some(Rgb::from_bits(0x00FF_00FF)),
        }),
        ..Default::default()
    }
}

/// Peer A's edit: clear the cell's style back to the grid defaults.
fn clear_style() -> StylePatch {
    StylePatch {
        bold: Some(false),
        italic: Some(false),
        underline: Some(false),
        strikethrough: Some(false),
        wrap: Some(false),
        h_align: Some(HAlign::default()),
        v_align: Some(VAlign::default()),
        fg: Some(None),
        bg: Some(None),
        font: Some(None),
        size: Some(None),
        format: Some(CellFormat::default()),
        decimals: Some(None),
        decimals_step: None,
        border: Some(BorderEdit {
            preset: BorderPreset::None,
            ..Default::default()
        }),
    }
}

/// Peer B's edit: rewrite the cell with a different whole style. Every field
/// differs from [`base_style`], so a register B loses is observable.
fn other_style() -> StylePatch {
    StylePatch {
        bold: Some(false),
        italic: Some(true),
        underline: Some(false),
        strikethrough: Some(true),
        wrap: Some(false),
        h_align: Some(HAlign::Left),
        v_align: Some(VAlign::Top),
        fg: Some(Some(Rgb::from_bits(0x0000_00FF))),
        bg: Some(Some(Rgb::from_bits(0x00FF_FF00))),
        font: Some(Some(ambre_core::FontFamily::from_label("Times"))),
        size: Some(Some(ambre_core::FontSize::from_whole_points(9))),
        format: Some(CellFormat::General),
        decimals: Some(None),
        decimals_step: None,
        border: Some(BorderEdit {
            preset: BorderPreset::Outline,
            style: BorderStyle::Dashed,
            color: Some(Rgb::from_bits(0x0000_FFFF)),
        }),
    }
}

/// Peer A clears A1's style while peer B rewrites it, concurrently. After a
/// full history exchange both peers must agree (CRDT convergence). The
/// style they agree on must also be one of the two written styles, not a
/// per-container blend of A's and B's registers.
#[test]
fn concurrent_whole_cell_style_edits_do_not_blend_across_containers() {
    // Shared base: A1 styled, history known to both peers.
    let mut a = TrameState::new();
    a.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "42".to_owned(),
    });
    apply_style(&mut a, base_style());
    let base = a.export_snapshot();
    let mut b = TrameState::new();
    b.import_snapshot(&base)
        .expect("a snapshot this build wrote reads back at the current schema");

    // Concurrent whole-cell rewrites of the same cell.
    apply_style(&mut a, clear_style());
    apply_style(&mut b, other_style());

    // The style each peer intended, as it reads back locally before exchange.
    let a_intent = read_style(&mut a);
    let b_intent = read_style(&mut b);
    assert_ne!(a_intent, b_intent, "the two edits must actually differ");

    // Full bidirectional exchange.
    let sa = a.export_snapshot();
    let sb = b.export_snapshot();
    a.import_snapshot(&sb).expect("the snapshot imports");
    b.import_snapshot(&sa).expect("the snapshot imports");

    let conv_a = read_style(&mut a);
    let conv_b = read_style(&mut b);
    assert_eq!(conv_a, conv_b, "peers diverged after full history exchange");

    assert!(
        conv_a == a_intent || conv_a == b_intent,
        "converged style is a blend neither peer wrote:\n\
         converged: {conv_a:?}\n\
         peer A wrote: {a_intent:?}\n\
         peer B wrote: {b_intent:?}"
    );
}

/// Regression: a whole-cell style edit must stay atomic even against a peer that
/// touches only a *subset* of the style containers. Two full concurrent rewrites
/// already converge atomically (each cell's `format` and rich-`style` commits sit
/// at adjacent Lamports, so the same writer wins every register). The gap the old
/// multi-container schema left open is asymmetric edits: a colour-only patch does
/// not carry bold, so `set_cell_style` skips the bold container entirely.
///
/// Peer A recolours A1 (text colour only); peer B, concurrently, rewrites A1's
/// whole style (bold + both colours). The bold container is written by B alone,
/// while the text-colour container races. If A wins its colour, the cell
/// converges on B's bold plus A's colour, a style neither peer ever wrote. A
/// single `cellstyle_` register makes the edit atomic: one race, one winner,
/// always a style one peer actually wrote.
#[test]
fn subset_style_edit_does_not_tear_a_whole_cell_rewrite() {
    // Shared base: plain A1 with a value, no style.
    let mut a = TrameState::new();
    a.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "x".to_owned(),
    });
    let base = a.export_snapshot();
    let mut b = TrameState::new();
    b.import_snapshot(&base)
        .expect("a snapshot this build wrote reads back at the current schema");

    // Peer A runs several unrelated edits first, so its recolour op carries a
    // strictly higher Lamport than peer B's write. It wins the text-colour
    // register outright, the exact ordering that tore the cell under the old
    // schema. The patch carries no bold, so it never touches the bold container.
    for i in 0..5 {
        a.handle_command(TrameCommand::SetCellText {
            row: Row::new(10),
            col: Col::new(0),
            text: i.to_string(),
        });
    }
    apply_style(
        &mut a,
        StylePatch {
            fg: Some(Some(Rgb::from_bits(0x00FF_0000))),
            ..Default::default()
        },
    );
    let a_intent = read_style(&mut a);

    // Peer B: one whole-cell rewrite -- bold plus different text/fill colours.
    apply_style(
        &mut b,
        StylePatch {
            bold: Some(true),
            fg: Some(Some(Rgb::from_bits(0x0000_00FF))),
            bg: Some(Some(Rgb::from_bits(0x0044_5566))),
            ..Default::default()
        },
    );
    let b_intent = read_style(&mut b);
    assert_ne!(a_intent, b_intent, "the two edits must actually differ");

    // Full bidirectional exchange.
    let sa = a.export_snapshot();
    let sb = b.export_snapshot();
    a.import_snapshot(&sb).expect("the snapshot imports");
    b.import_snapshot(&sa).expect("the snapshot imports");

    let conv_a = read_style(&mut a);
    let conv_b = read_style(&mut b);
    assert_eq!(conv_a, conv_b, "peers diverged after full history exchange");
    assert!(
        conv_a == a_intent || conv_a == b_intent,
        "torn style neither peer wrote:\n\
         converged: {conv_a:?}\n\
         peer A wrote: {a_intent:?}\n\
         peer B wrote: {b_intent:?}"
    );
}

/// The tab order the strip shows: the ids of the `SheetList` event.
fn tab_order(state: &TrameState) -> Vec<String> {
    match state.sheet_list_event() {
        TrameEvent::SheetList { sheets, .. } => sheets.into_iter().map(|t| t.id.0).collect(),
        other => panic!("expected SheetList, got {other:?}"),
    }
}

/// Two users reorder tabs at once, from a stored order that does not list the
/// implicit first sheet (`TRAME-074`). Both peers end on one order, and that
/// order shows each sheet once.
#[test]
fn concurrent_tab_moves_converge_on_one_order() {
    let mut a = TrameState::new();
    a.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Two"),
    });
    a.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Three"),
    });
    let base = a.export_snapshot();
    let mut b = TrameState::new();
    b.import_snapshot(&base)
        .expect("a snapshot this build wrote reads back at the current schema");
    let ids = tab_order(&a);
    assert_eq!(ids.len(), 3);

    a.handle_command(TrameCommand::MoveSheet {
        sheet_id: ambre_bus::SheetId(ids[2].clone()),
        to_index: crate::SheetTabPos::ZERO,
    });
    b.handle_command(TrameCommand::MoveSheet {
        sheet_id: ambre_bus::SheetId(ids[0].clone()),
        to_index: crate::SheetTabPos::new(2),
    });

    let sa = a.export_snapshot();
    let sb = b.export_snapshot();
    a.import_snapshot(&sb).expect("the snapshot imports");
    b.import_snapshot(&sa).expect("the snapshot imports");

    let order = tab_order(&a);
    assert_eq!(order, tab_order(&b), "peers diverged after full exchange");
    let mut sorted = order.clone();
    sorted.sort();
    let mut expected = ids.clone();
    expected.sort();
    assert_eq!(sorted, expected, "each sheet shows exactly once: {order:?}");
}
