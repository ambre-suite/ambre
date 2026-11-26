//! The amendments of a locked segment against their contract rows. They hold
//! where a late change writes, what a member reads over the segment, and the
//! step of the authorship check that guards the document.

use ambre_bus::Char;
use ambre_core::index::Span;
use std::collections::HashMap;

use crate::message::{MessageDraft, MessageView, MurmureEvent, StyleMark, TextStyle};
use ambre_bus::{ActivityKind, Authored, Instant};
use ambre_core::id::{Channel, Document, Id, Message, Segment, User};
use ambre_core::{DeltaBytes, Payload, PeerEntryBytes, Relay, Seq, SnapshotBytes};
use loro::{LoroDoc, LoroMap, VersionVector};

use crate::logic::channel::read_messages;
use crate::logic::schema::{AmendmentKey, Root, SegmentKey};
use crate::logic::session::next_peer;
use crate::{
    ChangeTarget, ChannelState, ChannelType, MurmureError, Refusal, Replay, Sent, Signer,
    ThreadTarget,
};
use ambre_core::crdt::CrdtName as _;

const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

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
            Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
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
) -> (Id<Document>, Payload<DeltaBytes>, Payload<DeltaBytes>) {
    let ChangeTarget::Create(amendments) = state.change_target(segment, NOW).expect("target")
    else {
        panic!("a locked segment with no amendments creates them");
    };
    let peer = next_peer();
    let first = state
        .adopt_session(
            amendments,
            peer,
            Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
        )
        .expect("adopt")
        .expect("the header travels");
    let linked = state.amendments_linked(segment, amendments).expect("link");
    assert!(linked.open == crate::LinkedOpening::Open && linked.replaced.is_none());
    (amendments, first, linked.meta.expect("meta names them"))
}

/// A second member's copy of `state`, opened from its live segments.
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

