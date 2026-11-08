// ambre-trame/src/logic/remote_tests.rs
// ============================================================================
// The carried remote apply against a full rebuild (`TRAME-282`), the
// reference-model regime of `ambre-spec-testing.md` section 3.
//
// Two authors edit one workbook, each on its own peer, and sync with each
// other now and then. Their deltas queue per author. Two receivers apply the
// same batches in the same order, under one peer id. The first carries its
// engine by what each batch wrote. The second is the reference model: it
// reseeds the whole engine after every batch, as the code did before the
// carried path existed. A receiver also edits locally, structural edits
// included, and switches sheets.
//
// Both receivers take the carried path on a local structural edit. The two
// then write the same operations, so a later concurrent write wins or loses
// on both alike. That path reads what the last batch left the engine. A
// batch that carried a state only a reseed may hold shows on the next local
// edit. It also shows in the skipped mark, compared after each step.
//
// A batch may carry a delta whose causal past has not arrived. The engine
// imports it pending, and applies it with the batch that brings that past.
//
// After every step the two receivers must agree on what the document stores.
// They must agree on each value, each edge, each range a formula reads, and
// each formula a cycle refused.
// ============================================================================

use crate::hash::HashMap;
use std::collections::VecDeque;

use ambre_core::index::{Len, Pos};
use proptest::prelude::*;

use super::TrameState;
use super::remote::RemotePath;
use super::structural::StructuralPath;
use super::structural_tests::{
    COLS, Engine, ROWS, at, engine, same_values, seeded, stored, text, write,
};
use crate::formula::{AxisId, CellId};
use crate::message::{HeaderRow, SortOrder};
use crate::{CellPos, CellRect, Col, Row, TrameCommand};

/// The peer each author writes under.
const AUTHORS: [u64; 2] = [11, 12];

/// The peer both receivers write under, so both mint the same uids.
const RECEIVER: u64 = 21;

