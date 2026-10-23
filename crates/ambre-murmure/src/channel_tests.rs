//! The engine core against its contract rows: the documents a channel holds,
//! the header check, the routing of a remote delta, and the send.

use ambre_bus::CharSpan;
use std::collections::{HashMap, HashSet};

use ambre_bus::{
    Mention, MentionTarget, MessageDraft, MurmureEvent, Notice, Timestamp, Unreadable,
};
use ambre_core::schema::SchemaError;
use ambre_core::{ChannelId, DocumentId, MetaId, SegmentId, Seq, UserId};
use loro::{Container, ExportMode, LoroDoc, LoroValue, ValueOrContainer};

use crate::channel::push_segment_entry;
use crate::schema::{self, ChannelKey, DocumentKind, HeaderKey, MessageKey, Root, SegmentState};
use crate::{
    ChannelState, ChannelType, MurmureError, Opening, OpeningChannel, Query, SCHEMA_VERSION, Signer,
};

const NOW: Timestamp = Timestamp(1_700_000_000_000);

fn draft(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    }
}

fn snapshot(doc: &LoroDoc) -> Vec<u8> {
    doc.export(ExportMode::Snapshot).expect("snapshot")
}

/// A segment document of `channel`, as a peer would have stored it.
fn segment_doc(channel: ChannelId) -> LoroDoc {
    let doc = LoroDoc::new();
    schema::write_header(&doc, DocumentKind::Segment, channel, NOW).expect("header");
    doc.commit();
    doc
}

/// A stored channel with one final, one closing and one active segment.
struct Stored {
    channel: ChannelId,
    meta_id: MetaId,
    meta: Vec<u8>,
    final_id: SegmentId,
    closing_id: SegmentId,
    active_id: SegmentId,
    segments: HashMap<SegmentId, Vec<u8>>,
}

fn stored_channel() -> Stored {
    let channel = ChannelId::new();
    let (meta_id, final_id, closing_id, active_id) = (
        MetaId::new(),
        SegmentId::new(),
        SegmentId::new(),
        SegmentId::new(),
    );
    let meta = LoroDoc::new();
    schema::write_header(&meta, DocumentKind::Meta, channel, NOW).expect("header");
    for (id, state) in [
        (final_id, SegmentState::Final),
        (closing_id, SegmentState::Closing),
        (active_id, SegmentState::Active),
    ] {
        push_segment_entry(&meta, id, NOW, state).expect("entry");
    }
    meta.commit();
    let segments = [final_id, closing_id, active_id]
        .into_iter()
        .map(|id| (id, snapshot(&segment_doc(channel))))
        .collect();
    Stored {
        channel,
        meta_id,
        meta: snapshot(&meta),
        final_id,
        closing_id,
        active_id,
        segments,
    }
}

fn open(stored: &Stored) -> ChannelState {
    ChannelState::open(
        stored.channel,
        stored.meta_id,
        &stored.meta,
        &stored.segments,
    )
    .expect("open")
}

/// A second peer of the same channel, opened from the first one's documents.
/// Who signed what `from` wrote in `segment`: `user`, under its session.
fn signer(from: &ChannelState, segment: SegmentId, user: UserId) -> Signer {
    Signer {
        peer: from.peer(segment.document_id()).expect("a session"),
        user: Some(user),
    }
}

fn peer_of(state: &ChannelState) -> ChannelState {
    let segments = HashMap::from([(
        state.active_id(),
        state
            .snapshot(state.active_id().document_id())
            .expect("active"),
    )]);
    let meta = state.snapshot(state.meta_id().document_id()).expect("meta");
    ChannelState::open(state.channel_id(), state.meta_id(), &meta, &segments)
        .expect("peer")
        .identify()
}

/// The roots a document holds. Loro reports every root it was ever asked
/// for, so an empty one is not a container the document holds.
fn root_keys(doc: &LoroDoc) -> Vec<String> {
    let LoroValue::Map(roots) = doc.get_deep_value() else {
        return Vec::new();
    };
    roots
        .iter()
        .filter(|(_, value)| match value {
            LoroValue::Map(m) => !m.is_empty(),
            LoroValue::List(l) => !l.is_empty(),
            _ => true,
        })
        .map(|(key, _)| key.clone())
        .collect()
}

// --- MURMURE-021, MURMURE-022, MURMURE-001 ---

#[test]
fn a_channel_is_held_as_its_documents_apart() {
    let state =
        ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW).expect("create");
    assert_ne!(
        state.meta_id().document_id(),
        state.active_id().document_id()
    );
    let meta = state.doc(state.meta_id().document_id()).expect("meta");
    let active = state.doc(state.active_id().document_id()).expect("active");
    let mut meta_roots = root_keys(meta);
    meta_roots.sort();
    assert_eq!(
        meta_roots,
        [
            Root::Channel.as_str(),
            Root::Header.as_str(),
            ambre_core::crdt::FoundationRoot::Meta.as_str(),
            Root::Segments.as_str()
        ]
    );
    let mut active_roots = root_keys(active);
    active_roots.sort();
    assert_eq!(
        active_roots,
        [
            Root::Header.as_str(),
            ambre_core::crdt::FoundationRoot::Meta.as_str()
        ]
    );
}

#[test]
fn every_document_carries_a_header_and_the_foundation_version() {
    let channel = ChannelId::new();
    let state = ChannelState::create(channel, ChannelType::Private, None, NOW).expect("create");
    for (id, kind) in [
        (state.meta_id().document_id(), DocumentKind::Meta),
        (state.active_id().document_id(), DocumentKind::Segment),
    ] {
        let doc = state.doc(id).expect("held");
        let header = crate::schema::header(doc);
        assert_eq!(
            schema::string(&header, HeaderKey::Kind).as_deref(),
            Some(kind.as_str())
        );
        assert_eq!(
            schema::string(&header, HeaderKey::Channel),
            Some(channel.to_string())
        );
        assert_eq!(
            schema::unsigned(
                &crate::schema::Fields::of(
                    doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
                ),
                ambre_core::schema::MetaKey::SchemaVersion
            ),
            Some(u64::from(SCHEMA_VERSION.get()))
        );
    }
}

