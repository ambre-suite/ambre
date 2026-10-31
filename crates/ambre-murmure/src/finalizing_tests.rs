//! The finalization of a closing segment against its contract rows. They hold
//! when the lease is asked, the shallow snapshot at the horizon, and the
//! final mark every member settles.

use std::collections::HashMap;

use ambre_bus::{MessageDraft, MurmureEvent, SnapshotVerdict, Timestamp};
use ambre_core::id::{Channel, Id, Lease, Segment, User};
use ambre_core::{Payload, PeerEntryBytes, Seq, SnapshotBytes};
use loro::{Container, ExportMode, LoroDoc, ValueOrContainer};

use crate::schema::{MessageKey, Root};
use crate::session::next_peer;
use crate::{ChannelState, ChannelType, FINALIZE_RETRY, Order, Signer};

const NOW: Timestamp = Timestamp::new(1_700_000_000_000);
const LEASE_MS: u64 = 30_000;

fn at(ms: u64) -> Timestamp {
    Timestamp::new(NOW.get() + ms)
}

fn draft(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    }
}

/// A channel of one-message segments, holding `texts` in a closing segment.
/// Returns it with that segment.
fn closed_with(texts: &[&str]) -> (ChannelState, Id<Segment>) {
    let mut state = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create")
        .with_segment_size(crate::MessageCount::of_len(texts.len()))
        .identify();
    let author = Id::<User>::mint();
    for text in texts {
        state.send(&draft(text), author, NOW).expect("send");
    }
    let closing = state.wants_close().expect("full");
    let successor = Id::<Segment>::mint();
    state
        .close_into(closing, successor, crate::Successor::Created, NOW)
        .expect("close");
    let peer = next_peer();
    state
        .adopt_session(
            successor.document_id(),
            peer,
            PeerEntryBytes::new(peer.get().to_le_bytes().to_vec()),
        )
        .expect("adopt");
    (state, closing)
}

/// A second member's copy of `state`, opened from its live documents.
fn peer_of(state: &ChannelState) -> ChannelState {
    let segments: HashMap<Id<Segment>, Payload<SnapshotBytes>> = state
        .held_segments()
        .into_iter()
        .map(|segment| {
            let bytes = state.snapshot(segment.document_id()).expect("segment");
            (segment, bytes)
        })
        .collect();
    let meta = state.snapshot(state.meta_id().document_id()).expect("meta");
    ChannelState::open(state.channel_id(), state.meta_id(), &meta, &segments)
        .expect("peer")
        .identify()
}

/// Delete the whole body of the message at `index` of `segment`, as an edit
/// before the finalization would.
fn erase_body(state: &ChannelState, segment: Id<Segment>, index: usize) {
    let doc = state.doc(segment.document_id()).expect("held");
    let Some(message) = crate::schema::root_list(doc, Root::Messages)
        .get(index)
        .and_then(crate::schema::fields_of)
    else {
        panic!("no message at {index}");
    };
    let Some(ValueOrContainer::Container(Container::Text(body))) = message.get(MessageKey::Content)
    else {
        panic!("no body");
    };
    body.delete(0, body.len_unicode()).expect("delete");
    doc.commit();
}

fn holds(bytes: &[u8], word: &str) -> bool {
    bytes.windows(word.len()).any(|w| w == word.as_bytes())
}

// --- MURMURE-067, MURMURE-062 ---

/// A closing segment asks for its lease at the first tick, then as late as
/// the relay says. The relay holds the grace period, so a message younger
/// than it never reaches a final segment.
#[test]
fn a_closing_segment_asks_for_its_lease_as_the_relay_says() {
    let mut state = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    assert_eq!(state.lease_to_ask(NOW), None, "no segment is closing");
    let (mut state, closing) = closed_with(&["one"]);
    assert_eq!(
        state.lease_to_ask(NOW),
        Some(closing),
        "the first tick asks"
    );
    assert_eq!(state.lease_to_ask(NOW), None, "asked once");

    // The grace period runs: the relay says when to ask again.
    state.lease_deferred(closing, ambre_core::Millis::new(5_000));
    assert_eq!(state.lease_to_ask(NOW), None, "the tick dates it");
    assert_eq!(state.lease_to_ask(at(4_999)), None);
    assert_eq!(state.lease_to_ask(at(5_000)), Some(closing));

    // A failed request waits a delay that doubles.
    state.finalize_failed(closing);
    assert_eq!(state.lease_to_ask(at(5_000)), None, "the tick dates it");
    assert_eq!(
        state.lease_to_ask(at(5_000 + FINALIZE_RETRY.get())),
        Some(closing)
    );
    state.finalize_failed(closing);
    let base = 5_000 + FINALIZE_RETRY.get();
    assert_eq!(state.lease_to_ask(at(base)), None);
    assert_eq!(state.lease_to_ask(at(base + FINALIZE_RETRY.get())), None);
    assert_eq!(
        state.lease_to_ask(at(base + 2 * FINALIZE_RETRY.get())),
        Some(closing)
    );
}

