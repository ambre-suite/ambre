// ambre-trame/src/logic/structural_tests.rs
// ============================================================================
// The incremental structural path against a full rebuild (`TRAME-280`), the
// reference-model regime of `ambre-spec-testing.md` section 3.
//
// Two states run one generated program in lockstep, under one peer id, so each
// mints the same uids and writes the same bytes. The first carries the engine
// across each structural edit: a walk narrowed to the formulas the engine
// names, then a carried step. The second is the reference model. It walks
// every formula and reseeds the whole engine after each edit, as the code did
// before the narrowed path existed.
//
// After every step the two must agree on what the document stores. Their
// engines must agree too: each value, each edge, each range a formula reads,
// and each formula a cycle refused.
//
// A loop closed through a range shows `#CIRCULAR!` on each of its cells and
// downstream of it, whatever order a pass takes (`TRAME-281`). The two paths
// evaluate in different orders, so the generator writes such loops on purpose,
// and every value must still match.
//
// The generator writes no clock
// function, and no function that hides an error, such as `COUNT` or `IFERROR`.
// ============================================================================

use crate::hash::{HashMap, HashSet};

use ambre_core::index::{Len, Pos};
use loro::LoroValue;
use proptest::prelude::*;

use super::structural::StructuralPath;
use super::{SheetContainer, SheetRoot, TrameState};
use crate::formula::CellId;
use crate::message::{ClipboardCell, ClipboardPayload, HeaderRow, SelectionKind, SortOrder};
use crate::{CellPos, CellRect, Col, Row, TrameCommand};

/// The window the generator addresses: small, so a structural edit meets the
/// formulas that read the lines it edits.
pub(super) const ROWS: u32 = 6;
pub(super) const COLS: u32 = 4;

/// The peer both states write under, so both mint the same uids.
pub(super) const PEER: u64 = 7;

/// One step of a program.
#[derive(Debug, Clone)]
pub(super) enum Step {
    Write {
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
    /// Delete the rows `from..=to` in one edit (`TRAME-088`).
    DeleteRows {
        from: u32,
        to: u32,
    },
    Sort {
        order: SortOrder,
    },
    Merge {
        row: u32,
        col: u32,
    },
    Unmerge {
        row: u32,
        col: u32,
    },
    Undo,
    Redo,
    /// A peer writes a cell, and the delta merges into both states. Both
    /// carry it, as both carry a local write: the remote path has its own
    /// reference model (`TRAME-282`).
    Remote {
        row: u32,
        col: u32,
        text: String,
    },
}

/// What a cell holds: numbers and formulas. The formulas read cells, ranges,
/// whole lines, positions past the used range, their own position, a name and
/// the other sheet. Cycles, static or through a range, come out of the mix.
pub(super) fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        (0i64..9).prop_map(|n| n.to_string()),
        Just("=A1+B2".to_owned()),
        Just("=B1*2".to_owned()),
        Just("=A3-C1".to_owned()),
        Just("=A1".to_owned()),
        Just("=C2+1".to_owned()),
        Just("=SUM(A1:A3)".to_owned()),
        Just("=SUM(B2:C4)".to_owned()),
        Just("=SUM(A:A)".to_owned()),
        Just("=SUM(2:3)".to_owned()),
        Just("=A9+1".to_owned()),
        Just("=ROW()*10+COLUMN()".to_owned()),
        Just("=SUM(TOTAL)".to_owned()),
        Just("=Sheet2!A1+1".to_owned()),
        Just(String::new()),
    ]
}

pub(super) fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (0..ROWS, 0..COLS, text()).prop_map(|(row, col, text)| Step::Write { row, col, text }),
        1 => (0..ROWS).prop_map(|at| Step::InsertRow { at }),
        2 => (0..ROWS).prop_map(|at| Step::DeleteRow { at }),
        1 => (0..COLS).prop_map(|at| Step::InsertColumn { at }),
        1 => (0..COLS).prop_map(|at| Step::DeleteColumn { at }),
        1 => (0..ROWS, 0..ROWS).prop_map(|(from, to)| Step::MoveRow { from, to }),
        1 => (0..COLS, 0..COLS).prop_map(|(from, to)| Step::MoveColumn { from, to }),
        1 => (0..ROWS, 0..ROWS).prop_map(|(a, b)| Step::DeleteRows { from: a.min(b), to: a.max(b) }),
        1 => prop_oneof![Just(SortOrder::Ascending), Just(SortOrder::Descending)]
            .prop_map(|order| Step::Sort { order }),
        1 => (0..ROWS - 1, 0..COLS - 1).prop_map(|(row, col)| Step::Merge { row, col }),
        1 => (0..ROWS, 0..COLS).prop_map(|(row, col)| Step::Unmerge { row, col }),
        1 => Just(Step::Undo),
        1 => Just(Step::Redo),
        1 => (0..ROWS, 0..COLS, text()).prop_map(|(row, col, text)| Step::Remote { row, col, text }),
    ]
}

