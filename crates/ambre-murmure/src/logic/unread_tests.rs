//! The relay's number of each message, and the unread count it gives
//! (`features/ambre-spec-murmure-unread.md section 3`).

use crate::Receipt;
use crate::message::{MessageDraft, MurmureEvent};
use ambre_bus::{Authored, Instant, Unread};
use ambre_core::id::{Channel, Id, Segment, User};
use ambre_core::{DeltaBytes, LoroPayloadRef, Payload, Relay, Seq, SnapshotBytes};
use loro::{Container, ValueOrContainer};

use crate::logic::schema::{MessageKey, Root};
use crate::{
    ChannelState, ChannelType, MurmureError, Opening, OpeningChannel, Order, Position, Signer,
};

const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

/// Who signed what `from` wrote in `segment`: `user`, under its session.
fn signer(from: &ChannelState, segment: Id<Segment>, user: Id<User>) -> Signer {
    Signer {
        nonce: crate::logic::delta::TEST_NONCE,
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
fn open_from(state: &ChannelState, covered: Option<Seq<Relay>>) -> ChannelState {
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
    bob.apply_signed(
        segment.document_id(),
        Seq::<Relay>::new(5),
        &first.delta,
        by,
    )
    .expect("apply");
    bob.apply_signed(
        segment.document_id(),
        Seq::<Relay>::new(6),
        &second.delta,
        by,
    )
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
            Some(Position::Exact(Seq::<Relay>::new(5))),
            Some(Position::Exact(Seq::<Relay>::new(6)))
        ]
    );
    let me = Id::<User>::mint();
    assert_eq!(
        bob.unread(segment, Seq::<Relay>::new(4), me)
            .expect("count"),
        ambre_core::Count::<Unread>::new(2)
    );
    assert_eq!(
        bob.unread(segment, Seq::<Relay>::new(5), me)
            .expect("count"),
        ambre_core::Count::<Unread>::new(1)
    );
    assert_eq!(
        bob.unread(segment, Seq::<Relay>::new(6), me)
            .expect("count"),
        ambre_core::Count::<Unread>::ZERO
    );
    assert_eq!(bob.delivered(segment.document_id()), Seq::<Relay>::new(6));
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
        alice
            .unread(segment, Seq::<Relay>::ZERO, me)
            .expect("count"),
        ambre_core::Count::<Unread>::ZERO
    );

    // The relay hands every member the update, its author included.
    let by = signer(&alice, segment, me);
    alice
        .apply_signed(segment.document_id(), Seq::<Relay>::new(9), &sent.delta, by)
        .expect("echo");
    assert_eq!(
        alice.position(segment, id),
        Some(Position::Exact(Seq::<Relay>::new(9)))
    );
}

/// `MURMURE-674`, `MURMURE-675`: the echo of this member's own delta numbers the message it
/// created and answers nothing, since the view already shows that message.
/// The same delta from another member still answers with the list.
#[test]
fn the_echo_of_an_own_delta_numbers_its_message_and_answers_nothing() {
    let (mut alice, mut bob) = pair();
    let segment = alice.active_id();
    let me = Id::<User>::mint();
    let first = alice.send(&draft("one"), me, NOW).expect("send");
    let second = alice.send(&draft("two"), me, NOW).expect("send");
    let by = signer(&alice, segment, me);
    for (seq, sent) in [(1, &first), (2, &second)] {
        let echoed = alice
            .apply_signed(
                segment.document_id(),
                Seq::<Relay>::new(seq),
                &sent.delta,
                by,
            )
            .expect("echo");
        assert!(echoed.is_empty(), "the echo {seq} answered {echoed:?}");
    }
    let numbers: Vec<Option<Position>> = alice
        .messages(segment)
        .expect("messages")
        .iter()
        .map(|message| alice.position(segment, message.id))
        .collect();
    assert_eq!(
        numbers,
        [1, 2].map(|seq| Some(Position::Exact(Seq::<Relay>::new(seq))))
    );

    let heard = bob
        .apply_signed(
            segment.document_id(),
            Seq::<Relay>::new(1),
            &first.delta,
            by,
        )
        .expect("apply");
    assert!(
        matches!(heard.as_slice(), [MurmureEvent::Messages { messages, .. }] if messages.len() == 1),
        "another member's copy answers with the list, got {heard:?}"
    );
}

/// `MURMURE-113`: the relay delivers an update in Loro's own form, with its number. It
/// numbers the message it created, as the JSON form of a segment delta does.
#[test]
fn a_numbered_update_in_loro_form_numbers_the_message_it_created() {
    let (mut alice, mut bob) = pair();
    let segment = alice.active_id();
    let before = alice
        .doc(segment.document_id())
        .expect("segment")
        .oplog_vv();
    alice
        .send(&draft("one"), Id::<User>::mint(), NOW)
        .expect("send");
    let doc = alice.doc(segment.document_id()).expect("segment");
    let update = Payload::<DeltaBytes>::new(
        doc.export(loro::ExportMode::updates(&before))
            .expect("export"),
    );
    bob.apply_remote_at(
        segment.document_id(),
        Some(Order::Update(Seq::<Relay>::new(4))),
        LoroPayloadRef::Delta(&update),
    )
    .expect("apply");
    let id = bob.messages(segment).expect("messages")[0].id;
    assert_eq!(
        bob.position(segment, id),
        Some(Position::Exact(Seq::<Relay>::new(4)))
    );
}

