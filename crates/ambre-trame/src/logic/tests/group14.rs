// ambre-trame/src/logic/tests/group14.rs
// A line a write adds (`ambre-spec-trame-rows-and-columns.md` section 3). The
// tests cover the derived uid, the generation record, the element a reference
// binds to, and the `#REF!` a delete freezes.

use super::*;
use crate::logic::{SheetContainer, SheetRoot};
use crate::model::{CellKey, Element};
use ambre_core::index::Len;
use loro::LoroValue;

/// Write `text` at grid `(row, col)`.
fn put(state: &mut TrameState, row: u32, col: u32, text: &str) {
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(row),
        col: Pos::<Col>::new(col),
        text: text.to_owned(),
    });
}

/// A peer that starts from the history of `from`.
fn peer_of(from: &TrameState) -> TrameState {
    let mut peer = TrameState::new();
    peer.import_snapshot(&from.export_snapshot())
        .expect("a snapshot this build wrote");
    peer
}

/// Two peers sharing a sheet of one written row, `A1`.
fn two_peers() -> (TrameState, TrameState) {
    let mut a = TrameState::new();
    put(&mut a, 0, 0, "1");
    let b = peer_of(&a);
    (a, b)
}

/// Merge the whole history of `from` into `into`.
fn receive(into: &mut TrameState, from: &TrameState) {
    into.import_snapshot(&from.export_snapshot())
        .expect("a snapshot this build wrote");
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

/// How many rows the grid shows.
fn rows(state: &TrameState) -> u32 {
    state.index.row_len().get()
}

/// The rows of column `col` holding `text`.
fn rows_holding(state: &mut TrameState, col: u32, text: &str) -> Vec<u32> {
    let last = rows(state);
    (0..last)
        .filter(|row| read_cell(state, *row, col) == text)
        .collect()
}

/// The stored text of the cell at grid `(row, col)`, as the document keeps it.
fn stored(state: &TrameState, row: u32, col: u32) -> Option<String> {
    let map = state
        .doc
        .get_map(SheetContainer::of(SheetRoot::Cells, &state.active_sheet).as_str());
    let at = CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col));
    match crate::logic::cells::cell_value(&map, &state.index, at)? {
        LoroValue::String(text) => Some(text.to_string()),
        _ => None,
    }
}

/// `TRAME-261`: two first writes in `A2` and `B2` land on one row 2, on both
/// peers, and row 3 stays empty.
#[test]
fn two_first_writes_in_one_new_row_land_on_one_row() {
    let (mut a, mut b) = two_peers();
    put(&mut a, 1, 0, "x");
    put(&mut b, 1, 1, "y");
    exchange(&mut [&mut a, &mut b]);
    for peer in [&mut a, &mut b] {
        assert_eq!(rows(peer), 2, "one new row, not two");
        assert_eq!(read_cell(peer, 1, 0), "x");
        assert_eq!(read_cell(peer, 1, 1), "y");
        assert_eq!(read_cell(peer, 2, 1), "", "no value moved down a row");
    }
}

/// `TRAME-262`: `A2` from one peer and `B3` from the other each land on the
/// line written, three rows in all.
#[test]
fn two_first_writes_on_two_new_rows_land_on_the_rows_written() {
    let (mut a, mut b) = two_peers();
    put(&mut a, 1, 0, "x");
    put(&mut b, 2, 1, "y");
    exchange(&mut [&mut a, &mut b]);
    for peer in [&mut a, &mut b] {
        assert_eq!(rows(peer), 3);
        assert_eq!(read_cell(peer, 1, 0), "x");
        assert_eq!(read_cell(peer, 2, 1), "y");
        assert_eq!(read_cell(peer, 3, 1), "");
    }
}

/// `TRAME-263`: one peer inserts a row above the last line, then writes `A3`.
/// The other writes `B2`. Both values sit on one line, row 3 on both screens.
#[test]
fn an_insert_above_the_last_line_keeps_two_first_writes_on_one_line() {
    let (mut a, mut b) = two_peers();
    a.handle_command(TrameCommand::InsertRow {
        at: Pos::<Row>::new(0),
    });
    put(&mut a, 2, 0, "x");
    put(&mut b, 1, 1, "y");
    exchange(&mut [&mut a, &mut b]);
    for peer in [&mut a, &mut b] {
        assert_eq!(rows(peer), 3);
        assert_eq!(read_cell(peer, 2, 0), "x");
        assert_eq!(read_cell(peer, 2, 1), "y");
    }
}

