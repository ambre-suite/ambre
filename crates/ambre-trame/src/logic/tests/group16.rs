// ambre-trame/src/logic/tests/group16.rs
// A block delete hides its lines under one register, and a sweep removes
// them once their session has ended (`TRAME-302` to `TRAME-305`).
//
// After each step the peers, and a peer rebuilt from the converged snapshot,
// show one grid.

use super::*;
use crate::logic::{SheetContainer, SheetRoot};
use ambre_core::authorship::PeerEntry;
use ambre_core::id::User;
use ambre_core::{FixedBytes, PeerId};

/// Write `text` at grid `(row, col)`, as one edit.
fn put(state: &mut TrameState, row: u32, col: u32, text: &str) {
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(row),
        col: Pos::<Col>::new(col),
        text: text.to_owned(),
    });
    state.handle_command(TrameCommand::CommitEdit);
}

/// Select the rows `first..=last` and delete them as one block.
fn delete_rows(state: &mut TrameState, first: u32, last: u32) -> TrameOutcome {
    state.handle_command(TrameCommand::SetActiveCell {
        row: Pos::<Row>::new(last),
        col: Pos::<Col>::new(0),
        anchor_row: Pos::<Row>::new(first),
        anchor_col: Pos::<Col>::new(0),
        extra: Vec::new(),
        kind: crate::SelectionKind::Cells,
    });
    state.handle_command(TrameCommand::DeleteSelectedRows)
}

/// A peer that starts from the history of `from`.
fn peer_of(from: &TrameState) -> TrameState {
    let mut peer = TrameState::new();
    peer.import_snapshot(&from.export_snapshot())
        .expect("a snapshot this build wrote");
    peer
}

/// Exchange the whole history of every peer with every other one.
fn exchange(peers: &mut [&mut TrameState]) {
    let snapshots: Vec<_> = peers.iter().map(|p| p.export_snapshot()).collect();
    for peer in peers.iter_mut() {
        for snapshot in &snapshots {
            peer.import_snapshot(snapshot)
                .expect("a snapshot this build wrote");
        }
    }
}

/// Every cell the grid shows, row by row, over the first two columns.
fn grid(state: &mut TrameState) -> Vec<[String; 2]> {
    let rows = state.index.row_len().get();
    (0..rows)
        .map(|row| [read_cell(state, row, 0), read_cell(state, row, 1)])
        .collect()
}

/// Both peers, and a third rebuilt from the converged snapshot, show one grid.
fn converged(a: &mut TrameState, b: &mut TrameState) -> Vec<[String; 2]> {
    let ga = grid(a);
    assert_eq!(ga, grid(b), "the two peers show one grid");
    let mut fresh = peer_of(a);
    assert_eq!(ga, grid(&mut fresh), "a rebuilt peer shows the same grid");
    ga
}

/// Ten rows: `i` in column A and `=A{i}*2` in column B.
fn sheet(peer: u64) -> TrameState {
    let mut a = TrameState::with_peer(PeerId::new(peer));
    for row in 0..10 {
        put(&mut a, row, 0, &(row + 1).to_string());
        put(&mut a, row, 1, &format!("=A{}*2", row + 1));
    }
    a
}

/// How many entries the sheet map `root` of the active sheet holds.
fn entries(state: &TrameState, root: SheetRoot) -> usize {
    state
        .doc
        .get_map(SheetContainer::of(root, &state.active_sheet).as_str())
        .len()
}

/// How many hiding registers the active sheet holds.
fn registers(state: &TrameState) -> usize {
    entries(state, SheetRoot::LineHidden)
}

/// Run a whole sweep, one slice per call, and count the slices that wrote:
/// the slices of the lines, then the one of their generations.
fn sweep(state: &mut TrameState) -> usize {
    state.start_sweep();
    let mut slices = 0;
    while state.has_pending_work() {
        if state.resume().update.is_some() {
            slices += 1;
        }
    }
    slices
}

/// `TRAME-302`: a block delete writes one register, and no cell, style,
/// generation or list entry. Its undo deletes the register alone.
#[test]
fn a_block_delete_writes_one_register_and_nothing_else() {
    let mut a = sheet(1);
    let cells = entries(&a, SheetRoot::Cells);
    let generations = entries(&a, SheetRoot::LineGen);
    let elements =
        a.axes().index().rows().len().get() as usize + a.axes().index().rows().hidden().to_usize();
    delete_rows(&mut a, 2, 5);
    assert_eq!(a.index.row_len().get(), 6, "four rows hidden");
    assert_eq!(registers(&a), 1);
    assert_eq!(entries(&a, SheetRoot::Cells), cells, "no cell key written");
    assert_eq!(entries(&a, SheetRoot::LineGen), generations);
    assert_eq!(
        a.doc
            .get_movable_list(SheetContainer::of(SheetRoot::Rows, &a.active_sheet).as_str())
            .len(),
        elements,
        "the list keeps every element"
    );
    a.handle_command(TrameCommand::Undo);
    assert_eq!(registers(&a), 0);
    assert_eq!(a.index.row_len().get(), 10);
    assert_eq!(read_cell(&mut a, 5, 1), "12", "the formula reads its row");
}