pub(super) fn at(row: u32, col: u32) -> CellPos {
    CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col))
}

pub(super) fn write(row: u32, col: u32, text: &str) -> TrameCommand {
    TrameCommand::SetCellText {
        row: Pos::<Row>::new(row),
        col: Pos::<Col>::new(col),
        text: text.to_owned(),
    }
}

/// The commands of one local step. A remote step has none.
pub(super) fn commands(step: &Step) -> Vec<TrameCommand> {
    match step.clone() {
        Step::Write { row, col, text } => vec![write(row, col, &text)],
        Step::InsertRow { at } => vec![TrameCommand::InsertRow {
            at: Pos::<Row>::new(at),
        }],
        Step::DeleteRow { at } => vec![TrameCommand::DeleteRow {
            at: Pos::<Row>::new(at),
        }],
        Step::InsertColumn { at } => vec![TrameCommand::InsertColumn {
            at: Pos::<Col>::new(at),
        }],
        Step::DeleteColumn { at } => vec![TrameCommand::DeleteColumn {
            at: Pos::<Col>::new(at),
        }],
        Step::MoveRow { from, to } => vec![TrameCommand::MoveRow {
            from: Pos::<Row>::new(from),
            to: Pos::<Row>::new(to),
        }],
        Step::MoveColumn { from, to } => vec![TrameCommand::MoveColumn {
            from: Pos::<Col>::new(from),
            to: Pos::<Col>::new(to),
        }],
        Step::DeleteRows { from, to } => vec![
            TrameCommand::SetActiveCell {
                row: Pos::<Row>::new(to),
                col: Pos::<Col>::new(0),
                anchor_row: Pos::<Row>::new(from),
                anchor_col: Pos::<Col>::new(0),
                extra: Vec::new(),
                kind: SelectionKind::Cells,
            },
            TrameCommand::DeleteSelectedRows,
        ],
        Step::Sort { order } => vec![TrameCommand::SortRange {
            sheet_id: super::default_sheet(),
            rect: CellRect::spanning(at(0, 0), at(ROWS - 1, COLS - 1)),
            sort_col: Len::<Col>::new(0),
            order,
            header: HeaderRow::Absent,
        }],
        Step::Merge { row, col } => vec![TrameCommand::MergeCells {
            rect: CellRect::spanning(at(row, col), at(row + 1, col + 1)),
        }],
        Step::Unmerge { row, col } => vec![TrameCommand::UnmergeCells {
            row: Pos::<Row>::new(row),
            col: Pos::<Col>::new(col),
        }],
        Step::Undo => vec![TrameCommand::Undo],
        Step::Redo => vec![TrameCommand::Redo],
        Step::Remote { .. } => Vec::new(),
    }
}

/// A workbook of two sheets: numbers and formulas on the first, and on the
/// second, formulas that read the first through a qualifier.
pub(super) fn seeded(path: StructuralPath) -> TrameState {
    let mut st = TrameState::with_peer(ambre_core::PeerId::new(PEER));
    st.structural_path = path;
    let first = st.active_sheet.clone();
    st.handle_command(TrameCommand::CreateSheet {
        name: crate::test_support::sheet("Sheet2"),
    });
    let second = st
        .sheet_name_to_id(&crate::test_support::sheet("Sheet2"))
        .expect("the sheet was just created");
    st.handle_command(TrameCommand::ActivateSheet { sheet_id: second });
    for (row, text) in ["=Sheet1!A2*2", "=SUM(Sheet1!A1:B3)", "=Sheet1!B9+1", "4"]
        .iter()
        .enumerate()
    {
        st.handle_command(write(row as u32, 0, text));
    }
    st.handle_command(TrameCommand::ActivateSheet { sheet_id: first });
    st.handle_command(TrameCommand::DefineName {
        name: crate::test_support::name("TOTAL"),
        reference: "B1:B3".to_owned(),
    });
    let mut cells = Vec::new();
    for row in 0..4 {
        for (col, raw) in [(0, format!("{}", row + 1)), (1, format!("=A{}*2", row + 1))] {
            cells.push(ClipboardCell {
                dr: Len::<Row>::new(row),
                dc: Len::<Col>::new(col),
                raw,
                ..ClipboardCell::default()
            });
        }
    }
    st.handle_command(TrameCommand::PasteRange {
        target_row: Pos::<Row>::new(0),
        target_col: Pos::<Col>::new(0),
        payload: ClipboardPayload {
            src_row: Pos::<Row>::new(0),
            src_col: Pos::<Col>::new(0),
            rows: Len::<Row>::new(4),
            cols: Len::<Col>::new(2),
            cells,
        },
    });
    st
}