fn signer(state: &ChannelState, document_id: Id<Document>, user: Id<User>) -> Signer {
    Signer {
        nonce: crate::logic::delta::TEST_NONCE,
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
fn json_since(doc: &LoroDoc, before: &VersionVector) -> Payload<DeltaBytes> {
    doc.commit();
    crate::logic::delta::encode(
        &doc.export_json_updates_without_peer_compression(before, &doc.oplog_vv()),
    )
    .expect("encode")
}

/// Bob's copy of Alice's amendments, held with a session of his own.
fn bob_holding(
    alice: &ChannelState,
    segment: Id<Segment>,
    amendments: Id<Document>,
    first: &Payload<DeltaBytes>,
    meta: &Payload<DeltaBytes>,
    author: Id<User>,
) -> ChannelState {
    let mut bob = peer_of(alice);
    bob.apply_remote(bob.meta_id().document_id(), meta.into())
        .expect("meta");
    assert_eq!(bob.amendments_to_open(), [amendments]);
    bob.apply_signed(
        amendments,
        Seq::<Relay>::new(1),
        first,
        signer(alice, amendments, author),
    )
    .expect("the header waits");
    bob.older_ready(amendments).expect("admitted");
    let peer = next_peer();
    bob.adopt_session(
        amendments,
        peer,
        Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
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
        ("uno", crate::message::Revision::Edited)
    );
    assert!(
        matches!(alice.edit(segment, ids[0], &draft("uno"), author), Ok(None)),
        "the body reads so already"
    );

    let deleted = alice.delete(segment, ids[1], author).expect("delete");
    assert_eq!(deleted.document, amendments);
    let read = alice.messages(segment).expect("held");
    assert_eq!(read[1].deleted, crate::message::Deletion::Deleted);
    assert!(read[1].text.is_empty());
    assert!(matches!(
        alice.edit(segment, ids[1], &draft("back"), author),
        Err(MurmureError::Deleted(id)) if id == ids[1]
    ));

    // `MURMURE-088`: the delete names its author.
    let doc = alice.doc(amendments).expect("held");
    let Some(entry) = crate::logic::schema::root_map(doc, Root::Amendments)
        .get(&ids[1].to_string())
        .and_then(crate::logic::schema::fields_of)
    else {
        panic!("an entry per message");
    };
    assert_eq!(
        crate::logic::schema::string(&entry, AmendmentKey::By).as_deref(),
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
        range: Span::<Char>::from_usize_range(0..2),
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
            .apply_signed(amendments, Seq::<Relay>::new(seq), payload, by_alice)
            .expect("waits");
        assert!(events.is_empty());
    }
    let ready = bob.older_ready(amendments).expect("admitted");
    assert_eq!(messages_of(&ready)[0].text, "uno");
    assert_eq!(messages_of(&ready)[0].styles, [bold]);

    let deleted = alice.delete(segment, ids[1], author).expect("delete");
    let events = bob
        .apply_signed(amendments, Seq::<Relay>::new(3), &deleted.delta, by_alice)
        .expect("applies");
    assert_eq!(events.len(), 1);
    assert_eq!(
        messages_of(&events[0])[1].deleted,
        crate::message::Deletion::Deleted
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
    crate::logic::schema::root_map(doc, Root::Amendments)
        .insert_container(&message.to_string(), LoroMap::new())
        .expect("forge");
    let forged = json_since(doc, &before);
    let by_bob = signer(&bob, amendments, bob_user);
    assert_eq!(
        alice.apply_signed(amendments, Seq::<Relay>::new(2), &forged, by_bob),
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
        Seq::<Relay>::new(2),
        &edited.delta,
        signer(&alice, amendments, alice_user),
    )
    .expect("the author's amendment");
    let doc = bob.doc(amendments).expect("held");
    let before = doc.oplog_vv();
    let Some(entry) = crate::logic::schema::root_map(doc, Root::Amendments)
        .get(&message.to_string())
        .and_then(crate::logic::schema::fields_of)
    else {
        panic!("the author's entry");
    };
    entry
        .insert(crate::logic::schema::AmendmentKey::Deleted, true)
        .expect("forge");
    let forged = json_since(doc, &before);
    assert_eq!(
        alice.apply_signed(
            amendments,
            Seq::<Relay>::new(3),
            &forged,
            signer(&bob, amendments, bob_user)
        ),
        Err(MurmureError::Refused(Refusal::ForeignMessage))
    );
    assert_eq!(
        alice.messages(segment).expect("held")[0].deleted,
        crate::message::Deletion::Live
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
    crate::logic::schema::root_map(doc, Root::Amendments)
        .insert_container(&Id::<Message>::mint().to_string(), LoroMap::new())
        .expect("forge");
    let forged = json_since(doc, &before);
    assert_eq!(
        alice.apply_signed(
            amendments,
            Seq::<Relay>::new(2),
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
        let Some(entry) = list.get(index).and_then(crate::logic::schema::fields_of) else {
            continue;
        };
        if crate::logic::schema::string(&entry, SegmentKey::Document) == Some(active.to_string()) {
            entry
                .insert(SegmentKey::Amendments, amendments.to_string())
                .expect("forge");
        }
    }
    assert_eq!(carol.amendments_to_open(), [amendments]);
    carol
        .apply_signed(
            amendments,
            Seq::<Relay>::new(1),
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
    locked_thread_under(state, segment, root, None)
}

/// [`locked_thread`], with the relay's `receipt` of the link.
fn locked_thread_under(
    state: &mut ChannelState,
    segment: Id<Segment>,
    root: Id<Message>,
    receipt: Option<ambre_core::FixedBytes<ambre_core::Signature>>,
) -> (Id<Document>, Sent) {
    let ThreadTarget::Create(thread) = state.reply_target(root, NOW).expect("target") else {
        panic!("a root with no thread creates one");
    };
    let peer = next_peer();
    state
        .adopt_session(
            thread,
            peer,
            Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
        )
        .expect("adopt");
    let linked = state
        .thread_linked(segment, root, thread, receipt)
        .expect("link");
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
            .thread_linked(segment, root, thread, None)
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
            crate::message::ReplyEcho::ThreadOnly,
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
        Seq::<Relay>::new(2),
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
        Seq::<Relay>::new(2),
        &recorded.delta,
        signer(&alice, amendments, author),
    )
    .expect("recorded");
    let forge = |write: &dyn Fn(&LoroMap)| {
        let doc = bob.doc(amendments).expect("held");
        let before = doc.oplog_vv();
        write(&crate::logic::schema::root_map(doc, Root::Threads));
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
                Seq::<Relay>::new(seq),
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

/// `MURMURE-732`, `MURMURE-733`, `MURMURE-734`, `MURMURE-735`: the record
/// of a late first reply carries the relay's receipt in the same delta.
/// Another member admits it and names it with its thread. A receipt written
/// once the thread is recorded, or that is no signature, is refused. A fold
/// carries the receipt into the root beside the thread.
#[test]
fn a_locked_record_carries_its_receipt_and_a_fold_keeps_it() {
    use crate::logic::channel::{named_receipts, recorded_receipts};
    type Receipt = ambre_core::FixedBytes<ambre_core::Signature>;
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["root"], author);
    let root = alice.messages(segment).expect("held")[0].id;
    let (amendments, first, meta) = amended(&mut alice, segment);
    let receipt = Receipt::from_array([0x5c; 64]);
    let (thread, recorded) = locked_thread_under(&mut alice, segment, root, Some(receipt));
    let mut bob = bob_holding(&alice, segment, amendments, &first, &meta, author);
    bob.apply_signed(
        amendments,
        Seq::<Relay>::new(2),
        &recorded.delta,
        signer(&alice, amendments, Id::<User>::mint()),
    )
    .expect("the receipt rides with the record");
    let held = ambre_bus::LinkReceipt {
        thread,
        signature: receipt,
    };
    assert_eq!(
        recorded_receipts(bob.doc(amendments).expect("held")),
        [held]
    );

    let forge = |value: &str| {
        let doc = bob.doc(amendments).expect("held");
        let before = doc.oplog_vv();
        crate::logic::schema::root_map(doc, Root::Receipts)
            .insert(&root.to_string(), value)
            .expect("forge");
        json_since(doc, &before)
    };
    let user = Id::<User>::mint();
    for (seq, value) in (3..).zip([Receipt::from_array([1; 64]).to_hex(), "zz".to_owned()]) {
        assert_eq!(
            alice.apply_signed(
                amendments,
                Seq::<Relay>::new(seq),
                &forge(&value),
                signer(&bob, amendments, user)
            ),
            Err(MurmureError::Refused(Refusal::ForeignMessage)),
            "{value}"
        );
    }

    alice.fold_in_place(segment).expect("fold");
    assert_eq!(
        named_receipts(alice.doc(segment.document_id()).expect("held")),
        [held]
    );
}

/// `MURMURE-739`, `MURMURE-740`: a member adds a fresh receipt in the
/// amendments, for a thread a locked root names, under their own key alone.
/// A fold carries it into the segment beside the first one.
#[test]
fn a_fresh_receipt_of_a_locked_root_rides_in_the_amendments_and_a_fold_keeps_it() {
    use crate::logic::channel::{named_receipts, recorded_receipts};
    type Receipt = ambre_core::FixedBytes<ambre_core::Signature>;
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["root"], author);
    let root = alice.messages(segment).expect("held")[0].id;
    let (amendments, first, meta) = amended(&mut alice, segment);
    let receipt = Receipt::from_array([0x5c; 64]);
    let (thread, recorded) = locked_thread_under(&mut alice, segment, root, Some(receipt));
    let mut bob = bob_holding(&alice, segment, amendments, &first, &meta, author);
    bob.apply_signed(
        amendments,
        Seq::<Relay>::new(2),
        &recorded.delta,
        signer(&alice, amendments, Id::<User>::mint()),
    )
    .expect("recorded");
    let bob_user = Id::<User>::mint();
    let fresh = Receipt::from_array([0x6d; 64]);
    let written = bob
        .write_fresh_receipt((segment, root), thread, fresh, bob_user)
        .expect("write")
        .expect("written");
    assert_eq!(
        written.document, amendments,
        "a locked segment takes no delta"
    );
    alice
        .apply_signed(
            amendments,
            Seq::<Relay>::new(3),
            &written.delta,
            signer(&bob, amendments, bob_user),
        )
        .expect("a fresh receipt of a recorded thread");
    let first_held = ambre_bus::LinkReceipt {
        thread,
        signature: receipt,
    };
    let fresh_held = ambre_bus::LinkReceipt {
        thread,
        signature: fresh,
    };
    assert_eq!(
        recorded_receipts(alice.doc(amendments).expect("held")),
        [first_held, fresh_held]
    );

    let doc = bob.doc(amendments).expect("held");
    let before = doc.oplog_vv();
    crate::logic::schema::root_map(doc, Root::ThreadReceipts)
        .insert(
            &crate::logic::schema::FreshKey {
                thread: Id::<Document>::mint(),
                by: bob_user,
            }
            .spell(),
            fresh.to_hex(),
        )
        .expect("forge");
    assert_eq!(
        alice.apply_signed(
            amendments,
            Seq::<Relay>::new(4),
            &json_since(doc, &before),
            signer(&bob, amendments, bob_user)
        ),
        Err(MurmureError::Refused(Refusal::ForeignReceipt)),
        "a thread the amendments and the segment name nowhere"
    );

    alice.fold_in_place(segment).expect("fold");
    assert_eq!(
        named_receipts(alice.doc(segment.document_id()).expect("held")),
        [first_held, fresh_held],
        "the fold keeps the fresh receipt"
    );
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
            range: Span::<Char>::from_usize_range(1..3),
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
    assert_eq!(read[1].deleted, crate::message::Deletion::Live);
    assert!(matches!(
        alice.change_target(segment, NOW),
        Ok(ChangeTarget::Create(_))
    ));
}

/// A failed link drops only amendments this client created and has not yet
/// identified: amendments already in use stay (`MURMURE-073`).
#[test]
fn a_failed_link_keeps_amendments_already_in_use() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["one"], author);
    let (amendments, _, _) = amended(&mut alice, segment);
    assert!(!alice.amendments_failed(segment, amendments));
    assert_eq!(alice.amendments_id(segment), Some(amendments));
}

// --- AUTHOR-026 ---

/// The identity the Sync worker issues for amendments still arriving is
/// kept, and the amendments take it once admitted. Until then this member
/// may not write them.
#[test]
fn an_identity_issued_before_the_amendments_arrive_is_adopted_on_admission() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["one"], author);
    let (amendments, first, meta) = amended(&mut alice, segment);
    let mut bob = peer_of(&alice);
    bob.apply_remote(bob.meta_id().document_id(), (&meta).into())
        .expect("meta");
    assert_eq!(bob.amendments_to_open(), [amendments]);
    let peer = next_peer();
    assert_eq!(
        bob.adopt_session(
            amendments,
            peer,
            Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
        ),
        Ok(None),
        "kept for the amendments still arriving"
    );
    assert!(!bob.identified(amendments), "not held yet");
    bob.apply_signed(
        amendments,
        Seq::<Relay>::new(1),
        &first,
        signer(&alice, amendments, author),
    )
    .expect("the header waits");
    bob.older_ready(amendments).expect("admitted");
    assert!(bob.identified(amendments));
    assert_eq!(bob.peer(amendments), Some(peer));
}

// --- MURMURE-073 ---

/// `meta` names amendments for a final segment this copy has not loaded:
/// they wait for the segment, as nothing here would read them yet.
#[test]
fn amendments_of_a_segment_not_held_are_not_asked_for() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = closing_with(&["one"], author);
    alice.mark_final(segment).expect("mark").expect("a delta");
    let amendments = Id::<Document>::mint();
    alice
        .amendments_linked(segment, amendments)
        .expect("link")
        .meta
        .expect("meta names them");
    let active = alice.active_id();
    let segments: HashMap<Id<Segment>, Payload<SnapshotBytes>> = HashMap::from([(
        active,
        alice.snapshot(active.document_id()).expect("active"),
    )]);
    let meta = alice.snapshot(alice.meta_id().document_id()).expect("meta");
    let mut bob = ChannelState::open(alice.channel_id(), alice.meta_id(), &meta, &segments)
        .expect("peer")
        .identify();
    assert!(
        bob.messages(segment).is_err(),
        "the final segment is not loaded"
    );
    assert_eq!(bob.amendments_to_open(), []);
}