/// One edit an author makes.
#[derive(Debug, Clone)]
enum Edit {
    /// A write on the first sheet.
    Write {
        row: u32,
        col: u32,
        text: String,
    },
    /// A write on the first line past the author's used range. Two authors
    /// that write there before they sync add one line on two elements
    /// (`TRAME-261`). Two values in one cell then sit under two keys
    /// (`TRAME-279`).
    WriteBelow {
        col: u32,
        text: String,
    },
    /// A write on the second sheet, whose formulas read the first.
    WriteOther {
        row: u32,
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
    /// A row insert on the second sheet.
    InsertRowOther {
        at: u32,
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
    /// Point the name `TOTAL` at another range.
    Rename {
        reference: &'static str,
    },
    Undo,
    Redo,
}

/// One step of a program.
#[derive(Debug, Clone)]
enum Step {
    /// An author edits, and queues the delta it sends.
    Author { who: usize, edit: Edit },
    /// The receivers take the next `n` queued deltas of one author as one
    /// batch.
    Deliver { who: usize, n: usize },
    /// The two authors exchange what each holds.
    SyncAuthors,
    /// The receivers make an edit themselves.
    Local { edit: Edit },
    /// The receivers switch to the other sheet.
    Switch,
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        6 => (0..ROWS, 0..COLS, text()).prop_map(|(row, col, text)| Edit::Write { row, col, text }),
        3 => (0..COLS, text()).prop_map(|(col, text)| Edit::WriteBelow { col, text }),
        1 => (0..ROWS, text()).prop_map(|(row, text)| Edit::WriteOther { row, text }),
        1 => (0..ROWS).prop_map(|at| Edit::InsertRow { at }),
        2 => (0..ROWS).prop_map(|at| Edit::DeleteRow { at }),
        1 => (0..COLS).prop_map(|at| Edit::InsertColumn { at }),
        1 => (0..COLS).prop_map(|at| Edit::DeleteColumn { at }),
        1 => (0..ROWS, 0..ROWS).prop_map(|(from, to)| Edit::MoveRow { from, to }),
        1 => (0..COLS, 0..COLS).prop_map(|(from, to)| Edit::MoveColumn { from, to }),
        1 => (0..ROWS).prop_map(|at| Edit::InsertRowOther { at }),
        1 => prop_oneof![Just(SortOrder::Ascending), Just(SortOrder::Descending)]
            .prop_map(|order| Edit::Sort { order }),
        1 => (0..ROWS - 1, 0..COLS - 1).prop_map(|(row, col)| Edit::Merge { row, col }),
        1 => (0..ROWS, 0..COLS).prop_map(|(row, col)| Edit::Unmerge { row, col }),
        1 => prop_oneof![Just("B1:B3"), Just("A2:C2"), Just("Sheet2!A1:A4")]
            .prop_map(|reference| Edit::Rename { reference }),
        1 => Just(Edit::Undo),
        1 => Just(Edit::Redo),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        8 => (0..2usize, edit()).prop_map(|(who, edit)| Step::Author { who, edit }),
        5 => (0..2usize, 1..4usize).prop_map(|(who, n)| Step::Deliver { who, n }),
        1 => Just(Step::SyncAuthors),
        2 => edit().prop_map(|edit| Step::Local { edit }),
        1 => Just(Step::Switch),
    ]
}

/// The id of the sheet named `name` on `st`.
fn sheet(st: &TrameState, name: &str) -> ambre_core::id::Id<ambre_core::id::Sheet> {
    st.sheet_name_to_id(&crate::test_support::sheet(name))
        .expect("the seed holds both sheets")
}

/// Run `commands` on the second sheet of `st`, then come back to the first.
fn on_other_sheet(st: &mut TrameState, commands: Vec<TrameCommand>) -> Vec<TrameCommand> {
    let (first, second) = (sheet(st, "Sheet1"), sheet(st, "Sheet2"));
    let mut all = vec![TrameCommand::ActivateSheet { sheet_id: second }];
    all.extend(commands);
    all.push(TrameCommand::ActivateSheet { sheet_id: first });
    all
}

/// The commands of one author edit.
fn commands(st: &mut TrameState, edit: &Edit) -> Vec<TrameCommand> {
    match edit.clone() {
        Edit::Write { row, col, text } => vec![write(row, col, &text)],
        Edit::WriteBelow { col, text } => {
            let row = st.index.row_len().get();
            vec![write(row, col, &text)]
        }
        Edit::WriteOther { row, text } => on_other_sheet(st, vec![write(row, 0, &text)]),
        Edit::InsertRow { at } => vec![TrameCommand::InsertRow {
            at: Pos::<Row>::new(at),
        }],
        Edit::DeleteRow { at } => vec![TrameCommand::DeleteRow {
            at: Pos::<Row>::new(at),
        }],
        Edit::InsertColumn { at } => vec![TrameCommand::InsertColumn {
            at: Pos::<Col>::new(at),
        }],
        Edit::DeleteColumn { at } => vec![TrameCommand::DeleteColumn {
            at: Pos::<Col>::new(at),
        }],
        Edit::MoveRow { from, to } => vec![TrameCommand::MoveRow {
            from: Pos::<Row>::new(from),
            to: Pos::<Row>::new(to),
        }],
        Edit::MoveColumn { from, to } => vec![TrameCommand::MoveColumn {
            from: Pos::<Col>::new(from),
            to: Pos::<Col>::new(to),
        }],
        Edit::InsertRowOther { at } => on_other_sheet(
            st,
            vec![TrameCommand::InsertRow {
                at: Pos::<Row>::new(at),
            }],
        ),
        Edit::Sort { order } => vec![TrameCommand::SortRange {
            sheet_id: super::default_sheet(),
            rect: CellRect::spanning(at(0, 0), at(ROWS - 1, COLS - 1)),
            sort_col: Len::<Col>::new(0),
            order,
            header: HeaderRow::Absent,
        }],
        Edit::Merge { row, col } => vec![TrameCommand::MergeCells {
            rect: CellRect::spanning(at(row, col), at(row + 1, col + 1)),
        }],
        Edit::Unmerge { row, col } => vec![TrameCommand::UnmergeCells {
            row: Pos::<Row>::new(row),
            col: Pos::<Col>::new(col),
        }],
        Edit::Rename { reference } => vec![TrameCommand::DefineName {
            name: crate::test_support::name("TOTAL"),
            reference: reference.to_owned(),
        }],
        Edit::Undo => vec![TrameCommand::Undo],
        Edit::Redo => vec![TrameCommand::Redo],
    }
}

/// A state on `peer` holding `snapshot`, its remote batches taking `path`.
/// Its local structural edits take the carried path.
fn opened(
    peer: u64,
    snapshot: &ambre_core::Payload<ambre_core::SnapshotBytes>,
    path: RemotePath,
) -> TrameState {
    let mut st = TrameState::with_peer(ambre_core::PeerId::new(peer));
    st.remote_path = path;
    st.import_snapshot(snapshot)
        .expect("a snapshot this build wrote reads back");
    st
}

/// The stored formulas that place on no cell, in a stable order.
fn unplaced(st: &TrameState) -> Vec<String> {
    let mut keys: Vec<String> = st
        .unplaced
        .iter()
        .flat_map(|(sheet, keys)| {
            keys.iter()
                .map(move |key| format!("{sheet}:{}", key.as_str()))
        })
        .collect();
    keys.sort();
    keys
}

/// The formulas that read positions, in a stable order.
fn positional(st: &TrameState) -> Vec<String> {
    let mut cells: Vec<String> = st
        .engine
        .bytecode
        .positional()
        .map(|c| format!("{c:?}"))
        .collect();
    cells.sort();
    cells
}

/// What the engine of `st` holds, each edge out of a cell named by its
/// lines.
///
/// A formula compiled before the line it reads existed names that cell by
/// its position. A local write that adds the line keeps that name, and a
/// reseed names the cell by the line. Both name the same cell until the lines move, and
/// every structural edit compiles such a formula again, since it reads
/// positions.
fn engine_by_lines(st: &TrameState) -> Engine {
    let indices = st.engine_indices();
    let by_line = |cell: CellId| -> CellId {
        let Some(index) = indices.get(&cell.sheet) else {
            return cell;
        };
        let row = match cell.row {
            AxisId::Virtual(pos) => index.row_id(pos).map_or(cell.row, AxisId::Bound),
            bound @ AxisId::Bound(_) => bound,
        };
        let col = match cell.col {
            AxisId::Virtual(pos) => index.col_id(pos).map_or(cell.col, AxisId::Bound),
            bound @ AxisId::Bound(_) => bound,
        };
        CellId::new(cell.sheet, row, col)
    };
    let mut held = engine(st);
    let mut edges = HashMap::default();
    for ((from, to), n) in held.edges.drain() {
        *edges.entry((by_line(from), by_line(to))).or_insert(0) += n;
    }
    held.edges = edges;
    held
}

/// The two receivers agree on the document and on the engine.
fn agree(carried: &TrameState, rebuilt: &TrameState, step: &Step) -> Result<(), TestCaseError> {
    prop_assert_eq!(
        stored(carried),
        stored(rebuilt),
        "the text differs after {:?}",
        step
    );
    prop_assert_eq!(
        engine_by_lines(carried),
        engine_by_lines(rebuilt),
        "the engine differs after {:?}",
        step
    );
    let same = same_values(carried, rebuilt);
    prop_assert!(same.is_ok(), "a value differs after {:?}: {:?}", step, same);
    prop_assert!(
        super::structural_tests::lines_kept(carried),
        "the index by line differs after {:?}",
        step
    );
    // What decides whether the next local structural edit carries or reseeds:
    // a cell the last reseed left out, and the formulas that read positions.
    prop_assert_eq!(
        carried.engine_skipped,
        rebuilt.engine_skipped,
        "the skipped mark differs after {:?}",
        step
    );
    prop_assert_eq!(
        positional(carried),
        positional(rebuilt),
        "the positional set differs after {:?}",
        step
    );
    prop_assert_eq!(
        unplaced(carried),
        unplaced(rebuilt),
        "the unplaced formulas differ after {:?}",
        step
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// A receiver that carries its engine across each remote batch holds what
    /// a receiver that reseeds after each batch holds (`TRAME-282`). That is
    /// the stored text, each value, each dependency, each error and each
    /// cycle refusal.
    #[test]
    fn a_carried_remote_apply_equals_a_full_rebuild(program in proptest::collection::vec(step(), 1..24)) {
        run(&program)?;
    }
}

/// What the active sheet of `st` shows over every cell the generator
/// reaches, and a margin past it.
fn screen(st: &TrameState) -> Vec<(CellPos, String)> {
    let window = CellRect::spanning(at(0, 0), at(ROWS + 8, COLS + 4));
    st.pack_cells(window)
        .into_iter()
        .map(|cell| (CellPos::new(cell.row, cell.col), format!("{cell:?}")))
        .collect()
}

/// The repaint `events` asks for covers each cell whose display changed
/// between `before` and `after` (`TRAME-167`, `TRAME-284`). A whole-window
/// repaint covers them all.
fn repainted(
    before: &[(CellPos, String)],
    after: &[(CellPos, String)],
    events: &[crate::TrameEvent],
    step: &Step,
) -> Result<(), TestCaseError> {
    if events
        .iter()
        .any(|e| matches!(e, crate::TrameEvent::DataReset))
    {
        return Ok(());
    }
    let ranges: Vec<CellRect> = events
        .iter()
        .filter_map(|e| match e {
            crate::TrameEvent::CellsChanged { range, .. } => Some(*range),
            _ => None,
        })
        .collect();
    prop_assert!(ranges.len() <= 1, "one repaint per batch after {:?}", step);
    for ((at, was), (_, now)) in before.iter().zip(after) {
        prop_assert!(
            was == now || ranges.iter().any(|r| r.contains(*at)),
            "{:?} changed and no repaint names it after {:?}: {:?}",
            at,
            step,
            ranges
        );
    }
    Ok(())
}

/// Run `program` on two authors and two receivers, and hold the receivers
/// equal after each step.
fn run(program: &[Step]) -> Result<(), TestCaseError> {
    let base = seeded(StructuralPath::Carry).export_snapshot();
    let mut authors = AUTHORS.map(|peer| opened(peer, &base, RemotePath::Carry));
    let mut queues: [VecDeque<ambre_core::Payload<ambre_core::DeltaBytes>>; 2] = Default::default();
    let mut carried = opened(RECEIVER, &base, RemotePath::Carry);
    let mut rebuilt = opened(RECEIVER, &base, RemotePath::Reseed);
    for step in program {
        match step {
            Step::Author { who, edit } => {
                let author = &mut authors[*who];
                for command in commands(author, edit) {
                    if let Some(delta) = author.handle_command(command).update {
                        queues[*who].push_back(delta);
                    }
                }
            }
            Step::Deliver { who, n } => {
                let take = (*n).min(queues[*who].len());
                let batch: Vec<_> = queues[*who].drain(..take).collect();
                let before = screen(&carried);
                let events = carried.apply_remote_batch(&batch);
                repainted(&before, &screen(&carried), &events, step)?;
                rebuilt.apply_remote_batch(&batch);
            }
            Step::SyncAuthors => {
                let [a, b] = &mut authors;
                let (sa, sb) = (a.export_snapshot(), b.export_snapshot());
                a.import_snapshot(&sb)
                    .expect("a snapshot this build wrote reads back");
                b.import_snapshot(&sa)
                    .expect("a snapshot this build wrote reads back");
            }
            Step::Local { edit } => {
                for command in commands(&mut carried, edit) {
                    let before = screen(&carried);
                    let events = carried.handle_command(command.clone()).display;
                    repainted(&before, &screen(&carried), &events, step)?;
                    rebuilt.handle_command(command);
                }
            }
            Step::Switch => {
                let next = if carried.active_sheet == sheet(&carried, "Sheet1") {
                    sheet(&carried, "Sheet2")
                } else {
                    sheet(&carried, "Sheet1")
                };
                carried.handle_command(TrameCommand::ActivateSheet {
                    sheet_id: next.clone(),
                });
                rebuilt.handle_command(TrameCommand::ActivateSheet { sheet_id: next });
            }
        }
        agree(&carried, &rebuilt, step)?;
    }
    Ok(())
}

/// A peer that wrote `=A1*2` into B1 over a sheet whose A1 holds 3, and the
/// receiver that holds it. Both start from one snapshot.
fn pair() -> (TrameState, TrameState) {
    let mut author = TrameState::with_peer(ambre_core::PeerId::new(AUTHORS[0]));
    author.handle_command(write(0, 0, "3"));
    author.handle_command(write(0, 1, "=A1*2"));
    author.handle_command(write(5, 3, "=SUM(A1:A9)"));
    let receiver = opened(RECEIVER, &author.export_snapshot(), RemotePath::Carry);
    (author, receiver)
}

/// The displayed value of the engine cell at `row`, `col` of the active sheet.
fn shown(st: &TrameState, row: u32, col: u32) -> Option<String> {
    st.engine.evaluated_text(st.active_cell_id(at(row, col)))
}

/// A peer's cell write reaches the receiver's formulas with no reseed: the
/// write and what reads it recompute (`TRAME-282`).
#[test]
fn a_remote_cell_write_recomputes_its_readers_without_a_reseed() {
    let (mut author, mut receiver) = pair();
    let reseeds = receiver.reseeds;
    let delta = author
        .handle_command(write(0, 0, "5"))
        .update
        .expect("a write sends a delta");
    receiver.apply_remote_batch(&[delta]);
    assert_eq!(receiver.reseeds, reseeds, "the batch carried the engine");
    assert_eq!(shown(&receiver, 0, 1).as_deref(), Some("10"));
    assert_eq!(shown(&receiver, 5, 3).as_deref(), Some("5"));
}

/// A peer's row delete reaches the receiver with no reseed (`TRAME-282`).
/// The range that read the row shrinks, and a reference into it reads
/// `#REF!`.
#[test]
fn a_remote_row_delete_carries_the_engine_without_a_reseed() {
    let (mut author, mut receiver) = pair();
    let setup: Vec<_> = [write(1, 0, "4"), write(2, 2, "=A2+1")]
        .into_iter()
        .filter_map(|command| author.handle_command(command).update)
        .collect();
    receiver.apply_remote_batch(&setup);
    let reseeds = receiver.reseeds;
    let delta = author
        .handle_command(TrameCommand::DeleteRow {
            at: Pos::<Row>::new(1),
        })
        .update
        .expect("a delete sends a delta");
    receiver.apply_remote_batch(&[delta]);
    assert_eq!(receiver.reseeds, reseeds, "the batch carried the engine");
    assert_eq!(shown(&receiver, 1, 2).as_deref(), Some("#REF!"));
    assert_eq!(
        shown(&receiver, 4, 3).as_deref(),
        Some("3"),
        "the sum lost the gone row"
    );
}

/// A batch that renames a sheet reseeds: a qualifier resolves through the
/// sheet maps, which only a reseed rebuilds.
#[test]
fn a_remote_sheet_rename_reseeds_the_engine() {
    let (mut author, mut receiver) = pair();
    let reseeds = receiver.reseeds;
    let first = sheet(&author, "Sheet1");
    let delta = author
        .handle_command(TrameCommand::RenameSheet {
            sheet_id: first,
            name: crate::test_support::sheet("Data"),
        })
        .update
        .expect("a rename sends a delta");
    receiver.apply_remote_batch(&[delta]);
    assert_eq!(
        receiver.reseeds,
        reseeds + 1,
        "the batch reseeded the engine"
    );
}

/// Two authors that each add the same new line, on two elements
/// (`TRAME-261`), and a receiver holding the base they started from. The
/// first author wrote `1` into A5, the second `2` into B5.
fn one_line_on_two_elements() -> (
    TrameState,
    TrameState,
    TrameState,
    Vec<ambre_core::Payload<ambre_core::DeltaBytes>>,
) {
    let base = seeded(StructuralPath::Carry).export_snapshot();
    let mut a = opened(AUTHORS[0], &base, RemotePath::Carry);
    let mut b = opened(AUTHORS[1], &base, RemotePath::Carry);
    let receiver = opened(RECEIVER, &base, RemotePath::Carry);
    let row = a.index.row_len().get();
    let mut sent = Vec::new();
    sent.extend(a.handle_command(write(row, 0, "1")).update);
    sent.extend(b.handle_command(write(row, 1, "2")).update);
    (a, b, receiver, sent)
}

/// A delete by a peer that knew one element of a line leaves the line on the
/// other. A key under the deleted element then reads nowhere, though no
/// operation of the batch names it. Only a reseed drops its cell
/// (`TRAME-282`).
#[test]
fn a_line_left_on_another_element_reseeds_the_engine() {
    let (mut a, mut b, mut receiver, sent) = one_line_on_two_elements();
    let row = a.index.row_len().get() - 1;
    receiver.apply_remote_batch(&sent);
    // The second author learns the first one's element, and writes under the
    // element it sees.
    b.import_snapshot(&a.export_snapshot())
        .expect("a snapshot this build wrote reads back");
    let formula = b
        .handle_command(write(row, 2, "=7*6"))
        .update
        .expect("a write sends a delta");
    receiver.apply_remote_batch(&[formula]);
    assert_eq!(shown(&receiver, row, 2).as_deref(), Some("42"));
    // The first author never saw the second element, and deletes the line.
    let delete = a
        .handle_command(TrameCommand::DeleteRow {
            at: Pos::<Row>::new(row),
        })
        .update
        .expect("a delete sends a delta");
    let reseeds = receiver.reseeds;
    receiver.apply_remote_batch(&[delete]);
    let rebuilt = opened(RECEIVER + 1, &receiver.export_snapshot(), RemotePath::Carry);
    let same = same_values(&receiver, &rebuilt);
    assert!(same.is_ok(), "{same:?}");
    assert!(receiver.reseeds > reseeds, "the batch reseeded the engine");
}

/// A value a peer writes into a cell that already holds one under another
/// element sits under a second key. A reseed leaves such a
/// cell out and marks the engine, so the next local structural edit reseeds
/// too. The batch reseeds to hold the same mark (`TRAME-282`).
#[test]
fn a_second_key_on_a_cell_reseeds_the_engine() {
    let (a, mut b, mut receiver, sent) = one_line_on_two_elements();
    let row = a.index.row_len().get() - 1;
    receiver.apply_remote_batch(&sent);
    assert!(!receiver.engine_skipped, "one key per cell so far");
    // The second author never saw the first one's element, and writes into
    // the cell the first one wrote.
    let second = b
        .handle_command(write(row, 0, "3"))
        .update
        .expect("a write sends a delta");
    let reseeds = receiver.reseeds;
    receiver.apply_remote_batch(&[second]);
    assert!(receiver.engine_skipped, "the cell sits under two keys");
    assert_eq!(
        receiver.reseeds,
        reseeds + 1,
        "the batch reseeded the engine"
    );
}

/// A peer deletes a row and undoes the delete, while the receiver wrote a
/// value on that row. The undo brings the row back on its own element, so the
/// receiver's value reads again, though no operation of the batch names it
/// (`TRAME-282`).
#[test]
fn an_undone_remote_delete_brings_back_a_value_written_on_the_gone_row() {
    let base = seeded(StructuralPath::Carry).export_snapshot();
    let mut author = opened(AUTHORS[0], &base, RemotePath::Carry);
    let mut receiver = opened(RECEIVER, &base, RemotePath::Carry);
    let last = author.index.row_len().get() - 1;
    let delete = author
        .handle_command(TrameCommand::DeleteRow {
            at: Pos::<Row>::new(last),
        })
        .update
        .expect("a delete sends a delta");
    receiver.handle_command(write(last, 2, "5"));
    receiver.apply_remote_batch(&[delete]);
    let undo = author
        .handle_command(TrameCommand::Undo)
        .update
        .expect("an undo sends a delta");
    receiver.apply_remote_batch(&[undo]);
    assert_eq!(shown(&receiver, last, 2).as_deref(), Some("5"));
}

/// A peer undoes a row insert and deletes a row in one batch. The batch writes
/// a formula of the other sheet with the text it already held, over a row the
/// batch removed. The formula compiles again all the same, and reads `#REF!`
/// as a reseed has it (`TRAME-282`).
#[test]
fn a_formula_written_with_its_own_text_over_a_removed_row_compiles_again() {
    use Edit::{DeleteRow, InsertRow, Undo};
    let program = [
        Step::Author {
            who: 1,
            edit: InsertRow { at: 0 },
        },
        Step::Switch,
        Step::Deliver { who: 1, n: 1 },
        Step::Switch,
        Step::Author { who: 1, edit: Undo },
        Step::Author {
            who: 1,
            edit: DeleteRow { at: 1 },
        },
        Step::Deliver { who: 1, n: 2 },
    ];
    let held = run(&program);
    assert!(held.is_ok(), "{held:?}");
}

/// The receiver writes a cell twice, and a peer adds the column of that cell
/// on an element of its own in between. The second write lands under the
/// column's visible element, so the cell sits under two keys. The engine is
/// marked as a reseed marks it, and a later batch carries that mark
/// (`TRAME-282`).
#[test]
fn a_local_write_under_a_second_key_marks_the_engine_as_a_reseed_does() {
    use Edit::Write;
    let program = [
        Step::Local {
            edit: Write {
                row: 2,
                col: 2,
                text: "0".to_owned(),
            },
        },
        Step::Author {
            who: 1,
            edit: Write {
                row: 0,
                col: 2,
                text: "=A1".to_owned(),
            },
        },
        Step::Deliver { who: 1, n: 1 },
        Step::Local {
            edit: Write {
                row: 2,
                col: 2,
                text: "0".to_owned(),
            },
        },
        Step::Author {
            who: 0,
            edit: Write {
                row: 0,
                col: 0,
                text: "2".to_owned(),
            },
        },
        Step::Deliver { who: 0, n: 1 },
    ];
    let held = run(&program);
    assert!(held.is_ok(), "{held:?}");
}

/// A peer deletes a column while another writes a formula into a new row of
/// it. The formula then places on no cell. The receiver deletes that row, and
/// its purge takes the formula with it, so no later walk reads it
/// (`TRAME-280`, `TRAME-282`).
#[test]
fn a_purge_drops_a_formula_left_under_a_removed_column() {
    use Edit::{DeleteColumn, DeleteRow, Sort, Write, WriteBelow};
    let program = [
        Step::Author {
            who: 0,
            edit: Sort {
                order: SortOrder::Descending,
            },
        },
        Step::Author {
            who: 1,
            edit: WriteBelow {
                col: 1,
                text: "=A1+B2".to_owned(),
            },
        },
        Step::Author {
            who: 0,
            edit: DeleteColumn { at: 1 },
        },
        Step::Deliver { who: 1, n: 1 },
        Step::Author {
            who: 0,
            edit: Write {
                row: 4,
                col: 0,
                text: "0".to_owned(),
            },
        },
        Step::Deliver { who: 0, n: 1 },
        Step::Deliver { who: 0, n: 2 },
        Step::Local {
            edit: DeleteRow { at: 5 },
        },
        Step::Author {
            who: 1,
            edit: Write {
                row: 0,
                col: 0,
                text: "=C2+1".to_owned(),
            },
        },
        Step::Deliver { who: 1, n: 1 },
    ];
    let held = run(&program);
    assert!(held.is_ok(), "{held:?}");
}

/// A peer deletes the column a merge ends on. The merge map is untouched,
/// and the merge's span shrinks all the same. The batch names the block
/// (`TRAME-284`).
#[test]
fn a_merge_that_loses_its_corner_line_repaints_its_block() {
    use Edit::{DeleteColumn, Merge};
    let program = [
        Step::Author {
            who: 1,
            edit: Merge { row: 0, col: 2 },
        },
        Step::Deliver { who: 1, n: 1 },
        Step::Author {
            who: 1,
            edit: DeleteColumn { at: 3 },
        },
        Step::Deliver { who: 1, n: 1 },
    ];
    let held = run(&program);
    assert!(held.is_ok(), "{held:?}");
}