/// `TRAME-303`: peer A hides rows 3 to 6. Peer B, at the same moment, types
/// into row 4, inserts a line inside the block and types into it. A undoes
/// the delete, B undoes its last write, and A redoes.
#[test]
fn a_hidden_block_converges_under_a_write_an_insert_and_two_undos() {
    let mut a = sheet(1);
    let mut b = peer_of(&a);
    delete_rows(&mut a, 2, 5);
    put(&mut b, 3, 0, "99");
    b.handle_command(TrameCommand::InsertRow {
        at: Pos::<Row>::new(4),
    });
    put(&mut b, 4, 0, "new");
    exchange(&mut [&mut a, &mut b]);
    let merged = converged(&mut a, &mut b);
    // The line B inserted inside the block shows, and the value B typed
    // into a hidden row shows nowhere.
    assert_eq!(merged.len(), 7, "{merged:?}");
    assert!(merged.iter().any(|r| r[0] == "new"), "{merged:?}");
    assert!(!merged.iter().any(|r| r[0] == "99"), "{merged:?}");

    a.handle_command(TrameCommand::Undo);
    exchange(&mut [&mut a, &mut b]);
    let undone = converged(&mut a, &mut b);
    assert_eq!(undone.len(), 11, "{undone:?}");
    let at = undone
        .iter()
        .position(|r| r[0] == "99")
        .expect("B's value is back");
    assert_eq!(undone[at][1], "198", "its formula reads it: {undone:?}");

    b.handle_command(TrameCommand::Undo);
    exchange(&mut [&mut a, &mut b]);
    let last = converged(&mut a, &mut b);
    assert!(!last.iter().any(|r| r[0] == "new"), "{last:?}");

    a.handle_command(TrameCommand::Redo);
    exchange(&mut [&mut a, &mut b]);
    let redone = converged(&mut a, &mut b);
    assert_eq!(
        redone.len(),
        7,
        "the blank line B inserted stays: {redone:?}"
    );
}

/// `TRAME-303`: two peers hide overlapping blocks. One undoes, and the rows
/// the other hid stay hidden on both peers.
#[test]
fn an_undo_drops_its_own_register_and_keeps_the_other_peers() {
    let mut a = sheet(1);
    let mut b = peer_of(&a);
    delete_rows(&mut a, 2, 5);
    delete_rows(&mut b, 4, 7);
    exchange(&mut [&mut a, &mut b]);
    let both = converged(&mut a, &mut b);
    assert_eq!(both.len(), 4, "rows 3 to 8 hidden: {both:?}");
    a.handle_command(TrameCommand::Undo);
    exchange(&mut [&mut a, &mut b]);
    let left = converged(&mut a, &mut b);
    assert_eq!(left.len(), 6, "rows 5 to 8 stay hidden: {left:?}");
    assert_eq!(left[2][0], "3", "{left:?}");
    assert_eq!(left[3][0], "4", "{left:?}");
}

/// `TRAME-304`: a formula outside the block that reads a hidden cell reads
/// `#REF!`, and the formulas of the hidden lines keep their text. An undo
/// brings the reference back.
#[test]
fn a_hidden_formula_keeps_its_text_and_a_reference_to_it_freezes() {
    let mut a = sheet(1);
    put(&mut a, 9, 1, "=B3+1");
    delete_rows(&mut a, 2, 3);
    assert_eq!(read_cell(&mut a, 7, 1), "#REF!");
    let hidden_formulas = a
        .doc
        .get_map(SheetContainer::of(SheetRoot::Cells, &a.active_sheet).as_str())
        .get_deep_value();
    let hidden_formulas = format!("{hidden_formulas:?}");
    assert!(
        hidden_formulas.matches("#REF!").count() == 1,
        "only the formula outside the block freezes: {hidden_formulas}"
    );
    a.handle_command(TrameCommand::Undo);
    assert_eq!(read_cell(&mut a, 9, 1), "7");
}

/// Bob's user.
const BOB: Id<User> = Id::from_bytes([2; 16]);

