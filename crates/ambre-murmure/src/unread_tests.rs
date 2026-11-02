//! The relay's number of each message, and the unread count it gives
//! (`features/ambre-spec-murmure-unread.md section 3`).

use crate::Receipt;
use ambre_bus::{Authored, Instant, MessageDraft, Unread};
use ambre_core::id::{Channel, Id, Segment, User};
use ambre_core::{Payload, Seq, SnapshotBytes};
use loro::{Container, ValueOrContainer};

use crate::schema::{MessageKey, Root};
use crate::{
    ChannelState, ChannelType, MurmureError, Opening, OpeningChannel, Order, Position, Signer,
};

const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

/// Who signed what `from` wrote in `segment`: `user`, under its session.
fn signer(from: &ChannelState, segment: Id<Segment>, user: Id<User>) -> Signer {
    Signer {
        peer: from.peer(segment.document_id()).expect("a session"),
        user: Some(user),
    }
}

fn draft(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    }
}

/// A channel just created, and a second member opened from its documents
/// with no number known.
fn pair() -> (ChannelState, ChannelState) {
    let alice = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    let bob = open_from(&alice, None);
    (alice, bob)
}

/// A member opening `state`'s channel over the relay: `meta`, then the
/// active segment as one snapshot folding every update up to `covered`.
fn open_from(state: &ChannelState, covered: Option<Seq>) -> ChannelState {
    let meta_id = state.meta_id();
    let active = state.active_id().document_id();
    let mut opening = OpeningChannel::new(state.channel_id(), meta_id);
    let meta = state.snapshot(meta_id.document_id()).expect("meta");
    assert_eq!(
        opening.receive(meta_id.document_id(), (&meta).into()),
        Receipt::Kept
    );
    assert!(matches!(
        opening.ready(meta_id.document_id()),
        Ok(Opening::OpenSegments(_))
    ));
    let segment = state.snapshot(active).expect("segment");
    assert_eq!(
        opening.receive_at(active, covered.map(Order::Snapshot), (&segment).into()),
        Receipt::Kept
    );
    match opening.ready(active) {
        Ok(Opening::Open(state)) => *state,
        other => panic!("the channel opens: {other:?}"),
    }
}

// --- MURMURE-113 ---

#[test]
fn a_message_takes_the_number_of_the_update_that_created_it() {
    let (mut alice, mut bob) = pair();
    let author = Id::<User>::mint();
    let segment = alice.active_id();
    let first = alice.send(&draft("one"), author, NOW).expect("send");
    let second = alice.send(&draft("two"), author, NOW).expect("send");
    let by = signer(&alice, segment, author);
    bob.apply_signed(segment.document_id(), Seq::new(5), &first.delta, by)
        .expect("apply");
    bob.apply_signed(segment.document_id(), Seq::new(6), &second.delta, by)
        .expect("apply");

    let ids: Vec<_> = bob
        .messages(segment)
        .expect("messages")
        .iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(
        ids.iter()
            .map(|id| bob.position(segment, *id))
            .collect::<Vec<_>>(),
        [
            Some(Position::Exact(Seq::new(5))),
            Some(Position::Exact(Seq::new(6)))
        ]
    );
    let me = Id::<User>::mint();
    assert_eq!(
        bob.unread(segment, Seq::new(4), me).expect("count"),
        ambre_core::Tally::<Unread>::new(2)
    );
    assert_eq!(
        bob.unread(segment, Seq::new(5), me).expect("count"),
        ambre_core::Tally::<Unread>::new(1)
    );
    assert_eq!(
        bob.unread(segment, Seq::new(6), me).expect("count"),
        ambre_core::Tally::<Unread>::ZERO
    );
    assert_eq!(bob.delivered(segment.document_id()), Seq::new(6));
}

#[test]
fn the_authors_own_message_is_numbered_by_its_echo_and_never_counts() {
    let mut alice = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    let me = Id::<User>::mint();
    let segment = alice.active_id();
    let sent = alice.send(&draft("mine"), me, NOW).expect("send");
    let id = alice.messages(segment).expect("messages")[0].id;
    assert_eq!(alice.position(segment, id), Some(Position::Unknown));
    assert_eq!(
        alice.unread(segment, Seq::ZERO, me).expect("count"),
        ambre_core::Tally::<Unread>::ZERO
    );

    // The relay hands every member the update, its author included.
    let by = signer(&alice, segment, me);
    alice
        .apply_signed(segment.document_id(), Seq::new(9), &sent.delta, by)
        .expect("echo");
    assert_eq!(
        alice.position(segment, id),
        Some(Position::Exact(Seq::new(9)))
    );
}

#[test]
fn a_snapshot_hands_on_the_numbers_its_writer_knew() {
    let (mut alice, mut bob) = pair();
    let segment = alice.active_id();
    let author = Id::<User>::mint();
    let sent = alice.send(&draft("one"), author, NOW).expect("send");
    let by = signer(&alice, segment, author);
    bob.apply_signed(segment.document_id(), Seq::new(3), &sent.delta, by)
        .expect("apply");
    let id = bob.messages(segment).expect("messages")[0].id;

    let carol = open_from(&bob, Some(Seq::new(3)));
    assert_eq!(
        carol.position(segment, id),
        Some(Position::Exact(Seq::new(3)))
    );
    assert_eq!(carol.delivered(segment.document_id()), Seq::new(3));
}