/// The stored text of every cell of every sheet.
pub(super) fn stored(st: &TrameState) -> Vec<LoroValue> {
    st.sheets_by_idx
        .iter()
        .map(|sheet| {
            st.doc
                .get_map(SheetContainer::of(SheetRoot::Cells, sheet).as_str())
                .get_deep_value()
        })
        .collect()
}

/// Everything the engine holds that a later edit depends on, values aside.
#[derive(Debug, PartialEq)]
pub(super) struct Engine {
    pub(super) edges: HashMap<(CellId, CellId), usize>,
    pub(super) ranges: HashMap<CellId, Vec<String>>,
    pub(super) formulas: Vec<String>,
    pub(super) cyclic: bool,
}

pub(super) fn engine(st: &TrameState) -> Engine {
    let e = &st.engine;
    let mut edges = HashMap::default();
    for edge in e.deps.cell_dag.edge_indices() {
        if let Some((a, b)) = e.deps.cell_dag.edge_endpoints(edge) {
            *edges
                .entry((e.deps.cell_dag[a], e.deps.cell_dag[b]))
                .or_insert(0) += 1;
        }
    }
    let ranges = e
        .deps
        .range_deps
        .iter()
        .map(|(cell, deps)| (*cell, deps.iter().map(|d| format!("{d:?}")).collect()))
        .collect();
    // Each formula with its bytecode: a parked bytecode an undo takes back
    // must equal what a compile of its text gives (`TRAME-301`).
    let mut formulas: Vec<String> = e
        .bytecode
        .cells()
        .map(|c| format!("{c:?} {:?}", e.bytecode.get(c)))
        .collect();
    formulas.sort();
    Engine {
        edges,
        ranges,
        formulas,
        cyclic: e.has_cyclic(),
    }
}

/// The value of each cell the engine holds, as its kind and its display.
fn values(st: &TrameState) -> HashMap<CellId, (String, String)> {
    st.engine
        .formula_cache
        .borrow()
        .iter()
        .map(|(cell, v)| (*cell, (format!("{v:?}"), v.to_display())))
        .collect()
}

/// The two engines hold the same value for each cell, errors included. A
/// cell on a loop closed through a range shows `#CIRCULAR!` on both, whatever
/// order each pass took (`TRAME-281`).
pub(super) fn same_values(carried: &TrameState, rebuilt: &TrameState) -> Result<(), String> {
    let (a, b) = (values(carried), values(rebuilt));
    let keys: HashSet<&CellId> = a.keys().chain(b.keys()).collect();
    for cell in keys {
        let (va, vb) = (a.get(cell), b.get(cell));
        if va != vb {
            return Err(format!("{cell:?}: carried {va:?}, rebuilt {vb:?}"));
        }
    }
    Ok(())
}

/// Whether the index by line the state keeps equals a walk of its maps
/// (`TRAME-081`).
pub(super) fn lines_kept(st: &TrameState) -> bool {
    *super::line_cells::read(&st.lines) == super::line_cells::LineCells::walk(&st.doc)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Each structural edit leaves the incrementally carried engine, and the
    /// text the narrowed walk wrote, equal to a full walk and a full reseed
    /// (`TRAME-280`). So does each undo and each redo (`TRAME-285`). The index
    /// by line equals a walk of the maps after each step (`TRAME-081`).
    #[test]
    fn a_carried_engine_equals_a_full_rebuild(program in proptest::collection::vec(step(), 1..16)) {
        let mut carried = seeded(StructuralPath::Carry);
        let mut rebuilt = seeded(StructuralPath::Reseed);
        let mut peer = TrameState::with_peer(ambre_core::PeerId::new(PEER + 1));
        peer.import_snapshot(&carried.export_snapshot())
            .expect("a snapshot this build wrote reads back");
        prop_assert_eq!(stored(&carried), stored(&rebuilt), "the seed differs");
        for step in &program {
            if let Step::Remote { row, col, text } = step {
                peer.import_snapshot(&carried.export_snapshot())
                    .expect("a snapshot this build wrote reads back");
                let delta = peer.handle_command(write(*row, *col, text)).update;
                if let Some(delta) = delta {
                    carried.apply_remote(&delta);
                    rebuilt.apply_remote(&delta);
                }
            }
            for command in commands(step) {
                carried.handle_command(command.clone());
                rebuilt.handle_command(command);
            }
            prop_assert_eq!(stored(&carried), stored(&rebuilt), "the text differs after {:?}", step);
            prop_assert_eq!(engine(&carried), engine(&rebuilt), "the engine differs after {:?}", step);
            let same = same_values(&carried, &rebuilt);
            prop_assert!(same.is_ok(), "a value differs after {:?}: {:?}", step, same);
            prop_assert!(lines_kept(&carried), "the index by line differs after {:?}", step);
        }
    }
}

