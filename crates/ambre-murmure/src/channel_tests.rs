//! The engine core against its contract rows: the documents a channel holds,
//! the header check, the routing of a remote delta, and the send.

use std::collections::{HashMap, HashSet};

use ambre_bus::{MentionTarget, MessageDraft, MurmureEvent, Timestamp};
use ambre_core::schema::{SCHEMA_META_MAP, SCHEMA_VERSION_KEY, SchemaError};
use ambre_core::{ChannelId, DocumentId, UserId};
use loro::{Container, ExportMode, LoroDoc, LoroValue, ValueOrContainer};

use crate::channel::push_segment_entry;
use crate::schema::{
    self, CHANNEL, CHANNEL_NAME, DocumentKind, HEADER, HEADER_CHANNEL, HEADER_KIND,
    MESSAGE_CONTENT, MESSAGES, SEGMENTS, SegmentState,
};
use crate::{ChannelState, ChannelType, MurmureError, Opening, OpeningChannel, SCHEMA_VERSION};

const NOW: Timestamp = Timestamp(1_700_000_000_000);

fn draft(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
    }
}

fn snapshot(doc: &LoroDoc) -> Vec<u8> {
    doc.export(ExportMode::Snapshot).expect("snapshot")
}

/// A segment document of `channel`, as a peer would have stored it.
fn segment_doc(channel: ChannelId) -> LoroDoc {
    let doc = LoroDoc::new();
    schema::write_header(&doc, DocumentKind::Segment, channel, NOW.0).expect("header");
    doc.commit();
    doc
}

/// A stored channel with one final, one closing and one active segment.
struct Stored {
    channel: ChannelId,
    meta_id: DocumentId,
    meta: Vec<u8>,
    final_id: DocumentId,
    closing_id: DocumentId,
    active_id: DocumentId,
    segments: HashMap<DocumentId, Vec<u8>>,
}

fn stored_channel() -> Stored {
    let channel = ChannelId::new();
    let (meta_id, final_id, closing_id, active_id) = (
        DocumentId::new(),
        DocumentId::new(),
        DocumentId::new(),
        DocumentId::new(),
    );
    let meta = LoroDoc::new();
    schema::write_header(&meta, DocumentKind::Meta, channel, NOW.0).expect("header");
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
fn peer_of(state: &ChannelState) -> ChannelState {
    let segments = HashMap::from([(
        state.active_id(),
        state.snapshot(state.active_id()).expect("active"),
    )]);
    let meta = state.snapshot(state.meta_id()).expect("meta");
    ChannelState::open(state.channel_id(), state.meta_id(), &meta, &segments).expect("peer")
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
    assert_ne!(state.meta_id(), state.active_id());
    let meta = state.doc(state.meta_id()).expect("meta");
    let active = state.doc(state.active_id()).expect("active");
    let mut meta_roots = root_keys(meta);
    meta_roots.sort();
    assert_eq!(meta_roots, [CHANNEL, HEADER, SCHEMA_META_MAP, SEGMENTS]);
    let mut active_roots = root_keys(active);
    active_roots.sort();
    assert_eq!(active_roots, [HEADER, SCHEMA_META_MAP]);
}

#[test]
fn every_document_carries_a_header_and_the_foundation_version() {
    let channel = ChannelId::new();
    let state = ChannelState::create(channel, ChannelType::Private, None, NOW).expect("create");
    for (id, kind) in [
        (state.meta_id(), DocumentKind::Meta),
        (state.active_id(), DocumentKind::Segment),
    ] {
        let doc = state.doc(id).expect("held");
        let header = doc.get_map(HEADER);
        assert_eq!(
            schema::string(&header, HEADER_KIND).as_deref(),
            Some(kind.as_str())
        );
        assert_eq!(
            schema::string(&header, HEADER_CHANNEL),
            Some(channel.to_string())
        );
        assert_eq!(
            schema::unsigned(&doc.get_map(SCHEMA_META_MAP), SCHEMA_VERSION_KEY),
            Some(u64::from(SCHEMA_VERSION))
        );
    }
}

#[test]
fn an_open_channel_holds_meta_the_active_and_the_closing_segment() {
    let stored = stored_channel();
    let state = open(&stored);
    assert!(state.holds(stored.meta_id));
    assert!(state.holds(stored.active_id));
    assert!(state.holds(stored.closing_id));
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
        Some(MurmureError::SegmentMissing(stored.closing_id))
    );
}