/// Alice and Bob on one workbook. Bob writes under a session the registry
/// ties to his user, then hides rows 3 to 6, and Alice reads both writes.
fn bob_hid_a_block() -> (TrameState, TrameState) {
    let mut alice = sheet(1);
    let mut bob = peer_of(&alice);
    let entry = PeerEntry {
        user: BOB,
        nonce: FixedBytes::from_array([9; 16]),
        signature: FixedBytes::from_array([0; 64]),
    };
    assert!(bob.adopt_session(PeerId::new(22), entry));
    bob.handle_command(TrameCommand::ResizeRow {
        version: ambre_core::Seq::ZERO,
        sheet_id: bob.active_sheet.clone(),
        row: Pos::<Row>::new(3),
        height: ambre_core::Px::<ambre_core::Doc>::new(40.0),
    });
    delete_rows(&mut bob, 2, 5);
    alice
        .import_snapshot(&bob.export_snapshot())
        .expect("Bob's writes import");
    (alice, bob)
}

/// `TRAME-305`: a block hidden by a session that ended leaves the document
/// with its cells, and the grid does not change. The removal reaches the
/// other peer, and a second sweep writes nothing.
#[test]
fn a_block_hidden_by_a_session_that_ended_is_removed_for_good() {
    let (mut alice, mut bob) = bob_hid_a_block();
    let before = grid(&mut alice);
    let cells = entries(&alice, SheetRoot::Cells);
    let generations = entries(&alice, SheetRoot::LineGen);
    assert_eq!(sweep(&mut alice), 2, "the lines, then their generations");
    assert_eq!(registers(&alice), 0);
    assert_eq!(entries(&alice, SheetRoot::Cells), cells - 8);
    assert_eq!(entries(&alice, SheetRoot::RowHeights), 0, "its height went");
    assert!(entries(&alice, SheetRoot::LineGen) < generations);
    assert_eq!(grid(&mut alice), before);
    bob.import_snapshot(&alice.export_snapshot())
        .expect("the sweep imports");
    converged(&mut alice, &mut bob);
    assert_eq!(registers(&bob), 0);
    assert_eq!(sweep(&mut alice), 0, "a second sweep writes nothing");
}

/// `TRAME-305`: a session that runs keeps its block. Its own sweep leaves it,
/// a peer that knows its user is live leaves it, and its undo still
/// restores it. Once its user leaves, the peer removes it.
#[test]
fn a_block_hidden_by_a_running_session_stays() {
    let (mut alice, mut bob) = bob_hid_a_block();
    assert_eq!(sweep(&mut bob), 0, "its own session runs");
    alice.note_live(BOB);
    assert_eq!(sweep(&mut alice), 0, "its user has a session open");
    assert_eq!(registers(&alice), 1);
    bob.handle_command(TrameCommand::Undo);
    assert_eq!(bob.index.row_len().get(), 10, "the undo restores it");
    bob.handle_command(TrameCommand::Redo);
    alice
        .import_snapshot(&bob.export_snapshot())
        .expect("Bob's redo imports");
    alice.drop_live(BOB);
    assert_eq!(sweep(&mut alice), 2);
    assert_eq!(registers(&alice), 0);
}

/// `TRAME-305`: a newcomer is answered once, and the sample says nothing.
#[test]
fn a_newcomer_is_answered_once() {
    let mut alice = sheet(1);
    assert!(alice.take_greeting().is_none(), "nobody joined");
    alice.note_live(BOB);
    assert!(alice.take_greeting().is_some());
    assert!(alice.take_greeting().is_none(), "once");
    alice.note_live(BOB);
    assert!(
        alice.take_greeting().is_none(),
        "a known user is no newcomer"
    );
}

/// `TRAME-305`, `TRAME-307`: a sweep removes 32 elements at most per pass,
/// and its removal is no step of the history.
#[test]
fn a_sweep_runs_in_slices_and_records_no_step() {
    let mut alice = TrameState::with_peer(PeerId::new(1));
    for row in 0..150 {
        put(&mut alice, row, 0, &row.to_string());
    }
    let mut bob = peer_of(&alice);
    delete_rows(&mut bob, 0, 139);
    alice
        .import_snapshot(&bob.export_snapshot())
        .expect("Bob's writes import");
    let depth = alice.undo_depth();
    let before = grid(&mut alice);
    assert_eq!(before.len(), 10);
    alice.start_sweep();
    assert!(alice.has_pending_work(), "a register is left to sweep");
    assert!(alice.resume().update.is_some());
    assert!(
        alice.has_pending_work(),
        "140 rows take more than one slice"
    );
    assert_eq!(
        sweep(&mut alice),
        5,
        "four slices more, then the generations"
    );
    assert_eq!(alice.undo_depth(), depth, "no step recorded");
    assert_eq!(grid(&mut alice), before);
    assert_eq!(registers(&alice), 0);
}