/// Run `before`, a remote write of `text` at `row`, `col`, an undo, then
/// `edit`, on a carried state and on the reference model. The undo removes
/// the line `before` added. The remote formula then stays stored under an
/// element no line carries any more. It places on no cell, and the engine
/// holds nothing of it. The two must still store the same text after `edit`.
fn orphan_then(before: TrameCommand, row: u32, col: u32, text: &str, edit: &[TrameCommand]) {
    let mut carried = seeded(StructuralPath::Carry);
    let mut rebuilt = seeded(StructuralPath::Reseed);
    for st in [&mut carried, &mut rebuilt] {
        st.handle_command(before.clone());
    }
    let mut peer = TrameState::with_peer(ambre_core::PeerId::new(PEER + 1));
    peer.import_snapshot(&carried.export_snapshot())
        .expect("a snapshot this build wrote reads back");
    let delta = peer
        .handle_command(write(row, col, text))
        .update
        .expect("a write sends a delta");
    for st in [&mut carried, &mut rebuilt] {
        st.apply_remote(&delta);
        st.handle_command(TrameCommand::Undo);
    }
    assert_eq!(
        stored(&carried),
        stored(&rebuilt),
        "the undo leaves both alike"
    );
    for command in edit {
        carried.handle_command(command.clone());
        rebuilt.handle_command(command.clone());
    }
    assert_eq!(
        stored(&carried),
        stored(&rebuilt),
        "the walk reads the stored formula"
    );
}

/// The shrunk seed of `a_carried_engine_equals_a_full_rebuild`: a formula
/// left under a column an undo removed, then a row insert (`TRAME-280`).
#[test]
fn a_row_insert_rewrites_a_formula_left_under_a_removed_column() {
    orphan_then(
        TrameCommand::InsertColumn {
            at: Pos::<Col>::new(2),
        },
        0,
        2,
        "=A3-C1",
        &[TrameCommand::InsertRow {
            at: Pos::<Row>::new(0),
        }],
    );
}

/// A formula left under a row an undo removed, then a column insert.
#[test]
fn a_column_insert_rewrites_a_formula_left_under_a_removed_row() {
    orphan_then(
        TrameCommand::InsertRow {
            at: Pos::<Row>::new(1),
        },
        1,
        0,
        "=B2+A1",
        &[TrameCommand::InsertColumn {
            at: Pos::<Col>::new(0),
        }],
    );
}

/// A formula left under a row an undo removed, then a move.
#[test]
fn a_move_rewrites_a_formula_left_under_a_removed_row() {
    orphan_then(
        TrameCommand::InsertRow {
            at: Pos::<Row>::new(1),
        },
        1,
        0,
        "=B2+A1",
        &[TrameCommand::MoveRow {
            from: Pos::<Row>::new(0),
            to: Pos::<Row>::new(2),
        }],
    );
}

/// A formula left under a row an undo removed, then a sort.
#[test]
fn a_sort_rewrites_a_formula_left_under_a_removed_row() {
    orphan_then(
        TrameCommand::InsertRow {
            at: Pos::<Row>::new(1),
        },
        1,
        0,
        "=B2+A1",
        &[TrameCommand::SortRange {
            sheet_id: super::default_sheet(),
            rect: CellRect::spanning(at(0, 0), at(ROWS - 1, COLS - 1)),
            sort_col: Len::<Col>::new(0),
            order: SortOrder::Descending,
            header: HeaderRow::Absent,
        }],
    );
}

/// A formula left under a column an undo removed, then a delete of another
/// column.
#[test]
fn a_column_delete_rewrites_a_formula_left_under_a_removed_column() {
    orphan_then(
        TrameCommand::InsertColumn {
            at: Pos::<Col>::new(2),
        },
        0,
        2,
        "=A3-C1",
        &[TrameCommand::DeleteColumn {
            at: Pos::<Col>::new(0),
        }],
    );
}