#[test]
fn a_snapshot_row_past_its_horizon_is_no_number() {
    let (mut alice, mut bob) = pair();
    let segment = alice.active_id();
    let author = Id::<User>::mint();
    let sent = alice.send(&draft("one"), author, NOW).expect("send");
    let by = signer(&alice, segment, author);
    bob.apply_signed(segment.document_id(), Seq::new(8), &sent.delta, by)
        .expect("apply");
    let id = bob.messages(segment).expect("messages")[0].id;

    // A table claiming 8 inside a snapshot folding up to 4 is a writer's
    // mistake: the message reads as bounded by the horizon.
    let carol = open_from(&bob, Some(Seq::new(4)));
    assert_eq!(
        carol.position(segment, id),
        Some(Position::AtMost(Seq::new(4)))
    );
}

#[test]
fn a_message_known_only_from_a_snapshot_lies_within_its_horizon() {
    let mut alice = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    let segment = alice.active_id();
    alice
        .send(&draft("one"), Id::<User>::mint(), NOW)
        .expect("send");
    let id = alice.messages(segment).expect("messages")[0].id;

    // Alice knows no number, so her snapshot carries no table.
    let bob = open_from(&alice, Some(Seq::new(12)));
    let me = Id::<User>::mint();
    assert_eq!(
        bob.position(segment, id),
        Some(Position::AtMost(Seq::new(12)))
    );
    assert_eq!(
        bob.unread(segment, Seq::new(11), me).expect("count"),
        ambre_core::Tally::<Unread>::new(1)
    );
    assert_eq!(
        bob.unread(segment, Seq::new(12), me).expect("count"),
        ambre_core::Tally::<Unread>::ZERO
    );
}

#[test]
fn an_edit_creates_no_message_and_never_counts() {
    let (mut alice, mut bob) = pair();
    let segment = alice.active_id();
    let author = Id::<User>::mint();
    let sent = alice.send(&draft("base"), author, NOW).expect("send");
    let by = signer(&alice, segment, author);
    bob.apply_signed(segment.document_id(), Seq::new(5), &sent.delta, by)
        .expect("apply");

    let doc = alice.doc(segment.document_id()).expect("segment");
    let before = doc.oplog_vv();
    let Some(message) = crate::schema::root_list(doc, Root::Messages)
        .get(0)
        .and_then(crate::schema::fields_of)
    else {
        panic!("a message is a map container");
    };
    let Some(ValueOrContainer::Container(Container::Text(content))) =
        message.get(MessageKey::Content)
    else {
        panic!("the content is a text");
    };
    content.insert(4, " edited").expect("edit");
    doc.commit();
    let edit = crate::delta::encode(
        &doc.export_json_updates_without_peer_compression(&before, &doc.oplog_vv()),
    )
    .expect("delta");
    bob.apply_signed(segment.document_id(), Seq::new(6), &edit, by)
        .expect("apply");

    let id = bob.messages(segment).expect("messages")[0].id;
    assert_eq!(
        bob.position(segment, id),
        Some(Position::Exact(Seq::new(5)))
    );
    assert_eq!(
        bob.unread(segment, Seq::new(5), Id::<User>::mint())
            .expect("count"),
        ambre_core::Tally::<Unread>::ZERO
    );
}

#[test]
fn a_truncated_table_is_refused_before_any_import() {
    let (alice, mut bob) = pair();
    let segment = alice.active_id().document_id();
    let mut payload = crate::sequence::FRAME_MAGIC.to_vec();
    payload.extend_from_slice(&3u32.to_le_bytes());
    payload.extend_from_slice(&[0; 10]);
    let payload = Payload::<SnapshotBytes>::new(payload);
    assert!(matches!(
        bob.apply_remote_at(
            segment,
            Some(Order::Snapshot(Seq::new(2))),
            (&payload).into()
        ),
        Err(MurmureError::Crdt(_))
    ));
    assert_eq!(bob.delivered(segment), Seq::ZERO);
}

// --- MURMURE-499 ---

#[test]
fn the_first_unread_message_is_the_first_past_the_cursor_by_another() {
    let (mut alice, mut bob) = pair();
    let (author, me) = (Id::<User>::mint(), Id::<User>::mint());
    let segment = alice.active_id();
    let sent = [("one", author), ("mine", me), ("two", author)];
    for (seq, (text, from)) in (5..).zip(sent) {
        let sent = alice.send(&draft(text), from, NOW).expect("send");
        let by = signer(&alice, segment, from);
        bob.apply_signed(segment.document_id(), Seq::new(seq), &sent.delta, by)
            .expect("apply");
    }
    let ids: Vec<_> = bob
        .messages(segment)
        .expect("messages")
        .iter()
        .map(|m| m.id)
        .collect();
    let first = |read: Seq| bob.first_unread(|_| read, me);
    assert_eq!(first(Seq::new(4)), ids.first().copied());
    assert_eq!(
        first(Seq::new(5)),
        ids.get(2).copied(),
        "the reader's own message is never the first unread"
    );
    assert_eq!(first(Seq::new(7)), None);
}