/// `TRAME-264`: a move of the new line carries the value the other peer wrote
/// into it at the same moment.
#[test]
fn a_move_of_a_new_line_carries_the_value_written_into_it_at_once() {
    let (mut a, mut b) = two_peers();
    put(&mut a, 1, 0, "x");
    a.handle_command(TrameCommand::MoveRow {
        from: Pos::<Row>::new(1),
        to: Pos::<Row>::new(0),
    });
    put(&mut b, 1, 1, "y");
    exchange(&mut [&mut a, &mut b]);
    for peer in [&mut a, &mut b] {
        assert_eq!(rows(peer), 2);
        let x = rows_holding(peer, 0, "x");
        assert_eq!(x.len(), 1, "the line shows once");
        assert_eq!(
            rows_holding(peer, 1, "y"),
            x,
            "both values on the moved line"
        );
    }
}

/// `TRAME-265`: a peer that never received the new line writes into another
/// cell of it while another peer deletes it. The line survives with that value
/// only.
#[test]
fn a_delete_keeps_the_value_a_peer_that_never_had_the_line_wrote_into_it() {
    let (mut a, mut d) = two_peers();
    let mut c = peer_of(&a);
    put(&mut a, 1, 0, "x");
    receive(&mut c, &a);
    c.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(1),
    });
    put(&mut d, 1, 2, "z");
    exchange(&mut [&mut a, &mut c, &mut d]);
    for peer in [&mut a, &mut c, &mut d] {
        assert_eq!(rows(peer), 2, "the line kept for the value written into it");
        assert_eq!(read_cell(peer, 1, 2), "z");
        assert_eq!(
            read_cell(peer, 1, 0),
            "",
            "the other value went with the delete"
        );
    }
}

/// `TRAME-265`: one peer derives a line, writes `A2` and deletes the line.
/// A peer that never received it writes `A2` too, so both derive one uid and
/// write one cell. The delete purges the cell of its own element alone, and
/// the value of the other peer stays.
#[test]
fn a_delete_keeps_the_value_written_into_its_own_cell_by_a_peer_without_the_line() {
    let (mut a, mut b) = two_peers();
    put(&mut a, 1, 0, "7");
    a.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(1),
    });
    put(&mut b, 1, 0, "9");
    exchange(&mut [&mut a, &mut b]);
    for peer in [&mut a, &mut b] {
        assert_eq!(rows(peer), 2, "the line kept for the value written into it");
        assert_eq!(read_cell(peer, 1, 0), "9");
    }
}

/// `TRAME-279`: two peers write `A2` at once, the first write of row 2 on
/// each. Both show the value written under the visible element. The other
/// value stays in the document, and a clear of the cell removes both.
#[test]
fn two_values_in_one_cell_of_a_shared_line_show_the_visible_elements() {
    let (mut a, mut b) = two_peers();
    put(&mut a, 1, 0, "x");
    put(&mut b, 1, 0, "y");
    let mine = a
        .index
        .rows()
        .element(Pos::<Row>::new(1))
        .expect("a's element");
    let theirs = b
        .index
        .rows()
        .element(Pos::<Row>::new(1))
        .expect("b's element");
    let shown = if mine.id < theirs.id { "x" } else { "y" };
    exchange(&mut [&mut a, &mut b]);
    for peer in [&mut a, &mut b] {
        assert_eq!(rows(peer), 2);
        assert_eq!(read_cell(peer, 1, 0), shown, "the visible element's value");
    }
    let cells = |state: &TrameState| match state
        .doc
        .get_map(SheetContainer::of(SheetRoot::Cells, &state.active_sheet).as_str())
        .get_value()
    {
        LoroValue::Map(map) => map.len(),
        _ => 0,
    };
    assert_eq!(cells(&a), 3, "the other value stays in the document");
    put(&mut b, 1, 0, "");
    exchange(&mut [&mut a, &mut b]);
    for peer in [&mut a, &mut b] {
        assert_eq!(read_cell(peer, 1, 0), "", "a clear shows no hidden value");
    }
    assert_eq!(
        cells(&a),
        1,
        "the clear removed the cell under both elements"
    );
}

