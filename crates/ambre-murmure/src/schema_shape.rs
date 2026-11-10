//! The schema shape belt: the key set and the value types a build writes,
//! frozen per `SCHEMA_VERSION` (`MURMURE-437`, `SCHEMA-018`).
//!
//! A shape is a channel and a space with every value removed: one line per
//! reachable path, each naming the type under it. A line starts with the
//! document kind, since a channel is several documents (`MURMURE-021`). `*`
//! stands for a list index, and `{id}` for the identifier keying a map entry.
//!
//! The walk reads containers, not the deep value. A deep value turns a
//! `LoroText` into a string. A body stored as a plain string is the drift
//! `MURMURE-004` forbids, so the two must render apart.
//!
//! [`every_declared_key_appears_in_the_shape`] reads `schema.rs` and demands a
//! line for every key it declares, so a key no scenario reaches cannot drop
//! out of the freeze unseen.
//!
//! **The scenario is fixed.** Its PeerIDs, its registry entries and its
//! author are constants, so the reading belt in `schema_reading.rs` can
//! freeze the values the same scenario stores.
//!
//! The same file holds the two `AUTHOR-024` belts. Murmure writes no
//! scaffolding under a shared identity: a document is written once, by the
//! peer that creates it, under its session. The scaffold belt freezes that
//! answer. The migration belt freezes what each step writes, and the registry
//! holds no step yet.

use ambre_bus::Char;
use ambre_core::Validated;
use ambre_core::crdt::CrdtName;
use ambre_core::index::Span;
use std::collections::BTreeSet;

use crate::message::{Attachment, Mention, MentionTarget, MessageDraft, StyleMark, TextStyle};
use ambre_bus::{
    Authored, ChannelTopic, ContentHash, Instant, MediaType, Octets, SealedChannelKey,
};
use ambre_core::Count;
use ambre_core::authorship::{ENTRY_SIGNATURE_LEN, PeerEntry, SESSION_NONCE_LEN};
use ambre_core::id::{Category, Channel, Document, Id, Message, Segment, Space, User};
use ambre_core::schema::{migration_peer, step_fingerprint};
use ambre_core::{FixedBytes, Payload, PeerEntryBytes};
use loro::{Container, ExportMode, LoroDoc, LoroValue, TextDelta, ValueOrContainer};

use crate::schema::{DocumentKind, MIGRATIONS, SCHEMA_VERSION};
use crate::{ChangeTarget, ChannelState, ChannelType, Listing, SpaceState, ThreadTarget};

/// The frozen shapes, one section per `SCHEMA_VERSION`.
const FROZEN: &str = include_str!("../schema/shapes.txt");

/// The frozen scaffolding identities, one section per `SCHEMA_VERSION`.
const SCAFFOLD: &str = include_str!("../schema/scaffold.txt");

/// What each migration step writes, one section per `SCHEMA_VERSION`.
const STEPS: &str = include_str!("../schema/migrations.txt");

/// The call of the call message of the scenario.
pub(crate) const SCENARIO_CALL: Id<ambre_core::id::Call> =
    Id::<ambre_core::id::Call>::from_bytes([0xCA; 16]);

/// The segment a list index collapses to.
pub(crate) const ANY: &str = "*";

/// The segment an identifier key collapses to.
pub(crate) const ID: &str = "{id}";

/// The PeerIDs the scenario's documents are written under.
pub(crate) const META_PEER: u64 = 0x11;
pub(crate) const SEGMENT_PEER: u64 = 0x12;
pub(crate) const SPACE_PEER: u64 = 0x13;
pub(crate) const SUCCESSOR_PEER: u64 = 0x14;
pub(crate) const AMENDMENTS_PEER: u64 = 0x15;
pub(crate) const THREAD_PEER: u64 = 0x16;

/// The user who writes every document of the scenario.
pub(crate) const AUTHOR: Id<User> = Id::<User>::from_bytes([0xa1; 16]);

/// The member the scenario's notice announces.
pub(crate) const MEMBER: Id<User> = Id::<User>::from_bytes([0xb2; 16]);

/// The registry entry the scenario's session carries: its author, a fixed
/// nonce and a fixed signature, so the stored bytes are the same each run.
pub(crate) fn entry() -> Vec<u8> {
    PeerEntry {
        user: AUTHOR,
        nonce: FixedBytes::from_array([0x5a; SESSION_NONCE_LEN]),
        signature: FixedBytes::from_array([0xc3; ENTRY_SIGNATURE_LEN]),
    }
    .to_bytes()
}