/// A copy holding an edit the relay refused never asks for the lease again.
#[test]
fn a_withdrawn_copy_never_asks_for_the_lease() {
    let (mut state, closing) = closed_with(&["one"]);
    assert_eq!(state.lease_to_ask(NOW), Some(closing));
    state.finalize_withdrawn(closing);
    state.finalize_failed(closing);
    assert_eq!(state.lease_to_ask(at(3_600_000)), None);
}

// --- MURMURE-068, MURMURE-063 ---

/// The lease holder seals the snapshot once it holds every delta up to the
/// horizon. It is shallow: the text deleted before it is gone.
#[test]
fn the_lease_holder_seals_a_shallow_snapshot_at_the_horizon() {
    let (mut state, closing) = closed_with(&["kept zanzibar-quokka", "erased marzipan-wombat"]);
    erase_body(&state, closing, 1);
    let full = state.export_all(closing.document_id()).expect("full");
    assert!(
        holds(full.as_bytes(), "marzipan-wombat"),
        "the history keeps it"
    );

    let lease = Id::<Lease>::mint();
    assert_eq!(state.lease_to_ask(NOW), Some(closing));
    state.lease_granted(
        closing,
        lease,
        ambre_core::Seq::new(7),
        ambre_core::Millis::new(LEASE_MS),
    );
    let early = state
        .final_snapshot(NOW, ambre_core::Seq::new(6))
        .expect("export");
    assert!(early.is_none(), "a delta below the horizon is missing");
    let snapshot = state
        .final_snapshot(at(1), ambre_core::Seq::new(7))
        .expect("export");
    let snapshot = snapshot.expect("every delta is in");
    assert_eq!(snapshot.segment, closing);
    assert_eq!(snapshot.lease, lease);
    assert_eq!(snapshot.horizon, ambre_core::Seq::new(7));
    assert!(holds(snapshot.payload.as_bytes(), "zanzibar-quokka"));
    assert!(
        !holds(snapshot.payload.as_bytes(), "marzipan-wombat"),
        "shallow"
    );
    assert!(
        state
            .final_snapshot(at(2), ambre_core::Seq::new(7))
            .expect("export")
            .is_none(),
        "sealed once"
    );

    let doc = LoroDoc::new();
    doc.import(snapshot.payload.as_bytes())
        .expect("a fresh copy imports it");
}

/// A lease that runs out before the snapshot leaves seals nothing, and is
/// asked again.
#[test]
fn a_lease_that_runs_out_is_asked_again() {
    let (mut state, closing) = closed_with(&["one"]);
    assert_eq!(state.lease_to_ask(NOW), Some(closing));
    state.lease_granted(
        closing,
        Id::<Lease>::mint(),
        ambre_core::Seq::new(9),
        ambre_core::Millis::new(LEASE_MS),
    );
    assert!(
        state
            .final_snapshot(NOW, ambre_core::Seq::new(1))
            .expect("export")
            .is_none()
    );
    assert_eq!(state.lease_to_ask(at(LEASE_MS - 1)), None, "still held");
    let late = state
        .final_snapshot(at(LEASE_MS), ambre_core::Seq::new(9))
        .expect("export");
    assert!(late.is_none(), "the lease ran out");
    assert_eq!(state.lease_to_ask(at(LEASE_MS)), Some(closing));
}

// --- MURMURE-069, MURMURE-070 ---

/// The accepted commit marks the segment final in `meta`. Every member then
/// holds it as a final segment, whose messages read as before, and the
/// closing slot is free.
#[test]
fn an_accepted_commit_marks_the_segment_final_for_every_member() {
    let (mut alice, closing) = closed_with(&["one", "two"]);
    let mut bob = peer_of(&alice);
    let before = alice.messages(closing).expect("messages");
    assert_eq!(alice.lease_to_ask(NOW), Some(closing));
    alice.lease_granted(
        closing,
        Id::<Lease>::mint(),
        ambre_core::Seq::new(2),
        ambre_core::Millis::new(LEASE_MS),
    );
    let snapshot = alice
        .final_snapshot(NOW, ambre_core::Seq::new(2))
        .expect("export")
        .expect("in");

    let meta = alice.mark_final(closing).expect("mark").expect("a delta");
    assert_eq!(alice.closing_id(), None, "the slot is free");
    assert_eq!(alice.final_segments(), vec![closing]);
    assert_eq!(alice.messages(closing).expect("held"), before);
    assert_eq!(alice.lease_to_ask(at(1)), None, "nothing left to finalize");
    // A final segment takes no operation past its finalization (`MURMURE-012`).
    assert!(matches!(
        alice.react(
            closing,
            before[0].id,
            &ambre_bus::Emoji::from("\u{1F44D}"),
            Id::<User>::mint(),
            at(1)
        ),
        Err(crate::MurmureError::Final(_))
    ));

    assert_eq!(bob.settle_final(), None, "meta still names it closing");
    bob.apply_remote(bob.meta_id().document_id(), (&meta).into())
        .expect("meta");
    assert_eq!(bob.settle_final(), Some(closing));
    assert_eq!(bob.closing_id(), None);
    assert_eq!(bob.messages(closing).expect("held"), before);

    // A member opening the channel later loads the final snapshot alone.
    let mut carol = peer_of(&alice);
    carol.cancel_older(closing.document_id());
    let loaded = carol
        .load_older(closing, &snapshot.payload)
        .expect("the final snapshot loads");
    let MurmureEvent::Messages { messages, .. } = loaded else {
        panic!("messages");
    };
    assert_eq!(messages, before, "finalizing changes no message");
}