#[test]
fn an_open_channel_holds_meta_the_active_and_the_closing_segment() {
    let stored = stored_channel();
    let state = open(&stored);
    assert!(state.holds(stored.meta_id.document_id()));
    assert!(state.holds(stored.active_id.document_id()));
    assert!(state.holds(stored.closing_id.document_id()));
    assert_eq!(state.active_id(), stored.active_id);
    assert_eq!(state.closing_id(), Some(stored.closing_id));
}

#[test]
fn a_live_segment_with_no_bytes_refuses_the_open() {
    let mut stored = stored_channel();
    stored.segments.remove(&stored.closing_id);
    let refused = ChannelState::open(
        stored.channel,
        stored.meta_id,
        &stored.meta,
        &stored.segments,
    );
    assert_eq!(
        refused.err(),
        Some(MurmureError::SegmentMissing(
            stored.closing_id.document_id()
        ))
    );
}

// --- MURMURE-028, MURMURE-023 ---

#[test]
fn opening_a_channel_never_loads_a_final_segment() {
    let stored = stored_channel();
    // The final segment's bytes are on hand, and still not loaded.
    assert!(stored.segments.contains_key(&stored.final_id));
    let state = open(&stored);
    assert!(!state.holds(stored.final_id.document_id()));
}

#[test]
fn an_older_segment_loads_only_when_the_view_asks() {
    let stored = stored_channel();
    let mut state = open(&stored);
    assert!(!state.holds(stored.final_id.document_id()));
    assert_eq!(state.older_than(stored.active_id), Some(stored.final_id));
    let bytes = stored.segments.get(&stored.final_id).expect("bytes");
    let event = state.load_older(stored.final_id, bytes).expect("load");
    assert!(state.holds(stored.final_id.document_id()));
    assert_eq!(
        event,
        MurmureEvent::Messages {
            segment: stored.final_id,
            messages: Vec::new()
        }
    );
    assert_eq!(state.older_than(stored.final_id), None);
}

/// Reaching the latest message reads every final segment, loaded or not,
/// and names no live one.
#[test]
fn the_final_segments_are_listed_loaded_or_not() {
    let stored = stored_channel();
    let state = open(&stored);
    assert!(!state.holds(stored.final_id.document_id()));
    assert_eq!(state.final_segments(), [stored.final_id]);
}

/// Marking a message unread moves the cursors of the segments listed after
/// its own, and of no other (`MURMURE-154`).
#[test]
fn the_segments_after_one_are_those_listed_later() {
    let stored = stored_channel();
    let state = open(&stored);
    assert_eq!(
        state.segments_after(stored.final_id),
        [stored.closing_id, stored.active_id]
    );
    assert_eq!(state.segments_after(stored.active_id), []);
    assert_eq!(state.held_segments(), [stored.active_id, stored.closing_id]);
}

#[test]
fn a_live_segment_is_not_an_older_one() {
    let stored = stored_channel();
    let mut state = open(&stored);
    let bytes = stored.segments.get(&stored.closing_id).expect("bytes");
    assert_eq!(
        state.load_older(stored.closing_id, bytes).err(),
        Some(MurmureError::NotOlder(stored.closing_id.document_id()))
    );
}

// --- MURMURE-024 ---

#[test]
fn a_remote_delta_reaches_the_document_it_names_and_no_other() {
    let stored = stored_channel();
    let mut state = open(&stored);
    // A remote peer writes into the closing segment.
    let remote = LoroDoc::new();
    remote
        .import(stored.segments.get(&stored.closing_id).expect("bytes"))
        .expect("import");
    let before = remote.oplog_vv();
    let map = crate::schema::Fields::<MessageKey>::of(
        remote
            .get_list(Root::Messages.as_str())
            .push_container(loro::LoroMap::new())
            .expect("push"),
    );
    map.insert(MessageKey::Id, "x").expect("insert");
    remote.commit();
    let delta = remote.export(ExportMode::updates(&before)).expect("delta");

    let active_before = state
        .doc(stored.active_id.document_id())
        .expect("active")
        .get_deep_value();
    let meta_before = state
        .doc(stored.meta_id.document_id())
        .expect("meta")
        .get_deep_value();
    state
        .apply_remote(stored.closing_id.document_id(), &delta)
        .expect("apply");
    assert_eq!(
        state
            .doc(stored.closing_id.document_id())
            .expect("closing")
            .get_list(Root::Messages.as_str())
            .len(),
        1
    );
    assert_eq!(
        state
            .doc(stored.active_id.document_id())
            .expect("active")
            .get_deep_value(),
        active_before
    );
    assert_eq!(
        state
            .doc(stored.meta_id.document_id())
            .expect("meta")
            .get_deep_value(),
        meta_before
    );
}

#[test]
fn a_delta_for_a_document_the_channel_does_not_hold_is_refused() {
    let stored = stored_channel();
    let mut state = open(&stored);
    let stranger = DocumentId::new();
    assert_eq!(
        state.apply_remote(stranger, &[1, 2, 3]).err(),
        Some(MurmureError::NotHeld(stranger))
    );
    // The final segment is listed but not loaded, so it is not held either.
    assert_eq!(
        state.apply_remote(stored.final_id.document_id(), &[]).err(),
        Some(MurmureError::NotHeld(stored.final_id.document_id()))
    );
}

// --- MURMURE-027, MURMURE-019 ---