// --- MURMURE-028, MURMURE-023 ---

#[test]
fn opening_a_channel_never_loads_a_final_segment() {
    let stored = stored_channel();
    // The final segment's bytes are on hand, and still not loaded.
    assert!(stored.segments.contains_key(&stored.final_id));
    let state = open(&stored);
    assert!(!state.holds(stored.final_id));
}

#[test]
fn an_older_segment_loads_only_when_the_view_asks() {
    let stored = stored_channel();
    let mut state = open(&stored);
    assert!(!state.holds(stored.final_id));
    assert_eq!(state.older_than(stored.active_id), Some(stored.final_id));
    let bytes = stored.segments.get(&stored.final_id).expect("bytes");
    let event = state.load_older(stored.final_id, bytes).expect("load");
    assert!(state.holds(stored.final_id));
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
    assert!(!state.holds(stored.final_id));
    assert_eq!(state.final_segments(), [stored.final_id]);
}

#[test]
fn a_live_segment_is_not_an_older_one() {
    let stored = stored_channel();
    let mut state = open(&stored);
    let bytes = stored.segments.get(&stored.closing_id).expect("bytes");
    assert_eq!(
        state.load_older(stored.closing_id, bytes).err(),
        Some(MurmureError::NotOlder(stored.closing_id))
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
    let map = remote
        .get_list(MESSAGES)
        .push_container(loro::LoroMap::new())
        .expect("push");
    map.insert("id", "x").expect("insert");
    remote.commit();
    let delta = remote.export(ExportMode::updates(&before)).expect("delta");

    let active_before = state
        .doc(stored.active_id)
        .expect("active")
        .get_deep_value();
    let meta_before = state.doc(stored.meta_id).expect("meta").get_deep_value();
    state
        .apply_remote(stored.closing_id, &delta)
        .expect("apply");
    assert_eq!(
        state
            .doc(stored.closing_id)
            .expect("closing")
            .get_list(MESSAGES)
            .len(),
        1
    );
    assert_eq!(
        state
            .doc(stored.active_id)
            .expect("active")
            .get_deep_value(),
        active_before
    );
    assert_eq!(
        state.doc(stored.meta_id).expect("meta").get_deep_value(),
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
        state.apply_remote(stored.final_id, &[]).err(),
        Some(MurmureError::NotHeld(stored.final_id))
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
            document_id: stored.active_id,
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
        .get_map(SCHEMA_META_MAP)
        .insert(SCHEMA_VERSION_KEY, i64::from(SCHEMA_VERSION) + 1)
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
            document: SCHEMA_VERSION + 1,
            binary: SCHEMA_VERSION,
        }))
    );
}

#[test]
fn a_document_with_no_version_finds_no_migration_and_is_refused() {
    let stored = stored_channel();
    let bare = LoroDoc::new();
    bare.get_map(HEADER)
        .insert(HEADER_KIND, "meta")
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
            from: 0
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
    let mut state =
        ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW).expect("create");
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
    let mut state =
        ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW).expect("create");
    let (bob, carol) = (UserId::new(), UserId::new());
    let with_mentions = MessageDraft {
        text: "@bob @carol @bob @channel".to_owned(),
        mentions: vec![
            MentionTarget::User(bob),
            MentionTarget::User(carol),
            MentionTarget::User(bob),
            MentionTarget::Channel,
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
    let mut state =
        ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW).expect("create");
    state.send(&draft("one"), UserId::new(), NOW).expect("send");
    state.send(&draft("two"), UserId::new(), NOW).expect("send");
    let list = state
        .doc(state.active_id())
        .expect("active")
        .get_list(MESSAGES);
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
    let mut alice =
        ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW).expect("create");
    let mut bob = peer_of(&alice);
    let sent = alice
        .send(&draft("hi bob"), UserId::new(), NOW)
        .expect("send");
    let events = bob.apply_remote(sent.segment, &sent.delta).expect("apply");
    assert_eq!(
        events,
        [MurmureEvent::Messages {
            segment: alice.active_id(),
            messages: alice.messages(alice.active_id()).expect("messages"),
        }]
    );
}