/// `TRAME-266`: undoing one of two concurrent first writes leaves the other
/// value on that line.
#[test]
fn undoing_one_of_two_first_writes_leaves_the_other_on_the_line() {
    let (mut a, mut b) = two_peers();
    put(&mut a, 1, 0, "x");
    put(&mut b, 1, 1, "y");
    exchange(&mut [&mut a, &mut b]);
    a.handle_command(TrameCommand::Undo);
    exchange(&mut [&mut a, &mut b]);
    for peer in [&mut a, &mut b] {
        assert_eq!(rows(peer), 2);
        assert_eq!(read_cell(peer, 1, 0), "", "the undone value is gone");
        assert_eq!(read_cell(peer, 1, 1), "y", "the other value stays on row 2");
    }
}

/// `TRAME-267`: a write past the used range after a delete derives a new uid,
/// and the reference to the deleted line stays `#REF!`.
#[test]
fn a_write_past_the_used_range_turns_no_ref_back_into_a_value() {
    let mut state = TrameState::new();
    put(&mut state, 0, 0, "1");
    put(&mut state, 1, 0, "7");
    let gone = state.index.row_id(Pos::<Row>::new(1)).expect("row 2");
    put(&mut state, 0, 2, "=A2");
    assert_eq!(read_cell(&mut state, 0, 2), "7");
    state.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(1),
    });
    assert_eq!(read_cell(&mut state, 0, 2), "#REF!");
    put(&mut state, 1, 0, "8");
    let again = state.index.row_id(Pos::<Row>::new(1)).expect("row 2");
    assert_ne!(
        again, gone,
        "the next write after the same line derives a new uid"
    );
    assert_eq!(read_cell(&mut state, 0, 2), "#REF!");
}

/// `TRAME-268`: a move after two peers added one line shows no second copy of
/// it.
#[test]
fn a_move_shows_no_second_copy_of_a_line_two_peers_added() {
    let (mut a, mut b) = two_peers();
    put(&mut a, 1, 0, "x");
    put(&mut b, 1, 1, "y");
    exchange(&mut [&mut a, &mut b]);
    assert_eq!(
        a.index.rows().hidden(),
        Len::new(1),
        "two elements, one line"
    );
    a.handle_command(TrameCommand::MoveRow {
        from: Pos::<Row>::new(1),
        to: Pos::<Row>::new(0),
    });
    exchange(&mut [&mut a, &mut b]);
    for peer in [&mut a, &mut b] {
        assert_eq!(rows(peer), 2);
        assert_eq!(rows_holding(peer, 0, "x"), vec![0]);
        assert_eq!(rows_holding(peer, 1, "y"), vec![0]);
        assert_eq!(rows_holding(peer, 0, "1"), vec![1]);
    }
}

/// `TRAME-269`: deleting a line two peers added leaves no copy of it.
#[test]
fn a_delete_of_a_line_two_peers_added_leaves_no_copy() {
    let (mut a, mut b) = two_peers();
    put(&mut a, 1, 0, "x");
    put(&mut b, 1, 1, "y");
    exchange(&mut [&mut a, &mut b]);
    b.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(1),
    });
    exchange(&mut [&mut a, &mut b]);
    for peer in [&mut a, &mut b] {
        assert_eq!(rows(peer), 1, "no copy of the line is left");
        assert_eq!(
            peer.index.rows().hidden(),
            Len::ZERO,
            "no hidden element either"
        );
        assert_eq!(read_cell(peer, 1, 0), "");
        assert_eq!(read_cell(peer, 1, 1), "");
    }
}

/// `TRAME-270`: two pushes of one uid read as one line, at the position of the
/// element with the smallest element id, on both peers.
#[test]
fn a_uid_pushed_twice_reads_as_one_line_at_its_smallest_element() {
    let (mut a, mut b) = two_peers();
    put(&mut a, 1, 0, "x");
    put(&mut b, 1, 1, "y");
    let mine = a
        .index
        .rows()
        .element(Pos::<Row>::new(1))
        .expect("a's element");
    let theirs = b
        .index
        .rows()
        .element(Pos::<Row>::new(1))
        .expect("b's element");
    assert_eq!(mine.uid, theirs.uid, "both peers derived one uid");
    assert_ne!(mine.id, theirs.id, "each peer pushed its own element");
    exchange(&mut [&mut a, &mut b]);
    let smallest = if mine.id < theirs.id { mine } else { theirs };
    for peer in [&a, &b] {
        assert_eq!(
            peer.index.rows().element(Pos::<Row>::new(1)),
            Some(smallest)
        );
        assert_eq!(peer.index.rows().hidden(), Len::new(1));
    }
}

