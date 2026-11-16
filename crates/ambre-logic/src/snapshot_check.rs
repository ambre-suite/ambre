//! The snapshot check: a snapshot's operations against the signed deltas it
//! claims to fold in (`ambre-spec-op-authorship.md` section 3).
//!
//! A snapshot is built by one member and read by all. Nothing signs its
//! operations one by one, so a reader holding the deltas compares the two
//! before vouching for it (`AUTHOR-016`, `AUTHOR-017`). The check imports the
//! snapshot into a document of its own and never writes the live one
//! (`AUTHOR-018`).
//!
//! Loro keeps the sender's cut of a run of operations on every path tried, but
//! nothing promises it. A cut that differed would read as a forgery. So both
//! sides are cut at the union of their boundaries, Loro slices each piece
//! itself, and the two sides compare piece by piece.

use std::collections::BTreeSet;

use ambre_bus::{SnapshotPurpose as Purpose, SnapshotVerdict as Outcome};
use ambre_core::crdt::CrdtName as _;
use ambre_core::{Relay, Seq};
use ambre_crdt::ledger::SeqLedger;
use loro::{
    Counter, IdSpan, Lamport, LoroDoc, LoroValue, PeerID, ValueOrContainer, VersionVector,
    json::JsonChange,
};

/// Check `snapshot`, which claims to fold in every delta up to `covered`,
/// against the document `own` and what `ledger` recorded. `own` is only read.
pub(crate) fn check_snapshot(
    own: &LoroDoc,
    ledger: &SeqLedger,
    snapshot: &ambre_core::Payload<ambre_core::SnapshotBytes>,
    covered: Seq<Relay>,
    purpose: Purpose,
) -> Outcome {
    if covered < ledger.floor() {
        return Outcome::Unchecked;
    }
    if purpose == Purpose::Confirm && ledger.contiguous_end() < covered {
        return Outcome::Unchecked;
    }
    let theirs = LoroDoc::new();
    if theirs.import(snapshot.as_bytes()).is_err() {
        return Outcome::Mismatch;
    }
    let their_vv = theirs.oplog_vv();
    let held = |vv: &VersionVector, peer: &PeerID| vv.get(peer).copied().unwrap_or(0);

    // A delta the snapshot claims to cover, and does not hold.
    let omits = ledger
        .ends_through(covered)
        .any(|(peer, end)| held(&their_vv, &peer) < end);
    if omits {
        return Outcome::Mismatch;
    }
    // What an earlier snapshot brought cannot be traced to a delta here.
    if ledger
        .base()
        .iter()
        .any(|(peer, end)| held(&their_vv, peer) < *end)
    {
        return Outcome::Unchecked;
    }

    let own_vv = own.oplog_vv();
    let peers: BTreeSet<PeerID> = own_vv.keys().chain(their_vv.keys()).copied().collect();
    let mut extras = Vec::new();
    for peer in peers {
        let ours = held(&own_vv, &peer);
        let theirs_end = held(&their_vv, &peer);
        let common = ours.min(theirs_end);
        if common > 0 {
            match same_operations(own, &theirs, peer, common) {
                Some(true) => {}
                Some(false) => return Outcome::Mismatch,
                None => return Outcome::Unchecked,
            }
        }
        if theirs_end > ours {
            match registry_entry(&theirs, peer) {
                Some(entry) => extras.push((
                    ambre_core::PeerId::new(peer),
                    ambre_core::Payload::<ambre_core::PeerEntryBytes>::new(entry),
                )),
                None => return Outcome::Unchecked,
            }
        }
    }
    Outcome::Match { extras }
}

/// The registry entry `doc` holds for `peer` (`AUTHOR-019`).
fn registry_entry(doc: &LoroDoc, peer: PeerID) -> Option<Vec<u8>> {
    match doc
        .get_map(ambre_core::crdt::FoundationRoot::Peers.as_str())
        .get(ambre_core::PeerKey::of(ambre_core::PeerId::new(peer)).as_str())
    {
        Some(ValueOrContainer::Value(LoroValue::Binary(bytes))) => Some(bytes.to_vec()),
        _ => None,
    }
}

