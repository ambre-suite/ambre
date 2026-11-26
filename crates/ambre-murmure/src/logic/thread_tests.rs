//! The threads of a channel against their contract rows. They hold where a
//! reply writes, what a root shows of its thread, and the one field of a
//! root another member may write.

use ambre_bus::Char;
use ambre_core::crdt::CrdtName;
use ambre_core::index::Span;
use std::collections::HashMap;

use crate::message::{Mention, MentionTarget, MessageAt, MessageDraft, MessageView, MurmureEvent};
use ambre_bus::{ActivityKind, Authored, Instant, Mentions};
use ambre_core::id::{Channel, Document, Id, Message, Meta, Segment, User};
use ambre_core::{DeltaBytes, Payload, PeerEntryBytes, Relay, Seq, SnapshotBytes};
use loro::json::{JsonOpContent, MapOp};
use loro::{ExportMode, LoroDoc, VersionVector};

use crate::logic::channel::push_segment_entry;
use crate::logic::schema::{self, DocumentKind, MessageKey, Root, SegmentState};
use crate::logic::session::next_peer;
use crate::{
    ChannelState, ChannelType, MurmureError, Refusal, SeekStep, Sent, Signer, ThreadTarget,
};

const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

fn draft(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    }
}

/// A channel whose active segment holds `texts` by `author`.
fn channel_with(texts: &[&str], author: Id<User>) -> ChannelState {
    let mut state = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    for text in texts {
        state.send(&draft(text), author, NOW).expect("send");
    }
    state
}

/// The message at `index` of the active segment.
fn message_at(state: &ChannelState, index: usize) -> MessageView {
    state.messages(state.active_id()).expect("held")[index].clone()
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

/// Create the thread of `root`, adopt its session and link it, as the Sync
/// worker and the relay answer. Returns it, its first update, and the delta
/// that names it in the root.
fn threaded(
    state: &mut ChannelState,
    root: Id<Message>,
) -> (Id<Document>, Payload<DeltaBytes>, Sent) {
    let ThreadTarget::Create(thread) = state.reply_target(root, NOW).expect("target") else {
        panic!("a root with no thread creates one");
    };
    let peer = next_peer();
    let first = state
        .adopt_session(
            thread,
            peer,
            Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
        )
        .expect("adopt")
        .expect("the header travels");
    let linked = state
        .thread_linked(state.active_id(), root, thread, None)
        .expect("link");
    assert!(linked.open == crate::LinkedOpening::Open && linked.replaced.is_none());
    (thread, first, linked.root.expect("the root names it"))
}

/// The `thread` field the root reads in `state`.
fn thread_field(state: &ChannelState, index: usize) -> Option<Id<Document>> {
    message_at(state, index).thread.map(|head| head.document)
}

/// Every operation of `doc` past `before`, as a delta travels.
fn json_since(doc: &LoroDoc, before: &VersionVector) -> Payload<DeltaBytes> {
    doc.commit();
    crate::logic::delta::encode(
        &doc.export_json_updates_without_peer_compression(before, &doc.oplog_vv()),
    )
    .expect("encode")
}

// --- MURMURE-105 ---

/// A first reply creates one thread document for its root, and every later
/// reply writes it. Another root has its own.
#[test]
fn every_reply_to_a_root_writes_its_one_thread_document() {
    let alice = Id::<User>::mint();
    let mut state = channel_with(&["root", "other"], alice);
    let root = message_at(&state, 0).id;
    assert_eq!(state.root_segment(root), Some(state.active_id()));
    let ThreadTarget::Create(thread) = state.reply_target(root, NOW).expect("target") else {
        panic!("created");
    };
    assert_eq!(
        state.reply_target(root, NOW),
        Ok(ThreadTarget::Write(thread))
    );
    assert!(matches!(
        state.reply(root, &draft("early"), alice, NOW, crate::message::ReplyEcho::ThreadOnly),
        Err(MurmureError::NoSession(id)) if id == thread
    ));
    let other = message_at(&state, 1).id;
    assert!(matches!(
        state.reply(other, &draft("lost"), alice, NOW, crate::message::ReplyEcho::ThreadOnly),
        Err(MurmureError::NoThread(id)) if id == other
    ));
    assert!(matches!(
        state.reply_target(Id::<Message>::mint(), NOW),
        Err(MurmureError::NoMessage(_))
    ));
    let peer = next_peer();
    state
        .adopt_session(
            thread,
            peer,
            Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
        )
        .expect("adopt");
    state
        .thread_linked(state.active_id(), root, thread, None)
        .expect("link");
    assert_eq!(
        state.reply_target(root, NOW),
        Ok(ThreadTarget::Write(thread))
    );
    for text in ["one", "two"] {
        let replied = state
            .reply(
                root,
                &draft(text),
                alice,
                NOW,
                crate::message::ReplyEcho::ThreadOnly,
            )
            .expect("reply");
        assert_eq!(replied.reply.document, thread);
        assert!(replied.copy.is_none());
    }
    let Some(MurmureEvent::ThreadMessages { messages, .. }) = state.thread_view(thread) else {
        panic!("the thread reads its replies");
    };
    let texts: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, ["one", "two"]);
    assert_eq!(state.messages(state.active_id()).expect("held").len(), 2);
    let ThreadTarget::Create(second) = state.reply_target(other, NOW).expect("target") else {
        panic!("another root has its own thread");
    };
    assert_ne!(second, thread);
}

/// A failed link drops this client's own thread. A first reply to a root of
/// a locked segment waits for its amendments: no delta may set its field.
#[test]
fn a_failed_link_drops_the_own_thread_and_a_locked_root_waits() {
    let alice = Id::<User>::mint();
    let mut state = channel_with(&["root", "other"], alice);
    let root = message_at(&state, 0).id;
    let other = message_at(&state, 1).id;
    let ThreadTarget::Create(own) = state.reply_target(other, NOW).expect("target") else {
        panic!("created");
    };
    assert!(
        !state.thread_failed(other, Id::<Document>::mint()),
        "another thread keeps this one"
    );
    assert!(state.thread_failed(other, own));
    assert!(!state.thread_failed(other, own), "already dropped");
    let segment = state.active_id();
    let successor = Id::<Segment>::mint();
    state
        .close_into(segment, successor, crate::Successor::Created, NOW)
        .expect("close");
    let peer = next_peer();
    state
        .adopt_session(
            successor.document_id(),
            peer,
            Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
        )
        .expect("adopt");
    state
        .relocate_locked(segment.document_id(), &[])
        .expect("lock");
    assert_eq!(
        state.reply_target(root, NOW),
        Err(MurmureError::NoAmendments(segment))
    );
}