/// `TRAME-271`: a derived uid never repeats one derived after the same line,
/// however the lines after it are deleted or moved away.
#[cfg(not(target_arch = "wasm32"))]
mod no_uid_comes_back {
    use super::*;
    use crate::hash::HashSet;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Step {
        /// A first write in the row after the last one.
        Write,
        Delete(u32),
        Move(u32, u32),
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            3 => Just(Step::Write),
            2 => any::<u32>().prop_map(Step::Delete),
            1 => (any::<u32>(), any::<u32>()).prop_map(|(f, t)| Step::Move(f, t)),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn a_derived_uid_never_repeats_one_derived_after_the_same_line(
            steps in prop::collection::vec(step(), 1..24),
        ) {
            let mut state = TrameState::new();
            put(&mut state, 0, 0, "1");
            let mut seen: HashSet<Uid> = state.index.row_ids().iter().copied().collect();
            for step in steps {
                let n = rows(&state);
                match step {
                    Step::Write => {
                        put(&mut state, n, 0, "v");
                        let derived = state.index.row_id(Pos::<Row>::new(n)).expect("the new row");
                        prop_assert!(seen.insert(derived), "the uid {derived:?} came back");
                    }
                    Step::Delete(at) if n > 0 => {
                        state.handle_command(TrameCommand::DeleteRow { at: Pos::<Row>::new(at % n) });
                    }
                    Step::Move(from, to) if n > 0 => {
                        state.handle_command(TrameCommand::MoveRow {
                            from: Pos::<Row>::new(from % n),
                            to: Pos::<Row>::new(to % n),
                        });
                    }
                    Step::Delete(_) | Step::Move(..) => {}
                }
            }
        }
    }
}

/// The generation keys the record holds, in the order they sort.
fn record_keys(state: &TrameState) -> Vec<String> {
    let LoroValue::Map(entries) = state
        .doc
        .get_map(SheetContainer::of(SheetRoot::LineGen, &state.active_sheet).as_str())
        .get_value()
    else {
        return Vec::new();
    };
    let mut keys: Vec<String> = entries.keys().cloned().collect();
    keys.sort();
    keys
}

/// `TRAME-276`: each derivation drops the generation below the one it adds.
/// A peer can add that lower generation again at the same moment. The highest
/// one stays, so the next write after the line still derives a fresh uid.
#[test]
fn dropping_a_lower_generation_lets_no_later_write_repeat_a_uid() {
    let (mut a, mut b) = two_peers();
    let line = a
        .index
        .row_id(Pos::<Row>::ZERO)
        .expect("row 1")
        .to_key_part();
    let mut derived = Vec::new();
    for _ in 0..2 {
        put(&mut a, 1, 0, "x");
        derived.extend(a.index.row_id(Pos::<Row>::new(1)));
        a.handle_command(TrameCommand::DeleteRow {
            at: Pos::<Row>::new(1),
        });
    }
    let after_line: Vec<String> = record_keys(&a)
        .into_iter()
        .filter(|key| key.starts_with(&line))
        .collect();
    assert_eq!(
        after_line,
        vec![format!("{line}:00000001")],
        "the lower one dropped"
    );
    // B never saw either derivation: it records generation 0 again.
    put(&mut b, 1, 1, "y");
    derived.extend(b.index.row_id(Pos::<Row>::new(1)));
    exchange(&mut [&mut a, &mut b]);
    a.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(1),
    });
    put(&mut a, 1, 0, "z");
    let next = a.index.row_id(Pos::<Row>::new(1)).expect("row 2");
    assert!(
        !derived.contains(&next),
        "{next:?} repeats a uid derived after the line"
    );
    assert!(
        record_keys(&a).contains(&format!("{line}:00000002")),
        "the next write took the generation above the highest"
    );
}