// --- MURMURE-004, MURMURE-005 ---

#[test]
fn the_body_lives_in_a_text_and_no_map_holds_it() {
    let mut state =
        ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW).expect("create");
    state
        .send(&draft("the body"), UserId::new(), NOW)
        .expect("send");
    let list = state
        .doc(state.active_id())
        .expect("active")
        .get_list(MESSAGES);
    let Some(ValueOrContainer::Container(Container::Map(message))) = list.get(0) else {
        panic!("a message is a map container");
    };
    assert!(matches!(
        message.get(MESSAGE_CONTENT),
        Some(ValueOrContainer::Container(Container::Text(_)))
    ));
    let mut plain_strings = Vec::new();
    message.for_each(|_, v| {
        if let ValueOrContainer::Value(LoroValue::String(s)) = v {
            plain_strings.push(s.to_string());
        }
    });
    assert!(!plain_strings.iter().any(|s| s.contains("the body")));
}

#[test]
fn two_peers_typing_in_one_message_keep_both_texts() {
    let mut alice =
        ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW).expect("create");
    let mut bob = peer_of(&alice);
    let sent = alice
        .send(&draft("base"), UserId::new(), NOW)
        .expect("send");
    bob.apply_remote(sent.segment, &sent.delta).expect("apply");

    let segment = alice.active_id();
    let type_into = |state: &ChannelState, at: usize, text: &str| -> Vec<u8> {
        let doc = state.doc(segment).expect("segment");
        let before = doc.oplog_vv();
        let Some(ValueOrContainer::Container(Container::Map(message))) =
            doc.get_list(MESSAGES).get(0)
        else {
            panic!("a message is a map container");
        };
        let Some(ValueOrContainer::Container(Container::Text(content))) =
            message.get(MESSAGE_CONTENT)
        else {
            panic!("the content is a text");
        };
        content.insert(at, text).expect("type");
        doc.commit();
        doc.export(ExportMode::updates(&before)).expect("delta")
    };
    let from_alice = type_into(&alice, 0, "A:");
    let from_bob = type_into(&bob, 4, ":B");
    alice.apply_remote(segment, &from_bob).expect("apply");
    bob.apply_remote(segment, &from_alice).expect("apply");

    let on_alice = alice.messages(segment).expect("alice");
    let on_bob = bob.messages(segment).expect("bob");
    assert_eq!(on_alice, on_bob);
    assert_eq!(on_alice[0].text, "A:base:B");
}

// --- MURMURE-015, MURMURE-018 ---

#[test]
fn meta_holds_no_member_list() {
    let state = ChannelState::create(ChannelId::new(), ChannelType::Private, Some("team"), NOW)
        .expect("create");
    let meta = state.doc(state.meta_id()).expect("meta");
    let text = format!("{:?}", meta.get_deep_value());
    assert!(!text.contains("member"));
    let mut roots = root_keys(meta);
    roots.sort();
    assert_eq!(roots, [CHANNEL, HEADER, SCHEMA_META_MAP, SEGMENTS]);
}

#[test]
fn an_unset_property_is_an_absent_key() {
    let unnamed =
        ChannelState::create(ChannelId::new(), ChannelType::Direct, None, NOW).expect("create");
    let channel = unnamed
        .doc(unnamed.meta_id())
        .expect("meta")
        .get_map(CHANNEL);
    assert!(channel.get(CHANNEL_NAME).is_none());
    let named = ChannelState::create(ChannelId::new(), ChannelType::Public, Some("general"), NOW)
        .expect("create");
    let channel = named.doc(named.meta_id()).expect("meta").get_map(CHANNEL);
    assert_eq!(
        schema::string(&channel, CHANNEL_NAME).as_deref(),
        Some("general")
    );
}

// --- robustness ---

