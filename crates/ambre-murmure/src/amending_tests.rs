//! The amendments of a locked segment against their contract rows. They hold
//! where a late change writes, what a member reads over the segment, and the
//! step of the authorship check that guards the document.

use ambre_bus::CharSpan;
use std::collections::HashMap;

use ambre_bus::{
    ActivityKind, MessageDraft, MessageView, MurmureEvent, StyleMark, TextStyle, Timestamp,
};
use ambre_core::id::{Channel, Document, Id, Message, Segment, User};
use ambre_core::{DeltaBytes, PeerEntryBytes, Seq, SnapshotBytes};
use loro::{LoroDoc, LoroMap, VersionVector};

use crate::channel::read_messages;
use crate::schema::{AmendmentKey, Root, SegmentKey};
use crate::session::next_peer;
use crate::{
    ChangeTarget, ChannelState, ChannelType, MurmureError, Refusal, Replay, Sent, Signer,
    ThreadTarget,
};

const NOW: Timestamp = Timestamp(1_700_000_000_000);

fn draft(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    }
}

/// A channel whose active segment holds `texts` by `author`, and is closing.
fn closing_with(texts: &[&str], author: Id<User>) -> (ChannelState, Id<Segment>) {
    let mut state = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    for text in texts {
        state.send(&draft(text), author, NOW).expect("send");
    }
    let closing = state.active_id();
    let successor = Id::<Segment>::mint();
    state
        .close_into(closing, successor, crate::Successor::Created, NOW)
        .expect("close");
    let peer = next_peer();
    state
        .adopt_session(
            successor.document_id(),
            peer,
            PeerEntryBytes::new(peer.0.to_le_bytes().to_vec()),
        )
        .expect("adopt");
    (state, closing)
}

/// The same channel, its closing segment locked as the relay does once a
/// member holds its finalize lease.
fn locked_with(texts: &[&str], author: Id<User>) -> (ChannelState, Id<Segment>) {
    let (mut state, segment) = closing_with(texts, author);
    state
        .relocate_locked(segment.document_id(), &[])
        .expect("lock");
    (state, segment)
}

/// Create the amendments of `segment`, adopt their session and link them.
/// Returns them, their first update, and the `meta` delta that names them.
fn amended(
    state: &mut ChannelState,
    segment: Id<Segment>,
) -> (Id<Document>, DeltaBytes, DeltaBytes) {
    let ChangeTarget::Create(amendments) = state.change_target(segment, NOW).expect("target")
    else {
        panic!("a locked segment with no amendments creates them");
    };
    let peer = next_peer();
    let first = state
        .adopt_session(
            amendments,
            peer,
            PeerEntryBytes::new(peer.0.to_le_bytes().to_vec()),
        )
        .expect("adopt")
        .expect("the header travels");
    let linked = state.amendments_linked(segment, amendments).expect("link");
    assert!(linked.open == crate::LinkedOpening::Open && linked.replaced.is_none());
    (amendments, first, linked.meta.expect("meta names them"))
}

/// A second member's copy of `state`, opened from its live segments.
fn peer_of(state: &ChannelState) -> ChannelState {
    let segments: HashMap<Id<Segment>, SnapshotBytes> = state
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

fn signer(state: &ChannelState, document_id: Id<Document>, user: Id<User>) -> Signer {
    Signer {
        peer: state.peer(document_id).expect("a session"),
        user: Some(user),
    }
}

fn messages_of(event: &MurmureEvent) -> &[MessageView] {
    let MurmureEvent::Messages { messages, .. } = event else {
        panic!("messages");
    };
    messages
}

/// Every operation of `doc` past `before`, as a delta travels.
fn json_since(doc: &LoroDoc, before: &VersionVector) -> DeltaBytes {
    doc.commit();
    crate::delta::encode(&doc.export_json_updates_without_peer_compression(before, &doc.oplog_vv()))
        .expect("encode")
}

/// Bob's copy of Alice's amendments, held with a session of his own.
fn bob_holding(
    alice: &ChannelState,
    segment: Id<Segment>,
    amendments: Id<Document>,
    first: &DeltaBytes,
    meta: &DeltaBytes,
    author: Id<User>,
) -> ChannelState {
    let mut bob = peer_of(alice);
    bob.apply_remote(bob.meta_id().document_id(), meta.into())
        .expect("meta");
    assert_eq!(bob.amendments_to_open(), [amendments]);
    bob.apply_signed(amendments, Seq(1), first, signer(alice, amendments, author))
        .expect("the header waits");
    bob.older_ready(amendments).expect("admitted");
    let peer = next_peer();
    bob.adopt_session(
        amendments,
        peer,
        PeerEntryBytes::new(peer.0.to_le_bytes().to_vec()),
    )
    .expect("adopt");
    assert_eq!(bob.amendments_id(segment), Some(amendments));
    bob
}

// --- MURMURE-073 ---

/// A change of a locked segment waits for its amendments: the first one
/// creates them, and every later one writes them.
#[test]
fn a_change_of_a_locked_segment_waits_for_its_amendments() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["one"], author);
    let message = alice.messages(segment).expect("held")[0].id;
    assert!(matches!(
        alice.edit(segment, message, &draft("uno"), author),
        Err(MurmureError::NoAmendments(s)) if s == segment
    ));
    let ChangeTarget::Create(amendments) = alice.change_target(segment, NOW).expect("target")
    else {
        panic!("the first change creates the amendments");
    };
    assert_eq!(
        alice.change_target(segment, NOW),
        Ok(ChangeTarget::Write(amendments))
    );
    assert_eq!(
        alice.change_target(alice.active_id(), NOW),
        Ok(ChangeTarget::Write(alice.active_id().document_id())),
        "a live segment takes its own changes"
    );
    assert!(alice.amendments_failed(segment, amendments));
    assert!(matches!(
        alice.change_target(segment, NOW),
        Ok(ChangeTarget::Create(other)) if other != amendments
    ));
}