// --- MURMURE-106 ---

/// The root reads its count and its last replier from its thread document,
/// a deleted reply aside. A peer that does not hold the thread reads none.
#[test]
fn the_root_reads_its_reply_count_and_last_replier_from_its_thread() {
    let (alice, bob) = (Id::<User>::mint(), Id::<User>::mint());
    let mut state = channel_with(&["root"], alice);
    let root = message_at(&state, 0).id;
    let (thread, _, _) = threaded(&mut state, root);
    state
        .reply(
            root,
            &draft("first"),
            alice,
            NOW,
            crate::message::ReplyEcho::ThreadOnly,
        )
        .expect("reply");
    let replied = state
        .reply(
            root,
            &draft("second"),
            bob,
            NOW,
            crate::message::ReplyEcho::ThreadOnly,
        )
        .expect("reply");
    let MurmureEvent::Messages { messages, .. } = &replied.head else {
        panic!("the head moves");
    };
    let head = messages[0].thread.expect("a head");
    assert_eq!(head.document, thread);
    assert_eq!((head.replies.get(), head.last_replier), (2, Some(bob)));
    // A reply deleted in the thread no longer counts.
    let doc = state.doc(thread).expect("held");
    let Some(last) = crate::logic::schema::root_list(doc, Root::Messages)
        .get(1)
        .and_then(crate::logic::schema::fields_of)
    else {
        panic!("a reply");
    };
    last.insert(crate::logic::schema::MessageKey::Deleted, true)
        .expect("delete");
    doc.commit();
    let head = message_at(&state, 0).thread.expect("a head");
    assert_eq!((head.replies.get(), head.last_replier), (1, Some(alice)));
    // A peer that holds the root but not the thread shows no count yet.
    let peer = peer_of(&state);
    let head = message_at(&peer, 0).thread.expect("the root names it");
    assert_eq!(
        (head.document, head.replies.get(), head.last_replier),
        (thread, 0, None)
    );
}

// --- MURMURE-107, MURMURE-110 ---

/// The link sets the absent `thread` field of the root, and the delta
/// writes nothing else of it.
#[test]
fn the_link_sets_the_root_thread_field_and_nothing_else() {
    let alice = Id::<User>::mint();
    let mut state = channel_with(&["root"], alice);
    let before = message_at(&state, 0);
    assert_eq!(before.thread, None);
    let (thread, _, root) = threaded(&mut state, before.id);
    let after = message_at(&state, 0);
    assert_eq!(thread_field(&state, 0), Some(thread));
    assert_eq!(
        MessageView {
            thread: None,
            ..after
        },
        before
    );
    let update = crate::logic::delta::read(&root.delta).expect("a JSON update");
    let ops: Vec<&JsonOpContent> = update
        .changes
        .iter()
        .flat_map(|c| &c.ops)
        .map(|op| &op.content)
        .collect();
    assert_eq!(ops.len(), 1, "one write of the root");
    assert!(matches!(
        ops[0],
        JsonOpContent::Map(MapOp::Insert { key, .. }) if MessageKey::parse(key) == Some(MessageKey::Thread)
    ));
    let linked = state
        .thread_linked(state.active_id(), before.id, thread, None)
        .expect("link again");
    assert!(linked.root.is_none(), "a set field takes no second write");
}

// --- MURMURE-108 ---

/// Two members create a thread for one root at once, and the relay links
/// Alice's. Bob drops his own, and both roots name the one document.
#[test]
fn two_first_replies_converge_on_the_thread_the_relay_linked() {
    let (alice_user, bob_user) = (Id::<User>::mint(), Id::<User>::mint());
    let mut alice = channel_with(&["root"], alice_user);
    let mut bob = peer_of(&alice);
    let root = message_at(&alice, 0).id;
    let segment = alice.active_id();
    let ThreadTarget::Create(own) = bob.reply_target(root, NOW).expect("target") else {
        panic!("Bob creates his own");
    };
    let (winner, _, alice_root) = threaded(&mut alice, root);
    let linked = bob
        .thread_linked(segment, root, winner, None)
        .expect("link");
    assert_eq!(
        (linked.open, linked.replaced),
        (crate::LinkedOpening::Open, Some(own))
    );
    let bob_root = linked.root.expect("Bob names the winner too");
    assert_eq!(bob.reply_target(root, NOW), Ok(ThreadTarget::Write(winner)));
    // Each imports the other's write of the same value.
    let doc = segment.document_id();
    alice
        .apply_signed(
            doc,
            Seq::<Relay>::new(10),
            &bob_root.delta,
            signer(&bob, doc, bob_user),
        )
        .expect("the same value passes");
    bob.apply_signed(
        doc,
        Seq::<Relay>::new(11),
        &alice_root.delta,
        signer(&alice, doc, alice_user),
    )
    .expect("the author's own write passes");
    assert_eq!(thread_field(&alice, 0), Some(winner));
    assert_eq!(thread_field(&bob, 0), Some(winner));
}

// --- MURMURE-109 ---

/// A reply also sent to the channel writes a copy in the active segment,
/// which names the thread it came from.
#[test]
fn a_reply_also_sent_to_the_channel_writes_a_copy_naming_its_thread() {
    let alice = Id::<User>::mint();
    let mut state = channel_with(&["root"], alice);
    let root = message_at(&state, 0).id;
    let (thread, _, _) = threaded(&mut state, root);
    let replied = state
        .reply(
            root,
            &draft("also here"),
            alice,
            NOW,
            crate::message::ReplyEcho::AlsoInChannel,
        )
        .expect("reply");
    let copy = replied.copy.expect("a copy");
    assert_eq!(copy.document, state.active_id().document_id());
    let copied = message_at(&state, 1);
    assert_eq!(
        (copied.text.as_str(), copied.also_in_thread),
        ("also here", Some(thread))
    );
    assert_eq!(message_at(&state, 0).also_in_thread, None);
}