/// `TRAME-277`: deleting a line freezes each reference to it into a literal
/// `#REF!`, in the delete's own undo step.
#[test]
fn a_delete_freezes_each_reference_to_its_line_in_one_undo_step() {
    let mut state = TrameState::new();
    put(&mut state, 0, 0, "1");
    put(&mut state, 1, 0, "7");
    put(&mut state, 0, 2, "=A2+SUM(A2)");
    let before = stored(&state, 0, 2).expect("the formula");
    assert!(before.contains('{'), "the formula names the line: {before}");
    state.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(1),
    });
    let frozen = stored(&state, 0, 2).expect("the formula");
    assert_eq!(
        frozen, "=#REF!+SUM(#REF!)",
        "no element of the deleted line is left"
    );
    state.handle_command(TrameCommand::Undo);
    assert_eq!(
        stored(&state, 0, 2),
        Some(before),
        "the undo restores each reference"
    );
    assert_eq!(read_cell(&mut state, 0, 2), "14");
}

/// `TRAME-274`: a `#REF!` a delete produced stays `#REF!` when a peer that
/// never received the line derives its uid again.
#[test]
fn a_ref_a_delete_produced_reads_no_value_when_its_uid_comes_back() {
    let (mut a, mut b) = two_peers();
    put(&mut a, 1, 0, "7");
    let gone = a.index.row_id(Pos::<Row>::new(1)).expect("row 2");
    put(&mut a, 0, 2, "=A2");
    a.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(1),
    });
    put(&mut b, 1, 3, "9");
    assert_eq!(
        b.index.row_id(Pos::<Row>::new(1)),
        Some(gone),
        "the peer that never had the line derives its uid again"
    );
    exchange(&mut [&mut a, &mut b]);
    for peer in [&mut a, &mut b] {
        assert_eq!(read_cell(peer, 1, 3), "9", "the uid lives again");
        assert_eq!(read_cell(peer, 0, 2), "#REF!");
    }
}

/// `TRAME-278`: four peers. One derives a line, and a second binds a formula to
/// it. The first deletes it, and a fourth that never had it derives the uid
/// again. The formula reads `#REF!`: its element is gone.
#[test]
fn a_formula_citing_a_line_deleted_at_once_reads_ref() {
    let (mut p1, mut p4) = two_peers();
    let mut p2 = peer_of(&p1);
    put(&mut p1, 1, 0, "7");
    receive(&mut p2, &p1);
    put(&mut p2, 0, 2, "=A2");
    assert_eq!(read_cell(&mut p2, 0, 2), "7");
    p1.handle_command(TrameCommand::DeleteRow {
        at: Pos::<Row>::new(1),
    });
    put(&mut p4, 1, 3, "9");
    let mut p3 = peer_of(&p4);
    exchange(&mut [&mut p1, &mut p2, &mut p3, &mut p4]);
    for peer in [&mut p1, &mut p2, &mut p3, &mut p4] {
        assert_eq!(
            read_cell(peer, 1, 3),
            "9",
            "the fourth peer's line holds its value"
        );
        assert_eq!(read_cell(peer, 0, 2), "#REF!", "the bound element is gone");
    }
}

/// `TRAME-272`: a document of schema version 1 reads under version 2. Every
/// line keeps its position, and every cell its line. Each reference binds to
/// the bare minted element.
#[test]
fn a_version_1_document_reads_with_every_line_and_cell_in_place() {
    let doc = loro::LoroDoc::new();
    let sheet = default_sheet();
    let _ = doc
        .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), 1_i64);
    let uid = |n: u32| Uid {
        peer: ambre_core::PeerId::new(5),
        counter: ambre_core::IdCounter::new(n),
    };
    let rows_list = doc.get_movable_list(SheetContainer::of(SheetRoot::Rows, &sheet).as_str());
    for n in [2, 0, 1] {
        rows_list
            .push(uid(n).to_key_part().as_str())
            .expect("a row");
    }
    doc.get_movable_list(SheetContainer::of(SheetRoot::Cols, &sheet).as_str())
        .push(uid(9).to_key_part().as_str())
        .expect("a column");
    let cells = doc.get_map(SheetContainer::of(SheetRoot::Cells, &sheet).as_str());
    for (row, text) in [(2, "10"), (0, "20"), (1, "={5_2|5_9}+{5_0|5_9}")] {
        cells
            .insert(
                CellKey::of(Element::minted(uid(row)), Element::minted(uid(9))).as_str(),
                text,
            )
            .expect("a cell");
    }
    doc.commit();
    let mut state = TrameState::new();
    state
        .import_snapshot(&ambre_core::Payload::new(
            doc.export(loro::ExportMode::Snapshot).expect("a snapshot"),
        ))
        .expect("a version-1 workbook migrates");
    assert_eq!(state.schema_version(), crate::logic::SCHEMA_VERSION);
    assert_eq!(read_cell(&mut state, 0, 0), "10");
    assert_eq!(read_cell(&mut state, 1, 0), "20");
    assert_eq!(
        read_cell(&mut state, 2, 0),
        "30",
        "each reference binds to its minted line"
    );
    assert_eq!(read_raw(&mut state, 2, 0), "=A1+A2");
    assert_eq!(
        state.index.rows().element(Pos::<Row>::ZERO),
        Some(Element::minted(uid(2))),
        "a version-1 element is a bare minted uid"
    );
}

