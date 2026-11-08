// ambre-trame/src/logic/line_history_tests.rs
// ============================================================================
// A recorded line delete against the undo manager (`TRAME-286`), the
// reference-model regime of `ambre-spec-testing.md` section 3.
//
// Two states run one generated program in lockstep, under one peer id, so each
// mints the same uids. The first records each line delete and replays it on an
// undo or a redo. The second is the reference model: the undo manager holds
// every delete, as the code did before the recorded step existed. After every
// step the two must store the same document and offer the same undo and redo.
// A third state receives every delta the first sends, and must store what the
// first stores.
//
// A delete of a line an item of the manager placed goes to the manager. The
// property so holds both paths of a delete.
//
// One difference is the rule, not a defect. A peer may write a key the delete
// wrote, after the delete. An undo or a redo of the recorded step keeps the
// peer's value (`TRAME-287`), where the manager writes its own over it. Such a
// key may then hold the peer's value on the first state alone.
// ============================================================================

use std::collections::{BTreeMap, BTreeSet};

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

/// One stored value: a key of a root map, or a whole root list.
type Slot = (String, Option<String>);

/// Every value [`content`] holds, one per key of a root map and one per root
/// list.
fn slots(st: &TrameState) -> BTreeMap<Slot, LoroValue> {
    let mut out = BTreeMap::new();
    for (root, value) in content(st) {
        match value {
            LoroValue::Map(map) => {
                for (key, value) in map.iter() {
                    out.insert((root.clone(), Some(key.clone())), value.clone());
                }
            }
            other => {
                out.insert((root, None), other);
            }
        }
    }
    out
}

/// Check that `recorded` stores what `managed` stores, outside the keys in
/// `kept`. A key joins `kept` only through `TRAME-287`. The step undid or
/// redid a recorded delete, which `replayed` says. And the first state holds
/// the value a peer last wrote there, as `by_peer` holds it. A key leaves
/// `kept` once both agree.
fn agrees(
    recorded: &TrameState,
    managed: &TrameState,
    kept: &mut BTreeSet<Slot>,
    by_peer: &BTreeMap<Slot, Option<LoroValue>>,
    replayed: bool,
) -> Result<(), String> {
    let (ours, theirs) = (slots(recorded), slots(managed));
    kept.retain(|slot| ours.get(slot) != theirs.get(slot));
    let differ: Vec<&Slot> = ours
        .keys()
        .chain(theirs.keys())
        .filter(|slot| !kept.contains(*slot) && ours.get(*slot) != theirs.get(*slot))
        .collect();
    for slot in differ {
        let peer_wrote = by_peer
            .get(slot)
            .is_some_and(|value| value.as_ref() == ours.get(slot));
        if !(replayed && peer_wrote) {
            return Err(format!(
                "{slot:?} holds {:?}, the manager {:?}",
                ours.get(slot),
                theirs.get(slot)
            ));
        }
        kept.insert(slot.clone());
    }
    Ok(())
}

/// What the two history affordances offer.
fn offered(st: &TrameState) -> (ambre_core::Availability, ambre_core::Availability) {
    (st.can_undo(), st.can_redo())
}

/// Whether `step` may leave an item of the undo manager that places a line:
/// an insert, a move, a sort, and any undo or redo.
fn places(step: &Step) -> bool {
    matches!(
        step,
        Step::InsertRow { .. }
            | Step::InsertColumn { .. }
            | Step::MoveRow { .. }
            | Step::MoveColumn { .. }
            | Step::Sort { .. }
            | Step::Undo
            | Step::Redo
    )
}