/// The maps keyed by an identifier, whose keys are values and not schema.
/// The PeerID registry each document carries is one of them
/// (`ambre-spec-op-authorship.md section 3`).
pub(crate) const KEYED_BY_ID: [&str; 12] = [
    "amendments:amendments",
    "amendments:peers",
    "amendments:threads",
    "meta:peers",
    "segment:messages/*/call_end",
    "segment:messages/*/reactions",
    "segment:peers",
    "space:categories",
    "space:channels",
    "space:peers",
    "thread:messages/*/reactions",
    "thread:peers",
];

/// The declared `&str` constants that are stored values, not keys.
const NOT_SCHEMA_KEYS: [&str; 0] = [];

/// A channel's shape: the frozen lines, and the key segments met on the way.
#[derive(Default)]
struct Shape {
    lines: BTreeSet<String>,
    keys: BTreeSet<String>,
}

/// The type a plain value holds, in the spelling the freeze uses.
fn type_name(value: &LoroValue) -> &'static str {
    match value {
        LoroValue::Null => "null",
        LoroValue::Bool(_) => "bool",
        LoroValue::Double(_) => "f64",
        LoroValue::I64(_) => "i64",
        LoroValue::Binary(_) => "binary",
        LoroValue::String(_) => "string",
        LoroValue::List(_) => "list",
        LoroValue::Map(_) => "map",
        LoroValue::Container(_) => "container",
    }
}

impl Shape {
    /// Record `path`, then descend into it when it is a container.
    fn walk(&mut self, path: &str, item: ValueOrContainer) {
        match item {
            ValueOrContainer::Value(value) => {
                self.lines.insert(format!("{path}: {}", type_name(&value)));
            }
            ValueOrContainer::Container(Container::Map(map)) => {
                self.lines.insert(format!("{path}: map"));
                let by_id = KEYED_BY_ID.contains(&path);
                map.for_each(|key, child| {
                    if by_id {
                        self.walk(&format!("{path}/{ID}"), child);
                    } else {
                        self.keys.insert(key.to_owned());
                        self.walk(&format!("{path}/{key}"), child);
                    }
                });
            }
            ValueOrContainer::Container(Container::List(list)) => {
                self.lines.insert(format!("{path}: list"));
                list.for_each(|child| self.walk(&format!("{path}/{ANY}"), child));
            }
            ValueOrContainer::Container(Container::Text(text)) => {
                self.lines.insert(format!("{path}: text"));
                // A mark is stored in the body, so its key and value type
                // are part of the shape too: `#` names the mark.
                for span in text.to_delta() {
                    let TextDelta::Insert {
                        attributes: Some(attributes),
                        ..
                    } = span
                    else {
                        continue;
                    };
                    for (key, value) in attributes {
                        self.lines
                            .insert(format!("{path}#{key}: {}", type_name(&value)));
                        self.keys.insert(key);
                    }
                }
            }
            // Murmure writes no tree, movable list or counter. Naming the
            // container kind still moves a line if one ever appears.
            ValueOrContainer::Container(other) => {
                self.lines.insert(format!("{path}: {:?}", other.get_type()));
            }
        }
    }

    /// Walk every root of one document, under its kind.
    fn walk_doc(&mut self, kind: DocumentKind, doc: &LoroDoc) {
        let LoroValue::Map(roots) = doc.get_value() else {
            return;
        };
        for (name, root) in roots.iter() {
            let LoroValue::Container(id) = root else {
                continue;
            };
            let Some(container) = doc.get_container(id.clone()) else {
                continue;
            };
            self.keys.insert(name.clone());
            self.walk(
                &format!("{}:{name}", kind.as_str()),
                ValueOrContainer::Container(container),
            );
        }
    }
}