#[test]
fn a_document_naming_another_channel_is_refused() {
    let mut stored = stored_channel();
    let other = ChannelId::new();
    stored
        .segments
        .insert(stored.active_id, snapshot(&segment_doc(other)));
    let refused = ChannelState::open(
        stored.channel,
        stored.meta_id,
        &stored.meta,
        &stored.segments,
    );
    assert_eq!(
        refused.err(),
        Some(MurmureError::ForeignChannel {
            document_id: stored.active_id.document_id(),
            expected: stored.channel,
            found: other,
        })
    );
}

#[test]
fn a_meta_document_of_another_channel_is_refused() {
    let stored = stored_channel();
    let other = ChannelId::new();
    let refused = ChannelState::open(other, stored.meta_id, &stored.meta, &stored.segments);
    assert!(matches!(
        refused,
        Err(MurmureError::ForeignChannel { found, .. }) if found == stored.channel
    ));
}

#[test]
fn a_segment_in_the_place_of_meta_is_refused() {
    let stored = stored_channel();
    let bytes = &stored.segments[&stored.active_id];
    let refused = ChannelState::open(stored.channel, stored.meta_id, bytes, &stored.segments);
    assert!(matches!(
        refused,
        Err(MurmureError::WrongKind {
            expected: "meta",
            ..
        })
    ));
}

#[test]
fn a_document_a_newer_client_wrote_is_refused_whole() {
    let mut stored = stored_channel();
    let newer = segment_doc(stored.channel);
    newer
        .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .insert(
            ambre_core::schema::MetaKey::SchemaVersion.as_str(),
            i64::from(SCHEMA_VERSION.get()) + 1,
        )
        .expect("bump");
    newer.commit();
    stored.segments.insert(stored.active_id, snapshot(&newer));
    let refused = ChannelState::open(
        stored.channel,
        stored.meta_id,
        &stored.meta,
        &stored.segments,
    );
    assert_eq!(
        refused.err(),
        Some(MurmureError::Schema(SchemaError::DocumentTooNew {
            document: SCHEMA_VERSION.next(),
            binary: SCHEMA_VERSION,
        }))
    );
}

/// A segment stamped at version 1 opens, and the walk writes nothing in it:
/// no member would sign that write (`MURMURE-566`).
#[test]
fn an_older_document_opens_and_the_walk_writes_nothing() {
    let mut stored = stored_channel();
    let older = segment_doc(stored.channel);
    older
        .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .insert(ambre_core::schema::MetaKey::SchemaVersion.as_str(), 1)
        .expect("stamp");
    older.commit();
    let before = older.oplog_vv();
    stored.segments.insert(stored.active_id, snapshot(&older));
    let state = ChannelState::open(
        stored.channel,
        stored.meta_id,
        &stored.meta,
        &stored.segments,
    )
    .expect("a version 1 segment opens");
    let opened = state.doc(stored.active_id.document_id()).expect("held");
    assert_eq!(opened.oplog_vv(), before, "the walk wrote an operation");
    assert_eq!(
        schema::unsigned(
            &crate::schema::Fields::of(
                opened.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
            ),
            ambre_core::schema::MetaKey::SchemaVersion
        ),
        Some(1)
    );
}

#[test]
fn a_document_with_no_version_finds_no_migration_and_is_refused() {
    let stored = stored_channel();
    let bare = LoroDoc::new();
    crate::schema::header(&bare)
        .insert(HeaderKey::Kind, "meta")
        .expect("kind");
    bare.commit();
    let refused = ChannelState::open(
        stored.channel,
        stored.meta_id,
        &snapshot(&bare),
        &stored.segments,
    );
    assert_eq!(
        refused.err(),
        Some(MurmureError::Schema(SchemaError::NoMigrationPath {
            from: ambre_core::schema::SCHEMA_VERSION_ABSENT
        }))
    );
}

#[test]
fn bytes_the_crdt_engine_refuses_never_reach_the_header_check() {
    let stored = stored_channel();
    let refused = ChannelState::open(
        stored.channel,
        stored.meta_id,
        b"not a loro document",
        &stored.segments,
    );
    assert_eq!(
        refused.err(),
        Some(MurmureError::Schema(SchemaError::PayloadRejected))
    );
}

// --- MURMURE-080, MURMURE-002, MURMURE-003, MURMURE-081, MURMURE-083 ---

#[test]
fn a_sent_message_lands_in_the_active_segment_with_a_fresh_id_and_its_author() {
    let mut state = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    let alice = UserId::new();
    let first = state.send(&draft("hello"), alice, NOW).expect("send");
    let second = state.send(&draft("again"), alice, NOW).expect("send");
    assert_eq!(first.segment, state.active_id());
    let messages = state.messages(state.active_id()).expect("messages");
    assert_eq!(messages.len(), 2);
    assert_ne!(messages[0].id, messages[1].id);
    assert_eq!(messages[0].author, alice);
    assert_eq!(messages[0].text, "hello");
    assert_eq!(messages[0].ts, NOW);
    let MurmureEvent::Messages { messages: sent, .. } = &second.event else {
        panic!("a send answers its messages");
    };
    assert_eq!(sent.as_slice(), &messages[1..]);
}

#[test]
fn a_sent_message_carries_a_content_summary_with_its_mentions() {
    let mut state = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    let (bob, carol) = (UserId::new(), UserId::new());
    let with_mentions = MessageDraft {
        text: "@bob @carol @bob @channel".to_owned(),
        styles: Vec::new(),
        mentions: vec![
            Mention {
                target: MentionTarget::User(bob),
                range: CharSpan::from_usize_range(0..4),
            },
            Mention {
                target: MentionTarget::User(carol),
                range: CharSpan::from_usize_range(5..11),
            },
            Mention {
                target: MentionTarget::User(bob),
                range: CharSpan::from_usize_range(12..16),
            },
            Mention {
                target: MentionTarget::Channel,
                range: CharSpan::from_usize_range(17..25),
            },
        ],
    };
    let sent = state
        .send(&with_mentions, UserId::new(), NOW)
        .expect("send");
    assert_eq!(sent.summary.kind, ambre_bus::ActivityKind::Content);
    assert_eq!(sent.summary.mentions, [bob, carol]);
    assert!(sent.summary.mentions_all);
    let plain = state
        .send(&draft("no one"), UserId::new(), NOW)
        .expect("send");
    assert!(plain.summary.mentions.is_empty());
    assert!(!plain.summary.mentions_all);
}