/// Before the PeerID of the active segment lands, a reply to the thread
/// alone leaves. A reply also sent to the channel waits for that PeerID
/// (`MURMURE-109`).
#[test]
fn a_reply_to_its_thread_alone_needs_no_active_session() {
    let alice = Id::<User>::mint();
    let mut state = channel_with(&["root"], alice);
    let root = message_at(&state, 0).id;
    let (thread, _, _) = threaded(&mut state, root);
    let successor = Id::<Segment>::mint();
    state
        .close_into(state.active_id(), successor, crate::Successor::Created, NOW)
        .expect("close");
    assert!(matches!(
        state.reply(root, &draft("both"), alice, NOW, crate::message::ReplyEcho::AlsoInChannel),
        Err(MurmureError::NoSession(id)) if id == successor.document_id()
    ));
    let replied = state
        .reply(
            root,
            &draft("alone"),
            alice,
            NOW,
            crate::message::ReplyEcho::ThreadOnly,
        )
        .expect("the thread has its session");
    assert_eq!(replied.reply.document, thread);
    assert!(replied.copy.is_none());
}

// --- MURMURE-538 ---

/// A copy is found from its reply, under an identifier of its own. A reply
/// sent to its thread alone has none, and another root finds nothing.
#[test]
fn a_copy_is_found_from_its_reply() {
    let alice = Id::<User>::mint();
    let mut state = channel_with(&["root"], alice);
    let root = message_at(&state, 0).id;
    let (thread, _, _) = threaded(&mut state, root);
    state
        .reply(
            root,
            &draft("also here"),
            alice,
            NOW,
            crate::message::ReplyEcho::AlsoInChannel,
        )
        .expect("reply");
    state
        .reply(
            root,
            &draft("here alone"),
            alice,
            NOW,
            crate::message::ReplyEcho::ThreadOnly,
        )
        .expect("reply");
    let Some(MurmureEvent::ThreadMessages { messages, .. }) = state.thread_view(thread) else {
        panic!("the thread is held");
    };
    let (copied, alone) = (messages[0].id, messages[1].id);
    let copy = message_at(&state, 1).id;
    assert_ne!(copy, copied, "the copy has an identifier of its own");
    assert_eq!(state.copy_of(root, copied), Some((state.active_id(), copy)));
    assert_eq!(state.copy_of(root, alone), None, "no copy was sent");
    assert_eq!(
        state.copy_of(Id::<Message>::mint(), copied),
        None,
        "no such thread"
    );
}

// --- MURMURE-255 ---

/// Another member sets the absent `thread` field of Alice's message, and
/// the check accepts it. A second value, another field, or a field set on
/// a reply of a thread are refused.
#[test]
fn the_check_lets_another_member_set_an_absent_thread_field_alone() {
    let (alice_user, bob_user, carol_user) =
        (Id::<User>::mint(), Id::<User>::mint(), Id::<User>::mint());
    let mut alice = channel_with(&["root"], alice_user);
    let mut bob = peer_of(&alice);
    let mut carol = peer_of(&alice);
    let root = message_at(&alice, 0).id;
    let segment = alice.active_id();
    let doc = segment.document_id();
    let thread = Id::<Document>::mint();
    let named = bob
        .thread_linked(segment, root, thread, None)
        .expect("link");
    alice
        .apply_signed(
            doc,
            Seq::<Relay>::new(10),
            &named.root.expect("named").delta,
            signer(&bob, doc, bob_user),
        )
        .expect("an absent field takes Bob's value");
    assert_eq!(thread_field(&alice, 0), Some(thread));
    let other = carol
        .thread_linked(segment, root, Id::<Document>::mint(), None)
        .expect("link");
    assert_eq!(
        alice.apply_signed(
            doc,
            Seq::<Relay>::new(11),
            &other.root.expect("named").delta,
            signer(&carol, doc, carol_user),
        ),
        Err(MurmureError::Refused(Refusal::ForeignMessage)),
        "a set field takes no other value"
    );
    // Bob writes another field of Alice's message.
    let bob_doc = bob.doc(doc).expect("held");
    let before = bob_doc.oplog_vv();
    let Some(map) = crate::logic::schema::root_list(bob_doc, Root::Messages)
        .get(0)
        .and_then(crate::logic::schema::fields_of)
    else {
        panic!("the root");
    };
    map.insert(MessageKey::Edited, true).expect("write");
    assert_eq!(
        alice.apply_signed(
            doc,
            Seq::<Relay>::new(12),
            &json_since(bob_doc, &before),
            signer(&bob, doc, bob_user)
        ),
        Err(MurmureError::Refused(Refusal::ForeignMessage)),
        "no other field of the root"
    );
    assert_eq!(thread_field(&alice, 0), Some(thread));
}

