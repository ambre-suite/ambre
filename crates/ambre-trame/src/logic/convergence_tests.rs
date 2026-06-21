// ambre-trame/src/logic/convergence_tests.rs
// ============================================================================
// CRDT convergence property tests (Epic `ET`, `ambre-spec-trame-logic.md` §14;
// `ambre-spec-architecture.md` §4 "Generalize proptest convergence beyond
// ambre-crypto -- Trame first").
//
// `proptest` generates interleaved sequences of the document-mutating
// `TrameCommand`s (`SetCellText` / insert-delete row-column / `SortRange` /
// merge / `FillRange`), routes each op to one of two peers, then exchanges the
// full CRDT history both ways. The invariant: after the exchange the two peers
// observe an *identical* materialized grid -- same text, same formula results
// (recompute determinism), same merge geometry. This exercises the own-uid cell
// model (§2.A/§12): references must follow data across a concurrent sort, and
// structural ops on both peers must reconcile to one converged order.
//
// The exchange uses full snapshots rather than incremental `SendUpdate` deltas:
// each peer mints an implicit-`s0` op in `new()` under a local origin, so an
// incremental delta would carry a dangling dependency on the other peer's
// private op (the same reason `cross_sheet_reference_converges_across_peers_on_
// rename` merges snapshots). A snapshot carries the full history, so importing
// it yields the union -- the CRDT convergence guarantee.
// ============================================================================

use proptest::prelude::*;

use ambre_bus::{FillMode, FilterKind, FilterOp, GridRange};

use super::TrameState;
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
    /// filters on the two peers resolve by Loro LWW -- this folds the explicit
    /// `concurrent_filters_converge_lww` case into the property test: the
    /// converged `is_filtered` set is part of the asserted snapshot.
    Filter {
        op: FilterOp,
        value: u32,
    },
    ClearFilter,
}

impl Op {
    /// Translate to the `TrameCommand` the Logic dispatcher would receive.
    fn into_command(self) -> TrameCommand {
        match self {
            Op::SetText { row, col, text } => TrameCommand::SetCellText { row, col, text },
            Op::InsertRow { at } => TrameCommand::InsertRow { at },
            Op::DeleteRow { at } => TrameCommand::DeleteRow { at },
            Op::InsertColumn { at } => TrameCommand::InsertColumn { at },
            Op::DeleteColumn { at } => TrameCommand::DeleteColumn { at },
            Op::Sort { ascending } => TrameCommand::SortRange {
                sheet_id: SHEET.to_owned(),
                row_start: 0,
                row_end: ROWS - 1,
                col_start: 0,
                col_end: COLS - 1,
                sort_col: 0,
                ascending,
                has_header: false,
            },
            Op::Merge {
                row,
                col,
                row_end,
                col_end,
            } => TrameCommand::MergeCells {
                row,
                col,
                row_end,
                col_end,
            },
            Op::Unmerge { row, col } => TrameCommand::UnmergeCells { row, col },
            Op::Filter { op, value } => TrameCommand::ApplyFilter {
                sheet_id: SHEET.to_owned(),
                col: 0,
                kind: FilterKind::Condition {
                    op,
                    value: value.to_string(),
                },
            },
            Op::ClearFilter => TrameCommand::ClearFilter {
                sheet_id: SHEET.to_owned(),
            },
            Op::Fill => TrameCommand::FillRange {
                sheet_id: SHEET.to_owned(),
                src: GridRange {
                    row: 0,
                    col: 0,
                    row_end: 0,
                    col_end: 0,
                },
                dest: GridRange {
                    row: 0,
                    col: 0,
                    row_end: ROWS - 1,
                    col_end: 0,
                },
                mode: FillMode::Auto,
            },
        }
    }
}

/// Cell contents the generator picks from: plain numbers, short strings, and a
/// few formulas that reference other window cells (so a concurrent sort can
/// move the data out from under them -- the §12 guarantee under test).
fn text_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        (0i64..20).prop_map(|n| n.to_string()),
        Just("foo".to_owned()),
        Just("bar".to_owned()),
        Just("=A1+B1".to_owned()),
        Just("=SUM(A1:A3)".to_owned()),
        Just("=B2*2".to_owned()),
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
type Snapshot = Vec<(u32, u32, String, bool, bool, Option<(u32, u32)>, bool)>;

fn read_window(state: &mut TrameState) -> Snapshot {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: u64::MAX,
        row_start: 0,
        row_end: ROWS - 1,
        col_start: 0,
        col_end: COLS - 1,
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
                c.row,
                c.col,
                c.text.clone(),
                c.bold,
                c.is_merged,
                c.merge_span,
                c.is_filtered,
            )
        })
        .collect();
    rows.sort_by_key(|(r, c, ..)| (*r, *c));
    rows
}

proptest! {
    // Convergence is the contract; default 256 cases is plenty of interleavings
    // for a 14-step program over a 5x4 grid and keeps the suite fast.
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Two peers applying an interleaved op stream converge to an identical
    /// grid after exchanging full history both ways.
    #[test]
    fn two_peers_converge_under_interleaved_ops(program in program_strategy()) {
        // Shared base: A seeds it, B imports it, so both share the implicit-s0
        // history before diverging. Any op past the seeded extent simply
        // materializes lazily -- identically on both once history is merged.
        let mut a = TrameState::new();
        for (r, text) in ["10", "5", "20", "1"].iter().enumerate() {
            a.handle_command(TrameCommand::SetCellText {
                row: r as u32,
                col: 0,
                text: (*text).to_owned(),
            });
        }
        let base = a.export_snapshot();
        let mut b = TrameState::new();
        b.import_snapshot(&base);

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
        a.apply_remote(&sb);
        b.apply_remote(&sa);

        let grid_a = read_window(&mut a);
        let grid_b = read_window(&mut b);
        prop_assert_eq!(
            &grid_a,
            &grid_b,
            "peers diverged after full history exchange"
        );

        // Recompute determinism: a fresh peer rebuilt from the converged
        // snapshot renders the same grid (no derived-state path dependence).
        let mut c = TrameState::new();
        c.import_snapshot(&a.export_snapshot());
        prop_assert_eq!(read_window(&mut c), grid_a, "reload diverged from live peer");
    }
}