/// Whether `step` deletes lines.
fn deletes(step: &Step) -> bool {
    matches!(
        step,
        Step::DeleteRow { .. } | Step::DeleteColumn { .. } | Step::DeleteRows { .. }
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Each line delete records its own step, and each undo and redo of it
    /// leaves the document the undo manager would leave (`TRAME-286`). The
    /// two histories offer the same steps in the same order, line deletes
    /// mixed with every other edit (`TRAME-288`). A peer that receives each
    /// delta stores the same document. A delete records its step at least
    /// until a step that may place a line runs. A key a peer wrote after a
    /// delete keeps the peer's value through its undo and redo (`TRAME-287`).
    #[test]
    fn a_recorded_line_delete_undoes_as_the_undo_manager(
        program in proptest::collection::vec(step(), 1..16),
    ) {
        let mut recorded = seeded(StructuralPath::Carry);
        let mut managed = seeded(StructuralPath::Carry);
        managed.history_path = HistoryPath::Managed;
        let mut observer = TrameState::with_peer(ambre_core::PeerId::new(PEER + 2));
        observer.import_snapshot(&recorded.export_snapshot())
            .expect("a snapshot this build wrote reads back");
        let mut peer = TrameState::with_peer(ambre_core::PeerId::new(PEER + 1));
        let mut placed = false;
        let mut kept = BTreeSet::new();
        let mut by_peer = BTreeMap::new();
        for step in &program {
            if let Step::Remote { row, col, text } = step {
                peer.import_snapshot(&recorded.export_snapshot())
                    .expect("a snapshot this build wrote reads back");
                if let Some(delta) = peer.handle_command(write(*row, *col, text)).update {
                    let before = slots(&recorded);
                    recorded.apply_remote(&delta);
                    managed.apply_remote(&delta);
                    observer.apply_remote(&delta);
                    let after = slots(&recorded);
                    for slot in before.keys().chain(after.keys()) {
                        if before.get(slot) != after.get(slot) {
                            by_peer.insert(slot.clone(), after.get(slot).cloned());
                        }
                    }
                }
            }
            let replayed = match step {
                Step::Undo => recorded.last_step_is_recorded(),
                Step::Redo => recorded.next_redo_is_recorded(),
                _ => false,
            };
            for command in commands(step) {
                let out = recorded.handle_command(command.clone());
                managed.handle_command(command);
                if let Some(delta) = out.update {
                    observer.apply_remote(&delta);
                    if deletes(step) && !placed {
                        prop_assert!(recorded.last_step_is_recorded(), "{:?} went to the manager", step);
                    }
                }
            }
            if let Err(why) = agrees(&recorded, &managed, &mut kept, &by_peer, replayed) {
                prop_assert!(false, "the document differs after {:?}: {}", step, why);
            }
            prop_assert_eq!(offered(&recorded), offered(&managed), "the history differs after {:?}", step);
            prop_assert_eq!(content(&observer), content(&recorded), "the peer differs after {:?}", step);
            placed |= places(step);
        }
    }
}

/// Run `program` on a state that records each line delete and on the
/// reference model. After each step both store the same document and offer
/// the same undo and redo.
fn runs_as_the_undo_manager(program: &[Step]) -> TrameState {
    let mut recorded = seeded(StructuralPath::Carry);
    let mut managed = seeded(StructuralPath::Carry);
    managed.history_path = HistoryPath::Managed;
    for step in program {
        for command in commands(step) {
            recorded.handle_command(command.clone());
            managed.handle_command(command);
        }
        assert_eq!(
            content(&recorded),
            content(&managed),
            "the document differs after {step:?}"
        );
        assert_eq!(
            offered(&recorded),
            offered(&managed),
            "the history differs after {step:?}"
        );
    }
    recorded
}

/// A row is inserted, deleted, and both are undone. The undo of the delete
/// puts back a new element, which the undo manager's item for the insert does
/// not name. So the delete goes to the manager, and the undo of the insert
/// takes the row out (`TRAME-286`). The shrunk case of the proptest.
#[test]
fn an_undone_delete_of_an_inserted_row_lets_the_insert_undo() {
    runs_as_the_undo_manager(&[
        Step::InsertRow { at: 0 },
        Step::DeleteRows { from: 0, to: 0 },
        Step::Undo,
        Step::Undo,
    ]);
}

/// The same through a move, a sort, a column, and a delete undone twice. Each
/// item that put a line where it stands still takes it back.
#[test]
fn an_undone_delete_of_a_placed_line_lets_its_item_undo() {
    let programs = [
        vec![
            Step::MoveRow { from: 0, to: 2 },
            Step::DeleteRow { at: 2 },
            Step::Undo,
            Step::Undo,
            Step::Redo,
            Step::Redo,
        ],
        vec![
            Step::Sort {
                order: crate::message::SortOrder::Descending,
            },
            Step::DeleteRow { at: 0 },
            Step::Undo,
            Step::Undo,
        ],
        vec![
            Step::InsertColumn { at: 1 },
            Step::DeleteColumn { at: 1 },
            Step::Undo,
            Step::Undo,
        ],
        vec![
            Step::InsertRow { at: 0 },
            Step::DeleteRow { at: 0 },
            Step::Undo,
            Step::DeleteRow { at: 0 },
            Step::Undo,
            Step::Undo,
        ],
    ];
    for program in programs {
        runs_as_the_undo_manager(&program);
    }
}

/// A line no item of the manager placed keeps its recorded step, an insert
/// elsewhere on the axis notwithstanding.
#[test]
fn a_delete_out_of_reach_of_the_manager_stays_recorded() {
    let st = runs_as_the_undo_manager(&[Step::InsertRow { at: 0 }, Step::DeleteRow { at: 2 }]);
    assert!(st.last_step_is_recorded(), "the delete is recorded");
}

/// A formula reads a row, the row is deleted, and a peer types over the
/// formula the delete rewrote. The undo brings the row back and keeps what
/// the peer typed (`TRAME-287`), where the undo manager would write the
/// formula back over it. The shrunk case of the proptest, whose reference
/// model held the manager's value.
#[test]
fn an_undone_delete_keeps_a_peer_write_the_manager_would_overwrite() {
    let mut st = seeded(StructuralPath::Carry);
    st.handle_command(write(0, 1, "=SUM(2:3)"));
    st.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(1),
    });
    assert!(st.last_step_is_recorded(), "the delete is recorded");
    let mut peer = TrameState::with_peer(ambre_core::PeerId::new(PEER + 1));
    peer.import_snapshot(&st.export_snapshot())
        .expect("a snapshot this build wrote reads back");
    let typed = peer
        .handle_command(write(0, 1, "0"))
        .update
        .expect("the write writes");
    st.apply_remote(&typed);
    st.handle_command(TrameCommand::Undo);
    assert_eq!(shows(&st, 1, 0), "2", "the row is back");
    assert_eq!(shows(&st, 0, 1), "0", "the peer's write stands");
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
