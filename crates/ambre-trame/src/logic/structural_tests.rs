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
// A loop closed through a range is caught while evaluating (`TRAME-042`). Which
// error such a loop shows depends on the order of the evaluation pass. A cell
// read from outside the loop first meets another error, one read from inside
// meets the loop. The two paths evaluate in different orders. Each cell on such
// a loop, or downstream of one, must therefore read an error on both. Every
// other cell must read the same value.
//
// The generator writes no clock
// function, and no function that hides an error, such as `COUNT` or `IFERROR`.
// ============================================================================

use std::collections::{HashMap, HashSet};

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
const ROWS: u32 = 6;
const COLS: u32 = 4;

/// The peer both states write under, so both mint the same uids.
const PEER: u64 = 7;

/// One step of a program.
#[derive(Debug, Clone)]
enum Step {
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
    /// A peer writes a cell, and the delta merges into both states.
    Remote {
        row: u32,
        col: u32,
        text: String,
    },
}

/// What a cell holds: numbers and formulas. The formulas read cells, ranges,
/// whole lines, positions past the used range, their own position, a name and
/// the other sheet. Cycles, static or through a range, come out of the mix.
fn text() -> impl Strategy<Value = String> {
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

fn step() -> impl Strategy<Value = Step> {
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
        1 => (0..ROWS, 0..COLS, text()).prop_map(|(row, col, text)| Step::Remote { row, col, text }),
    ]
}

fn at(row: u32, col: u32) -> CellPos {
    CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col))
}

fn write(row: u32, col: u32, text: &str) -> TrameCommand {
    TrameCommand::SetCellText {
        row: Pos::<Row>::new(row),
        col: Pos::<Col>::new(col),
        text: text.to_owned(),
    }
}

/// The commands of one local step. A remote step has none.
fn commands(step: &Step) -> Vec<TrameCommand> {
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
        Step::Remote { .. } => Vec::new(),
    }
}

/// A workbook of two sheets: numbers and formulas on the first, and on the
/// second, formulas that read the first through a qualifier.
fn seeded(path: StructuralPath) -> TrameState {
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
fn stored(st: &TrameState) -> Vec<LoroValue> {
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
struct Engine {
    edges: HashMap<(CellId, CellId), usize>,
    ranges: HashMap<CellId, Vec<String>>,
    formulas: Vec<String>,
    cyclic: bool,
}

fn engine(st: &TrameState) -> Engine {
    let e = &st.engine;
    let mut edges = HashMap::new();
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
    let mut formulas: Vec<String> = e.bytecode.cells().map(|c| format!("{c:?}")).collect();
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

/// The cells on a loop closed through a range, and each cell downstream of
/// one. Whatever reads a cell is downstream of it, through an edge of the
/// graph or through a range that contains the cell.
fn order_dependent(st: &TrameState) -> HashSet<CellId> {
    let e = &st.engine;
    let indices = st.engine_indices();
    let mut readers: HashMap<CellId, Vec<CellId>> = HashMap::new();
    for edge in e.deps.cell_dag.edge_indices() {
        if let Some((a, b)) = e.deps.cell_dag.edge_endpoints(edge) {
            readers
                .entry(e.deps.cell_dag[a])
                .or_default()
                .push(e.deps.cell_dag[b]);
        }
    }
    for (formula, deps) in &e.deps.range_deps {
        for dep in deps {
            let Some(index) = indices.get(&dep.sheet) else {
                continue;
            };
            for &row in index.rows().lines().iter() {
                for &col in index.cols().lines().iter() {
                    if dep.contains(row, col, index) {
                        readers
                            .entry(CellId::bound(dep.sheet, row, col))
                            .or_default()
                            .push(*formula);
                    }
                }
            }
        }
    }
    let reach = |from: CellId| {
        let mut seen = HashSet::new();
        let mut queue = vec![from];
        while let Some(cur) = queue.pop() {
            for &next in readers.get(&cur).into_iter().flatten() {
                if seen.insert(next) {
                    queue.push(next);
                }
            }
        }
        seen
    };
    let mut out = HashSet::new();
    for &cell in readers.keys() {
        if !out.contains(&cell) && reach(cell).contains(&cell) {
            out.extend(reach(cell));
        }
    }
    out
}

/// The two engines hold the same values. Outside every loop closed through a
/// range they are equal. Inside one, or downstream of one, both are errors.
fn same_values(carried: &TrameState, rebuilt: &TrameState) -> Result<(), String> {
    let (a, b) = (values(carried), values(rebuilt));
    let loose = order_dependent(rebuilt);
    let keys: HashSet<&CellId> = a.keys().chain(b.keys()).collect();
    for cell in keys {
        let (va, vb) = (a.get(cell), b.get(cell));
        if loose.contains(cell) {
            let error =
                |v: Option<&(String, String)>| v.is_some_and(|(k, _)| k.starts_with("Error"));
            if !(error(va) && error(vb)) {
                return Err(format!(
                    "{cell:?} on a loop: carried {va:?}, rebuilt {vb:?}"
                ));
            }
        } else if va != vb {
            return Err(format!("{cell:?}: carried {va:?}, rebuilt {vb:?}"));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Each structural edit leaves the incrementally carried engine, and the
    /// text the narrowed walk wrote, equal to a full walk and a full reseed
    /// (`TRAME-280`).
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
        }
    }
}