/// Two members created amendments for one segment, and the relay linked
/// the other's: this client drops its own and fetches the winner.
#[test]
fn a_lost_race_fetches_the_amendments_the_relay_linked() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["one"], author);
    let ChangeTarget::Create(own) = alice.change_target(segment, NOW).expect("target") else {
        panic!("created");
    };
    let winner = Id::<Document>::mint();
    let linked = alice.amendments_linked(segment, winner).expect("link");
    assert_eq!(
        (linked.open, linked.replaced),
        (crate::LinkedOpening::Open, Some(own))
    );
    assert!(linked.meta.is_some(), "meta names the winner");
    assert_eq!(
        alice.change_target(segment, NOW),
        Ok(ChangeTarget::Write(winner)),
        "a change waits for the winner"
    );
    assert!(!alice.amendments_failed(segment, own), "already dropped");
}

/// An edit and a delete of a locked segment write its amendments, and the
/// segment itself never moves. The view reads each one over its message.
#[test]
fn an_edit_and_a_delete_of_a_locked_segment_write_amendments() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["one", "two"], author);
    let (amendments, _, _) = amended(&mut alice, segment);
    let ids: Vec<Id<Message>> = alice
        .messages(segment)
        .expect("held")
        .iter()
        .map(|m| m.id)
        .collect();

    let edited = alice
        .edit(segment, ids[0], &draft("uno"), author)
        .expect("edit")
        .expect("a change");
    assert_eq!((edited.segment, edited.document), (segment, amendments));
    let read = messages_of(&edited.event);
    assert_eq!(
        (read[0].text.as_str(), read[0].edited),
        ("uno", ambre_bus::Revision::Edited)
    );
    assert!(
        matches!(alice.edit(segment, ids[0], &draft("uno"), author), Ok(None)),
        "the body reads so already"
    );

    let deleted = alice.delete(segment, ids[1], author).expect("delete");
    assert_eq!(deleted.document, amendments);
    let read = alice.messages(segment).expect("held");
    assert_eq!(read[1].deleted, ambre_bus::Deletion::Deleted);
    assert!(read[1].text.is_empty());
    assert!(matches!(
        alice.edit(segment, ids[1], &draft("back"), author),
        Err(MurmureError::Deleted(id)) if id == ids[1]
    ));

    // `MURMURE-088`: the delete names its author.
    let doc = alice.doc(amendments).expect("held");
    let Some(entry) = crate::schema::root_map(doc, Root::Amendments)
        .get(&ids[1].to_string())
        .and_then(crate::schema::fields_of)
    else {
        panic!("an entry per message");
    };
    assert_eq!(
        crate::schema::string(&entry, AmendmentKey::By).as_deref(),
        Some("author")
    );
    let raw = read_messages(
        alice.doc(segment.document_id()).expect("held"),
        alice.channel_id(),
    );
    assert_eq!(
        raw.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        ["one", "two"],
        "the segment holds what it held when it locked"
    );
}