/// A channel that reaches every key the schema declares: a name, and a
/// message in the active segment, with a reaction.
pub(crate) fn channel_of_every_key() -> ChannelState {
    let mut state = ChannelState::create(
        Id::<Channel>::mint(),
        ChannelType::Public,
        Some(&crate::fit_name("general")),
        Instant::<Authored>::new(1_700_000_000_000),
    )
    .expect("create");
    for (id, peer) in [
        (state.meta_id().document_id(), META_PEER),
        (state.active_id().document_id(), SEGMENT_PEER),
    ] {
        state
            .adopt_session(
                id,
                ambre_core::PeerId::new(peer),
                Payload::<PeerEntryBytes>::new(entry()),
            )
            .expect("adopt");
    }
    // The first message carries a file, by its address alone.
    let file = Attachment {
        media_ref: Validated::<ContentHash>::parse("ab".repeat(32)).expect("an address"),
        name: "plan.pdf".to_owned(),
        media_type: MediaType::new("application/pdf".to_owned()),
        size: Count::<Octets>::new(2_048),
    };
    state
        .send_attached(
            &MessageDraft {
                text: "hello @channel".to_owned(),
                styles: Vec::new(),
                mentions: vec![Mention {
                    target: MentionTarget::Channel,
                    range: Span::<Char>::from_usize_range(6..14),
                }],
            },
            &[file],
            AUTHOR,
            Instant::<Authored>::new(1_700_000_000_001),
        )
        .expect("send");
    let segment = state.active_id();
    let message = state.messages(segment).expect("held")[0].id;
    state
        .react(
            segment,
            message,
            &ambre_bus::Emoji::from("\u{1F44D}"),
            AUTHOR,
            Instant::<Authored>::new(1_700_000_000_002),
        )
        .expect("react");
    state
        .announce(MEMBER, AUTHOR, Instant::<Authored>::new(1_700_000_000_003))
        .expect("announce");
    // A third message the author edits, and a fourth one she deletes.
    for (text, ts) in [("draft", 1_700_000_000_004), ("gone", 1_700_000_000_005)] {
        let draft = MessageDraft {
            text: text.to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
        };
        state
            .send(&draft, AUTHOR, Instant::<Authored>::new(ts))
            .expect("send");
    }
    let messages = state.messages(segment).expect("held");
    let edited = MessageDraft {
        text: "edited".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    };
    state
        .edit(segment, messages[2].id, &edited, AUTHOR)
        .expect("edit");
    state
        .delete(segment, messages[3].id, AUTHOR)
        .expect("delete");
    // A call message, whose last leave wrote its end (`MURMURE-408`, `MURMURE-591`).
    state
        .post_call(
            SCENARIO_CALL,
            AUTHOR,
            Instant::<Authored>::new(1_700_000_000_006),
        )
        .expect("a call message");
    state
        .end_call(
            SCENARIO_CALL,
            AUTHOR,
            Instant::<Authored>::new(1_700_000_000_007),
        )
        .expect("an end");
    reply_in_thread(&mut state, segment, messages[0].id);
    amend_after_lock(&mut state, segment);
    state
}

/// Start the thread of `root`, link it as the relay does, and reply in it
/// with a copy sent to the channel.
fn reply_in_thread(state: &mut ChannelState, segment: Id<Segment>, root: Id<Message>) {
    let ThreadTarget::Create(thread) = state
        .reply_target(root, Instant::<Authored>::new(1_700_000_000_005))
        .expect("target")
    else {
        panic!("a root with no thread creates one");
    };
    state
        .adopt_session(
            thread,
            ambre_core::PeerId::new(THREAD_PEER),
            Payload::<PeerEntryBytes>::new(entry()),
        )
        .expect("adopt");
    state.thread_linked(segment, root, thread).expect("link");
    let reply = MessageDraft {
        text: "a reply".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    };
    state
        .reply(
            root,
            &reply,
            AUTHOR,
            Instant::<Authored>::new(1_700_000_000_005),
            crate::message::ReplyEcho::AlsoInChannel,
        )
        .expect("reply");
}

/// The root of the scenario's thread: the first message of its segment. It
/// is read from the stored list, so a belt that breaks its body finds it.
pub(crate) fn scenario_root(state: &ChannelState) -> Option<Id<Message>> {
    let doc = state.doc(amended(state).0.document_id())?;
    let message = crate::schema::root_list(doc, crate::schema::Root::Messages)
        .get(0)
        .and_then(crate::schema::fields_of)?;
    let id = crate::schema::string(&message, crate::schema::MessageKey::Id)?;
    Id::<Message>::parse_str(&id).ok()
}

/// The thread document of the scenario: the one of its first message.
pub(crate) fn scenario_thread(state: &ChannelState) -> Id<Document> {
    scenario_root(state)
        .and_then(|root| state.thread_id(root))
        .expect("the first message has a thread")
}

