// ambre-trame/src/logic/history_tests.rs
// ============================================================================
// Trame's one history against the undo manager (`TRAME-286`), the
// reference-model regime of `ambre-spec-testing.md` section 3.
//
// Two states run one generated program in lockstep, under one peer id, so each
// mints the same uids. The first records the inverse of each edit and replays
// it on an undo or a redo. The second is the reference model: the undo manager
// holds every edit, one item per command. After every step the two must store
// the same document and offer the same undo and redo. A third state receives
// every delta the first sends, and must store what the first stores.
// ============================================================================

use crate::hash::HashMap;
use std::collections::{BTreeMap, BTreeSet};

use ambre_core::crdt::FoundationRoot;
use ambre_core::index::Pos;
use loro::LoroValue;
use proptest::prelude::*;

use super::history::HistoryPath;
use super::structural::StructuralPath;
use super::structural_tests::{PEER, Step, commands, seeded, step, write};
use super::{SheetContainer, SheetRoot, TrameState};
use crate::{CellPos, Col, Row, SortOrder, TrameCommand};

/// Every root the document stores, the peer registry aside: the registry is
/// bookkeeping that no undo touches (`AUTHOR-009`). An empty root is left out:
/// a read opens one, and an empty container stores nothing. A key of
/// `contested` is left out of every map: a peer wrote it.
fn content_but(st: &TrameState, contested: &BTreeSet<String>) -> BTreeMap<String, LoroValue> {
    let LoroValue::Map(roots) = st.doc.get_deep_value() else {
        return BTreeMap::new();
    };
    roots
        .iter()
        .filter(|(name, _)| name.as_str() != FoundationRoot::Peers.as_str())
        .map(|(name, value)| {
            let value = match value {
                LoroValue::Map(map) => LoroValue::Map(
                    map.iter()
                        .filter(|(key, _)| !contested.contains(key.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<loro::LoroMapValue>(),
                ),
                other => other.clone(),
            };
            (name.clone(), value)
        })
        .filter(|(_, value)| match value {
            LoroValue::Map(map) => !map.is_empty(),
            LoroValue::List(list) => !list.is_empty(),
            _ => true,
        })
        .collect()
}

/// Every root the document stores, as [`content_but`] reads it with no key
/// left out.
fn content(st: &TrameState) -> BTreeMap<String, LoroValue> {
    content_but(st, &BTreeSet::new())
}

/// The keys of every map whose value differs between `before` and `after`.
fn changed(
    before: &BTreeMap<String, LoroValue>,
    after: &BTreeMap<String, LoroValue>,
) -> BTreeSet<String> {
    let keys = |side: &BTreeMap<String, LoroValue>, root: &str| -> HashMap<String, LoroValue> {
        match side.get(root) {
            Some(LoroValue::Map(map)) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => HashMap::default(),
        }
    };
    let roots: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    roots
        .into_iter()
        .flat_map(|root| {
            let (b, a) = (keys(before, root), keys(after, root));
            let all: BTreeSet<String> = b.keys().chain(a.keys()).cloned().collect();
            all.into_iter()
                .filter(move |k| b.get(k) != a.get(k))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// What the two history affordances offer.
fn offered(st: &TrameState) -> (ambre_core::Availability, ambre_core::Availability) {
    (st.can_undo(), st.can_redo())
}

/// Run `program` on a state that records the inverse of each edit, beside
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
    let mut contested = BTreeSet::new();
    let mut remote = 0;
    for step in program {
        if let Step::Remote { row, col, text } = step {
            // Each state takes the write of a peer that read it. The two emit
            // different operations for one undo. A delta built on one then
            // depends on operations the other never made.
            // One identity per remote step, the same for both states: a
            // reused identity would mint operations the document holds already.
            remote += 1;
            let typed = |st: &TrameState| {
                let mut peer = TrameState::with_peer(ambre_core::PeerId::new(PEER + 10 + remote));
                peer.import_snapshot(&st.export_snapshot())
                    .expect("a snapshot this build wrote reads back");
                peer.handle_command(write(*row, *col, text)).update
            };
            if let (Some(mine), Some(theirs)) = (typed(&recorded), typed(&managed)) {
                // An undo leaves a key a peer wrote to the peer (`UNDO-007`).
                // The undo manager does not, so the two are compared on every
                // other key.
                let before = content(&recorded);
                recorded.apply_remote(&mine);
                contested.extend(changed(&before, &content(&recorded)));
                managed.apply_remote(&theirs);
                observer.apply_remote(&mine);
            }
        }
        for command in commands(step) {
            let out = recorded.handle_command(command.clone());
            managed.handle_command(command);
            if let Some(delta) = out.update {
                observer.apply_remote(&delta);
            }
        }
        prop_assert_eq!(
            content_but(&recorded, &contested),
            content_but(&managed, &contested),
            "the document differs after {:?}",
            step
        );
        // A key a peer wrote can hold a value here and another there. The two
        // documents then differ by design, and every later step reads that
        // difference: a sort orders the value, a merge masks it. The program
        // is compared up to there.
        if content(&recorded) != content(&managed) {
            prop_assert_eq!(
                content(&observer),
                content(&recorded),
                "the peer differs after {:?}",
                step
            );
            return Ok(());
        }
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

    /// Each edit records its own step, and each undo and redo of it leaves
    /// the document the undo manager would leave. The two histories offer the
    /// same steps in the same order (`TRAME-288`). A peer that receives each
    /// delta stores the same document.
    #[test]
    fn a_recorded_line_delete_undoes_as_the_undo_manager(
        program in proptest::collection::vec(step(), 1..16),
    ) {
        lockstep(&program)?;
    }
}

/// A row deleted after two sorts, then undone twice. A line delete once
/// recorded its own step beside the undo manager. The undo of the delete put
/// back a new list element. The manager's undo of the sort then revived the
/// one the delete took, and the row list held the line twice. The proptest drew it on
/// 2026-11-08. One history names each element by its value, so the sort moves
/// the element the undo put back.
#[test]
fn an_undone_delete_keeps_one_element_through_a_managed_sort() {
    let program = [
        Step::Write {
            row: 0,
            col: 0,
            text: "2".to_owned(),
        },
        Step::Sort {
            order: SortOrder::Ascending,
        },
        Step::Sort {
            order: SortOrder::Descending,
        },
        Step::DeleteRows { from: 0, to: 0 },
        Step::Undo,
        Step::Undo,
    ];
    if let Err(failure) = lockstep(&program) {
        panic!("{failure}");
    }
}

/// A row inserted, deleted, then undone twice. Beside the undo manager, the
/// undo of the recorded delete put back a new list element. The manager's undo
/// of the insert took out the old one, gone already, and the inserted row
/// stayed. The proptest drew it on 2026-11-08.
#[test]
fn an_undone_delete_of_an_inserted_row_lets_its_insert_undo() {
    let program = [
        Step::InsertRow { at: 0 },
        Step::DeleteRows { from: 0, to: 0 },
        Step::Undo,
        Step::Undo,
    ];
    if let Err(failure) = lockstep(&program) {
        panic!("{failure}");
    }
}

/// A row insert, then a move of the new row, each undone and redone. A line
/// insert that recorded its own step beside the undo manager put a new list
/// element back on its redo. The manager's redo of the move then revived the
/// element the undo took out, and the row list held the line twice.
#[test]
fn a_redone_insert_keeps_one_element_through_a_managed_move() {
    let program = [
        Step::Write {
            row: 0,
            col: 0,
            text: "9".to_owned(),
        },
        Step::InsertRow { at: 0 },
        Step::MoveRow { from: 0, to: 2 },
        Step::Undo,
        Step::Undo,
        Step::Redo,
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

/// A commit no step can replay ends the history: no undo crosses it
/// (`TRAME-290`). Each of the three writes such a commit makes, after a cell
/// write the history holds:
///
/// 1. a write that leaves a container as the value;
/// 2. a write to a map nested in another container;
/// 3. a write over a value that is a container.
#[test]
fn a_commit_over_a_container_ends_the_history() {
    let mut st = TrameState::with_peer(ambre_core::PeerId::new(1));
    let root = st.doc.get_map("cells_s0");
    let holds_one = |st: &mut TrameState, text: &str| {
        st.handle_command(write(0, 0, text));
        assert_eq!(st.undo_depth(), 1, "the cell write is a step");
    };
    holds_one(&mut st, "1");
    let nested = root
        .insert_container("over", loro::LoroMap::new())
        .expect("a root map takes a container");
    st.doc.commit();
    assert_eq!(st.undo_depth(), 0, "a container written ends the history");
    holds_one(&mut st, "2");
    nested
        .insert("key", "1")
        .expect("a nested map takes a value");
    st.doc.commit();
    assert_eq!(st.undo_depth(), 0, "a nested map written ends the history");
    holds_one(&mut st, "3");
    root.insert("over", "1").expect("a root map takes a value");
    st.doc.commit();
    assert_eq!(
        st.undo_depth(),
        0,
        "a container overwritten ends the history"
    );
}

/// A row whose cell holds a container, as a peer's write can leave it. Its
/// delete cannot record the purge of that cell, so it ends the history, and
/// an undo then writes nothing (`TRAME-290`).
#[test]
fn a_line_delete_over_a_container_ends_the_history() {
    let mut st = TrameState::with_peer(ambre_core::PeerId::new(1));
    for (row, text) in [(0, "1"), (1, "2"), (2, "3")] {
        st.handle_command(write(row, 0, text));
    }
    let cells = st
        .doc
        .get_map(SheetContainer::of(SheetRoot::Cells, &st.active_sheet).as_str());
    let key = cells
        .keys()
        .map(|key| key.to_string())
        .find(|key| {
            matches!(cells.get(key), Some(loro::ValueOrContainer::Value(LoroValue::String(s))) if s.as_str() == "2")
        })
        .expect("the second row holds its cell");
    cells
        .insert_container(&key, loro::LoroMap::new())
        .expect("a cell key takes a container");
    st.doc.commit();
    st.handle_command(write(3, 0, "4"));
    assert_eq!(
        st.undo_depth(),
        1,
        "the write after the container is a step"
    );
    st.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(1),
    });
    assert_eq!(st.undo_depth(), 0, "the delete ended the history");
    assert_eq!(shows(&st, 1, 0), "3", "the row below moved up");
    let before = content(&st);
    assert!(
        st.handle_command(TrameCommand::Undo).update.is_none(),
        "an undo writes nothing"
    );
    assert_eq!(
        content(&st),
        before,
        "the document stays as the delete left it"
    );
}

/// Alice writes a cell, Bob types over it, and Alice undoes. Bob's value
/// stands: an undo reverts this peer's own work and never a peer's
/// (`UNDO-007`). The undo manager put Alice's old value back over Bob's.
#[test]
fn an_undone_write_keeps_a_later_write_of_a_peer() {
    let mut alice = TrameState::with_peer(ambre_core::PeerId::new(1));
    alice.handle_command(write(0, 0, "1"));
    let mut bob = TrameState::with_peer(ambre_core::PeerId::new(2));
    bob.import_snapshot(&alice.export_snapshot())
        .expect("a snapshot this build wrote reads back");
    let mine = alice
        .handle_command(write(0, 0, "2"))
        .update
        .expect("the write writes");
    bob.apply_remote(&mine);
    let typed = bob
        .handle_command(write(0, 0, "3"))
        .update
        .expect("the write writes");
    alice.apply_remote(&typed);
    alice.handle_command(TrameCommand::Undo);
    assert_eq!(shows(&alice, 0, 0), "3", "the peer's value stands");
}