/// A second member fetches the amendments `meta` names, and reads them over
/// the segment: those that arrived first, then each later one.
#[test]
fn a_member_reads_the_amendments_over_the_segment() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["one", "two"], author);
    let (amendments, first, meta) = amended(&mut alice, segment);
    let ids: Vec<Id<Message>> = alice
        .messages(segment)
        .expect("held")
        .iter()
        .map(|m| m.id)
        .collect();
    // `MURMURE-562`: the amendment carries the styles of the edit.
    let bold = StyleMark {
        style: TextStyle::Bold,
        range: CharSpan::from_usize_range(0..2),
    };
    let styled = MessageDraft {
        styles: vec![bold.clone()],
        ..draft("uno")
    };
    let edited = alice
        .edit(segment, ids[0], &styled, author)
        .expect("edit")
        .expect("a change");

    let mut bob = peer_of(&alice);
    bob.apply_remote(bob.meta_id().document_id(), (&meta).into())
        .expect("meta");
    assert_eq!(bob.amendments_to_open(), [amendments]);
    assert_eq!(bob.amendments_to_open(), [], "asked once");
    let by_alice = signer(&alice, amendments, author);
    for (seq, payload) in [(1, &first), (2, &edited.delta)] {
        let events = bob
            .apply_signed(amendments, Seq(seq), payload, by_alice)
            .expect("waits");
        assert!(events.is_empty());
    }
    let ready = bob.older_ready(amendments).expect("admitted");
    assert_eq!(messages_of(&ready)[0].text, "uno");
    assert_eq!(messages_of(&ready)[0].styles, [bold]);

    let deleted = alice.delete(segment, ids[1], author).expect("delete");
    let events = bob
        .apply_signed(amendments, Seq(3), &deleted.delta, by_alice)
        .expect("applies");
    assert_eq!(events.len(), 1);
    assert_eq!(
        messages_of(&events[0])[1].deleted,
        ambre_bus::Deletion::Deleted
    );
    assert_eq!(bob.messages(segment), alice.messages(segment));
}

// --- MURMURE-253 ---

/// The check refuses an amendment of another author's message, whether it
/// creates the entry or writes into the author's own.
#[test]
fn an_amendment_of_another_authors_message_is_refused() {
    let (alice_user, bob_user) = (Id::<User>::mint(), Id::<User>::mint());
    let (mut alice, segment) = locked_with(&["one"], alice_user);
    let (amendments, first, meta) = amended(&mut alice, segment);
    let message = alice.messages(segment).expect("held")[0].id;
    let mut bob = bob_holding(&alice, segment, amendments, &first, &meta, alice_user);
    assert!(
        matches!(
            bob.edit(segment, message, &draft("mine"), bob_user),
            Err(MurmureError::NotAuthor(id)) if id == message
        ),
        "refused before it is written"
    );

    let doc = bob.doc(amendments).expect("held");
    let before = doc.oplog_vv();
    crate::schema::root_map(doc, Root::Amendments)
        .insert_container(&message.to_string(), LoroMap::new())
        .expect("forge");
    let forged = json_since(doc, &before);
    let by_bob = signer(&bob, amendments, bob_user);
    assert_eq!(
        alice.apply_signed(amendments, Seq(2), &forged, by_bob),
        Err(MurmureError::Refused(Refusal::ForeignMessage))
    );

    // Into the author's own entry, once she wrote it.
    let edited = alice
        .edit(segment, message, &draft("uno"), alice_user)
        .expect("edit")
        .expect("a change");
    let mut bob = bob_holding(&alice, segment, amendments, &first, &meta, alice_user);
    bob.apply_signed(
        amendments,
        Seq(2),
        &edited.delta,
        signer(&alice, amendments, alice_user),
    )
    .expect("the author's amendment");
    let doc = bob.doc(amendments).expect("held");
    let before = doc.oplog_vv();
    let Some(entry) = crate::schema::root_map(doc, Root::Amendments)
        .get(&message.to_string())
        .and_then(crate::schema::fields_of)
    else {
        panic!("the author's entry");
    };
    entry
        .insert(crate::schema::AmendmentKey::Deleted, true)
        .expect("forge");
    let forged = json_since(doc, &before);
    assert_eq!(
        alice.apply_signed(
            amendments,
            Seq(3),
            &forged,
            signer(&bob, amendments, bob_user)
        ),
        Err(MurmureError::Refused(Refusal::ForeignMessage))
    );
    assert_eq!(
        alice.messages(segment).expect("held")[0].deleted,
        ambre_bus::Deletion::Live
    );
}

// --- MURMURE-013 ---