#[test]
fn sending_a_message_changes_no_other_message() {
    let mut state = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    state.send(&draft("one"), UserId::new(), NOW).expect("send");
    state.send(&draft("two"), UserId::new(), NOW).expect("send");
    let list = state
        .doc(state.active_id().document_id())
        .expect("active")
        .get_list(Root::Messages.as_str());
    let before: Vec<LoroValue> = (0..list.len())
        .filter_map(|i| list.get(i).map(|v| v.get_deep_value()))
        .collect();
    state
        .send(&draft("three"), UserId::new(), NOW)
        .expect("send");
    let after: Vec<LoroValue> = (0..list.len())
        .filter_map(|i| list.get(i).map(|v| v.get_deep_value()))
        .collect();
    assert_eq!(after.len(), 3);
    assert_eq!(&after[..2], before.as_slice());
}

#[test]
fn a_sent_delta_brings_a_peer_to_the_same_messages() {
    let mut alice = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    let mut bob = peer_of(&alice);
    let author = UserId::new();
    let sent = alice.send(&draft("hi bob"), author, NOW).expect("send");
    let events = bob
        .apply_signed(
            sent.segment.document_id(),
            Seq(1),
            &sent.delta,
            signer(&alice, sent.segment, author),
        )
        .expect("apply");
    assert_eq!(
        events,
        [MurmureEvent::Messages {
            segment: alice.active_id(),
            messages: alice.messages(alice.active_id()).expect("messages"),
        }]
    );
}

/// A remote delta names the message it brought, once, and an own send
/// names none (`MURMURE-507`).
#[test]
fn a_remote_delta_names_the_message_it_brought_once() {
    let mut alice = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    let mut bob = peer_of(&alice);
    let author = UserId::new();
    let sent = alice.send(&draft("hi bob"), author, NOW).expect("send");
    assert_eq!(alice.take_arrived(), None, "an own send");
    bob.apply_signed(
        sent.segment.document_id(),
        Seq(1),
        &sent.delta,
        signer(&alice, sent.segment, author),
    )
    .expect("apply");
    let id = alice.messages(alice.active_id()).expect("messages")[0].id;
    assert_eq!(bob.take_arrived(), Some((sent.segment, id)));
    assert_eq!(bob.take_arrived(), None, "taken");
}

// --- MURMURE-004, MURMURE-005 ---

#[test]
fn the_body_lives_in_a_text_and_no_map_holds_it() {
    let mut state = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    state
        .send(&draft("the body"), UserId::new(), NOW)
        .expect("send");
    let list = state
        .doc(state.active_id().document_id())
        .expect("active")
        .get_list(Root::Messages.as_str());
    let Some(message) = list.get(0).and_then(crate::schema::fields_of) else {
        panic!("a message is a map container");
    };
    assert!(matches!(
        message.get(MessageKey::Content),
        Some(ValueOrContainer::Container(Container::Text(_)))
    ));
    let mut plain_strings = Vec::new();
    message.map().for_each(|_, v| {
        if let ValueOrContainer::Value(LoroValue::String(s)) = v {
            plain_strings.push(s.to_string());
        }
    });
    assert!(!plain_strings.iter().any(|s| s.contains("the body")));
}

#[test]
fn two_peers_typing_in_one_message_keep_both_texts() {
    let mut alice = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    let mut bob = peer_of(&alice);
    let author = UserId::new();
    let sent = alice.send(&draft("base"), author, NOW).expect("send");
    let by = signer(&alice, sent.segment, author);
    bob.apply_signed(sent.segment.document_id(), Seq(1), &sent.delta, by)
        .expect("apply");

    let segment = alice.active_id();
    let known = |state: &ChannelState| {
        state
            .doc(segment.document_id())
            .expect("segment")
            .oplog_vv()
    };
    let (alice_knew, bob_knew) = (known(&alice), known(&bob));
    // What the other peer lacks, so a peer's registry entry rides with its
    // first edit (`AUTHOR-009`).
    let type_into = |state: &ChannelState,
                     other_knew: &loro::VersionVector,
                     at: usize,
                     text: &str|
     -> Vec<u8> {
        let doc = state.doc(segment.document_id()).expect("segment");
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
        content.insert(at, text).expect("type");
        doc.commit();
        doc.export(ExportMode::updates(other_knew)).expect("delta")
    };
    let from_alice = type_into(&alice, &bob_knew, 0, "A:");
    let from_bob = type_into(&bob, &alice_knew, 4, ":B");
    alice
        .apply_remote(segment.document_id(), &from_bob)
        .expect("apply");
    bob.apply_remote(segment.document_id(), &from_alice)
        .expect("apply");

    let on_alice = alice.messages(segment).expect("alice");
    let on_bob = bob.messages(segment).expect("bob");
    assert_eq!(on_alice, on_bob);
    assert_eq!(on_alice[0].text, "A:base:B");
}

// --- MURMURE-015, MURMURE-018 ---

