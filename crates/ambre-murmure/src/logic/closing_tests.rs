//! The close of the active segment against its contract rows. They hold the
//! size that asks for it, the states it writes, and two writers of one close.

use ambre_bus::Char;
use ambre_core::index::Span;
use std::collections::HashMap;

use crate::message::{MessageDraft, MurmureEvent, StyleMark, TextStyle};
use ambre_bus::{Authored, Instant};
use ambre_core::id::{Channel, Id, Lease, Segment, User};
use ambre_core::{Count, DeltaBytes, Octets, Payload, PeerEntryBytes, Relay, Seq, SnapshotBytes};

use crate::logic::channel::live_segments;
use crate::logic::session::next_peer;
use crate::{ChannelState, ChannelType, MurmureError, Signer};
use ambre_core::{Ms, TimeSpan};

const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

fn draft(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    }
}

/// A channel whose segments close past `size` messages.
fn channel(size: usize) -> ChannelState {
    ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create")
        .with_segment_size(ambre_core::Count::<ambre_bus::Messages>::of(size))
        .identify()
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

/// Close the active segment of `state` into a successor it created, and
/// adopt the successor's identity. Returns the successor, the `meta` delta,
/// and the successor's first update.
fn close_created(
    state: &mut ChannelState,
) -> (Id<Segment>, Payload<DeltaBytes>, Payload<DeltaBytes>) {
    let closing = state.active_id();
    let successor = Id::<Segment>::mint();
    let closed = state
        .close_into(closing, successor, crate::Successor::Created, NOW)
        .expect("close");
    assert_eq!(closed.fetch, None, "the creator fetches nothing");
    let peer = next_peer();
    let first = state
        .adopt_session(
            successor.document_id(),
            peer,
            Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
        )
        .expect("adopt")
        .expect("the successor's first update");
    (successor, closed.meta, first)
}

fn live_of(state: &ChannelState) -> (Id<Segment>, Option<Id<Segment>>) {
    let meta = state.snapshot(state.meta_id().document_id()).expect("meta");
    let doc = loro::LoroDoc::new();
    doc.import(meta.as_bytes()).expect("import");
    let live = live_segments(&doc, state.channel_id()).expect("live");
    (live.active, live.closing)
}

// --- MURMURE-064 ---

/// A segment that reaches the segment size asks to close once, and asks
/// again only after a refusal.
#[test]
fn a_full_segment_asks_to_close_once() {
    let mut alice = channel(2);
    let author = Id::<User>::mint();
    alice.send(&draft("one"), author, NOW).expect("send");
    assert_eq!(alice.wants_close(), None, "below the segment size");
    alice.send(&draft("two"), author, NOW).expect("send");
    assert_eq!(alice.wants_close(), Some(alice.active_id()));
    assert_eq!(alice.wants_close(), None, "asked once");
    alice.close_refused();
    assert_eq!(alice.wants_close(), Some(alice.active_id()));
}

// --- MURMURE-060 ---

/// A rotation closes the active segment far below its size, once. Its
/// successor asks again only when full.
#[test]
fn a_rotation_closes_the_active_segment_whatever_it_holds() {
    let mut alice = channel(10);
    let author = Id::<User>::mint();
    alice.send(&draft("one"), author, NOW).expect("send");
    assert_eq!(alice.wants_close(), None, "below the segment size");
    alice.key_rotated();
    assert_eq!(alice.wants_close(), Some(alice.active_id()));
    alice.close_refused();
    assert_eq!(alice.wants_close(), Some(alice.active_id()), "asked again");
    let closing = alice.active_id();
    close_created(&mut alice);
    alice.lease_granted(
        closing,
        Id::<Lease>::mint(),
        ambre_core::Seq::<ambre_core::Relay>::new(1),
        TimeSpan::<Ms>::new(60_000),
    );
    alice.mark_final(closing).expect("mark").expect("a delta");
    assert_eq!(alice.closing_id(), None, "the slot is free");
    alice.send(&draft("two"), author, NOW).expect("send");
    assert_eq!(
        alice.wants_close(),
        None,
        "the successor is under the new key"
    );
}

/// A channel owes no close until a rotation retires its key, and owes one
/// from then on (`MURMURE-060`).
#[test]
fn a_rotation_owes_the_close_of_the_active_segment() {
    let mut alice = channel(10);
    assert!(!alice.owes_rotation_close());
    alice.key_rotated();
    assert!(alice.owes_rotation_close());
}

// --- MURMURE-481 ---

/// A segment of long messages asks to close at the send that takes its
/// snapshot past the byte bound, far below its message count.
#[test]
fn a_segment_past_its_bytes_asks_to_close_whatever_its_count() {
    const BYTES: Count<Octets> = Count::<Octets>::new(16 * 1024);
    let mut alice = channel(10_000).with_segment_snapshot_size(BYTES);
    let author = Id::<User>::mint();
    let mut sent = 0;
    loop {
        // Distinct words per message, so the snapshot does not compress them.
        let body: String = (0..40)
            .map(|w| format!("w{}x{w} ", sent * 97 + w))
            .collect();
        alice.send(&draft(&body), author, NOW).expect("send");
        sent += 1;
        let segment = alice.active_id();
        let bytes = alice
            .snapshot(segment.document_id())
            .expect("snapshot")
            .len();
        let asked = alice.wants_close();
        if bytes >= BYTES {
            assert_eq!(asked, Some(segment), "asked at the send that passes");
            break;
        }
        assert_eq!(asked, None, "below the bound at {sent} messages");
    }
    assert!(sent < 10_000, "the bytes bind before the count");
}

// --- MURMURE-482 ---

/// A successor that fails to open is fetched again past a delay that doubles
/// at each failure. Messages go to the old segment until it opens.
#[test]
fn a_successor_that_fails_to_open_is_fetched_again() {
    let mut alice = channel(1);
    let author = Id::<User>::mint();
    alice.send(&draft("one"), author, NOW).expect("send");
    let mut bob = peer_of(&alice);
    let old = alice.active_id();
    let (successor, meta, first) = close_created(&mut alice);
    bob.apply_remote(bob.meta_id().document_id(), (&meta).into())
        .expect("meta");
    assert_eq!(bob.successor_to_open(), Some(successor));

    // The Sync worker could not open it.
    assert!(bob.cancel_older(successor.document_id()));
    let at = |ms: u64| Instant::<Authored>::new(NOW.get() + ms);
    assert_eq!(bob.successor_to_retry(NOW), None, "the tick dates it");
    assert_eq!(bob.successor_to_retry(at(999)), None);
    assert_eq!(bob.successor_to_retry(at(1_000)), Some(successor));
    assert_eq!(bob.successor_to_retry(at(1_000)), None, "on its way");

    // It opened with no update yet, so its header check fails.
    assert!(bob.older_ready(successor.document_id()).is_err());
    let waiting = bob.send(&draft("meanwhile"), author, NOW).expect("send");
    assert_eq!(waiting.segment, old, "never written before it opens");
    assert_eq!(bob.successor_to_retry(at(5_000)), None, "the tick dates it");
    assert_eq!(bob.successor_to_retry(at(6_999)), None, "doubled");
    assert_eq!(bob.successor_to_retry(at(7_000)), Some(successor));

    let signer = Signer {
        peer: alice.peer(successor.document_id()).expect("a session"),
        user: Some(author),
    };
    bob.apply_signed(
        successor.document_id(),
        Seq::<Relay>::new(1),
        &first,
        signer,
    )
    .expect("arrival");
    bob.older_ready(successor.document_id()).expect("ready");
    assert_eq!((bob.active_id(), bob.closing_id()), (successor, Some(old)));
    assert_eq!(bob.successor_to_retry(at(1_000_000)), None, "nothing left");
}

/// A successor that failed five times is no longer fetched.
#[test]
fn a_successor_is_fetched_again_four_times_at_most() {
    let mut alice = channel(1);
    alice
        .send(&draft("one"), Id::<User>::mint(), NOW)
        .expect("send");
    let mut bob = peer_of(&alice);
    let (successor, meta, _) = close_created(&mut alice);
    bob.apply_remote(bob.meta_id().document_id(), (&meta).into())
        .expect("meta");
    assert_eq!(bob.successor_to_open(), Some(successor));
    let mut now = NOW.get();
    let mut fetches = 0;
    for _ in 0..10 {
        bob.cancel_older(successor.document_id());
        assert_eq!(bob.successor_to_retry(Instant::<Authored>::new(now)), None);
        now += 60_000;
        if bob
            .successor_to_retry(Instant::<Authored>::new(now))
            .is_some()
        {
            fetches += 1;
        }
    }
    assert_eq!(fetches, 4);
}

// --- MURMURE-065, MURMURE-066 ---

/// A close makes the old segment `closing` and the successor `active`, in
/// the channel and in `meta`. No message moves, and the next one goes to the
/// successor.
#[test]
fn a_close_makes_the_segment_closing_and_its_successor_active() {
    let mut alice = channel(1);
    let author = Id::<User>::mint();
    let before = alice.send(&draft("before"), author, NOW).expect("send");
    let old = alice.active_id();
    let kept = alice.messages(old).expect("messages");
    let (successor, _, _) = close_created(&mut alice);

    assert_eq!(alice.active_id(), successor);
    assert_eq!(alice.closing_id(), Some(old));
    assert_eq!(live_of(&alice), (successor, Some(old)));
    assert_eq!(alice.messages(old).expect("messages"), kept);
    assert!(alice.messages(successor).expect("messages").is_empty());

    let after = alice.send(&draft("after"), author, NOW).expect("send");
    assert_eq!((before.segment, after.segment), (old, successor));
    assert_eq!(alice.messages(old).expect("messages"), kept);
    assert_eq!(alice.messages(successor).expect("messages").len(), 1);
}

/// A close of a segment no longer active, or while another one is still
/// closing, is refused, and asks for none.
#[test]
fn a_close_while_a_segment_is_closing_is_refused() {
    let mut alice = channel(1);
    alice
        .send(&draft("one"), Id::<User>::mint(), NOW)
        .expect("send");
    let old = alice.active_id();
    let (successor, _, _) = close_created(&mut alice);
    alice
        .send(&draft("two"), Id::<User>::mint(), NOW)
        .expect("send");
    assert_eq!(alice.wants_close(), None, "a segment is still closing");
    assert!(matches!(
        alice.close_into(old, Id::<Segment>::mint(), crate::Successor::Created, NOW),
        Err(MurmureError::NotActive(_))
    ));
    assert!(matches!(
        alice.close_into(
            successor,
            Id::<Segment>::mint(),
            crate::Successor::Created,
            NOW
        ),
        Err(MurmureError::StillClosing(_))
    ));
}

// --- MURMURE-010, MURMURE-011 ---

/// Two members write one close at once: the creator holds the successor,
/// the other fetches it. Their `meta` documents merge into one active and
/// one closing segment.
#[test]
fn two_writers_of_one_close_converge_on_one_successor() {
    let mut alice = channel(1);
    alice
        .send(&draft("one"), Id::<User>::mint(), NOW)
        .expect("send");
    let mut bob = peer_of(&alice);
    let old = alice.active_id();
    let (successor, alice_meta, _) = close_created(&mut alice);
    let bob_close = bob
        .close_into(old, successor, crate::Successor::Fetched, NOW)
        .expect("the loser writes the same answer");
    assert_eq!(bob_close.fetch, Some(successor));

    let meta = alice.meta_id().document_id();
    alice
        .apply_remote(meta, (&bob_close.meta).into())
        .expect("merge");
    bob.apply_remote(meta, (&alice_meta).into()).expect("merge");
    for state in [&alice, &bob] {
        assert_eq!(live_of(state), (successor, Some(old)));
        assert_eq!(state.segments_after(old), [successor], "one successor");
    }
}

// --- MURMURE-066 ---

/// A member who did not write the close follows it from `meta`. It fetches
/// the successor once and keeps sending to the old segment meanwhile. Once
/// the successor is in, it holds it as active.
#[test]
fn a_remote_close_is_followed_once_its_successor_arrives() {
    let mut alice = channel(1);
    let author = Id::<User>::mint();
    alice.send(&draft("one"), author, NOW).expect("send");
    let mut bob = peer_of(&alice);
    let old = alice.active_id();
    let (successor, meta, first) = close_created(&mut alice);

    bob.apply_remote(bob.meta_id().document_id(), (&meta).into())
        .expect("meta");
    assert_eq!(bob.successor_to_open(), Some(successor));
    assert_eq!(bob.successor_to_open(), None, "asked once");
    let waiting = bob.send(&draft("meanwhile"), author, NOW).expect("send");
    assert_eq!(
        waiting.segment, old,
        "the old segment takes it while it closes"
    );

    let signer = Signer {
        peer: alice.peer(successor.document_id()).expect("a session"),
        user: Some(author),
    };
    bob.apply_signed(
        successor.document_id(),
        Seq::<Relay>::new(1),
        &first,
        signer,
    )
    .expect("arrival");
    let event = bob.older_ready(successor.document_id()).expect("ready");
    assert_eq!(
        event,
        MurmureEvent::Messages {
            segment: successor,
            messages: Vec::new()
        }
    );
    assert_eq!((bob.active_id(), bob.closing_id()), (successor, Some(old)));
}

// --- MURMURE-263 ---

/// Bob writes to the old segment while its close is in flight, and the relay
/// refuses both deltas for the lock. The old segment goes back to the
/// relay's state, and the message moves to the active segment with its
/// identifier and Bob's reaction. Alice's check accepts it there.
#[test]
fn a_message_refused_by_the_lock_moves_to_the_active_segment() {
    move_refused_message(false);
}

/// The rewind starts at the least counter the refused deltas name. A caller
/// that lists them out of order still moves every refused message
/// (`MURMURE-263`).
#[test]
fn the_rewind_starts_at_the_first_refused_operation_in_any_order() {
    move_refused_message(true);
}

/// Bob's message and reaction, refused for the lock, listed in the order
/// they left or in the reverse one.
fn move_refused_message(reversed: bool) {
    let mut alice = channel(1);
    let (alice_user, bob_user) = (Id::<User>::mint(), Id::<User>::mint());
    alice.send(&draft("one"), alice_user, NOW).expect("send");
    let mut bob = peer_of(&alice);
    let old = alice.active_id();
    let (successor, meta, first) = close_created(&mut alice);

    bob.apply_remote(bob.meta_id().document_id(), (&meta).into())
        .expect("meta");
    bob.successor_to_open();
    // `MURMURE-562`: the moved message keeps its styles.
    let code = StyleMark {
        style: TextStyle::Code,
        range: Span::<Char>::from_usize_range(0..4),
    };
    let styled = MessageDraft {
        styles: vec![code.clone()],
        ..draft("meanwhile")
    };
    let waiting = bob.send(&styled, bob_user, NOW).expect("send");
    let MurmureEvent::Messages { messages, .. } = &waiting.event else {
        panic!("a message");
    };
    let moved = messages[0].id;
    let reacted = bob
        .react(
            old,
            moved,
            &ambre_bus::Emoji::from("\u{1F44D}"),
            bob_user,
            NOW,
        )
        .expect("react")
        .expect("a reaction");
    let signer = Signer {
        peer: alice.peer(successor.document_id()).expect("a session"),
        user: Some(alice_user),
    };
    bob.apply_signed(
        successor.document_id(),
        Seq::<Relay>::new(1),
        &first,
        signer,
    )
    .expect("arrival");
    bob.older_ready(successor.document_id()).expect("ready");
    let peer = next_peer();
    bob.adopt_session(
        successor.document_id(),
        peer,
        Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
    )
    .expect("adopt");

    let mut refused = [waiting.delta, reacted.delta];
    if reversed {
        refused.reverse();
    }
    let relocated = bob
        .relocate_locked(old.document_id(), &refused)
        .expect("relocate");
    assert!(relocated.replay.is_empty(), "no older message changed");
    let sent = relocated.sent.expect("a message moves");
    assert_eq!(sent.segment, successor);
    let MurmureEvent::Messages { segment, messages } = &sent.event else {
        panic!("the moved message");
    };
    assert_eq!(*segment, successor);
    assert_eq!(
        messages
            .iter()
            .map(|m| (m.id, m.text.as_str()))
            .collect::<Vec<_>>(),
        [(moved, "meanwhile")]
    );
    assert_eq!(messages[0].reactions[0].users, [bob_user]);
    assert_eq!(messages[0].styles, std::slice::from_ref(&code));
    assert_eq!(
        bob.messages(old).expect("old"),
        alice.messages(old).expect("old"),
        "the old segment holds what the relay holds"
    );

    let signer = Signer {
        peer,
        user: Some(bob_user),
    };
    alice
        .apply_signed(
            successor.document_id(),
            Seq::<Relay>::new(2),
            &sent.delta,
            signer,
        )
        .expect("the check accepts the moved message");
    let held = alice.messages(successor).expect("successor");
    assert_eq!(held, bob.messages(successor).expect("successor"));
    let written = held
        .iter()
        .find(|m| m.id == moved)
        .expect("the active segment holds the moved message");
    assert_eq!(written.text, "meanwhile");
    assert_eq!(written.styles, [code]);
    assert_eq!(written.reactions[0].users, [bob_user]);
}

/// A refusal that created no message moves nothing, and the active segment
/// is never rewound.
#[test]
fn a_refused_reaction_alone_moves_nothing() {
    let mut alice = channel(1);
    let author = Id::<User>::mint();
    let sent = alice.send(&draft("one"), author, NOW).expect("send");
    let MurmureEvent::Messages { messages, .. } = &sent.event else {
        panic!("a message");
    };
    let message = messages[0].id;
    let old = alice.active_id();
    close_created(&mut alice);
    let reacted = alice
        .react(
            old,
            message,
            &ambre_bus::Emoji::from("\u{1F44D}"),
            author,
            NOW,
        )
        .expect("react")
        .expect("a reaction");
    let moved = alice
        .relocate_locked(old.document_id(), &[reacted.delta])
        .expect("relocate");
    assert!(moved.sent.is_none());
    assert!(moved.replay.is_empty(), "a reaction is not replayed");
    let kept = alice.messages(old).expect("old");
    assert!(kept[0].reactions.is_empty(), "the refused reaction is gone");
    assert!(matches!(
        alice.relocate_locked(alice.active_id().document_id(), &[]),
        Err(MurmureError::NotActive(_))
    ));
}

/// The byte bound of a segment is the 2 MB of the budget row, in binary
/// megabytes (`MURMURE-481`).
#[test]
fn the_byte_bound_of_a_segment_is_two_megabytes() {
    assert_eq!(crate::SEGMENT_SNAPSHOT_SIZE.get(), 2_097_152);
}

// --- AUTHOR-026 ---

/// The identity the Sync worker issues for a successor still arriving is
/// kept, and the successor takes it once admitted. Until then this member
/// may not write it.
#[test]
fn an_identity_issued_before_the_successor_arrives_is_adopted_on_admission() {
    let mut alice = channel(1);
    let author = Id::<User>::mint();
    alice.send(&draft("one"), author, NOW).expect("send");
    let mut bob = peer_of(&alice);
    let (successor, meta, first) = close_created(&mut alice);
    bob.apply_remote(bob.meta_id().document_id(), (&meta).into())
        .expect("meta");
    assert_eq!(bob.successor_to_open(), Some(successor));
    let peer = next_peer();
    assert_eq!(
        bob.adopt_session(
            successor.document_id(),
            peer,
            Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
        ),
        Ok(None),
        "kept for the successor still arriving"
    );
    assert!(!bob.identified(successor.document_id()), "not held yet");
    let signer = Signer {
        peer: alice.peer(successor.document_id()).expect("a session"),
        user: Some(author),
    };
    bob.apply_signed(
        successor.document_id(),
        Seq::<Relay>::new(1),
        &first,
        signer,
    )
    .expect("arrival");
    bob.older_ready(successor.document_id()).expect("ready");
    assert!(bob.identified(successor.document_id()));
    assert_eq!(bob.peer(successor.document_id()), Some(peer));
}