/// The check refuses an entry for a message the segment does not hold, and
/// an amendments document whose header names another segment is refused.
#[test]
fn an_amendment_outside_its_segment_is_refused() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["one"], author);
    // Carol opens the channel before `meta` names the amendments.
    let mut carol = peer_of(&alice);
    let (amendments, first, meta) = amended(&mut alice, segment);
    let bob = bob_holding(&alice, segment, amendments, &first, &meta, author);

    let doc = bob.doc(amendments).expect("held");
    let before = doc.oplog_vv();
    crate::schema::root_map(doc, Root::Amendments)
        .insert_container(&Id::<Message>::mint().to_string(), LoroMap::new())
        .expect("forge");
    let forged = json_since(doc, &before);
    assert_eq!(
        alice.apply_signed(
            amendments,
            Seq(2),
            &forged,
            signer(&bob, amendments, author)
        ),
        Err(MurmureError::Refused(Refusal::NotInSegment))
    );

    // Her `meta` names them for the active segment instead.
    let active = carol.active_id();
    let list = carol
        .doc(carol.meta_id().document_id())
        .expect("meta")
        .get_list(Root::Segments.as_str());
    for index in 0..list.len() {
        let Some(entry) = list.get(index).and_then(crate::schema::fields_of) else {
            continue;
        };
        if crate::schema::string(&entry, SegmentKey::Document) == Some(active.to_string()) {
            entry
                .insert(SegmentKey::Amendments, amendments.to_string())
                .expect("forge");
        }
    }
    assert_eq!(carol.amendments_to_open(), [amendments]);
    carol
        .apply_signed(
            amendments,
            Seq(1),
            &first,
            signer(&alice, amendments, author),
        )
        .expect("waits");
    assert_eq!(
        carol.older_ready(amendments),
        Err(MurmureError::ForeignSegment(amendments))
    );
}

// --- MURMURE-484 ---

/// Start the thread of `root` of the locked `segment` once its amendments
/// are held, and link it. Returns it and the delta that records it.
fn locked_thread(
    state: &mut ChannelState,
    segment: Id<Segment>,
    root: Id<Message>,
) -> (Id<Document>, Sent) {
    let ThreadTarget::Create(thread) = state.reply_target(root, NOW).expect("target") else {
        panic!("a root with no thread creates one");
    };
    let peer = next_peer();
    state
        .adopt_session(
            thread,
            peer,
            PeerEntryBytes::new(peer.0.to_le_bytes().to_vec()),
        )
        .expect("adopt");
    let linked = state.thread_linked(segment, root, thread).expect("link");
    (thread, linked.root.expect("the amendments record it"))
}

/// A first reply to a root of a locked segment waits for its amendments,
/// which record the thread. The segment itself takes no delta.
#[test]
fn a_thread_of_a_locked_root_is_recorded_in_its_amendments() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["root"], author);
    let root = alice.messages(segment).expect("held")[0].id;
    assert_eq!(
        alice.reply_target(root, NOW),
        Err(MurmureError::NoAmendments(segment))
    );
    let (amendments, _, _) = amended(&mut alice, segment);
    let (thread, recorded) = locked_thread(&mut alice, segment, root);
    assert_eq!(recorded.document, amendments);
    assert_eq!(recorded.summary.kind, ActivityKind::Edit);
    assert!(
        alice
            .thread_linked(segment, root, thread)
            .expect("again")
            .root
            .is_none(),
        "recorded once"
    );
    alice
        .reply(
            root,
            &draft("late"),
            author,
            NOW,
            ambre_bus::ReplyEcho::ThreadOnly,
        )
        .expect("reply");
    let head = alice.messages(segment).expect("held")[0]
        .thread
        .expect("the root names its thread");
    assert_eq!((head.document, head.replies.get()), (thread, 1));
    let raw = read_messages(
        alice.doc(segment.document_id()).expect("held"),
        alice.channel_id(),
    );
    assert!(raw[0].thread.is_none(), "the segment is left as it was");
    assert_eq!(
        alice.reply_target(root, NOW),
        Ok(ThreadTarget::Write(thread))
    );
}

/// Another member reads the thread the amendments record over the root,
/// and asks for it.
#[test]
fn another_member_finds_the_thread_of_a_locked_root() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["root"], author);
    let root = alice.messages(segment).expect("held")[0].id;
    let (amendments, first, meta) = amended(&mut alice, segment);
    let (thread, recorded) = locked_thread(&mut alice, segment, root);
    let mut bob = bob_holding(&alice, segment, amendments, &first, &meta, author);
    assert_eq!(bob.messages(segment).expect("held")[0].thread, None);
    let replier = Id::<User>::mint();
    bob.apply_signed(
        amendments,
        Seq(2),
        &recorded.delta,
        signer(&alice, amendments, replier),
    )
    .expect("any member records a thread");
    let head = bob.messages(segment).expect("held")[0]
        .thread
        .expect("read over the root");
    assert_eq!((head.document, head.replies.get()), (thread, 0));
    assert_eq!(bob.threads_to_open(), [thread]);
    assert_eq!(bob.reply_target(root, NOW), Ok(ThreadTarget::Write(thread)));
}