/// Close `segment`, lock it as the relay does, and amend it: the first
/// message edited, the third one deleted.
fn amend_after_lock(state: &mut ChannelState, segment: Id<Segment>) {
    let successor = Id::<Segment>::mint();
    state
        .close_into(
            segment,
            successor,
            crate::Successor::Created,
            Instant::<Authored>::new(1_700_000_000_006),
        )
        .expect("close");
    state
        .adopt_session(
            successor.document_id(),
            ambre_core::PeerId::new(SUCCESSOR_PEER),
            Payload::<PeerEntryBytes>::new(entry()),
        )
        .expect("adopt");
    state
        .relocate_locked(segment.document_id(), &[])
        .expect("lock");
    let ChangeTarget::Create(amendments) = state
        .change_target(segment, Instant::<Authored>::new(1_700_000_000_007))
        .expect("target")
    else {
        panic!("a locked segment with no amendments creates them");
    };
    state
        .adopt_session(
            amendments,
            ambre_core::PeerId::new(AMENDMENTS_PEER),
            Payload::<PeerEntryBytes>::new(entry()),
        )
        .expect("adopt");
    state.amendments_linked(segment, amendments).expect("link");
    let messages = state.messages(segment).expect("held");
    // Every style over "hi", so each key is one run of its own.
    let styles = [
        TextStyle::Bold,
        TextStyle::Italic,
        TextStyle::Code,
        TextStyle::Link("https://ambre.dev".into()),
        TextStyle::CodeBlock,
        TextStyle::Quote,
        TextStyle::List,
    ];
    let amended = MessageDraft {
        text: "hi @channel".to_owned(),
        styles: styles
            .into_iter()
            .map(|style| StyleMark {
                style,
                range: Span::<Char>::from_usize_range(0..2),
            })
            .collect(),
        mentions: vec![Mention {
            target: MentionTarget::Channel,
            range: Span::<Char>::from_usize_range(3..11),
        }],
    };
    state
        .edit(segment, messages[0].id, &amended, AUTHOR)
        .expect("amend");
    state
        .delete(segment, messages[2].id, AUTHOR)
        .expect("amend");
    // A first reply to a locked root records its thread (`MURMURE-484`).
    let root = messages[1].id;
    let ThreadTarget::Create(thread) = state
        .reply_target(root, Instant::<Authored>::new(1_700_000_000_008))
        .expect("target")
    else {
        panic!("a locked root with no thread creates one");
    };
    state.thread_linked(segment, root, thread).expect("record");
    // A fold writes the segment's header and the `meta` entry
    // (`MURMURE-075`, `MURMURE-076`).
    let amendments = state.amendments_id(segment).expect("amended");
    state.fold_in_place(segment).expect("fold");
    state.mark_folded(segment, amendments).expect("mark");
}

/// The segment of the scenario that holds its messages, and its
/// amendments document.
pub(crate) fn amended(state: &ChannelState) -> (Id<Segment>, Id<Document>) {
    let segment = state
        .closing_id()
        .expect("the scenario's segment is closing");
    let amendments = state.amendments_id(segment).expect("amended");
    (segment, amendments)
}

/// A space that reaches every key the schema declares: a name, a category,
/// and a listed channel with a topic and a category.
pub(crate) fn space_of_every_key(channel: &ChannelState) -> SpaceState {
    let mut space = SpaceState::create(
        Id::<Space>::mint(),
        &crate::fit_name("Acme"),
        Instant::<Authored>::new(1_700_000_000_000),
    )
    .expect("create");
    space
        .adopt_session(
            ambre_core::PeerId::new(SPACE_PEER),
            Payload::<PeerEntryBytes>::new(entry()),
        )
        .expect("adopt");
    let category = Id::<Category>::mint();
    space
        .put_category(category, &crate::fit_name("Teams"), ambre_bus::Rank::new(0))
        .expect("category");
    let topic = ambre_core::Validated::<ChannelTopic>::parse("everything").expect("a topic");
    space
        .list_channel(
            channel,
            &Listing {
                topic: Some(&topic),
                category: Some(category),
                position: ambre_bus::Rank::new(0),
                key: SealedChannelKey::from_bytes(vec![1; 72]),
            },
        )
        .expect("list");
    space
}

fn shape_of(state: &ChannelState) -> Shape {
    let mut shape = Shape::default();
    let (segment, amendments) = amended(state);
    for (kind, id) in [
        (DocumentKind::Meta, state.meta_id().document_id()),
        (DocumentKind::Segment, segment.document_id()),
        (DocumentKind::Amendments, amendments),
        (DocumentKind::Thread, scenario_thread(state)),
    ] {
        shape.walk_doc(kind, state.doc(id).expect("held"));
    }
    shape.walk_doc(DocumentKind::Space, space_of_every_key(state).doc());
    shape
}