/// `MURMURE-732`, `MURMURE-733`, `MURMURE-734`: the link writes the relay's
/// receipt beside the `thread` field, in the same delta, and a removal names
/// it with its thread. Another member writes no receipt once the field is
/// set, and no receipt that is no signature.
#[test]
fn a_link_writes_its_receipt_with_the_thread_field_alone() {
    type Receipt = ambre_core::FixedBytes<ambre_core::Signature>;
    let (alice_user, bob_user, carol_user) =
        (Id::<User>::mint(), Id::<User>::mint(), Id::<User>::mint());
    let mut alice = channel_with(&["root", "other"], alice_user);
    let mut bob = peer_of(&alice);
    let segment = alice.active_id();
    let doc = segment.document_id();
    let (root, other) = (message_at(&alice, 0).id, message_at(&alice, 1).id);
    let thread = Id::<Document>::mint();
    let receipt = Receipt::from_array([0x5c; 64]);
    let named = bob
        .thread_linked(segment, root, thread, Some(receipt))
        .expect("link")
        .root
        .expect("named");
    alice
        .apply_signed(
            doc,
            Seq::<Relay>::new(10),
            &named.delta,
            signer(&bob, doc, bob_user),
        )
        .expect("the receipt rides with the thread field");
    let crate::SweepStep::Done(swept) = alice.sweep_step() else {
        panic!("the segment is held");
    };
    assert_eq!(swept.threads, [thread]);
    assert_eq!(
        swept.receipts,
        [ambre_bus::LinkReceipt {
            thread,
            signature: receipt
        }]
    );

    // Carol copies Alice, Bob's link included.
    let carol = peer_of(&alice);
    let carol_doc = carol.doc(doc).expect("held");
    let message = |at: u32| {
        crate::logic::schema::root_list(carol_doc, Root::Messages)
            .get(at as usize)
            .and_then(crate::logic::schema::fields_of)
            .expect("a message")
    };
    let late = Receipt::from_array([1; 64]).to_hex();
    let forged: [&dyn Fn(); 2] = [
        &|| {
            message(0)
                .insert(MessageKey::Receipt, late.clone())
                .expect("write");
        },
        &|| {
            let map = message(1);
            map.insert(MessageKey::Thread, Id::<Document>::mint().to_string())
                .expect("write");
            map.insert(MessageKey::Receipt, "zz").expect("write");
        },
    ];
    for (seq, forge) in (11..).zip(forged) {
        let before = carol_doc.oplog_vv();
        forge();
        assert_eq!(
            alice.apply_signed(
                doc,
                Seq::<Relay>::new(seq),
                &json_since(carol_doc, &before),
                signer(&carol, doc, carol_user)
            ),
            Err(MurmureError::Refused(Refusal::ForeignMessage))
        );
    }
    assert_eq!(message_at(&alice, 1).id, other);
    assert_eq!(thread_field(&alice, 1), None);
}

/// `MURMURE-739`: once a root names its thread, each member adds a fresh
/// receipt under their own key, beside the first one. A removal names each
/// of them, so a garbage entry of a hostile member shadows no receipt that
/// counts. The check refuses an entry under another member's key, or of a
/// thread no root names. It refuses a value that is no signature, a key
/// spelled otherwise, and a removal.
#[test]
fn a_member_adds_a_fresh_receipt_under_their_own_key_alone() {
    type Receipt = ambre_core::FixedBytes<ambre_core::Signature>;
    let (alice_user, bob_user, carol_user) =
        (Id::<User>::mint(), Id::<User>::mint(), Id::<User>::mint());
    let mut alice = channel_with(&["root"], alice_user);
    let mut bob = peer_of(&alice);
    let mut carol = peer_of(&alice);
    let segment = alice.active_id();
    let doc = segment.document_id();
    let root = message_at(&alice, 0).id;
    let thread = Id::<Document>::mint();
    let first = Receipt::from_array([0x5c; 64]);
    let named = alice
        .thread_linked(segment, root, thread, Some(first))
        .expect("link")
        .root
        .expect("named");
    for peer in [&mut bob, &mut carol] {
        peer.apply_signed(
            doc,
            Seq::<Relay>::new(10),
            &named.delta,
            signer(&alice, doc, alice_user),
        )
        .expect("named");
    }
    let garbage = Receipt::from_array([1; 64]);
    let carols = carol
        .write_fresh_receipt((segment, root), thread, garbage, carol_user)
        .expect("write")
        .expect("written");
    let fresh = Receipt::from_array([0x6d; 64]);
    let bobs = bob
        .write_fresh_receipt((segment, root), thread, fresh, bob_user)
        .expect("write")
        .expect("written");
    assert!(
        matches!(
            bob.write_fresh_receipt((segment, root), thread, fresh, bob_user),
            Ok(None)
        ),
        "an entry that holds the receipt takes no second write"
    );
    assert!(
        matches!(
            bob.write_fresh_receipt((segment, root), thread, first, bob_user),
            Ok(None)
        ),
        "a receipt the root holds takes no entry"
    );
    assert!(
        matches!(
            bob.write_fresh_receipt((segment, root), Id::<Document>::mint(), fresh, bob_user),
            Ok(None)
        ),
        "a thread the root does not name takes no entry"
    );
    for (seq, (sent, by, user)) in
        (11..).zip([(&carols, &carol, carol_user), (&bobs, &bob, bob_user)])
    {
        alice
            .apply_signed(
                doc,
                Seq::<Relay>::new(seq),
                &sent.delta,
                signer(by, doc, user),
            )
            .expect("each member's own entry");
    }
    let crate::SweepStep::Done(swept) = alice.sweep_step() else {
        panic!("the segment is held");
    };
    let mut held: Vec<Receipt> = swept.receipts.iter().map(|r| r.signature).collect();
    held.sort_unstable_by_key(|r| r.to_hex());
    let mut expected = vec![first, garbage, fresh];
    expected.sort_unstable_by_key(|r| r.to_hex());
    assert_eq!(held, expected, "the first receipt, and one per member");
    assert!(swept.receipts.iter().all(|r| r.thread == thread));

    // Carol copies Alice, every entry included, then forges.
    let carol = peer_of(&alice);
    let carol_doc = carol.doc(doc).expect("held");
    let fresh_root = || crate::logic::schema::root_map(carol_doc, Root::ThreadReceipts);
    let key = |thread, by| schema::FreshKey { thread, by }.spell();
    let hex = garbage.to_hex();
    let forged: [&dyn Fn(); 5] = [
        &|| {
            fresh_root()
                .insert(&key(thread, bob_user), hex.clone())
                .expect("write");
        },
        &|| {
            fresh_root()
                .insert(&key(Id::<Document>::mint(), carol_user), hex.clone())
                .expect("write");
        },
        &|| {
            fresh_root()
                .insert(&key(thread, carol_user), "zz")
                .expect("write");
        },
        &|| {
            fresh_root()
                .insert(&key(thread, carol_user).to_uppercase(), hex.clone())
                .expect("write");
        },
        &|| fresh_root().delete(&key(thread, bob_user)).expect("delete"),
    ];
    for (seq, forge) in (20..).zip(forged) {
        let before = carol_doc.oplog_vv();
        forge();
        assert_eq!(
            alice.apply_signed(
                doc,
                Seq::<Relay>::new(seq),
                &json_since(carol_doc, &before),
                signer(&carol, doc, carol_user)
            ),
            Err(MurmureError::Refused(Refusal::ForeignReceipt))
        );
    }
    let crate::SweepStep::Done(swept) = alice.sweep_step() else {
        panic!("the segment is held");
    };
    assert_eq!(swept.receipts.len(), 3, "no forgery moved an entry");
}