/// One slice of operations, in a form that does not depend on how a peer
/// stored the run it belongs to.
#[derive(Debug, PartialEq)]
struct Piece {
    start: Counter,
    lamport: Lamport,
    deps: Vec<String>,
    container: String,
    content: serde_json::Value,
}

/// Whether `a` and `b` hold the same operations of `peer` below `end`. `None`
/// when Loro answers in a shape the comparison does not expect.
fn same_operations(a: &LoroDoc, b: &LoroDoc, peer: PeerID, end: Counter) -> Option<bool> {
    let span = IdSpan::new(peer, 0, end);
    let changes_a = a.export_json_in_id_span(span);
    let changes_b = b.export_json_in_id_span(span);
    let mut cuts: BTreeSet<Counter> = BTreeSet::new();
    for change in changes_a.iter().chain(&changes_b) {
        cuts.extend(change.ops.iter().map(|op| op.counter));
    }
    cuts.insert(end);
    Some(pieces(a, peer, &changes_a, &cuts)? == pieces(b, peer, &changes_b, &cuts)?)
}

/// The operations of `changes`, cut at every counter of `cuts`. An operation
/// a cut falls inside is exported again piece by piece, so Loro slices it.
fn pieces(
    doc: &LoroDoc,
    peer: PeerID,
    changes: &[JsonChange],
    cuts: &BTreeSet<Counter>,
) -> Option<Vec<Piece>> {
    let mut out = Vec::new();
    let starts: Vec<Counter> = changes
        .iter()
        .flat_map(|change| change.ops.iter().map(|op| op.counter))
        .collect();
    let mut index = 0;
    for change in changes {
        for op in &change.ops {
            index += 1;
            let op_end = match starts.get(index) {
                Some(next) => *next,
                None => *cuts.last()?,
            };
            let inner: Vec<Counter> = cuts.range(op.counter + 1..op_end).copied().collect();
            if inner.is_empty() {
                out.push(piece(change, op.counter, op)?);
                continue;
            }
            let mut from = op.counter;
            for to in inner.into_iter().chain(std::iter::once(op_end)) {
                let sliced = doc.export_json_in_id_span(IdSpan::new(peer, from, to));
                let [single] = sliced.as_slice() else {
                    return None;
                };
                let [sliced_op] = single.ops.as_slice() else {
                    return None;
                };
                out.push(piece(single, sliced_op.counter, sliced_op)?);
                from = to;
            }
        }
    }
    Some(out)
}