fn render(shape: &Shape) -> String {
    let mut out = String::new();
    for line in &shape.lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The section of the shape freeze that names this version, if any.
fn frozen_section(version: ambre_core::Version<ambre_core::schema::Schema>) -> Option<String> {
    section_of(FROZEN, version)
}

/// The section of a frozen text that names this version, if any.
pub(crate) fn section_of(
    frozen: &str,
    version: ambre_core::Version<ambre_core::schema::Schema>,
) -> Option<String> {
    let header = format!("== v{version}");
    let mut taken = String::new();
    let mut inside = false;
    for line in frozen.lines() {
        if line.starts_with("== ") {
            if inside {
                break;
            }
            inside = line.trim_end() == header;
            continue;
        }
        if inside && !line.is_empty() {
            taken.push_str(line);
            taken.push('\n');
        }
    }
    inside.then_some(taken)
}

/// Every name the schema declares: each root, each key of each map and each
/// mark, read from the `ALL` of its set. A name declared without a variant
/// cannot exist, so no name escapes the belts.
pub(crate) fn declared_keys() -> Vec<String> {
    use crate::schema::{
        AmendmentKey, AttachmentKey, CategoryKey, ChannelKey, EntryKey, HeaderKey, Mark,
        MessageKey, Root, SegmentKey, SpaceKey,
    };
    let names = [
        Root::ALL.iter().map(|n| n.as_str()).collect::<Vec<_>>(),
        HeaderKey::ALL.iter().map(|n| n.as_str()).collect(),
        SpaceKey::ALL.iter().map(|n| n.as_str()).collect(),
        CategoryKey::ALL.iter().map(|n| n.as_str()).collect(),
        EntryKey::ALL.iter().map(|n| n.as_str()).collect(),
        ChannelKey::ALL.iter().map(|n| n.as_str()).collect(),
        SegmentKey::ALL.iter().map(|n| n.as_str()).collect(),
        AmendmentKey::ALL.iter().map(|n| n.as_str()).collect(),
        MessageKey::ALL.iter().map(|n| n.as_str()).collect(),
        AttachmentKey::ALL.iter().map(|n| n.as_str()).collect(),
        Mark::ALL.iter().map(|n| n.as_str()).collect(),
    ];
    names
        .into_iter()
        .flatten()
        .filter(|name| !NOT_SCHEMA_KEYS.contains(name))
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_shape_a_build_writes_is_the_shape_frozen_for_its_version() {
    let written = render(&shape_of(&channel_of_every_key()));
    let Some(frozen) = frozen_section(SCHEMA_VERSION) else {
        panic!(
            "`schema/shapes.txt` carries no `== v{SCHEMA_VERSION}` section. \
             The version rose without a freeze; add the section below:\n{written}"
        );
    };
    assert_eq!(
        written, frozen,
        "the shape this build writes is not the shape frozen for \
         SCHEMA_VERSION {SCHEMA_VERSION}. A key or a value type moved, so raise \
         SCHEMA_VERSION and freeze the new shape as its own section (`MURMURE-020`)."
    );
}

/// Who wrote one document of the scenario: its session alone, or the other
/// identities its operations carry, as hex.
fn writers(kind: DocumentKind, doc: &LoroDoc, session: u64) -> String {
    let mut others: Vec<String> = doc
        .oplog_vv()
        .keys()
        .copied()
        .filter(|peer| *peer != session)
        .map(|peer| format!("{peer:016x}"))
        .collect();
    others.sort();
    if others.is_empty() {
        format!("{}: its session alone\n", kind.as_str())
    } else {
        format!("{}: its session and {}\n", kind.as_str(), others.join(" "))
    }
}

/// Every build of one version writes the same scaffolding (`AUTHOR-024`).
/// Murmure's answer is that it writes none: every operation of a document
/// is its session's, so no other build can write it under another identity.
/// A scaffolding written under a shared identity moves a line.
#[test]
fn the_scaffold_a_build_writes_is_the_scaffold_frozen_for_its_version() {
    let channel = channel_of_every_key();
    let space = space_of_every_key(&channel);
    let (segment, amendments) = amended(&channel);
    let written = [
        writers(
            DocumentKind::Meta,
            channel.doc(channel.meta_id().document_id()).expect("held"),
            META_PEER,
        ),
        writers(
            DocumentKind::Segment,
            channel.doc(segment.document_id()).expect("held"),
            SEGMENT_PEER,
        ),
        writers(
            DocumentKind::Amendments,
            channel.doc(amendments).expect("held"),
            AMENDMENTS_PEER,
        ),
        writers(
            DocumentKind::Thread,
            channel.doc(scenario_thread(&channel)).expect("held"),
            THREAD_PEER,
        ),
        writers(DocumentKind::Space, space.doc(), SPACE_PEER),
    ]
    .concat();
    assert_eq!(
        section_of(SCAFFOLD, SCHEMA_VERSION).as_deref(),
        Some(written.as_str()),
        "the identities this build writes a document under are not the ones \
         frozen for SCHEMA_VERSION {SCHEMA_VERSION} in `schema/scaffold.txt`. \
         Another build of that version writes it otherwise, and a delta that \
         waits on it never applies. Raise SCHEMA_VERSION and append a section."
    );
}

/// The canonical document of the version a step leaves: that version's
/// layout, holding what the step rewrites. Each step reads its own, so a step
/// added later never moves what an earlier one writes. The registry holds no
/// step yet, so no version has one.
fn canonical(from: ambre_core::Version<ambre_core::schema::Schema>) -> LoroDoc {
    panic!("the step from version {from} has no canonical document: write one here")
}

/// Each step of the registry, run as the gate runs it on its canonical
/// document. One line per step: `from -> to` and what it wrote.
fn steps_written() -> Vec<String> {
    MIGRATIONS
        .iter()
        .map(|step| {
            let stored = canonical(step.from)
                .export(ExportMode::Snapshot)
                .expect("a snapshot");
            let doc = LoroDoc::new();
            doc.import(&stored).expect("the snapshot imports");
            let heads = doc.oplog_frontiers();
            doc.set_peer_id(
                migration_peer(heads.iter().map(|id| {
                    (
                        ambre_core::PeerId::new(id.peer),
                        ambre_core::OpCounter::new(id.counter),
                    )
                }))
                .get(),
            )
            .expect("the migration identity");
            let before = doc.oplog_vv();
            (step.apply)(&doc).expect("the step runs on its canonical document");
            let written = doc
                .export(ExportMode::updates(&before))
                .expect("the step's writes export");
            format!(
                "{} -> {} {}",
                step.from,
                step.to,
                step_fingerprint(&written)
            )
        })
        .collect()
}

/// Every build of one version runs each migration step alike (`AUTHOR-024`).
/// Version 1 runs none, and its section is empty. A step writes nothing
/// (`MURMURE-566`), so each line holds the fingerprint of no operation.
#[test]
fn each_migration_step_writes_what_its_version_froze() {
    let listed: String = steps_written()
        .iter()
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(
        section_of(STEPS, SCHEMA_VERSION).as_deref(),
        Some(listed.as_str()),
        "a migration step this build runs writes other operations than the \
         ones frozen for SCHEMA_VERSION {SCHEMA_VERSION} in \
         `schema/migrations.txt`. A peer on another build of that version \
         migrates the same document another way. Restore the step, or raise \
         SCHEMA_VERSION and append a section."
    );
}

#[test]
fn every_declared_key_appears_in_the_shape() {
    let shape = shape_of(&channel_of_every_key());
    let declared = declared_keys();
    assert!(!declared.is_empty(), "the declaration reader found no key");
    for key in declared {
        assert!(
            shape.keys.contains(&key),
            "`{key}` is declared as a schema key and no line of the shape \
             carries it. The scenario has to write it before the freeze can hold it."
        );
    }
}

#[test]
fn a_body_stored_as_a_string_moves_the_shape() {
    let state = channel_of_every_key();
    let before = render(&shape_of(&state));
    let doc = state.doc(amended(&state).0.document_id()).expect("held");
    let Some(message) = crate::schema::root_list(doc, crate::schema::Root::Messages)
        .get(0)
        .and_then(crate::schema::fields_of)
    else {
        panic!("a message is a map container");
    };
    message
        .insert(crate::schema::MessageKey::Content, "flat")
        .expect("overwrite");
    doc.commit();
    assert_ne!(render(&shape_of(&state)), before);
}

#[test]
fn a_value_changing_leaves_the_shape_alone() {
    assert_eq!(
        render(&shape_of(&channel_of_every_key())),
        render(&shape_of(&channel_of_every_key()))
    );
}