/// A thread's own replies start no thread: another member may not set the
/// `thread` field of a reply.
#[test]
fn a_reply_takes_no_thread_field_from_another_member() {
    let (alice_user, bob_user) = (Id::<User>::mint(), Id::<User>::mint());
    let mut alice = channel_with(&["root"], alice_user);
    let root = message_at(&alice, 0).id;
    let (thread, first, named) = threaded(&mut alice, root);
    let replied = alice
        .reply(
            root,
            &draft("reply"),
            alice_user,
            NOW,
            crate::message::ReplyEcho::ThreadOnly,
        )
        .expect("reply");
    let mut bob = peer_of(&alice);
    assert_eq!(bob.threads_to_open(), [thread]);
    bob.apply_signed(
        thread,
        Seq::<Relay>::new(1),
        &first,
        signer(&alice, thread, alice_user),
    )
    .expect("waits");
    bob.apply_signed(
        thread,
        Seq::<Relay>::new(2),
        &replied.reply.delta,
        signer(&alice, thread, alice_user),
    )
    .expect("waits");
    bob.older_ready(thread).expect("admitted");
    let peer = next_peer();
    bob.adopt_session(
        thread,
        peer,
        Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
    )
    .expect("adopt");
    let bob_doc = bob.doc(thread).expect("held");
    let before = bob_doc.oplog_vv();
    let Some(map) = crate::logic::schema::root_list(bob_doc, Root::Messages)
        .get(0)
        .and_then(crate::logic::schema::fields_of)
    else {
        panic!("the reply");
    };
    map.insert(MessageKey::Thread, Id::<Document>::mint().to_string())
        .expect("write");
    assert_eq!(
        alice.apply_signed(
            thread,
            Seq::<Relay>::new(3),
            &json_since(bob_doc, &before),
            signer(&bob, thread, bob_user)
        ),
        Err(MurmureError::Refused(Refusal::ForeignMessage))
    );
    drop(named);
}

// --- MURMURE-485 ---

/// A second member's copy of `alice`, holding `thread` from its updates,
/// with a session of its own on it.
fn peer_with_thread(
    alice: &ChannelState,
    thread: Id<Document>,
    updates: &[&Payload<DeltaBytes>],
    author: Id<User>,
) -> ChannelState {
    let mut bob = peer_of(alice);
    assert_eq!(bob.threads_to_open(), [thread]);
    for (seq, update) in (1..).zip(updates) {
        bob.apply_signed(
            thread,
            Seq::<Relay>::new(seq),
            update,
            signer(alice, thread, author),
        )
        .expect("waits");
    }
    bob.older_ready(thread).expect("admitted");
    let peer = next_peer();
    bob.adopt_session(
        thread,
        peer,
        Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
    )
    .expect("adopt");
    bob
}

/// The replies the thread view of `state` reads.
fn replies_of(state: &ChannelState, thread: Id<Document>) -> Vec<MessageView> {
    let Some(MurmureEvent::ThreadMessages { messages, .. }) = state.thread_view(thread) else {
        panic!("the thread reads its replies");
    };
    messages
}

/// The author edits, reacts to and deletes a reply: each change writes in
/// the thread, and the view reads the replies. A deleted reply leaves the
/// count of its root.
#[test]
fn an_edit_a_reaction_and_a_delete_on_a_reply_write_in_its_thread() {
    let alice = Id::<User>::mint();
    let mut state = channel_with(&["root"], alice);
    let root = message_at(&state, 0).id;
    let (thread, _, _) = threaded(&mut state, root);
    state
        .reply(
            root,
            &draft("hi"),
            alice,
            NOW,
            crate::message::ReplyEcho::ThreadOnly,
        )
        .expect("reply");
    let reply = replies_of(&state, thread)[0].id;
    let at = MessageAt::Thread(root);
    let in_thread = |sent: &Sent| {
        sent.document == thread
            && matches!(&sent.event, MurmureEvent::ThreadMessages { thread: t, .. } if *t == thread)
    };
    let edited = state
        .edit(at, reply, &draft("hello"), alice)
        .expect("edit")
        .expect("a change");
    assert!(in_thread(&edited));
    assert_eq!(edited.summary.kind, ActivityKind::Edit);
    let read = &replies_of(&state, thread)[0];
    assert_eq!(
        (read.text.as_str(), read.edited),
        ("hello", crate::message::Revision::Edited)
    );
    let reacted = state
        .react(at, reply, &ambre_bus::Emoji::from("\u{1F44D}"), alice, NOW)
        .expect("react")
        .expect("a change");
    assert!(in_thread(&reacted));
    assert_eq!(reacted.summary.kind, ActivityKind::Reaction);
    assert_eq!(replies_of(&state, thread)[0].reactions.len(), 1);
    let removed = state
        .unreact(at, reply, &ambre_bus::Emoji::from("\u{1F44D}"), alice)
        .expect("unreact")
        .expect("a change");
    assert!(in_thread(&removed));
    assert!(replies_of(&state, thread)[0].reactions.is_empty());
    let deleted = state.delete(at, reply, alice).expect("delete");
    assert!(in_thread(&deleted));
    assert_eq!(
        replies_of(&state, thread)[0].deleted,
        crate::message::Deletion::Deleted
    );
    let Some(MurmureEvent::Messages { messages, .. }) = state.root_view(root) else {
        panic!("the root's segment");
    };
    assert_eq!(messages[0].thread.map(|head| head.replies.get()), Some(0));
    assert_eq!(state.root_view(Id::<Message>::mint()), None);
    // The root itself still changes in its segment.
    assert!(
        state
            .react(
                state.active_id(),
                root,
                &ambre_bus::Emoji::from("\u{1F44D}"),
                alice,
                NOW
            )
            .expect("react")
            .is_some_and(|sent| sent.document == state.active_id().document_id())
    );
    assert!(matches!(
        state.react(
            MessageAt::Thread(Id::<Message>::mint()),
            reply,
            &ambre_bus::Emoji::from("\u{1F44D}"),
            alice,
            NOW
        ),
        Err(MurmureError::NoThread(_))
    ));
}