/// This client holds its own amendments for a segment while `meta` names
/// another member's: the race is the relay's to settle, so nothing is asked.
#[test]
fn held_amendments_ask_for_no_other_named_document() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["one"], author);
    let mut bob = peer_of(&alice);
    let ChangeTarget::Create(own) = alice.change_target(segment, NOW).expect("target") else {
        panic!("created");
    };
    let theirs = Id::<Document>::mint();
    let meta = bob
        .amendments_linked(segment, theirs)
        .expect("link")
        .meta
        .expect("meta names them");
    alice
        .apply_remote(alice.meta_id().document_id(), (&meta).into())
        .expect("meta");
    assert_eq!(alice.amendments_id(segment), Some(own));
    assert_eq!(alice.amendments_to_open(), []);
}

/// One amendments document that two segment entries name is asked for once,
/// for the first entry in list order.
#[test]
fn amendments_two_entries_name_are_asked_for_once() {
    let author = Id::<User>::mint();
    let (mut alice, closing) = closing_with(&["one"], author);
    let active = alice.active_id();
    let amendments = Id::<Document>::mint();
    for segment in [closing, active] {
        alice
            .amendments_linked(segment, amendments)
            .expect("link")
            .meta
            .expect("meta names them");
    }
    let mut bob = peer_of(&alice);
    assert_eq!(bob.amendments_to_open(), [amendments]);
}