/// Races of a third peer above the last line, and a write after it.
#[cfg(not(target_arch = "wasm32"))]
mod races {
    use super::*;
    use proptest::prelude::*;

    /// One edit of a third peer.
    #[derive(Debug, Clone)]
    enum Edit {
        Insert(u32),
        Delete(u32),
        Move(u32, u32),
        Write(u32),
        DeleteLast,
        MoveLastUp(u32),
        Sort,
        Append,
    }

    /// Apply `edit` to `state`, read against the rows it shows now.
    fn apply(state: &mut TrameState, edit: &Edit) {
        let n = rows(state);
        let below_last = |k: u32| (n > 1).then(|| k % (n - 1));
        let command = match *edit {
            Edit::Insert(at) => below_last(at).map(|at| TrameCommand::InsertRow {
                at: Pos::<Row>::new(at),
            }),
            Edit::Delete(at) => below_last(at).map(|at| TrameCommand::DeleteRow {
                at: Pos::<Row>::new(at),
            }),
            Edit::Move(from, to) => {
                below_last(from)
                    .zip(below_last(to))
                    .map(|(from, to)| TrameCommand::MoveRow {
                        from: Pos::<Row>::new(from),
                        to: Pos::<Row>::new(to),
                    })
            }
            Edit::Write(row) => below_last(row).map(|row| TrameCommand::SetCellText {
                row: Pos::<Row>::new(row),
                col: Pos::<Col>::new(2),
                text: "c".to_owned(),
            }),
            Edit::DeleteLast => (n > 1).then(|| TrameCommand::DeleteRow {
                at: Pos::<Row>::new(n - 1),
            }),
            Edit::MoveLastUp(to) => below_last(to).map(|to| TrameCommand::MoveRow {
                from: Pos::<Row>::new(n - 1),
                to: Pos::<Row>::new(to),
            }),
            Edit::Sort => Some(TrameCommand::SortRange {
                sheet_id: default_sheet(),
                rect: crate::CellRect::spanning(
                    CellPos::new(Pos::<Row>::ZERO, Pos::<Col>::ZERO),
                    CellPos::new(Pos::<Row>::new(n.saturating_sub(1)), Pos::<Col>::ZERO),
                ),
                sort_col: Len::<Col>::ZERO,
                order: SortOrder::Descending,
                header: HeaderRow::Absent,
            }),
            Edit::Append => Some(TrameCommand::SetCellText {
                row: Pos::<Row>::new(n),
                col: Pos::<Col>::new(3),
                text: "c".to_owned(),
            }),
        };
        if let Some(command) = command {
            state.handle_command(command);
        }
    }

    /// Edits that leave the last line and its record alone.
    fn above_last() -> impl Strategy<Value = Edit> {
        prop_oneof![
            any::<u32>().prop_map(Edit::Insert),
            any::<u32>().prop_map(Edit::Delete),
            (any::<u32>(), any::<u32>()).prop_map(|(f, t)| Edit::Move(f, t)),
            any::<u32>().prop_map(Edit::Write),
        ]
    }

    /// Any edit, the last line included.
    fn anywhere() -> impl Strategy<Value = Edit> {
        prop_oneof![
            3 => above_last(),
            1 => Just(Edit::DeleteLast),
            1 => any::<u32>().prop_map(Edit::MoveLastUp),
            1 => Just(Edit::Sort),
            1 => Just(Edit::Append),
        ]
    }

