//! The files a message carries against their contract rows
//! (`features/ambre-spec-murmure-attachments.md`).

use ambre_bus::{Attachment, ByteSize, MediaRef, MediaType, MessageDraft, MurmureEvent, Timestamp};
use ambre_core::{ChannelId, UserId};

use crate::{ChannelState, ChannelType, MurmureError, Opening, OpeningChannel};

const NOW: Timestamp = Timestamp(1_700_000_000_000);

fn file(hex: char, name: &str) -> Attachment {
    Attachment {
        media_ref: MediaRef::parse(&hex.to_string().repeat(64)).expect("an address"),
        name: name.to_owned(),
        media_type: MediaType("image/png".to_owned()),
        size: ByteSize(4_096),
    }
}

fn plain(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    }
}

fn channel() -> ChannelState {
    ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify()
}

/// Another member opening `state`'s channel from its snapshots.
fn open_from(state: &ChannelState) -> ChannelState {
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
    assert!(opening.receive(active, &segment));
    match opening.ready(active) {
        Ok(Opening::Open(state)) => *state,
        other => panic!("the channel opens: {other:?}"),
    }
}

/// `MURMURE-134`: a file travels as its address, its name, its type and its
/// size, and reads back whole once another member opens the segment. No byte
/// of the file is in the delta.
#[test]
fn a_file_travels_by_its_address_and_reads_back_whole() {
    let mut state = channel();
    let alice = UserId::new();
    let files = [file('a', "plan.png"), file('b', "map.png")];
    let sent = state
        .send_attached(&plain(""), &files, alice, NOW)
        .expect("send");
    let MurmureEvent::Messages { messages, .. } = &sent.event else {
        panic!("a send shows its message");
    };
    assert_eq!(messages[0].attachments, files);
    let read = open_from(&state).messages(state.active_id()).expect("held");
    assert_eq!(read[0].attachments, files);
    // The delta names the address, never what the file holds.
    let delta = String::from_utf8_lossy(&sent.delta);
    assert!(delta.contains(files[0].media_ref.as_str()));
}

/// A file with no name is refused before anything is written.
#[test]
fn a_file_with_no_name_writes_nothing() {
    let mut state = channel();
    let refused = state.send_attached(&plain("hi"), &[file('a', " ")], UserId::new(), NOW);
    assert_eq!(refused.err(), Some(MurmureError::BadAttachment));
    assert!(state.messages(state.active_id()).expect("held").is_empty());
}

/// A deleted message carries no file, and its list leaves the document.
#[test]
fn a_deleted_message_carries_no_file() {
    let mut state = channel();
    let alice = UserId::new();
    state
        .send_attached(&plain("x"), &[file('a', "a.png")], alice, NOW)
        .expect("send");
    let segment = state.active_id();
    let id = state.messages(segment).expect("held")[0].id;
    state.delete(segment, id, alice).expect("delete");
    let read = &state.messages(segment).expect("held")[0];
    assert!(read.deleted);
    assert!(read.attachments.is_empty());
}

/// `MURMURE-240`: the delete of a message names each of its files once, but
/// none another live message of its author still carries. Another author's
/// message, or a deleted one, names nothing.
#[test]
fn a_delete_names_the_files_no_other_own_message_carries() {
    let mut state = channel();
    let alice = UserId::new();
    let bob = UserId::new();
    let (a, b, c) = (file('a', "a.png"), file('b', "b.png"), file('c', "c.png"));
    state
        .send_attached(&plain("1"), &[a.clone(), b.clone(), a.clone()], alice, NOW)
        .expect("send");
    state
        .send_attached(&plain("2"), std::slice::from_ref(&b), alice, NOW)
        .expect("send");
    state
        .send_attached(&plain("3"), std::slice::from_ref(&c), bob, NOW)
        .expect("send");
    let segment = state.active_id();
    let ids: Vec<_> = state
        .messages(segment)
        .expect("held")
        .iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(state.erasable(segment, ids[0], alice), [a.media_ref]);
    assert!(state.erasable(segment, ids[2], alice).is_empty(), "Bob's");
    assert_eq!(state.erasable(segment, ids[2], bob), [c.media_ref]);
    state.delete(segment, ids[0], alice).expect("delete");
    assert!(state.erasable(segment, ids[0], alice).is_empty(), "deleted");
    assert_eq!(state.erasable(segment, ids[1], alice), [b.media_ref]);
}