/// The check refuses a record for a message outside the segment. It refuses
/// a second thread for a root, a value that names no document, and a removal.
#[test]
fn a_thread_record_outside_the_rule_is_refused() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["root"], author);
    let root = alice.messages(segment).expect("held")[0].id;
    let (amendments, first, meta) = amended(&mut alice, segment);
    let (_, recorded) = locked_thread(&mut alice, segment, root);
    let mut bob = bob_holding(&alice, segment, amendments, &first, &meta, author);
    bob.apply_signed(
        amendments,
        Seq(2),
        &recorded.delta,
        signer(&alice, amendments, author),
    )
    .expect("recorded");
    let forge = |write: &dyn Fn(&LoroMap)| {
        let doc = bob.doc(amendments).expect("held");
        let before = doc.oplog_vv();
        write(&crate::schema::root_map(doc, Root::Threads));
        json_since(doc, &before)
    };
    type Write<'a> = &'a dyn Fn(&LoroMap);
    let cases: [(Write<'_>, Refusal); 4] = [
        (
            &|map| {
                map.insert(
                    &Id::<Message>::mint().to_string(),
                    Id::<Document>::mint().to_string(),
                )
                .expect("forge");
            },
            Refusal::NotInSegment,
        ),
        (
            &|map| {
                map.insert(&root.to_string(), Id::<Document>::mint().to_string())
                    .expect("forge");
            },
            Refusal::ForeignMessage,
        ),
        (
            &|map| {
                map.insert(&root.to_string(), "not a document")
                    .expect("forge");
            },
            Refusal::ForeignMessage,
        ),
        (
            &|map| {
                map.delete(&root.to_string()).expect("forge");
            },
            Refusal::Removed,
        ),
    ];
    let user = Id::<User>::mint();
    for (seq, (write, refusal)) in (3..).zip(cases) {
        let forged = forge(write);
        assert_eq!(
            alice.apply_signed(
                amendments,
                Seq(seq),
                &forged,
                signer(&bob, amendments, user)
            ),
            Err(MurmureError::Refused(refusal))
        );
    }
}

/// A fold carries the recorded thread into the root's empty `thread` field.
#[test]
fn a_fold_carries_the_thread_of_a_locked_root() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["root"], author);
    let root = alice.messages(segment).expect("held")[0].id;
    amended(&mut alice, segment);
    let (thread, _) = locked_thread(&mut alice, segment, root);
    alice.fold_in_place(segment).expect("fold");
    let raw = read_messages(
        alice.doc(segment.document_id()).expect("held"),
        alice.channel_id(),
    );
    assert_eq!(raw[0].thread.map(|head| head.document), Some(thread));
}

/// An edit and a delete the lock refused come back as replays. The segment
/// reads as the relay holds it until they are written again.
#[test]
fn a_change_refused_by_the_lock_is_replayed() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = closing_with(&["one", "two"], author);
    let ids: Vec<Id<Message>> = alice
        .messages(segment)
        .expect("held")
        .iter()
        .map(|m| m.id)
        .collect();
    // `MURMURE-562`: the replay carries the styles of the edit.
    let styled = MessageDraft {
        styles: vec![StyleMark {
            style: TextStyle::Italic,
            range: CharSpan::from_usize_range(1..3),
        }],
        ..draft("uno")
    };
    let edited = alice
        .edit(segment, ids[0], &styled, author)
        .expect("edit")
        .expect("a change");
    assert_eq!(edited.document, segment.document_id(), "not locked yet");
    let deleted = alice.delete(segment, ids[1], author).expect("delete");

    let relocated = alice
        .relocate_locked(segment.document_id(), &[edited.delta, deleted.delta])
        .expect("relocate");
    assert!(relocated.sent.is_none(), "no new message moves");
    assert_eq!(
        relocated.replay,
        [
            Replay {
                segment,
                message: ids[0],
                draft: Some(styled),
            },
            Replay {
                segment,
                message: ids[1],
                draft: None,
            },
        ]
    );
    let read = alice.messages(segment).expect("held");
    assert_eq!(read[0].text, "one");
    assert_eq!(read[1].deleted, ambre_bus::Deletion::Live);
    assert!(matches!(
        alice.change_target(segment, NOW),
        Ok(ChangeTarget::Create(_))
    ));
}