#[test]
fn a_malformed_message_is_skipped_and_the_rest_still_reads() {
    let mut state =
        ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW).expect("create");
    let doc = state.doc(state.active_id()).expect("active");
    doc.get_list(MESSAGES).push("not a message").expect("push");
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
    assert!(opening.receive(stored.meta_id, &stored.meta));

    let Ok(Opening::OpenSegments(asked)) = opening.ready(stored.meta_id) else {
        panic!("meta names the live segments");
    };
    let asked: HashSet<DocumentId> = asked.into_iter().collect();
    assert_eq!(
        asked,
        HashSet::from([stored.closing_id, stored.active_id]),
        "the live segments and no final one"
    );
    assert!(!opening.expects(stored.final_id));

    for id in [stored.closing_id, stored.active_id] {
        assert!(opening.receive(id, &stored.segments[&id]));
    }
    assert!(matches!(
        opening.ready(stored.closing_id),
        Ok(Opening::Waiting)
    ));
    let Ok(Opening::Open(state)) = opening.ready(stored.active_id) else {
        panic!("the last live segment opens the channel");
    };
    assert_eq!(state.active_id(), stored.active_id);
    assert_eq!(state.closing_id(), Some(stored.closing_id));
    assert!(!state.holds(stored.final_id));
}

/// A delta on `meta` that lands between its ready and the channel's open is
/// kept, not dropped.
#[test]
fn a_meta_delta_during_the_opening_reaches_the_open_channel() {
    let stored = stored_channel();
    let mut opening = OpeningChannel::new(stored.channel, stored.meta_id);
    opening.receive(stored.meta_id, &stored.meta);
    let _ = opening.ready(stored.meta_id).expect("meta");

    let editor = LoroDoc::new();
    editor.import(&stored.meta).expect("import");
    let before = editor.oplog_vv();
    editor
        .get_map(CHANNEL)
        .insert(CHANNEL_NAME, "renamed")
        .expect("rename");
    editor.commit();
    let delta = editor.export(ExportMode::updates(&before)).expect("delta");
    assert!(opening.receive(stored.meta_id, &delta));

    opening.receive(stored.closing_id, &stored.segments[&stored.closing_id]);
    opening.receive(stored.active_id, &stored.segments[&stored.active_id]);
    let _ = opening.ready(stored.closing_id).expect("closing");
    let Ok(Opening::Open(state)) = opening.ready(stored.active_id) else {
        panic!("the last live segment opens the channel");
    };
    let meta = state.doc(stored.meta_id).expect("meta");
    assert_eq!(
        schema::string(&meta.get_map(CHANNEL), CHANNEL_NAME).as_deref(),
        Some("renamed")
    );
}

#[test]
fn a_meta_of_another_channel_ends_the_opening() {
    let stored = stored_channel();
    let mut opening = OpeningChannel::new(ChannelId::new(), stored.meta_id);
    opening.receive(stored.meta_id, &stored.meta);
    assert!(matches!(
        opening.ready(stored.meta_id),
        Err(MurmureError::ForeignChannel { .. })
    ));
}

#[test]
fn an_opening_refuses_a_document_it_did_not_ask_for() {
    let stored = stored_channel();
    let mut opening = OpeningChannel::new(stored.channel, stored.meta_id);
    assert!(!opening.receive(stored.active_id, &stored.segments[&stored.active_id]));
    assert_eq!(
        opening.ready(stored.active_id).err(),
        Some(MurmureError::NotHeld(stored.active_id))
    );
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
    assert!(state.holds(stored.final_id) && state.is_pending(stored.final_id));
    let produced = state
        .apply_remote(stored.final_id, &stored.segments[&stored.final_id])
        .expect("kept");
    assert!(
        produced.is_empty(),
        "nothing reads a segment still arriving"
    );

    let event = state.older_ready(stored.final_id).expect("load");
    assert_eq!(
        event,
        MurmureEvent::Messages {
            segment: stored.final_id,
            messages: Vec::new()
        }
    );
    assert!(!state.is_pending(stored.final_id));
    assert_eq!(
        state.request_older(stored.active_id),
        None,
        "already loaded"
    );
}

#[test]
fn an_older_segment_nobody_asked_for_is_not_ready() {
    let stored = stored_channel();
    let mut state = open(&stored);
    assert_eq!(
        state.older_ready(stored.final_id).err(),
        Some(MurmureError::NotHeld(stored.final_id))
    );
}