#[test]
fn meta_holds_no_member_list() {
    let state = ChannelState::create(
        ChannelId::new(),
        ChannelType::Private,
        Some(&crate::fit_name("team")),
        NOW,
    )
    .expect("create");
    let meta = state.doc(state.meta_id().document_id()).expect("meta");
    let text = format!("{:?}", meta.get_deep_value());
    assert!(!text.contains("member"));
    let mut roots = root_keys(meta);
    roots.sort();
    assert_eq!(
        roots,
        [
            Root::Channel.as_str(),
            Root::Header.as_str(),
            ambre_core::crdt::FoundationRoot::Meta.as_str(),
            Root::Segments.as_str()
        ]
    );
}

#[test]
fn an_unset_property_is_an_absent_key() {
    let unnamed =
        ChannelState::create(ChannelId::new(), ChannelType::Direct, None, NOW).expect("create");
    let channel =
        crate::schema::channel(unnamed.doc(unnamed.meta_id().document_id()).expect("meta"));
    assert!(channel.get(ChannelKey::Name).is_none());
    let named = ChannelState::create(
        ChannelId::new(),
        ChannelType::Public,
        Some(&crate::fit_name("general")),
        NOW,
    )
    .expect("create");
    let channel = crate::schema::channel(named.doc(named.meta_id().document_id()).expect("meta"));
    assert_eq!(
        schema::string(&channel, ChannelKey::Name).as_deref(),
        Some("general")
    );
}

// --- MURMURE-016, MURMURE-054, MURMURE-055: the name of a direct message ---

/// A second member of `state`, opened from every byte the first holds.
fn second_member(state: &ChannelState) -> ChannelState {
    let meta = state.meta_id();
    let segments = HashMap::from([(
        state.active_id(),
        state
            .export_all(state.active_id().document_id())
            .expect("segment"),
    )]);
    let meta_bytes = state.export_all(meta.document_id()).expect("meta");
    ChannelState::open(state.channel_id(), meta, &meta_bytes, &segments)
        .expect("open")
        .identify()
}

#[test]
fn a_name_one_member_sets_reaches_every_member() {
    let mut alice = ChannelState::create(ChannelId::new(), ChannelType::Direct, None, NOW)
        .expect("create")
        .identify();
    let mut bob = second_member(&alice);

    let named = alice
        .name_direct(Some(&crate::fit_name("  trip  ")))
        .expect("name");
    assert_eq!(named.meta, alice.meta_id());
    assert_eq!(
        named.event,
        MurmureEvent::Named {
            name: Some("trip".into())
        }
    );
    let events = bob
        .apply_remote(named.meta.document_id(), &named.delta)
        .expect("apply");
    assert_eq!(
        events,
        [MurmureEvent::Named {
            name: Some("trip".into())
        }]
    );
    assert_eq!(bob.name().as_deref(), Some("trip"));

    let renamed = bob
        .name_direct(Some(&crate::fit_name("holiday")))
        .expect("rename");
    alice
        .apply_remote(renamed.meta.document_id(), &renamed.delta)
        .expect("apply");
    assert_eq!(alice.name().as_deref(), Some("holiday"));
}

#[test]
fn a_notice_names_the_member_who_joins_for_every_member() {
    let mut alice = ChannelState::create(ChannelId::new(), ChannelType::Direct, None, NOW)
        .expect("create")
        .identify();
    let mut bob = peer_of(&alice);
    let (author, carol) = (UserId::new(), UserId::new());

    let sent = alice.announce(carol, author, NOW).expect("announce");
    assert_eq!(sent.segment, alice.active_id());
    let notice = Some(Notice::MemberAdded { member: carol });
    let MurmureEvent::Messages { messages, .. } = &sent.event else {
        panic!("a notice is a message: {:?}", sent.event);
    };
    assert_eq!(
        (messages[0].notice, messages[0].text.as_str()),
        (notice, "")
    );
    // The authorship check admits it: every key belongs to the inviter.
    let by = signer(&alice, sent.segment, author);
    bob.apply_signed(sent.segment.document_id(), Seq(1), &sent.delta, by)
        .expect("apply");
    let read = bob.messages(bob.active_id()).expect("held");
    assert_eq!(read.len(), 1);
    assert_eq!((read[0].author, read[0].notice), (author, notice));
    assert!(read[0].text.is_empty() && read[0].mentions.is_empty());
}

#[test]
fn a_notice_of_a_kind_no_build_wrote_is_skipped() {
    let mut alice = ChannelState::create(ChannelId::new(), ChannelType::Direct, None, NOW)
        .expect("create")
        .identify();
    alice
        .announce(UserId::new(), UserId::new(), NOW)
        .expect("announce");
    let doc = alice.doc(alice.active_id().document_id()).expect("held");
    let Some(message) = crate::schema::root_list(doc, Root::Messages)
        .get(0)
        .and_then(crate::schema::fields_of)
    else {
        panic!("the notice is a map");
    };
    message
        .insert(MessageKey::Notice, "member_renamed")
        .expect("insert");
    assert!(alice.messages(alice.active_id()).expect("held").is_empty());
}

#[test]
fn clearing_a_name_removes_its_key() {
    let mut alice = ChannelState::create(
        ChannelId::new(),
        ChannelType::Direct,
        Some(&crate::fit_name("trip")),
        NOW,
    )
    .expect("create")
    .identify();
    let mut bob = second_member(&alice);
    let meta = alice.meta_id().document_id();

    for cleared in [None, Some(crate::fit_name("   "))] {
        let named = alice.name_direct(cleared.as_ref()).expect("clear");
        assert_eq!(named.event, MurmureEvent::Named { name: None });
        bob.apply_remote(meta, &named.delta).expect("apply");
        for state in [&alice, &bob] {
            let channel = crate::schema::channel(state.doc(meta).expect("meta"));
            assert!(
                channel.get(ChannelKey::Name).is_none(),
                "no key, no empty name"
            );
            assert_eq!(state.name(), None);
        }
    }
}