/// The container that holds the amendment of `message`, `None` before one.
fn amendment_entry(
    state: &ChannelState,
    amendments: Id<Document>,
    message: Id<Message>,
) -> Option<loro::ContainerID> {
    match crate::logic::schema::root_map(state.doc(amendments)?, Root::Amendments)
        .get(&message.to_string())?
    {
        loro::ValueOrContainer::Container(loro::Container::Map(entry)) => {
            Some(loro::ContainerTrait::id(&entry))
        }
        _ => None,
    }
}

/// Every later amendment of a message writes the entry its first one
/// created, the container the authorship check owns. A delete empties the
/// edited body, so the deleted words leave the state of the document.
#[test]
fn later_amendments_of_a_message_write_its_first_entry() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["one"], author);
    let (amendments, _, _) = amended(&mut alice, segment);
    let message = alice.messages(segment).expect("held")[0].id;
    alice
        .edit(segment, message, &draft("secret"), author)
        .expect("edit")
        .expect("a change");
    let first = amendment_entry(&alice, amendments, message).expect("an entry");
    alice
        .edit(segment, message, &draft("hidden"), author)
        .expect("edit")
        .expect("a change");
    assert_eq!(
        amendment_entry(&alice, amendments, message),
        Some(first.clone())
    );
    alice.delete(segment, message, author).expect("delete");
    assert_eq!(amendment_entry(&alice, amendments, message), Some(first));
    let state = format!(
        "{:?}",
        alice.doc(amendments).expect("held").get_deep_value()
    );
    assert!(
        !state.contains("hidden"),
        "the deleted body is gone: {state}"
    );
}

