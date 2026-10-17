//! The relay's number of each message, and the unread count it gives
//! (`features/ambre-spec-murmure-unread.md section 3`).

use ambre_bus::{MessageDraft, Timestamp, UnreadCount};
use ambre_core::{ChannelId, Seq, UserId};
use loro::{Container, ExportMode, ValueOrContainer};

use crate::schema::{MESSAGE_CONTENT, MESSAGES};
use crate::{ChannelState, ChannelType, MurmureError, Opening, OpeningChannel, Order, Position};

const NOW: Timestamp = Timestamp(1_700_000_000_000);

fn draft(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
    }
}

/// A channel just created, and a second member opened from its documents
/// with no number known.
fn pair() -> (ChannelState, ChannelState) {
    let alice = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
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
    assert!(opening.receive(meta_id.document_id(), &meta));
    assert!(matches!(
        opening.ready(meta_id.document_id()),
        Ok(Opening::OpenSegments(_))
    ));
    let segment = state.snapshot(active).expect("segment");
    assert!(opening.receive_at(active, covered.map(Order::Snapshot), &segment));
    match opening.ready(active) {
        Ok(Opening::Open(state)) => *state,
        other => panic!("the channel opens: {other:?}"),
    }
}

// --- MURMURE-113 ---

#[test]
fn a_message_takes_the_number_of_the_update_that_created_it() {
    let (mut alice, mut bob) = pair();
    let author = UserId::new();
    let segment = alice.active_id();
    let first = alice.send(&draft("one"), author, NOW).expect("send");
    let second = alice.send(&draft("two"), author, NOW).expect("send");
    bob.apply_remote_at(
        segment.document_id(),
        Some(Order::Update(Seq(5))),
        &first.delta,
    )
    .expect("apply");
    bob.apply_remote_at(
        segment.document_id(),
        Some(Order::Update(Seq(6))),
        &second.delta,
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
        [Some(Position::Exact(Seq(5))), Some(Position::Exact(Seq(6)))]
    );
    let me = UserId::new();
    assert_eq!(
        bob.unread(segment, Seq(4), me).expect("count"),
        UnreadCount(2)
    );
    assert_eq!(
        bob.unread(segment, Seq(5), me).expect("count"),
        UnreadCount(1)
    );
    assert_eq!(
        bob.unread(segment, Seq(6), me).expect("count"),
        UnreadCount::ZERO
    );
    assert_eq!(bob.delivered(segment.document_id()), Seq(6));
}

#[test]
fn the_authors_own_message_is_numbered_by_its_echo_and_never_counts() {
    let mut alice = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    let me = UserId::new();
    let segment = alice.active_id();
    let sent = alice.send(&draft("mine"), me, NOW).expect("send");
    let id = alice.messages(segment).expect("messages")[0].id;
    assert_eq!(alice.position(segment, id), Some(Position::Unknown));
    assert_eq!(
        alice.unread(segment, Seq::ZERO, me).expect("count"),
        UnreadCount::ZERO
    );

    // The relay hands every member the update, its author included.
    alice
        .apply_remote_at(
            segment.document_id(),
            Some(Order::Update(Seq(9))),
            &sent.delta,
        )
        .expect("echo");
    assert_eq!(alice.position(segment, id), Some(Position::Exact(Seq(9))));
}

#[test]
fn a_snapshot_hands_on_the_numbers_its_writer_knew() {
    let (mut alice, mut bob) = pair();
    let segment = alice.active_id();
    let sent = alice.send(&draft("one"), UserId::new(), NOW).expect("send");
    bob.apply_remote_at(
        segment.document_id(),
        Some(Order::Update(Seq(3))),
        &sent.delta,
    )
    .expect("apply");
    let id = bob.messages(segment).expect("messages")[0].id;

    let carol = open_from(&bob, Some(Seq(3)));
    assert_eq!(carol.position(segment, id), Some(Position::Exact(Seq(3))));
    assert_eq!(carol.delivered(segment.document_id()), Seq(3));
}

#[test]
fn a_snapshot_row_past_its_horizon_is_no_number() {
    let (mut alice, mut bob) = pair();
    let segment = alice.active_id();
    let sent = alice.send(&draft("one"), UserId::new(), NOW).expect("send");
    bob.apply_remote_at(
        segment.document_id(),
        Some(Order::Update(Seq(8))),
        &sent.delta,
    )
    .expect("apply");
    let id = bob.messages(segment).expect("messages")[0].id;

    // A table claiming 8 inside a snapshot folding up to 4 is a writer's
    // mistake: the message reads as bounded by the horizon.
    let carol = open_from(&bob, Some(Seq(4)));
    assert_eq!(carol.position(segment, id), Some(Position::AtMost(Seq(4))));
}

#[test]
fn a_message_known_only_from_a_snapshot_lies_within_its_horizon() {
    let mut alice = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    let segment = alice.active_id();
    alice.send(&draft("one"), UserId::new(), NOW).expect("send");
    let id = alice.messages(segment).expect("messages")[0].id;

    // Alice knows no number, so her snapshot carries no table.
    let bob = open_from(&alice, Some(Seq(12)));
    let me = UserId::new();
    assert_eq!(bob.position(segment, id), Some(Position::AtMost(Seq(12))));
    assert_eq!(
        bob.unread(segment, Seq(11), me).expect("count"),
        UnreadCount(1)
    );
    assert_eq!(
        bob.unread(segment, Seq(12), me).expect("count"),
        UnreadCount::ZERO
    );
}

#[test]
fn an_edit_creates_no_message_and_never_counts() {
    let (mut alice, mut bob) = pair();
    let segment = alice.active_id();
    let sent = alice
        .send(&draft("base"), UserId::new(), NOW)
        .expect("send");
    bob.apply_remote_at(
        segment.document_id(),
        Some(Order::Update(Seq(5))),
        &sent.delta,
    )
    .expect("apply");

    let doc = alice.doc(segment.document_id()).expect("segment");
    let before = doc.oplog_vv();
    let Some(ValueOrContainer::Container(Container::Map(message))) = doc.get_list(MESSAGES).get(0)
    else {
        panic!("a message is a map container");
    };
    let Some(ValueOrContainer::Container(Container::Text(content))) = message.get(MESSAGE_CONTENT)
    else {
        panic!("the content is a text");
    };
    content.insert(4, " edited").expect("edit");
    doc.commit();
    let edit = doc.export(ExportMode::updates(&before)).expect("delta");
    bob.apply_remote_at(segment.document_id(), Some(Order::Update(Seq(6))), &edit)
        .expect("apply");

    let id = bob.messages(segment).expect("messages")[0].id;
    assert_eq!(bob.position(segment, id), Some(Position::Exact(Seq(5))));
    assert_eq!(
        bob.unread(segment, Seq(5), UserId::new()).expect("count"),
        UnreadCount::ZERO
    );
}

#[test]
fn a_truncated_table_is_refused_before_any_import() {
    let (alice, mut bob) = pair();
    let segment = alice.active_id().document_id();
    let mut payload = b"amsq".to_vec();
    payload.extend_from_slice(&3u32.to_le_bytes());
    payload.extend_from_slice(&[0; 10]);
    assert!(matches!(
        bob.apply_remote_at(segment, Some(Order::Snapshot(Seq(2))), &payload),
        Err(MurmureError::Crdt(_))
    ));
    assert_eq!(bob.delivered(segment), Seq::ZERO);
}