#[test]
fn only_a_direct_message_takes_a_name_from_a_member() {
    for kind in [ChannelType::Private, ChannelType::Public] {
        let mut state =
            ChannelState::create(ChannelId::new(), kind, Some(&crate::fit_name("team")), NOW)
                .expect("create");
        assert_eq!(
            state
                .name_direct(Some(&crate::fit_name("other")))
                .map(|n| n.event),
            Err(MurmureError::NotDirect(state.channel_id()))
        );
        assert_eq!(state.name().as_deref(), Some("team"));
    }
}

// --- MURMURE-148, MURMURE-149, MURMURE-151 ---

/// A search reads every held segment and lists the one it did not read. It
/// loads nothing: the final segment joins only once the view loaded it.
#[test]
fn a_search_reads_the_held_segments_and_lists_the_rest() {
    let mut stored = stored_channel();
    let mut writer = ChannelState::create(stored.channel, ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    writer
        .send(&draft("the Été budget"), UserId::new(), NOW)
        .expect("send");
    let older = writer
        .snapshot(writer.active_id().document_id())
        .expect("snapshot");
    stored.segments.insert(stored.final_id, older.clone());
    let mut state = open(&stored).identify();
    for text in ["budget review", "lunch"] {
        state.send(&draft(text), UserId::new(), NOW).expect("send");
    }
    let query = Query::parse("BUDG ete").expect("two words");
    let found = |state: &ChannelState| {
        let answer = state.search(&query);
        let matches: Vec<(SegmentId, String)> = answer
            .matches
            .iter()
            .map(|m| (m.segment, m.message.text.clone()))
            .collect();
        (answer.covered, answer.listed, matches)
    };

    let (covered, listed, matches) = found(&state);
    assert_eq!(listed, 3, "every segment meta lists");
    assert_eq!(covered, [stored.closing_id, stored.active_id]);
    assert!(
        matches.is_empty(),
        "the one match sits in the final segment"
    );
    assert!(!state.holds(stored.final_id.document_id()));
    assert!(!state.is_pending(stored.final_id.document_id()));

    state.load_older(stored.final_id, &older).expect("load");
    let (covered, _, matches) = found(&state);
    assert_eq!(
        covered,
        [stored.final_id, stored.closing_id, stored.active_id]
    );
    assert_eq!(matches, [(stored.final_id, "the Été budget".to_owned())]);
    let budget = Query::parse("budget").expect("a word");
    let segments: Vec<SegmentId> = state
        .search(&budget)
        .matches
        .iter()
        .map(|m| m.segment)
        .collect();
    assert_eq!(segments, [stored.final_id, stored.active_id], "list order");
}

// --- robustness ---

#[test]
fn a_malformed_message_is_skipped_and_the_rest_still_reads() {
    let mut state = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    let doc = state.doc(state.active_id().document_id()).expect("active");
    crate::schema::root_list(doc, Root::Messages)
        .push("not a message")
        .expect("push");
    doc.commit();
    state
        .send(&draft("fine"), UserId::new(), NOW)
        .expect("send");
    let messages = state.messages(state.active_id()).expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "fine");
}

// --- MURMURE-025: the staged open over the relay ---

/// `meta` first, then the live segments it names, each as a snapshot and a
/// replay, the way the relay serves a document.
#[test]
fn opening_takes_meta_then_its_live_segments_and_never_a_final_one() {
    let stored = stored_channel();
    let mut opening = OpeningChannel::new(stored.channel, stored.meta_id);
    assert!(opening.receive(stored.meta_id.document_id(), &stored.meta));

    let Ok(Opening::OpenSegments(asked)) = opening.ready(stored.meta_id.document_id()) else {
        panic!("meta names the live segments");
    };
    let asked: HashSet<SegmentId> = asked.into_iter().collect();
    assert_eq!(
        asked,
        HashSet::from([stored.closing_id, stored.active_id]),
        "the live segments and no final one"
    );
    assert!(!opening.expects(stored.final_id.document_id()));

    for id in [stored.closing_id, stored.active_id] {
        assert!(opening.receive(id.document_id(), &stored.segments[&id]));
    }
    assert!(matches!(
        opening.ready(stored.closing_id.document_id()),
        Ok(Opening::Waiting)
    ));
    let Ok(Opening::Open(state)) = opening.ready(stored.active_id.document_id()) else {
        panic!("the last live segment opens the channel");
    };
    assert_eq!(state.active_id(), stored.active_id);
    assert_eq!(state.closing_id(), Some(stored.closing_id));
    assert!(!state.holds(stored.final_id.document_id()));
}

/// A delta on `meta` that lands between its ready and the channel's open is
/// kept, not dropped.
#[test]
fn a_meta_delta_during_the_opening_reaches_the_open_channel() {
    let stored = stored_channel();
    let mut opening = OpeningChannel::new(stored.channel, stored.meta_id);
    opening.receive(stored.meta_id.document_id(), &stored.meta);
    let _ = opening.ready(stored.meta_id.document_id()).expect("meta");

    let editor = LoroDoc::new();
    editor.import(&stored.meta).expect("import");
    let before = editor.oplog_vv();
    crate::schema::channel(&editor)
        .insert(ChannelKey::Name, "renamed")
        .expect("rename");
    editor.commit();
    let delta = editor.export(ExportMode::updates(&before)).expect("delta");
    assert!(opening.receive(stored.meta_id.document_id(), &delta));

    opening.receive(
        stored.closing_id.document_id(),
        &stored.segments[&stored.closing_id],
    );
    opening.receive(
        stored.active_id.document_id(),
        &stored.segments[&stored.active_id],
    );
    let _ = opening
        .ready(stored.closing_id.document_id())
        .expect("closing");
    let Ok(Opening::Open(state)) = opening.ready(stored.active_id.document_id()) else {
        panic!("the last live segment opens the channel");
    };
    let meta = state.doc(stored.meta_id.document_id()).expect("meta");
    assert_eq!(
        schema::string(&crate::schema::channel(meta), ChannelKey::Name).as_deref(),
        Some("renamed")
    );
}