/// An amendment that takes a mention out and keeps its text leaves no mark
/// on that text.
#[test]
fn an_amendment_that_drops_a_mention_leaves_its_text_unmarked() {
    let (author, carol) = (Id::<User>::mint(), Id::<User>::mint());
    let (mut alice, segment) = locked_with(&["lunch?"], author);
    amended(&mut alice, segment);
    let message = alice.messages(segment).expect("held")[0].id;
    let mentioned = MessageDraft {
        mentions: vec![crate::message::Mention {
            target: crate::message::MentionTarget::User(carol),
            range: Span::<Char>::from_usize_range(0..6),
        }],
        ..draft("@carol lunch?")
    };
    alice
        .edit(segment, message, &mentioned, author)
        .expect("edit")
        .expect("a change");
    assert_eq!(alice.messages(segment).expect("held")[0].mentions.len(), 1);
    alice
        .edit(segment, message, &draft("@carol lunch?"), author)
        .expect("edit")
        .expect("a change");
    let shown = &alice.messages(segment).expect("held")[0];
    assert_eq!(shown.text, "@carol lunch?");
    assert!(
        shown.mentions.is_empty(),
        "the mark went with the amendment"
    );
}

/// A copy that has not seen the lock still finds the amendments `meta`
/// names. A change of the segment asks for them, then writes them.
#[test]
fn a_change_asks_for_the_amendments_meta_names() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["one"], author);
    let (amendments, _, _) = amended(&mut alice, segment);
    let mut bob = peer_of(&alice);
    assert_eq!(
        bob.change_target(segment, NOW),
        Ok(ChangeTarget::Open(amendments))
    );
    assert_eq!(
        bob.change_target(segment, NOW),
        Ok(ChangeTarget::Write(amendments))
    );
}