/// Bob reacts to Alice's reply, and her check accepts it. Bob may not edit
/// or delete her reply.
#[test]
fn another_member_reacts_to_a_reply_and_never_edits_it() {
    let (alice_user, bob_user) = (Id::<User>::mint(), Id::<User>::mint());
    let mut alice = channel_with(&["root"], alice_user);
    let root = message_at(&alice, 0).id;
    let (thread, first, _) = threaded(&mut alice, root);
    let replied = alice
        .reply(
            root,
            &draft("hi"),
            alice_user,
            NOW,
            crate::message::ReplyEcho::ThreadOnly,
        )
        .expect("reply");
    let mut bob = peer_with_thread(&alice, thread, &[&first, &replied.reply.delta], alice_user);
    let reply = replies_of(&bob, thread)[0].id;
    let at = MessageAt::Thread(root);
    let reacted = bob
        .react(
            at,
            reply,
            &ambre_bus::Emoji::from("\u{1F389}"),
            bob_user,
            NOW,
        )
        .expect("react")
        .expect("a change");
    alice
        .apply_signed(
            thread,
            Seq::<Relay>::new(3),
            &reacted.delta,
            signer(&bob, thread, bob_user),
        )
        .expect("a reaction of another member passes");
    assert_eq!(replies_of(&alice, thread)[0].reactions.len(), 1);
    assert!(matches!(
        bob.edit(at, reply, &draft("mine"), bob_user),
        Err(MurmureError::NotAuthor(id)) if id == reply
    ));
    assert!(matches!(
        bob.delete(at, reply, bob_user),
        Err(MurmureError::NotAuthor(id)) if id == reply
    ));
}

// --- MURMURE-009 ---

/// A member opens the thread a root names: its header names the channel
/// and that root. A remote reply then moves the head, and a header that
/// names another root is refused.
#[test]
fn a_thread_opens_once_its_header_names_the_root_that_names_it() {
    let alice_user = Id::<User>::mint();
    let mut alice = channel_with(&["root", "other", "third"], alice_user);
    let root = message_at(&alice, 0).id;
    let other = message_at(&alice, 1).id;
    let third = message_at(&alice, 2).id;
    let (thread, first, _) = threaded(&mut alice, root);
    let mut bob = peer_of(&alice);
    assert_eq!(bob.threads_to_open(), [thread]);
    assert!(bob.threads_to_open().is_empty(), "asked once");
    assert!(bob.holds(thread));
    bob.apply_signed(
        thread,
        Seq::<Relay>::new(1),
        &first,
        signer(&alice, thread, alice_user),
    )
    .expect("waits");
    let MurmureEvent::Messages { messages, .. } = bob.older_ready(thread).expect("admitted") else {
        panic!("the root's segment");
    };
    assert_eq!(messages[0].thread.map(|head| head.replies.get()), Some(0));
    let replied = alice
        .reply(
            root,
            &draft("hi"),
            alice_user,
            NOW,
            crate::message::ReplyEcho::ThreadOnly,
        )
        .expect("reply");
    let events = bob
        .apply_signed(
            thread,
            Seq::<Relay>::new(2),
            &replied.reply.delta,
            signer(&alice, thread, alice_user),
        )
        .expect("a remote reply");
    assert!(matches!(
        &events[0],
        MurmureEvent::ThreadMessages { root: r, thread: t, messages } if *r == root && *t == thread && messages.len() == 1
    ));
    assert!(matches!(
        &events[1],
        MurmureEvent::Messages { messages, .. } if messages[0].thread.map(|h| h.replies.get()) == Some(1)
    ));
    // Alice's thread of `other` is linked, by mistake, to `third` in another
    // copy: its header names `other`, so it is refused.
    let mut carol = peer_of(&alice);
    let (foreign, foreign_first, _) = threaded(&mut alice, other);
    let segment = alice.active_id();
    let misnamed = carol
        .thread_linked(segment, third, foreign, None)
        .expect("link");
    assert_eq!(misnamed.open, crate::LinkedOpening::Open);
    carol
        .apply_signed(
            foreign,
            Seq::<Relay>::new(1),
            &foreign_first,
            signer(&alice, foreign, alice_user),
        )
        .expect("waits");
    assert_eq!(
        carol.older_ready(foreign),
        Err(MurmureError::ForeignRoot(foreign))
    );
    assert!(!carol.holds(foreign), "a refused thread is dropped");
    assert_eq!(
        carol.reply_target(third, NOW),
        Ok(ThreadTarget::Open(foreign))
    );
    assert!(carol.cancel_older(foreign));
}

// --- MURMURE-261 ---