    /// Three peers on rows `1`, `2`, `3`. A third peer runs `edits`, the first
    /// writer receives the first `seen` of them, the second none. Each writer
    /// then writes the row after its own last line. Returns the converged
    /// peers and the uid each writer derived.
    fn race(edits: &[Edit], seen: usize) -> ([TrameState; 3], Uid, Uid) {
        let mut a = TrameState::new();
        for (row, text) in ["3", "1", "2"].iter().enumerate() {
            put(&mut a, row as u32, 0, text);
        }
        let mut b = peer_of(&a);
        let mut c = peer_of(&a);
        for (k, edit) in edits.iter().enumerate() {
            apply(&mut c, edit);
            if k + 1 == seen {
                receive(&mut a, &c);
            }
        }
        let (na, nb) = (rows(&a), rows(&b));
        put(&mut a, na, 0, "x");
        put(&mut b, nb, 1, "y");
        let ua = a.index.row_id(Pos::<Row>::new(na)).expect("a's new row");
        let ub = b.index.row_id(Pos::<Row>::new(nb)).expect("b's new row");
        exchange(&mut [&mut a, &mut b, &mut c]);
        ([a, b, c], ua, ub)
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        /// `TRAME-263`: a third peer inserts, deletes, moves or writes above
        /// the last line, and one writer receives any part of it. The two
        /// first writes still land on one line.
        #[test]
        fn edits_above_the_last_line_keep_two_first_writes_on_one_line(
            edits in prop::collection::vec(above_last(), 0..8),
            seen in 0usize..9,
        ) {
            let (mut peers, ua, ub) = race(&edits, seen);
            prop_assert_eq!(ua, ub, "both writers derived one uid");
            for peer in peers.iter_mut() {
                let x = rows_holding(peer, 0, "x");
                prop_assert_eq!(x.len(), 1);
                prop_assert_eq!(rows_holding(peer, 1, "y"), x);
            }
        }

        /// `TRAME-273`: when one writer received a change of the last line and
        /// the other did not, the two writes may derive two uids. Each value
        /// then keeps a line of its own; one uid keeps them on one line.
        #[test]
        fn two_first_writes_that_derive_two_uids_each_keep_a_line(
            edits in prop::collection::vec(anywhere(), 0..6),
            seen in 0usize..7,
        ) {
            let (mut peers, ua, ub) = race(&edits, seen);
            for peer in peers.iter_mut() {
                let x = rows_holding(peer, 0, "x");
                let y = rows_holding(peer, 1, "y");
                prop_assert_eq!(x.len(), 1, "the first value is kept, once");
                prop_assert_eq!(y.len(), 1, "the second value is kept, once");
                if ua == ub {
                    prop_assert_eq!(x, y);
                } else {
                    prop_assert_ne!(x, y, "two uids, two lines");
                }
            }
        }
    }
}

/// Four peers that push, delete, move, bind formulas and sync in any order
/// (`TRAME-270`, `TRAME-278`).
#[cfg(not(target_arch = "wasm32"))]
mod four_peers {
    use super::*;
    use crate::formula::parser::parse_formula;
    use crate::formula::types::{Expr, StoredAxis};
    use proptest::prelude::*;

    /// The row element a single stored reference `=A{n}` binds to, if any.
    fn bound_row(text: &str) -> Option<Element> {
        match parse_formula(text.strip_prefix('=')?).ok()? {
            Expr::StoredCell(cell) => match cell.row {
                StoredAxis::Line(element) => Some(element),
                _ => None,
            },
            _ => None,
        }
    }

    #[derive(Debug, Clone)]
    enum Act {
        /// A first write in the row after the last one, column A.
        Push(u32, u8),
        Delete(u32, u32),
        Move(u32, u32, u32),
        /// `=A{target}` written in column B of a row.
        Bind(u32, u32, u32),
        /// One peer receives another's whole history.
        Sync(u32, u32),
        /// A value, or a clear, written into column A of a row: the cells a
        /// push writes, so writes and deletes meet in one cell.
        Write(u32, u32, u8),
    }