#[test]
fn a_meta_of_another_channel_ends_the_opening() {
    let stored = stored_channel();
    let mut opening = OpeningChannel::new(ChannelId::new(), stored.meta_id);
    opening.receive(stored.meta_id.document_id(), &stored.meta);
    assert!(matches!(
        opening.ready(stored.meta_id.document_id()),
        Err(MurmureError::ForeignChannel { .. })
    ));
}

#[test]
fn an_opening_refuses_a_document_it_did_not_ask_for() {
    let stored = stored_channel();
    let mut opening = OpeningChannel::new(stored.channel, stored.meta_id);
    assert!(!opening.receive(
        stored.active_id.document_id(),
        &stored.segments[&stored.active_id]
    ));
    assert_eq!(
        opening.ready(stored.active_id.document_id()).err(),
        Some(MurmureError::NotHeld(stored.active_id.document_id()))
    );
}

// --- MURMURE-502, MURMURE-503: a closing segment that cannot open ---

/// An opening past `meta`, with the active segment in and ready.
fn opening_with_active(stored: &Stored) -> OpeningChannel {
    let mut opening = OpeningChannel::new(stored.channel, stored.meta_id);
    opening.receive(stored.meta_id.document_id(), &stored.meta);
    let _ = opening.ready(stored.meta_id.document_id()).expect("meta");
    opening.receive(
        stored.active_id.document_id(),
        &stored.segments[&stored.active_id],
    );
    opening
}

/// A closing segment whose key never came is given up: the channel opens
/// without it, and its band is handed once.
#[test]
fn a_closing_segment_with_no_key_opens_the_channel_without_it() {
    let stored = stored_channel();
    let mut opening = opening_with_active(&stored);
    assert!(
        opening.give_up(stored.active_id.document_id()).is_none(),
        "the active segment is never given up"
    );
    assert!(matches!(
        opening.give_up(stored.closing_id.document_id()),
        Some(Ok(Opening::Waiting))
    ));
    assert!(!opening.expects(stored.closing_id.document_id()));
    let Ok(Opening::Open(mut state)) = opening.ready(stored.active_id.document_id()) else {
        panic!("the active segment opens the channel");
    };
    assert_eq!(state.closing_id(), None);
    assert_eq!(
        state.take_closing_band(),
        Some(MurmureEvent::Unreadable {
            segment: stored.closing_id,
            cause: Unreadable::KeyNotReceived,
            from: NOW,
            to: Some(NOW),
        })
    );
    assert_eq!(state.take_closing_band(), None, "handed once");
}

/// A closing segment whose bytes fail the check opens as a damaged band.
#[test]
fn a_damaged_closing_segment_opens_the_channel_without_it() {
    let stored = stored_channel();
    let mut opening = opening_with_active(&stored);
    opening.receive(stored.closing_id.document_id(), &[0xff; 8]);
    let _ = opening
        .ready(stored.closing_id.document_id())
        .expect("closing");
    let Ok(Opening::Open(mut state)) = opening.ready(stored.active_id.document_id()) else {
        panic!("the active segment opens the channel");
    };
    assert_eq!(state.closing_id(), None);
    assert!(matches!(
        state.take_closing_band(),
        Some(MurmureEvent::Unreadable {
            cause: Unreadable::Damaged,
            ..
        })
    ));
}

/// The active segment has no such leniency: its failure ends the opening.
#[test]
fn a_damaged_active_segment_still_ends_the_opening() {
    let stored = stored_channel();
    let mut opening = OpeningChannel::new(stored.channel, stored.meta_id);
    opening.receive(stored.meta_id.document_id(), &stored.meta);
    let _ = opening.ready(stored.meta_id.document_id()).expect("meta");
    opening.receive(stored.active_id.document_id(), &[0xff; 8]);
    let _ = opening.give_up(stored.closing_id.document_id());
    assert!(opening.ready(stored.active_id.document_id()).is_err());
}

/// A closing segment the channel does not hold still occupies the slot: the
/// active segment never asks to close past it.
#[test]
fn a_closing_segment_not_held_blocks_the_next_close() {
    let stored = stored_channel();
    let mut opening = opening_with_active(&stored);
    let _ = opening.give_up(stored.closing_id.document_id());
    let Ok(Opening::Open(state)) = opening.ready(stored.active_id.document_id()) else {
        panic!("the active segment opens the channel");
    };
    let mut state = (*state).with_segment_size(0);
    assert_eq!(state.wants_close(), None);
}

/// The same channel holding no closing segment asks at once.
#[test]
fn a_channel_with_no_closing_segment_asks_to_close_at_the_size() {
    let (stored, _, _) = two_finals();
    let mut state = open(&stored).with_segment_size(0);
    assert_eq!(state.wants_close(), Some(stored.active_id));
}

// --- MURMURE-023: an older segment over the relay ---

#[test]
fn an_older_segment_asked_for_arrives_then_loads() {
    let stored = stored_channel();
    let mut state = open(&stored);
    assert_eq!(state.request_older(stored.active_id), Some(stored.final_id));
    assert_eq!(
        state.request_older(stored.active_id),
        None,
        "a segment on its way is asked for once"
    );
    assert!(
        state.holds(stored.final_id.document_id())
            && state.is_pending(stored.final_id.document_id())
    );
    let produced = state
        .apply_remote(
            stored.final_id.document_id(),
            &stored.segments[&stored.final_id],
        )
        .expect("kept");
    assert!(
        produced.is_empty(),
        "nothing reads a segment still arriving"
    );

    let event = state
        .older_ready(stored.final_id.document_id())
        .expect("load");
    assert_eq!(
        event,
        MurmureEvent::Messages {
            segment: stored.final_id,
            messages: Vec::new()
        }
    );
    assert!(!state.is_pending(stored.final_id.document_id()));
    assert_eq!(
        state.request_older(stored.active_id),
        None,
        "already loaded"
    );
}