// --- MURMURE-238 ---

/// The relay dropped the final snapshot Alice sealed, and Bob holds the
/// segment as a final one. He asks for its lease and seals it again at the
/// horizon. He sends no `meta` delta: the segment is final there already.
#[test]
fn a_dropped_final_snapshot_is_sealed_again_by_another_member() {
    let (mut alice, closing) = closed_with(&["one", "two"]);
    let mut bob = peer_of(&alice);
    assert_eq!(alice.lease_to_ask(NOW), Some(closing));
    alice.lease_granted(
        closing,
        Id::<Lease>::mint(),
        ambre_core::Seq::new(2),
        ambre_core::Millis::new(LEASE_MS),
    );
    alice
        .final_snapshot(NOW, ambre_core::Seq::new(2))
        .expect("export")
        .expect("in");
    let meta = alice.mark_final(closing).expect("mark").expect("a delta");
    bob.apply_remote(bob.meta_id().document_id(), (&meta).into())
        .expect("meta");
    assert_eq!(bob.settle_final(), Some(closing));

    assert!(
        !bob.refinalize(Id::<Segment>::mint()),
        "a segment he does not hold"
    );
    assert!(bob.refinalize(closing));
    assert_eq!(bob.finalizing_id(), Some(closing));
    assert_eq!(bob.lease_to_ask(NOW), Some(closing));
    let lease = Id::<Lease>::mint();
    bob.lease_granted(
        closing,
        lease,
        ambre_core::Seq::new(2),
        ambre_core::Millis::new(LEASE_MS),
    );
    let snapshot = bob
        .final_snapshot(NOW, ambre_core::Seq::new(2))
        .expect("export")
        .expect("in");
    assert_eq!((snapshot.segment, snapshot.lease), (closing, lease));
    assert!(holds(snapshot.payload.as_bytes(), "two"));
    assert_eq!(
        bob.mark_final(closing).expect("mark"),
        None,
        "meta holds it"
    );
    assert_eq!(bob.finalizing_id(), None);
    assert_eq!(bob.final_segments(), vec![closing]);

    // Another member sealed it first: the relay refuses his lease, and he
    // stops.
    assert!(bob.refinalize(closing));
    assert_eq!(bob.lease_to_ask(at(1)), Some(closing));
    bob.finalize_withdrawn(closing);
    assert_eq!(bob.finalizing_id(), None);
    assert_eq!(bob.lease_to_ask(at(3_600_000)), None);
}

/// Alice's channel of two-message segments, and Bob's copy of it holding
/// her first `received` deltas, numbered as the relay delivered them. Alice
/// then closes the segment, and seals its final snapshot at the horizon 2.
fn finalized_for(
    received: u64,
) -> (
    ChannelState,
    ChannelState,
    Id<Segment>,
    Payload<SnapshotBytes>,
) {
    let mut alice = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create")
        .with_segment_size(crate::MessageCount::new(2))
        .identify();
    let mut bob = peer_of(&alice);
    let segment = alice.active_id();
    let author = Id::<User>::mint();
    let signer = Signer {
        peer: alice.peer(segment.document_id()).expect("a session"),
        user: Some(author),
    };
    for (seq, text) in [(1, "one"), (2, "two")] {
        let sent = alice.send(&draft(text), author, NOW).expect("send");
        if seq <= received {
            bob.apply_signed(segment.document_id(), Seq::new(seq), &sent.delta, signer)
                .expect("delivered");
        }
    }
    let closing = alice.wants_close().expect("full");
    alice
        .close_into(
            closing,
            Id::<Segment>::mint(),
            crate::Successor::Created,
            NOW,
        )
        .expect("close");
    assert_eq!(alice.lease_to_ask(NOW), Some(closing));
    alice.lease_granted(
        closing,
        Id::<Lease>::mint(),
        ambre_core::Seq::new(2),
        ambre_core::Millis::new(LEASE_MS),
    );
    let snapshot = alice
        .final_snapshot(NOW, ambre_core::Seq::new(2))
        .expect("export")
        .expect("in");
    (alice, bob, closing, snapshot.payload)
}

