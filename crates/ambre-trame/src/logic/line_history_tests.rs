// ambre-trame/src/logic/line_history_tests.rs
// ============================================================================
// A recorded line insert or delete against the undo manager (`TRAME-286`,
// `TRAME-289`), the reference-model regime of `ambre-spec-testing.md`
// section 3.
//
// Two states run one generated program in lockstep, under one peer id, so each
// mints the same uids. The first records each line insert and delete, and
// replays it on an undo or a redo. The second is the reference model: the undo
// manager holds every insert and delete, as the code did before the recorded
// step existed. After every
// step the two must store the same document and offer the same undo and redo.
// A third state receives every delta the first sends, and must store what the
// first stores.
// ============================================================================

use std::collections::BTreeMap;

use ambre_core::crdt::FoundationRoot;
use ambre_core::index::Pos;
use loro::LoroValue;
use proptest::prelude::*;

use super::TrameState;
use super::line_history::HistoryPath;
use super::structural::StructuralPath;
use super::structural_tests::{PEER, Step, commands, seeded, step, write};
use crate::{CellPos, Col, Row, TrameCommand};

/// Every root the document stores, the peer registry aside: the registry is
/// bookkeeping that no undo touches (`AUTHOR-009`). An empty root is left out:
/// a read opens one, and an empty container stores nothing.
fn content(st: &TrameState) -> BTreeMap<String, LoroValue> {
    let LoroValue::Map(roots) = st.doc.get_deep_value() else {
        return BTreeMap::new();
    };
    roots
        .iter()
        .filter(|(name, _)| name.as_str() != FoundationRoot::Peers.as_str())
        .filter(|(_, value)| match value {
            LoroValue::Map(map) => !map.is_empty(),
            LoroValue::List(list) => !list.is_empty(),
            _ => true,
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// What the two history affordances offer.
fn offered(st: &TrameState) -> (ambre_core::Availability, ambre_core::Availability) {
    (st.can_undo(), st.can_redo())
}

/// Whether `step` inserts or deletes lines.
fn inserts_or_deletes(step: &Step) -> bool {
    matches!(
        step,
        Step::InsertRow { .. }
            | Step::InsertColumn { .. }
            | Step::DeleteRow { .. }
            | Step::DeleteColumn { .. }
            | Step::DeleteRows { .. }
    )
}

/// Run `program` on a state that records each line insert and delete, beside
/// the reference model and a peer that receives each delta. After each step
/// the three must store the same document, and the two histories must offer
/// the same undo and redo.
fn lockstep(program: &[Step]) -> Result<(), TestCaseError> {
    let mut recorded = seeded(StructuralPath::Carry);
    let mut managed = seeded(StructuralPath::Carry);
    managed.history_path = HistoryPath::Managed;
    let mut observer = TrameState::with_peer(ambre_core::PeerId::new(PEER + 2));
    observer
        .import_snapshot(&recorded.export_snapshot())
        .expect("a snapshot this build wrote reads back");
    let mut peer = TrameState::with_peer(ambre_core::PeerId::new(PEER + 1));
    for step in program {
        if let Step::Remote { row, col, text } = step {
            peer.import_snapshot(&recorded.export_snapshot())
                .expect("a snapshot this build wrote reads back");
            if let Some(delta) = peer.handle_command(write(*row, *col, text)).update {
                recorded.apply_remote(&delta);
                managed.apply_remote(&delta);
                observer.apply_remote(&delta);
            }
        }
        for command in commands(step) {
            let out = recorded.handle_command(command.clone());
            managed.handle_command(command);
            if let Some(delta) = out.update {
                observer.apply_remote(&delta);
                if inserts_or_deletes(step) {
                    prop_assert!(
                        recorded.last_step_is_recorded(),
                        "{:?} went to the manager",
                        step
                    );
                }
            }
        }
        prop_assert_eq!(
            content(&recorded),
            content(&managed),
            "the document differs after {:?}",
            step
        );
        prop_assert_eq!(
            offered(&recorded),
            offered(&managed),
            "the history differs after {:?}",
            step
        );
        prop_assert_eq!(
            content(&observer),
            content(&recorded),
            "the peer differs after {:?}",
            step
        );
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Each line insert and delete records its own step. Each undo and redo
    /// of it leaves the document the undo manager would leave (`TRAME-286`,
    /// `TRAME-289`). The two histories offer the same steps in
    /// the same order, line edits mixed with every other edit (`TRAME-288`).
    /// A peer that receives each delta stores the same document.
    #[test]
    fn a_recorded_line_delete_undoes_as_the_undo_manager(
        program in proptest::collection::vec(step(), 1..16),
    ) {
        lockstep(&program)?;
    }
}

/// Each insert, of a row and of a column, before and inside the formulas,
/// undone and redone twice. The generated programs rarely put an undo and a
/// redo right after an insert, so this program holds `TRAME-289` on its own.
#[test]
fn a_recorded_line_insert_undoes_as_the_undo_manager() {
    let program = [
        Step::InsertRow { at: 1 },
        Step::Undo,
        Step::Redo,
        Step::Undo,
        Step::Redo,
        Step::InsertColumn { at: 0 },
        Step::Write {
            row: 0,
            col: 0,
            text: "=B3+1".to_owned(),
        },
        Step::Undo,
        Step::Undo,
        Step::Redo,
        Step::Redo,
        Step::InsertRow { at: 0 },
        Step::Remote {
            row: 2,
            col: 1,
            text: "5".to_owned(),
        },
        Step::Undo,
        Step::Redo,
    ];
    if let Err(failure) = lockstep(&program) {
        panic!("{failure}");
    }
}

/// The text a cell shows on the active sheet.
fn shows(st: &TrameState, row: u32, col: u32) -> String {
    st.cell_text(CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)))
}

/// Alice deletes a row a formula reads, and the delete rewrites the formula.
/// Bob then types over that cell. Alice's undo brings the row back and keeps
/// what Bob typed: the step reverts Alice's own work alone (`TRAME-287`).
#[test]
fn an_undone_line_delete_keeps_a_later_write_of_a_peer() {
    let mut alice = TrameState::with_peer(ambre_core::PeerId::new(1));
    for (row, text) in [(0, "1"), (1, "2"), (2, "=A2*3")] {
        alice.handle_command(write(row, 0, text));
    }
    let mut bob = TrameState::with_peer(ambre_core::PeerId::new(2));
    bob.import_snapshot(&alice.export_snapshot())
        .expect("a snapshot this build wrote reads back");
    let delete = alice
        .handle_command(TrameCommand::DeleteRow {
            at: Pos::<Row>::new(1),
        })
        .update
        .expect("the delete writes");
    assert!(alice.last_step_is_recorded(), "the delete is recorded");
    bob.apply_remote(&delete);
    assert_eq!(shows(&bob, 1, 0), "#REF!", "the formula lost its row");
    let typed = bob
        .handle_command(write(1, 0, "7"))
        .update
        .expect("the write writes");
    alice.apply_remote(&typed);
    let undo = alice
        .handle_command(TrameCommand::Undo)
        .update
        .expect("the undo writes");
    bob.apply_remote(&undo);
    for st in [&alice, &bob] {
        assert_eq!(shows(st, 1, 0), "2", "the row is back");
        assert_eq!(shows(st, 2, 0), "7", "the peer's write stands");
    }
    let redo = alice
        .handle_command(TrameCommand::Redo)
        .update
        .expect("the redo writes");
    bob.apply_remote(&redo);
    for st in [&alice, &bob] {
        assert_eq!(shows(st, 0, 0), "1", "the row above stays");
        assert_eq!(
            shows(st, 1, 0),
            "7",
            "the peer's write stands after the redo"
        );
    }
}
