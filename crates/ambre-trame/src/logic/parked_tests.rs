// ambre-trame/src/logic/parked_tests.rs
// ============================================================================
// The formulas a line delete parks, and the undo that takes them back
// (`TRAME-300`, `TRAME-301`).
//
// A delete parks the bytecode and the source of each formula it removes. An
// undo that writes the same text back over the same lines takes the parked
// bytecode, and compiles nothing. A line the formula named that is gone, or a
// formula that reads positions, compiles as before. Another text compiles too:
// `formula::engine::tests` holds that case. The proptest of `structural_tests`
// holds every value and every bytecode against a full reseed, which compiles
// each formula again.
// ============================================================================

use ambre_core::index::Pos;
use proptest::prelude::*;

use super::TrameState;
use super::structural::StructuralPath;
use super::structural_tests::{
    COLS, PEER, ROWS, Step, commands, engine, same_values, seeded, step, stored, write,
};
use crate::{CellPos, Col, Row, TrameCommand};

/// The text a cell shows on the active sheet.
fn shows(st: &TrameState, row: u32, col: u32) -> String {
    st.cell_text(CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)))
}

/// A sheet of `rows` rows, as the block bench writes it. Column A holds a
/// number, and column B a formula that reads A of its own row.
fn sheet(peer: u64, rows: u32) -> TrameState {
    let mut st = TrameState::with_peer(ambre_core::PeerId::new(peer));
    for row in 0..rows {
        st.handle_command(write(row, 0, &row.to_string()));
        st.handle_command(write(row, 1, &format!("=A{}*2", row + 1)));
    }
    st
}

/// Delete the rows `from..=to` in one command (`TRAME-088`).
fn delete_rows(
    st: &mut TrameState,
    from: u32,
    to: u32,
) -> Option<ambre_core::Payload<ambre_core::DeltaBytes>> {
    commands(&Step::DeleteRows { from, to })
        .into_iter()
        .map(|command| st.handle_command(command).update)
        .last()
        .flatten()
}

/// What column B shows, row by row.
fn column_b(st: &TrameState, rows: u32) -> Vec<String> {
    (0..rows).map(|row| shows(st, row, 1)).collect()
}

/// The undo of a block delete compiles none of the formulas it restores, and
/// each shows what it showed before the delete. The redo parks them again, so
/// a second undo compiles none either.
#[test]
fn an_undone_block_delete_compiles_no_restored_formula() {
    let mut st = sheet(1, 20);
    let before = column_b(&st, 20);
    delete_rows(&mut st, 5, 14);
    let compiled = st.engine.bytecode.compiles();
    st.handle_command(TrameCommand::Undo);
    assert_eq!(
        st.engine.bytecode.compiles(),
        compiled,
        "the undo compiles no restored formula"
    );
    assert_eq!(column_b(&st, 20), before, "each formula shows its value");
    st.handle_command(TrameCommand::Redo);
    st.handle_command(TrameCommand::Undo);
    assert_eq!(
        st.engine.bytecode.compiles(),
        compiled,
        "the redo parks them again"
    );
    assert_eq!(column_b(&st, 20), before);
}

/// A peer deletes a row that a formula of the block names, after the block
/// delete. The undo writes the formula's text back, but its line is gone: the
/// formula compiles, and reads `#REF!` as a fresh compile does.
#[test]
fn a_formula_whose_line_went_meanwhile_compiles_on_the_undo() {
    let mut alice = sheet(1, 6);
    alice.handle_command(write(4, 2, "=A1+1"));
    let mut bob = TrameState::with_peer(ambre_core::PeerId::new(2));
    bob.import_snapshot(&alice.export_snapshot())
        .expect("a snapshot this build wrote reads back");
    let delete = delete_rows(&mut alice, 3, 4).expect("the delete writes");
    bob.apply_remote(&delete);
    let gone = bob
        .handle_command(TrameCommand::DeleteRow {
            at: Pos::<Row>::new(0),
        })
        .update
        .expect("the delete writes");
    alice.apply_remote(&gone);
    let compiled = alice.engine.bytecode.compiles();
    alice.handle_command(TrameCommand::Undo);
    assert!(
        alice.engine.bytecode.compiles() > compiled,
        "the formula that names the gone row compiles"
    );
    assert_eq!(shows(&alice, 3, 2), "#REF!", "its row is gone");
    assert_eq!(shows(&alice, 3, 1), "8", "a formula of the block computes");
}

/// A peer moves the rows a formula of the block reads by a range, while the
/// delete stands. A range reads positions, and the delete parks no such
/// formula. Its undo compiles it, and it sums the cells the range covers
/// after the move.
#[test]
fn a_formula_that_reads_positions_compiles_on_the_undo() {
    let mut alice = sheet(1, 6);
    alice.handle_command(write(4, 2, "=SUM(A1:A2)"));
    let (compiled, alice) = undo_after_a_peer_move(alice);
    assert!(
        alice.engine.bytecode.compiles() > compiled,
        "the range formula compiles"
    );
    assert_eq!(shows(&alice, 4, 2), "3", "it sums the rows now on 1 and 2");
}

/// Delete row 5 on `alice`; a peer then moves row 1 to row 3, and `alice`
/// undoes the delete. Returns the compiles before the undo, and `alice`.
fn undo_after_a_peer_move(mut alice: TrameState) -> (u64, TrameState) {
    let mut bob = TrameState::with_peer(ambre_core::PeerId::new(2));
    bob.import_snapshot(&alice.export_snapshot())
        .expect("a snapshot this build wrote reads back");
    let delete = delete_rows(&mut alice, 4, 4).expect("the delete writes");
    bob.apply_remote(&delete);
    let moved = bob
        .handle_command(TrameCommand::MoveRow {
            from: Pos::<Row>::new(0),
            to: Pos::<Row>::new(2),
        })
        .update
        .expect("the move writes");
    alice.apply_remote(&moved);
    let compiled = alice.engine.bytecode.compiles();
    alice.handle_command(TrameCommand::Undo);
    (compiled, alice)
}