/// A reply by another member that mentions me counts past my read cursor
/// on its thread, and only on a thread the channel holds.
#[test]
fn a_mention_in_a_held_thread_counts_past_its_read_cursor() {
    let (alice, bob) = (Id::<User>::mint(), Id::<User>::mint());
    let mut state = channel_with(&["root"], alice);
    let root = message_at(&state, 0).id;
    let (thread, _, _) = threaded(&mut state, root);
    let mentioning = MessageDraft {
        text: "@alice".to_owned(),
        styles: Vec::new(),
        mentions: vec![Mention {
            target: MentionTarget::User(alice),
            range: Span::<Char>::from_usize_range(0..6),
        }],
    };
    let replied = state
        .reply(
            root,
            &mentioning,
            bob,
            NOW,
            crate::message::ReplyEcho::ThreadOnly,
        )
        .expect("reply");
    state
        .reply(
            root,
            &draft("plain"),
            bob,
            NOW,
            crate::message::ReplyEcho::ThreadOnly,
        )
        .expect("reply");
    // The relay numbers the mention with its echo.
    state
        .apply_signed(
            thread,
            Seq::<Relay>::new(4),
            &replied.reply.delta,
            signer(&state, thread, bob),
        )
        .expect("echo");
    assert_eq!(
        state.thread_mentions(thread, Seq::<Relay>::new(3), alice),
        Ok(ambre_core::Count::<Mentions>::new(1))
    );
    assert_eq!(
        state.thread_mentions(thread, Seq::<Relay>::new(4), alice),
        Ok(ambre_core::Count::<Mentions>::new(0))
    );
    assert_eq!(
        state.thread_mentions(thread, Seq::<Relay>::new(3), bob),
        Ok(ambre_core::Count::<Mentions>::new(0))
    );
    assert_eq!(state.held_threads(), [thread]);
    let segment = state.active_id().document_id();
    assert_eq!(
        state.thread_mentions(segment, Seq::<Relay>::ZERO, alice),
        Err(MurmureError::NotHeld(segment))
    );
}

// --- MURMURE-545, MURMURE-546 and MURMURE-547 ---

/// The bytes of an empty segment of `channel`.
fn empty_segment(channel: Id<Channel>) -> Payload<SnapshotBytes> {
    let doc = LoroDoc::new();
    schema::write_header(&doc, DocumentKind::Segment, channel, NOW).expect("header");
    doc.commit();
    Payload::<SnapshotBytes>::new(doc.export(ExportMode::Snapshot).expect("snapshot"))
}

/// Bob's copy of the channel of `alice`, opened from a new active segment
/// alone. `meta` lists `oldest` first when given, then an empty final
/// segment, which is returned.
fn far_copy(alice: &ChannelState, oldest: Option<Id<Segment>>) -> (ChannelState, Id<Segment>) {
    let channel = alice.channel_id();
    let (middle, active) = (Id::<Segment>::mint(), Id::<Segment>::mint());
    let meta = LoroDoc::new();
    schema::write_header(&meta, DocumentKind::Meta, channel, NOW).expect("header");
    let entries = [
        oldest.map(|id| (id, 10, SegmentState::Final)),
        Some((middle, 20, SegmentState::Final)),
        Some((active, 30, SegmentState::Active)),
    ];
    for (id, from, state) in entries.into_iter().flatten() {
        push_segment_entry(&meta, id, Instant::<Authored>::new(from), state).expect("entry");
    }
    meta.commit();
    let meta = Payload::<SnapshotBytes>::new(meta.export(ExportMode::Snapshot).expect("snapshot"));
    let segments = HashMap::from([(active, empty_segment(channel))]);
    let bob = ChannelState::open(channel, Id::<Meta>::mint(), &meta, &segments)
        .expect("open")
        .identify();
    (bob, middle)
}

/// Load the older segment `segment` from `bytes`, as the Sync worker
/// delivers it.
fn load(state: &mut ChannelState, segment: Id<Segment>, bytes: &Payload<SnapshotBytes>) {
    let _ = state
        .apply_remote(segment.document_id(), bytes.into())
        .expect("kept");
    state.older_ready(segment.document_id()).expect("loaded");
}

/// A copy whose root lies two segments back. A probe reads the thread's
/// header. The segments then load one at a time, never past the one it
/// names, until the root is found. A held root is found at once.
#[test]
fn the_reply_on_a_far_copy_loads_down_to_its_root() {
    let alice_user = Id::<User>::mint();
    let mut alice = channel_with(&["root"], alice_user);
    let root = message_at(&alice, 0).id;
    let (thread, first, _) = threaded(&mut alice, root);
    let old = alice.active_id();
    assert_eq!(alice.seek_root(thread), None, "the root is held");
    assert!(!alice.is_probe(thread), "a held thread is no probe");
    let found = SeekStep::Found {
        thread,
        segment: old,
        root,
    };
    assert_eq!(alice.seek_step(), Some(found));

    let (mut bob, middle) = far_copy(&alice, Some(old));
    assert_eq!(bob.seek_step(), None, "no seek");
    assert_eq!(bob.seek_root(thread), Some(thread));
    assert_eq!(bob.seek_root(thread), None, "the probe is on its way");
    assert!(bob.is_probe(thread));
    assert_eq!(bob.seek_step(), Some(SeekStep::Wait));
    bob.apply_signed(
        thread,
        Seq::<Relay>::new(1),
        &first,
        signer(&alice, thread, alice_user),
    )
    .expect("waits");
    assert_eq!(bob.probe_ready(thread), Ok(()));
    assert!(!bob.holds(thread), "the probe is dropped");
    assert!(!bob.is_probe(thread));
    assert_eq!(bob.seek_step(), Some(SeekStep::Load(middle)));
    assert_eq!(bob.seek_step(), Some(SeekStep::Wait), "one at a time");
    load(&mut bob, middle, &empty_segment(alice.channel_id()));
    assert_eq!(bob.seek_step(), Some(SeekStep::Load(old)));
    let bytes = alice.snapshot(old.document_id()).expect("bytes");
    load(&mut bob, old, &bytes);
    assert_eq!(bob.seek_root(thread), None, "the root is held now");
    assert_eq!(bob.seek_step(), Some(found));
    assert_eq!(bob.seek_step(), None, "a found root ends the seek");
    assert_eq!(bob.threads_to_open(), [thread], "the thread opens anew");
}