/// A shallow snapshot of `state`'s copy of `segment` after `forge` wrote on
/// a fork of it.
fn forged(
    state: &ChannelState,
    segment: Id<Segment>,
    forge: impl FnOnce(&LoroDoc),
) -> Payload<SnapshotBytes> {
    let fork = state.doc(segment.document_id()).expect("held").fork();
    forge(&fork);
    fork.commit();
    Payload::<SnapshotBytes>::new(
        fork.export(ExportMode::shallow_snapshot(&fork.oplog_frontiers()))
            .expect("export"),
    )
}

// --- MURMURE-071, MURMURE-153 ---

/// Bob holds every delta the final snapshot covers: he rebuilds its state
/// and finds the same. The check writes neither his copy nor the snapshot.
#[test]
fn a_member_holding_the_deltas_confirms_the_final_snapshot() {
    let (_alice, bob, closing, payload) = finalized_for(2);
    let document_id = closing.document_id();
    let before = bob.doc(document_id).expect("held").oplog_vv();
    let messages = bob.messages(closing).expect("held");
    let sent = payload.clone();
    assert_eq!(
        bob.check_snapshot(document_id, ambre_core::Seq::new(2), &payload)
            .expect("check"),
        SnapshotVerdict::Match { extras: Vec::new() }
    );
    assert_eq!(bob.doc(document_id).expect("held").oplog_vv(), before);
    assert_eq!(bob.messages(closing).expect("held"), messages);
    assert_eq!(payload, sent, "the snapshot is only read");
    assert_eq!(
        bob.check_snapshot(document_id, ambre_core::Seq::new(1), &payload)
            .expect("check"),
        SnapshotVerdict::Unchecked,
        "his copy holds a delta past what it covers"
    );
}

// --- MURMURE-072, MURMURE-012 ---

/// A final snapshot holding an operation no delta carried is refused, and so
/// is one that drops a delta it covers.
#[test]
fn a_final_snapshot_at_odds_with_the_deltas_is_refused() {
    let (alice, bob, closing, _payload) = finalized_for(2);
    let document_id = closing.document_id();
    let added = forged(&alice, closing, |doc| {
        doc.set_peer_id(next_peer().get()).expect("peer");
        crate::schema::root_list(doc, Root::Messages)
            .push("planted")
            .expect("push");
    });
    assert_eq!(
        bob.check_snapshot(document_id, ambre_core::Seq::new(2), &added)
            .expect("check"),
        SnapshotVerdict::Mismatch
    );
    let doc = alice.doc(document_id).expect("held");
    let mut short = doc.oplog_vv();
    let (peer, end) = short
        .iter()
        .map(|(peer, end)| (*peer, *end))
        .max_by_key(|(_, end)| *end)
        .expect("operations");
    short.insert(peer, end - 1);
    let early = doc.fork_at(&doc.vv_to_frontiers(&short)).expect("fork");
    let dropped = Payload::<SnapshotBytes>::new(
        early
            .export(ExportMode::shallow_snapshot(&early.oplog_frontiers()))
            .expect("export"),
    );
    assert_eq!(
        bob.check_snapshot(document_id, ambre_core::Seq::new(2), &dropped)
            .expect("check"),
        SnapshotVerdict::Mismatch
    );
    assert_eq!(
        bob.check_snapshot(
            document_id,
            ambre_core::Seq::new(2),
            &Payload::<SnapshotBytes>::new(b"not a snapshot".to_vec())
        )
        .expect("check"),
        SnapshotVerdict::Mismatch
    );
}

// --- MURMURE-436 ---

/// Bob missed the second delta, and the relay purged it once the final
/// snapshot was confirmed. Loro keeps the shallow snapshot pending on his
/// copy, so he loads it whole and waits for no delta.
#[test]
fn a_copy_missing_purged_deltas_loads_the_final_snapshot() {
    let (alice, mut bob, closing, payload) = finalized_for(1);
    let document_id = closing.document_id();
    assert_eq!(bob.messages(closing).expect("held").len(), 1);
    bob.apply_remote_at(
        document_id,
        Some(Order::Snapshot(Seq::new(2))),
        (&payload).into(),
    )
    .expect("the snapshot loads");
    assert_eq!(
        bob.messages(closing).expect("held"),
        alice.messages(closing).expect("held")
    );
}