/// One operation of `change` as a [`Piece`].
fn piece(change: &JsonChange, start: Counter, op: &loro::json::JsonOp) -> Option<Piece> {
    let offset = u32::try_from(start.checked_sub(change.id.counter)?).ok()?;
    let deps = if start == change.id.counter {
        let mut deps: Vec<String> = change.deps.iter().map(ToString::to_string).collect();
        // Equal strings are identical, so the unstable sort gives the same
        // order, and it shares the copy of the sort Main links for strings.
        deps.sort_unstable();
        deps
    } else {
        vec![loro::ID::new(change.id.peer, start - 1).to_string()]
    };
    Some(Piece {
        start,
        lamport: change.lamport.checked_add(offset)?,
        deps,
        container: op.container.to_string(),
        content: serde_json::to_value(&op.content).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use loro::{ExportMode, LoroDoc};

    /// A document writing under `peer`.
    fn doc(peer: PeerID) -> LoroDoc {
        let doc = LoroDoc::new();
        doc.set_peer_id(peer).expect("peer id");
        doc
    }

    /// The delta `doc` wrote since `from`.
    fn delta(doc: &LoroDoc, from: &VersionVector) -> Vec<u8> {
        doc.export(ExportMode::updates(from)).expect("export")
    }

    /// Alice types and erases one key at a time, each a delta of its own. Bob
    /// receives every delta and records its seq; Alice snapshots her copy.
    fn typed_history() -> (LoroDoc, LoroDoc, SeqLedger) {
        let alice = doc(1);
        let bob = doc(2);
        let mut ledger = SeqLedger::default();
        let text = alice.get_text("t");
        let mut seq = 0;
        let mut send = |edit: &dyn Fn()| {
            let from = alice.oplog_vv();
            edit();
            alice.commit();
            let bytes = delta(&alice, &from);
            bob.import(&bytes).expect("import");
            seq += 1;
            ledger.record(
                ambre_core::Seq::<ambre_core::Relay>::new(seq),
                Some(&bytes.clone().into()),
            );
        };
        for (i, ch) in "hello world".chars().enumerate() {
            send(&|| text.insert(i, &ch.to_string()).expect("insert"));
        }
        // Backspace twice, then delete forward twice.
        send(&|| text.delete(10, 1).expect("delete"));
        send(&|| text.delete(9, 1).expect("delete"));
        send(&|| text.delete(0, 1).expect("delete"));
        send(&|| text.delete(0, 1).expect("delete"));
        (alice, bob, ledger)
    }

    /// Peer 1 writes `first` then `second`: one change when `apart` is false,
    /// two changes when the commits lie past Loro's merge interval.
    fn written(first: &str, second: &str, apart: bool) -> LoroDoc {
        let doc = doc(1);
        let text = doc.get_text("t");
        text.insert(0, first).expect("insert");
        doc.set_next_commit_timestamp(0);
        doc.commit();
        text.insert(first.len(), second).expect("insert");
        doc.set_next_commit_timestamp(if apart { 10_000 } else { 0 });
        doc.commit();
        doc
    }

    #[test]
    fn one_history_cut_two_ways_holds_the_same_operations() {
        let merged = written("abc", "def", false);
        let split = written("abc", "def", true);
        let starts = |doc: &LoroDoc| {
            doc.export_json_in_id_span(IdSpan::new(1, 0, 6))
                .iter()
                .map(|change| change.id.counter)
                .collect::<Vec<_>>()
        };
        // The premise: the same operations, cut at different counters.
        assert_eq!((starts(&merged), starts(&split)), (vec![0], vec![0, 3]));
        assert_eq!(same_operations(&merged, &split, 1, 6), Some(true));
        let forged = written("abc", "deX", true);
        assert_eq!(same_operations(&merged, &forged, 1, 6), Some(false));
    }

    #[test]
    fn a_snapshot_matches_the_deltas_it_covers() {
        let (alice, bob, ledger) = typed_history();
        let covered = ledger.contiguous_end();
        let snapshot = alice.export(ExportMode::Snapshot).expect("snapshot");
        assert_eq!(
            check_snapshot(
                &bob,
                &ledger,
                &ambre_core::Payload::<ambre_core::SnapshotBytes>::new(snapshot.to_vec()),
                covered,
                Purpose::Confirm
            ),
            Outcome::Match { extras: vec![] }
        );
    }

    #[test]
    fn a_forged_operation_is_a_mismatch() {
        let (_, bob, ledger) = typed_history();
        // Mallory rewrites Alice's history under Alice's PeerID.
        let forged = doc(1);
        let text = forged.get_text("t");
        for (i, ch) in "hello wOrld".chars().enumerate() {
            text.insert(i, &ch.to_string()).expect("insert");
            forged.commit();
        }
        for (pos, _) in [(10, ()), (9, ()), (0, ()), (0, ())] {
            text.delete(pos, 1).expect("delete");
            forged.commit();
        }
        let snapshot = forged.export(ExportMode::Snapshot).expect("snapshot");
        let covered = ledger.contiguous_end();
        assert_eq!(
            check_snapshot(
                &bob,
                &ledger,
                &ambre_core::Payload::<ambre_core::SnapshotBytes>::new(snapshot.to_vec()),
                covered,
                Purpose::Confirm
            ),
            Outcome::Mismatch
        );
    }

    #[test]
    fn a_snapshot_omitting_a_covered_delta_is_a_mismatch() {
        let (alice, bob, mut ledger) = typed_history();
        let early = alice.export(ExportMode::Snapshot).expect("snapshot");
        // Bob writes a delta the relay numbers next.
        let from = bob.oplog_vv();
        bob.get_text("t").insert(0, "B").expect("insert");
        bob.commit();
        let seq = ledger.contiguous_end().next();
        ledger.record(seq, Some(&delta(&bob, &from).into()));
        assert_eq!(
            check_snapshot(
                &bob,
                &ledger,
                &ambre_core::Payload::<ambre_core::SnapshotBytes>::new(early.to_vec()),
                seq,
                Purpose::Confirm
            ),
            Outcome::Mismatch
        );
    }

    #[test]
    fn a_gap_in_the_covered_deltas_leaves_the_snapshot_unchecked() {
        let (alice, bob, ledger) = typed_history();
        let snapshot = alice.export(ExportMode::Snapshot).expect("snapshot");
        let past = ledger.contiguous_end().next().next();
        assert_eq!(
            check_snapshot(
                &bob,
                &ledger,
                &ambre_core::Payload::<ambre_core::SnapshotBytes>::new(snapshot.to_vec()),
                past,
                Purpose::Confirm
            ),
            Outcome::Unchecked
        );
    }

    #[test]
    fn operations_only_the_snapshot_holds_are_named_with_their_entry() {
        let (alice, bob, ledger) = typed_history();
        let covered = ledger.contiguous_end();
        // Alice writes on, unsent, and registers her PeerID.
        alice.get_text("t").insert(0, "A").expect("insert");
        alice
            .get_map(ambre_core::crdt::FoundationRoot::Peers.as_str())
            .insert(
                ambre_core::PeerKey::of(ambre_core::PeerId::new(1)).as_str(),
                vec![7_u8; 4],
            )
            .expect("entry");
        alice.commit();
        let snapshot = alice.export(ExportMode::Snapshot).expect("snapshot");
        assert_eq!(
            check_snapshot(
                &bob,
                &ledger,
                &ambre_core::Payload::<ambre_core::SnapshotBytes>::new(snapshot.to_vec()),
                covered,
                Purpose::Confirm
            ),
            Outcome::Match {
                extras: vec![(
                    ambre_core::PeerId::new(1),
                    ambre_core::Payload::<ambre_core::PeerEntryBytes>::new(vec![7_u8; 4])
                )]
            }
        );
    }

    #[test]
    fn the_check_leaves_the_document_unchanged() {
        let (alice, bob, ledger) = typed_history();
        alice.get_text("t").insert(0, "A").expect("insert");
        alice.commit();
        let before = (bob.oplog_vv(), bob.get_deep_value());
        let snapshot = alice.export(ExportMode::Snapshot).expect("snapshot");
        let _ = check_snapshot(
            &bob,
            &ledger,
            &ambre_core::Payload::<ambre_core::SnapshotBytes>::new(snapshot.to_vec()),
            ambre_core::Seq::<ambre_core::Relay>::new(1),
            Purpose::Confirm,
        );
        assert_eq!(before, (bob.oplog_vv(), bob.get_deep_value()));
    }

    #[test]
    fn a_match_folds_the_covered_deltas_into_the_base() {
        let (_, bob, mut ledger) = typed_history();
        let end = ledger.contiguous_end();
        ledger.fold_through(end.before());
        assert_eq!(ledger.floor(), end.before());
        assert_eq!(ledger.ends_through(end).count(), 1);
        assert!(ledger.base().get(&1).is_some());
        let _ = bob;
    }
}