/// A header naming no listed segment, a segment that failed, a probe that
/// failed and a segment with no such root each end the seek. Nothing older
/// is asked for.
#[test]
fn a_seek_ends_on_what_it_cannot_follow() {
    let alice_user = Id::<User>::mint();
    let mut alice = channel_with(&["root"], alice_user);
    let root = message_at(&alice, 0).id;
    let (thread, first, _) = threaded(&mut alice, root);
    let old = alice.active_id();
    let probed = |bob: &mut ChannelState| {
        assert_eq!(bob.seek_root(thread), Some(thread));
        bob.apply_signed(
            thread,
            Seq::<Relay>::new(1),
            &first,
            signer(&alice, thread, alice_user),
        )
        .expect("waits");
        bob.probe_ready(thread)
    };

    let (mut unlisted, _) = far_copy(&alice, None);
    assert_eq!(
        probed(&mut unlisted),
        Err(MurmureError::ForeignRoot(thread))
    );
    assert_eq!(unlisted.seek_step(), Some(SeekStep::Lost { thread }));
    assert_eq!(unlisted.seek_step(), None, "said once");

    let (mut failed, middle) = far_copy(&alice, Some(old));
    assert_eq!(probed(&mut failed), Ok(()));
    assert_eq!(failed.seek_step(), Some(SeekStep::Load(middle)));
    assert!(failed.cancel_older(middle.document_id()));
    let lost = Some(SeekStep::Lost { thread });
    assert_eq!(failed.seek_step(), lost, "asked once");
    assert_eq!(failed.seek_step(), None);

    let (mut gone, _) = far_copy(&alice, Some(old));
    assert_eq!(gone.seek_root(thread), Some(thread));
    assert!(gone.cancel_older(thread));
    assert_eq!(gone.seek_step(), lost, "the probe failed");
    assert_eq!(gone.seek_step(), None);

    let (mut empty, middle) = far_copy(&alice, Some(old));
    assert_eq!(probed(&mut empty), Ok(()));
    let _ = empty.seek_step();
    load(&mut empty, middle, &empty_segment(alice.channel_id()));
    assert_eq!(empty.seek_step(), Some(SeekStep::Load(old)));
    load(&mut empty, old, &empty_segment(alice.channel_id()));
    assert_eq!(empty.seek_step(), lost, "no such root");
    assert_eq!(
        empty.probe_ready(thread),
        Err(MurmureError::NotHeld(thread))
    );
}

/// A probe of another thread that fails leaves the seek waiting on its own
/// probe (`MURMURE-547`).
#[test]
fn a_failed_probe_of_another_thread_leaves_the_seek_waiting() {
    let alice_user = Id::<User>::mint();
    let mut alice = channel_with(&["root"], alice_user);
    let root = message_at(&alice, 0).id;
    let (thread, _, _) = threaded(&mut alice, root);
    let old = alice.active_id();
    let (mut bob, _) = far_copy(&alice, Some(old));
    let other = Id::<Document>::mint();
    assert_eq!(bob.seek_root(other), Some(other));
    assert_eq!(bob.seek_root(thread), Some(thread));
    assert!(bob.cancel_older(other));
    assert_eq!(bob.seek_step(), Some(SeekStep::Wait));
}

// --- AUTHOR-026 ---

/// The identity the Sync worker issues for a thread still arriving is kept,
/// and the thread takes it once admitted. Until then this member may not
/// write it.
#[test]
fn an_identity_issued_before_a_thread_arrives_is_adopted_on_admission() {
    let alice_user = Id::<User>::mint();
    let mut alice = channel_with(&["root"], alice_user);
    let root = message_at(&alice, 0).id;
    let (thread, first, _) = threaded(&mut alice, root);
    let mut bob = peer_of(&alice);
    assert_eq!(bob.threads_to_open(), [thread]);
    let peer = next_peer();
    assert_eq!(
        bob.adopt_session(
            thread,
            peer,
            Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
        ),
        Ok(None),
        "kept for the thread still arriving"
    );
    assert!(!bob.identified(thread), "not held yet");
    bob.apply_signed(
        thread,
        Seq::<Relay>::new(1),
        &first,
        signer(&alice, thread, alice_user),
    )
    .expect("waits");
    bob.older_ready(thread).expect("admitted");
    assert!(bob.identified(thread));
    assert_eq!(bob.peer(thread), Some(peer));
}

// --- SYNC-321 ---

/// `SYNC-321`: a removal reads each segment the `meta` lists for the
/// threads its roots name, in list order. A segment the view loads makes
/// the read wait, and is read where it is held. A segment the channel does
/// not hold opens for the read alone, then drops.
#[test]
fn a_removal_reads_the_thread_each_segment_names() {
    let alice_user = Id::<User>::mint();
    let mut alice = channel_with(&["root"], alice_user);
    let root = message_at(&alice, 0).id;
    let (thread, _, _) = threaded(&mut alice, root);
    let old = alice.active_id();
    let bytes = alice.snapshot(old.document_id()).expect("bytes");

    let (mut bob, middle) = far_copy(&alice, Some(old));
    assert_eq!(bob.retry(old), Some(old), "the view loads it");
    assert_eq!(bob.sweep_step(), crate::SweepStep::Wait);
    load(&mut bob, old, &bytes);
    assert_eq!(
        bob.sweep_step(),
        crate::SweepStep::Open(middle.document_id()),
        "the held segment is read where it is"
    );
    assert!(bob.is_swept(middle.document_id()));
    assert!(!bob.is_pending(middle.document_id()));
    let _ = bob
        .apply_remote(
            middle.document_id(),
            (&empty_segment(alice.channel_id())).into(),
        )
        .expect("kept");
    assert_eq!(bob.sweep_ready(middle.document_id()), Ok(None));
    assert!(!bob.holds(middle.document_id()), "read, then dropped");
    let crate::SweepStep::Done(swept) = bob.sweep_step() else {
        panic!("every segment is read");
    };
    for document_id in [old.document_id(), middle.document_id()] {
        assert!(swept.documents.contains(&document_id), "{swept:?}");
    }
    assert_eq!(swept.threads, [thread], "each thread once, apart");

    let (mut carol, _) = far_copy(&alice, Some(old));
    assert_eq!(
        carol.sweep_step(),
        crate::SweepStep::Open(old.document_id())
    );
    let foreign = empty_segment(Id::<Channel>::mint());
    let _ = carol
        .apply_remote(old.document_id(), (&foreign).into())
        .expect("kept");
    assert!(
        carol.sweep_ready(old.document_id()).is_err(),
        "a segment of another channel ends the read"
    );
    assert!(!carol.is_swept(old.document_id()));
    assert_eq!(
        carol.sweep_ready(old.document_id()),
        Err(MurmureError::NotHeld(old.document_id()))
    );
}