/// Each segment's amendments are asked for, in the order `meta` lists the
/// segments.
#[test]
fn the_amendments_of_each_segment_are_asked_for_in_list_order() {
    let author = Id::<User>::mint();
    let (mut alice, closing) = closing_with(&["one"], author);
    let active = alice.active_id();
    let named = [Id::<Document>::mint(), Id::<Document>::mint()];
    for (segment, amendments) in [(closing, named[0]), (active, named[1])] {
        alice
            .amendments_linked(segment, amendments)
            .expect("link")
            .meta
            .expect("meta names them");
    }
    let mut bob = peer_of(&alice);
    assert_eq!(bob.amendments_to_open(), named);
}

// --- MURMURE-679 ---

/// Once the amendments document exists, a write to its header or its version
/// is refused: the header still names its segment, and the document opens.
#[test]
fn a_write_to_the_header_of_amendments_is_refused() {
    let author = Id::<User>::mint();
    let (mut alice, segment) = locked_with(&["one"], author);
    let (amendments, first, meta) = amended(&mut alice, segment);
    let bob = bob_holding(&alice, segment, amendments, &first, &meta, author);
    let was = alice.doc(amendments).expect("held").get_deep_value();

    let doc = bob.doc(amendments).expect("held");
    let before = doc.oplog_vv();
    crate::logic::schema::header(doc)
        .delete(crate::logic::schema::HeaderKey::Segment)
        .expect("forge");
    let unnamed = json_since(doc, &before);
    let before = doc.oplog_vv();
    doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), 99_i64)
        .expect("forge");
    let versioned = json_since(doc, &before);
    let by_bob = signer(&bob, amendments, author);
    for (forged, seq) in [(&unnamed, 2), (&versioned, 3)] {
        assert_eq!(
            alice.apply_signed(amendments, Seq::<Relay>::new(seq), forged, by_bob),
            Err(MurmureError::Refused(Refusal::ForeignRoot))
        );
    }
    assert_eq!(alice.doc(amendments).expect("held").get_deep_value(), was);
}

// --- SYNC-320 ---

/// The sealed `meta` names each segment it lists, the locked one and the
/// active one, and the amendments of the locked one. Once a fold marks those
/// amendments erased, the `meta` names them no more, and still names the
/// segment.
#[test]
fn the_meta_names_each_segment_and_the_amendments_no_fold_erased() {
    let (mut alice, segment) = locked_with(&["one"], Id::<User>::mint());
    let before = alice.named_documents();
    assert!(before.contains(&segment.document_id()));
    assert!(before.contains(&alice.active_id().document_id()));
    let (amendments, _, _) = amended(&mut alice, segment);
    let named = alice.named_documents();
    assert!(named.contains(&amendments), "{named:?}");
    assert_eq!(named.len(), before.len() + 1, "each document once");

    let list = alice
        .doc(alice.meta_id().document_id())
        .expect("meta")
        .get_list(Root::Segments.as_str());
    for index in 0..list.len() {
        let Some(entry) = list.get(index).and_then(crate::logic::schema::fields_of) else {
            continue;
        };
        if crate::logic::schema::string(&entry, SegmentKey::Document) == Some(segment.to_string()) {
            entry
                .insert(SegmentKey::Folded, amendments.to_string())
                .expect("fold");
        }
    }
    let folded = alice.named_documents();
    assert!(!folded.contains(&amendments));
    assert!(folded.contains(&segment.document_id()));
}