/// A row a parked formula names moves while the delete stands. The formula
/// names the row, not its position. The parked bytecode still binds the row,
/// so the undo reuses it, and the formula reads the row where it now sits.
#[test]
fn a_parked_formula_follows_a_line_that_moved() {
    let mut alice = sheet(1, 6);
    alice.handle_command(write(4, 2, "=A1+1"));
    let (compiled, alice) = undo_after_a_peer_move(alice);
    assert_eq!(
        alice.engine.bytecode.compiles(),
        compiled,
        "the moved row is still the row the formula names"
    );
    assert_eq!(shows(&alice, 4, 2), "1", "A of the moved row, plus one");
}

/// The parked formulas live while an undo can write them back. A history the
/// session empties drops them.
#[test]
fn an_emptied_history_drops_the_parked_formulas() {
    let mut st = sheet(1, 8);
    delete_rows(&mut st, 2, 5);
    assert!(st.engine.bytecode.has_parked(), "the delete parks");
    st.clear_history();
    assert!(
        !st.engine.bytecode.has_parked(),
        "nothing can write them back"
    );
}

/// What a parked block holds: five hundred formulas of the block bench, the
/// bytecode and the source of each.
#[test]
fn a_parked_block_of_five_hundred_rows_holds_its_bytecode() {
    let mut st = sheet(1, 600);
    delete_rows(&mut st, 50, 549);
    let bytes = st.engine.bytecode.parked_bytes();
    assert!(bytes <= 500 * 1024, "{bytes} bytes for 500 formulas");
}

/// One step of the program the oracle runs. It is a step of the structural
/// program, or a peer's edit of the lines that lands while a delete stands
/// parked.
#[derive(Debug, Clone)]
enum Parking {
    Local(Step),
    PeerDeletesRow { at: u32 },
    PeerDeletesColumn { at: u32 },
    PeerMovesRow { from: u32, to: u32 },
}

fn parking() -> impl Strategy<Value = Parking> {
    prop_oneof![
        4 => step().prop_map(Parking::Local),
        2 => (0..ROWS, 0..ROWS).prop_map(|(a, b)| Parking::Local(Step::DeleteRows { from: a.min(b), to: a.max(b) })),
        3 => Just(Parking::Local(Step::Undo)),
        1 => Just(Parking::Local(Step::Redo)),
        2 => (0..ROWS).prop_map(|at| Parking::PeerDeletesRow { at }),
        1 => (0..COLS).prop_map(|at| Parking::PeerDeletesColumn { at }),
        1 => (0..ROWS, 0..ROWS).prop_map(|(from, to)| Parking::PeerMovesRow { from, to }),
    ]
}

/// The peer's command of one step, or `None` for a step of this peer.
fn peer_command(step: &Parking) -> Option<TrameCommand> {
    match step {
        Parking::PeerDeletesRow { at } => Some(TrameCommand::DeleteRow {
            at: Pos::<Row>::new(*at),
        }),
        Parking::PeerDeletesColumn { at } => Some(TrameCommand::DeleteColumn {
            at: Pos::<Col>::new(*at),
        }),
        Parking::PeerMovesRow { from, to } => Some(TrameCommand::MoveRow {
            from: Pos::<Row>::new(*from),
            to: Pos::<Row>::new(*to),
        }),
        Parking::Local(_) => None,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Each step leaves the engine that reuses parked bytecode equal to a
    /// full reseed. The reseed compiles every formula from its text. The two
    /// hold the same bytecode, edge and value (`TRAME-301`). Peers delete and
    /// move lines while a delete of this peer stands parked.
    #[test]
    fn a_reused_bytecode_equals_a_fresh_compile(program in proptest::collection::vec(parking(), 1..12)) {
        let mut carried = seeded(StructuralPath::Carry);
        let mut rebuilt = seeded(StructuralPath::Reseed);
        let mut peer = TrameState::with_peer(ambre_core::PeerId::new(PEER + 1));
        for step in &program {
            if let Some(command) = peer_command(step) {
                peer.import_snapshot(&carried.export_snapshot())
                    .expect("a snapshot this build wrote reads back");
                if let Some(delta) = peer.handle_command(command).update {
                    carried.apply_remote(&delta);
                    rebuilt.apply_remote(&delta);
                }
            }
            if let Parking::Local(local) = step {
                if let Step::Remote { row, col, text } = local {
                    peer.import_snapshot(&carried.export_snapshot())
                        .expect("a snapshot this build wrote reads back");
                    if let Some(delta) = peer.handle_command(write(*row, *col, text)).update {
                        carried.apply_remote(&delta);
                        rebuilt.apply_remote(&delta);
                    }
                }
                for command in commands(local) {
                    carried.handle_command(command.clone());
                    rebuilt.handle_command(command);
                }
            }
            prop_assert_eq!(stored(&carried), stored(&rebuilt), "the text differs after {:?}", step);
            prop_assert_eq!(engine(&carried), engine(&rebuilt), "the engine differs after {:?}", step);
            let same = same_values(&carried, &rebuilt);
            prop_assert!(same.is_ok(), "a value differs after {:?}: {:?}", step, same);
        }
    }
}