    fn act() -> impl Strategy<Value = Act> {
        prop_oneof![
            3 => (0..4u32, 1..9u8).prop_map(|(p, v)| Act::Push(p, v)),
            1 => (0..4u32, any::<u32>()).prop_map(|(p, at)| Act::Delete(p, at)),
            1 => (0..4u32, any::<u32>(), any::<u32>()).prop_map(|(p, f, t)| Act::Move(p, f, t)),
            2 => (0..4u32, any::<u32>(), any::<u32>()).prop_map(|(p, r, t)| Act::Bind(p, r, t)),
            2 => (0..4u32, 0..4u32).prop_map(|(p, q)| Act::Sync(p, q)),
            2 => (0..4u32, any::<u32>(), 0..9u8).prop_map(|(p, r, v)| Act::Write(p, r, v)),
        ]
    }

    fn run(peers: &mut [TrameState], act: &Act) {
        match *act {
            Act::Push(p, value) => {
                let peer = &mut peers[p as usize];
                let n = rows(peer);
                put(peer, n, 0, &value.to_string());
            }
            Act::Delete(p, at) => {
                let peer = &mut peers[p as usize];
                let n = rows(peer);
                if n > 0 {
                    peer.handle_command(TrameCommand::DeleteRow {
                        at: Pos::<Row>::new(at % n),
                    });
                }
            }
            Act::Move(p, from, to) => {
                let peer = &mut peers[p as usize];
                let n = rows(peer);
                if n > 0 {
                    peer.handle_command(TrameCommand::MoveRow {
                        from: Pos::<Row>::new(from % n),
                        to: Pos::<Row>::new(to % n),
                    });
                }
            }
            Act::Bind(p, row, target) => {
                let peer = &mut peers[p as usize];
                let n = rows(peer);
                if n > 0 {
                    put(peer, row % n, 1, &format!("=A{}", target % n + 1));
                }
            }
            Act::Sync(p, q) if p != q => {
                let from = peers[q as usize].export_snapshot();
                peers[p as usize]
                    .import_snapshot(&from)
                    .expect("a snapshot this build wrote");
            }
            Act::Sync(..) => {}
            Act::Write(p, row, value) => {
                let peer = &mut peers[p as usize];
                let n = rows(peer);
                if n > 0 {
                    // Zero clears the cell.
                    let text = if value == 0 {
                        String::new()
                    } else {
                        value.to_string()
                    };
                    put(peer, row % n, 0, &text);
                }
            }
        }
    }

    /// Columns A and B of every row, as a peer renders them.
    fn grid(peer: &mut TrameState) -> Vec<(String, String)> {
        (0..rows(peer))
            .map(|row| (read_cell(peer, row, 0), read_cell(peer, row, 1)))
            .collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

        /// After full exchange, all peers show one grid. Every reference to a
        /// live element, visible or hidden, reads the value of its line, and
        /// every other reference reads `#REF!`. The oracle reads the stored
        /// element through the axis view, apart from the formula engine.
        #[test]
        fn every_reference_reads_its_live_element_or_ref(
            acts in prop::collection::vec(act(), 0..24),
        ) {
            let mut base = TrameState::new();
            put(&mut base, 0, 0, "1");
            let mut peers: Vec<TrameState> = (0..4).map(|_| peer_of(&base)).collect();
            for act in &acts {
                run(&mut peers, act);
            }
            {
                let mut all: Vec<&mut TrameState> = peers.iter_mut().collect();
                exchange(&mut all);
            }
            let first = grid(&mut peers[0]);
            for peer in peers.iter_mut().skip(1) {
                prop_assert_eq!(&grid(peer), &first, "the peers diverged");
            }
            let peer = &mut peers[0];
            for row in 0..rows(peer) {
                let Some(text) = stored(peer, row, 1) else { continue };
                let shown = read_cell(peer, row, 1);
                if text.contains("#REF!") {
                    prop_assert_eq!(shown, "#REF!", "a frozen reference");
                    continue;
                }
                let Some(element) = bound_row(&text) else { continue };
                match peer.index.rows().pos_of_element(&element) {
                    Some(line) => {
                        let value = read_cell(peer, line.get(), 0);
                        prop_assert_eq!(shown, value, "a live element reads its line");
                    }
                    None => prop_assert_eq!(shown, "#REF!", "a gone element reads #REF!"),
                }
            }
        }
    }
}