#[test]
fn a_snapshot_hands_on_the_numbers_its_writer_knew() {
    let (mut alice, mut bob) = pair();
    let segment = alice.active_id();
    let author = Id::<User>::mint();
    let sent = alice.send(&draft("one"), author, NOW).expect("send");
    let by = signer(&alice, segment, author);
    bob.apply_signed(segment.document_id(), Seq::<Relay>::new(3), &sent.delta, by)
        .expect("apply");
    let id = bob.messages(segment).expect("messages")[0].id;

    let carol = open_from(&bob, Some(Seq::<Relay>::new(3)));
    assert_eq!(
        carol.position(segment, id),
        Some(Position::Exact(Seq::<Relay>::new(3)))
    );
    assert_eq!(carol.delivered(segment.document_id()), Seq::<Relay>::new(3));
}

#[test]
fn a_snapshot_row_past_its_horizon_is_no_number() {
    let (mut alice, mut bob) = pair();
    let segment = alice.active_id();
    let author = Id::<User>::mint();
    let sent = alice.send(&draft("one"), author, NOW).expect("send");
    let by = signer(&alice, segment, author);
    bob.apply_signed(segment.document_id(), Seq::<Relay>::new(8), &sent.delta, by)
        .expect("apply");
    let id = bob.messages(segment).expect("messages")[0].id;

    // A table claiming 8 inside a snapshot folding up to 4 is a writer's
    // mistake: the message reads as bounded by the horizon.
    let carol = open_from(&bob, Some(Seq::<Relay>::new(4)));
    assert_eq!(
        carol.position(segment, id),
        Some(Position::AtMost(Seq::<Relay>::new(4)))
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
    let bob = open_from(&alice, Some(Seq::<Relay>::new(12)));
    let me = Id::<User>::mint();
    assert_eq!(
        bob.position(segment, id),
        Some(Position::AtMost(Seq::<Relay>::new(12)))
    );
    assert_eq!(
        bob.unread(segment, Seq::<Relay>::new(11), me)
            .expect("count"),
        ambre_core::Count::<Unread>::new(1)
    );
    assert_eq!(
        bob.unread(segment, Seq::<Relay>::new(12), me)
            .expect("count"),
        ambre_core::Count::<Unread>::ZERO
    );
}

#[test]
fn an_edit_creates_no_message_and_never_counts() {
    let (mut alice, mut bob) = pair();
    let segment = alice.active_id();
    let author = Id::<User>::mint();
    let sent = alice.send(&draft("base"), author, NOW).expect("send");
    let by = signer(&alice, segment, author);
    bob.apply_signed(segment.document_id(), Seq::<Relay>::new(5), &sent.delta, by)
        .expect("apply");

    let doc = alice.doc(segment.document_id()).expect("segment");
    let before = doc.oplog_vv();
    let Some(message) = crate::logic::schema::root_list(doc, Root::Messages)
        .get(0)
        .and_then(crate::logic::schema::fields_of)
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
    let edit = crate::logic::delta::encode(
        &doc.export_json_updates_without_peer_compression(&before, &doc.oplog_vv()),
    )
    .expect("delta");
    bob.apply_signed(segment.document_id(), Seq::<Relay>::new(6), &edit, by)
        .expect("apply");

    let id = bob.messages(segment).expect("messages")[0].id;
    assert_eq!(
        bob.position(segment, id),
        Some(Position::Exact(Seq::<Relay>::new(5)))
    );
    assert_eq!(
        bob.unread(segment, Seq::<Relay>::new(5), Id::<User>::mint())
            .expect("count"),
        ambre_core::Count::<Unread>::ZERO
    );
}

#[test]
fn a_truncated_table_is_refused_before_any_import() {
    let (alice, mut bob) = pair();
    let segment = alice.active_id().document_id();
    let mut payload = crate::logic::sequence::FRAME_MAGIC.to_vec();
    payload.extend_from_slice(&3u32.to_le_bytes());
    payload.extend_from_slice(&[0; 10]);
    let payload = Payload::<SnapshotBytes>::new(payload);
    assert!(matches!(
        bob.apply_remote_at(
            segment,
            Some(Order::Snapshot(Seq::<Relay>::new(2))),
            (&payload).into()
        ),
        Err(MurmureError::Crdt(_))
    ));
    assert_eq!(bob.delivered(segment), Seq::<Relay>::ZERO);
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
        bob.apply_signed(
            segment.document_id(),
            Seq::<Relay>::new(seq),
            &sent.delta,
            by,
        )
        .expect("apply");
    }
    let ids: Vec<_> = bob
        .messages(segment)
        .expect("messages")
        .iter()
        .map(|m| m.id)
        .collect();
    let first = |read: Seq<Relay>| bob.first_unread(|_| read, me);
    assert_eq!(first(Seq::<Relay>::new(4)), ids.first().copied());
    assert_eq!(
        first(Seq::<Relay>::new(5)),
        ids.get(2).copied(),
        "the reader's own message is never the first unread"
    );
    assert_eq!(first(Seq::<Relay>::new(7)), None);
}