// --- MURMURE-218 and MURMURE-221: a segment that cannot open ---

/// A stored channel with two final segments, spanning `from` to the next
/// one, then an active one. Returns it with the two finals, oldest first.
fn two_finals() -> (Stored, SegmentId, SegmentId) {
    let mut stored = stored_channel();
    let (older, newer) = (SegmentId::new(), SegmentId::new());
    let meta = LoroDoc::new();
    schema::write_header(&meta, DocumentKind::Meta, stored.channel, NOW).expect("header");
    for (id, from, state) in [
        (older, 10, SegmentState::Final),
        (newer, 20, SegmentState::Final),
        (stored.active_id, 30, SegmentState::Active),
    ] {
        push_segment_entry(&meta, id, Timestamp(from), state).expect("entry");
    }
    meta.commit();
    stored.meta = snapshot(&meta);
    stored.segments.retain(|id, _| *id == stored.active_id);
    (stored, older, newer)
}

/// The band of a final segment spans its own start to the start of the
/// next one. It names the cause it is handed.
#[test]
fn a_band_spans_its_segment_up_to_the_next_one() {
    let (stored, older, newer) = two_finals();
    let state = open(&stored);
    assert_eq!(
        state.band(newer.document_id(), Unreadable::KeyNotReceived),
        Some(MurmureEvent::Unreadable {
            segment: newer,
            cause: Unreadable::KeyNotReceived,
            from: Timestamp(20),
            to: Some(Timestamp(30)),
        })
    );
    assert!(matches!(
        state.band(older.document_id(), Unreadable::Damaged),
        Some(MurmureEvent::Unreadable {
            cause: Unreadable::Damaged,
            from: Timestamp(10),
            to: Some(Timestamp(20)),
            ..
        })
    ));
    assert_eq!(
        state.band(stored.active_id.document_id(), Unreadable::Damaged),
        None,
        "a live segment is no band"
    );
    assert_eq!(
        state.band(DocumentId::new(), Unreadable::Damaged),
        None,
        "nor is a document meta does not list"
    );
}

/// A loaded segment shows its messages, never a band.
#[test]
fn a_loaded_segment_is_no_band() {
    let stored = stored_channel();
    let mut state = open(&stored);
    let bytes = stored.segments.get(&stored.final_id).expect("bytes");
    state.load_older(stored.final_id, bytes).expect("load");
    assert_eq!(
        state.band(stored.final_id.document_id(), Unreadable::Damaged),
        None
    );
}

// --- MURMURE-514: a band asked for again ---

/// A final segment shown as a band is asked for once more, then loads in
/// place: its messages take the place of the band.
#[test]
fn a_final_band_asked_again_loads_in_place() {
    let stored = stored_channel();
    let mut state = open(&stored);
    assert_eq!(state.retry(stored.final_id), Some(stored.final_id));
    assert_eq!(state.retry(stored.final_id), None, "asked for once");
    let bytes = &stored.segments[&stored.final_id];
    let _ = state
        .apply_remote(stored.final_id.document_id(), bytes)
        .expect("kept");
    assert_eq!(
        state.older_ready(stored.final_id.document_id()),
        Ok(MurmureEvent::Messages {
            segment: stored.final_id,
            messages: Vec::new()
        })
    );
    assert_eq!(state.retry(stored.final_id), None, "already loaded");
}

/// Only a final segment `meta` lists is asked for again this way.
#[test]
fn only_a_listed_final_segment_is_asked_again() {
    let stored = stored_channel();
    let mut state = open(&stored);
    assert_eq!(state.retry(stored.active_id), None);
    assert_eq!(state.retry(stored.closing_id), None);
    assert_eq!(state.retry(SegmentId::new()), None);
    assert!(!state.lacks_closing(stored.closing_id), "held");
    assert!(!state.lacks_closing(stored.final_id), "no closing segment");
}

/// A closing segment the channel opened without is no final segment: only
/// a new opening fetches it again.
#[test]
fn a_closing_segment_opened_without_is_lacking() {
    let stored = stored_channel();
    let mut opening = opening_with_active(&stored);
    let _ = opening.give_up(stored.closing_id.document_id());
    let Ok(Opening::Open(mut state)) = opening.ready(stored.active_id.document_id()) else {
        panic!("the active segment opens the channel");
    };
    assert!(state.lacks_closing(stored.closing_id));
    assert_eq!(state.retry(stored.closing_id), None);
}

/// A segment given up still leads to the one before it (`MURMURE-221`).
#[test]
fn a_segment_given_up_still_leads_to_the_one_before() {
    let (stored, older, newer) = two_finals();
    let mut state = open(&stored);
    assert_eq!(state.request_older(stored.active_id), Some(newer));
    assert!(state.cancel_older(newer.document_id()));
    assert_eq!(state.request_older(newer), Some(older));
}

#[test]
fn an_older_segment_nobody_asked_for_is_not_ready() {
    let stored = stored_channel();
    let mut state = open(&stored);
    assert_eq!(
        state.older_ready(stored.final_id.document_id()).err(),
        Some(MurmureError::NotHeld(stored.final_id.document_id()))
    );
}

/// `MURMURE-579`: a channel takes a name of the longest length. One past it
/// makes no `Name`, so no channel is created under it.
#[test]
fn a_channel_takes_the_longest_name() {
    let longest = crate::fit_name(&"\u{1F600}".repeat(ambre_core::NAME_MAX_CHARS));
    for kind in [ChannelType::Public, ChannelType::Private] {
        let state = ChannelState::create(ChannelId::new(), kind, Some(&longest), NOW);
        assert!(state.is_ok());
    }
}
