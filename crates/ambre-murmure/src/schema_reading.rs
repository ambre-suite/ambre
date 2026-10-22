//! The schema reading belt: what each stored value means, frozen per
//! `SCHEMA_VERSION` (`SCHEMA-017`).
//!
//! The shape belt freezes the keys and the types. A timestamp stored in
//! milliseconds and read in seconds keeps both, so the shape stays green while
//! every older channel is read wrong. This belt freezes the other half: one
//! line per stored value, carrying the value as stored and what this build
//! reads it as.
//!
//! **Two columns, two drifts.** A line reads `path = stored => reading`. A
//! second peer that opened the first one's snapshots takes the reading,
//! through the readers the engine uses. A reader that moves moves the reading.
//! A writer and a reader that move together leave the reading alone and move
//! the stored value.
//!
//! **Identifiers.** The engine mints every document, message, channel and
//! space id fresh, so its stored value renders as `{uuid}`. Its reading names
//! what it resolves to: the active segment, message 1, the channel the
//! document opens as. A stored identifier that resolves to something else
//! moves the reading.
//!
//! **Values no reader reads.** A creation instant is stored and read by no
//! reader of this build. [`UNREAD`] names them, and their line says so. A
//! reader landing for one of them moves its line, which is the review it is
//! owed.
//!
//! **What holds the freeze honest.** A stored value with no reading fails
//! [`every_stored_value_has_a_reading`], and a declared key the scenario does
//! not reach fails [`every_declared_key_has_a_reading`]. The scenario is the
//! shape belt's, so the two belts read the same documents.

use std::collections::{BTreeSet, HashMap};

use ambre_bus::{MentionTarget, Notice, StyleMark, TextStyle};
use ambre_core::Uuid;
use ambre_core::authorship::{PEERS_MAP, PeerEntry};
use ambre_core::schema::{SCHEMA_META_MAP, SCHEMA_VERSION_KEY, verdict};
use ambre_core::{CategoryId, ChannelId};
use loro::{Container, LoroDoc, LoroValue, TextDelta, ValueOrContainer};

use crate::channel::{folded_amendments, header_fold, read_messages};
use crate::schema::{
    AMENDMENT_BY, AMENDMENT_CONTENT, AMENDMENT_DELETED, AMENDMENT_THREADS, AMENDMENTS,
    ATTACHMENT_MEDIA, ATTACHMENT_NAME, ATTACHMENT_SIZE, ATTACHMENT_TYPE, CATALOGUE, CATEGORIES,
    CATEGORY_NAME, CATEGORY_POSITION, CHANNEL, CHANNEL_NAME, CHANNEL_TYPE, DocumentKind,
    ENTRY_CATEGORY, ENTRY_KEY, ENTRY_NAME, ENTRY_POSITION, ENTRY_TOPIC, HEADER, HEADER_CHANNEL,
    HEADER_FOLDED, HEADER_FOLDED_SEQ, HEADER_KIND, HEADER_ROOT, HEADER_SEGMENT, HEADER_SPACE,
    MENTION, MESSAGE_ALSO_IN_THREAD, MESSAGE_ATTACHMENTS, MESSAGE_AUTHOR, MESSAGE_CONTENT,
    MESSAGE_DELETED, MESSAGE_EDITED, MESSAGE_ID, MESSAGE_MEMBER, MESSAGE_NOTICE, MESSAGE_REACTIONS,
    MESSAGE_THREAD, MESSAGE_TS, MESSAGES, SCHEMA_VERSION, SEGMENT_AMENDMENTS, SEGMENT_DOCUMENT,
    SEGMENT_FOLDED, SEGMENT_STATE, SEGMENTS, SPACE, SPACE_NAME, SegmentState,
};
use crate::schema_shape::{
    ANY, AUTHOR, ID, KEYED_BY_ID, MEMBER, SCHEMA_SOURCE, amended, channel_of_every_key,
    declared_string_consts, scenario_root, scenario_thread, section_of, space_of_every_key,
};
use crate::{ChannelState, SpaceState, style};

/// The frozen readings, one section per `SCHEMA_VERSION`.
const FROZEN: &str = include_str!("../schema/readings.txt");

/// The stored values no reader of this build reads.
const UNREAD: [&str; 6] = [
    "amendments:header/created_at",
    "meta:header/created_at",
    "meta:segments/*/from_ts",
    "segment:header/created_at",
    "space:header/created_at",
    "thread:header/created_at",
];

/// One segment below a root: a map key, a position in a list, or a mark
/// over a body.
#[derive(Clone)]
enum Segment {
    Key(String),
    At(usize),
    Mark(String),
}

/// One stored value: its document, the root it sits under, the segments down
/// to it, and the value.
struct Stored {
    kind: DocumentKind,
    root: String,
    segments: Vec<Segment>,
    value: LoroValue,
}

impl Stored {
    /// The key at `depth`, or `None` for a list position or nothing.
    fn key(&self, depth: usize) -> Option<&str> {
        match self.segments.get(depth)? {
            Segment::Key(key) => Some(key),
            Segment::At(_) | Segment::Mark(_) => None,
        }
    }

    /// The list position at `depth`, or `None` for a key or nothing.
    fn at(&self, depth: usize) -> Option<usize> {
        match self.segments.get(depth)? {
            Segment::At(at) => Some(*at),
            Segment::Key(_) | Segment::Mark(_) => None,
        }
    }

    /// The mark at `depth`, or `None` for anything else.
    fn mark(&self, depth: usize) -> Option<&str> {
        match self.segments.get(depth)? {
            Segment::Mark(mark) => Some(mark),
            Segment::Key(_) | Segment::At(_) => None,
        }
    }

    /// The stored string, or `None` for any other type.
    fn string(&self) -> Option<&str> {
        match &self.value {
            LoroValue::String(text) => Some(text.as_str()),
            _ => None,
        }
    }
}

/// Every stored value of one document. The deep value turns a body into its
/// string, which is what a message reads as.
fn stored_values(kind: DocumentKind, doc: &LoroDoc) -> Vec<Stored> {
    let mut found = Vec::new();
    let LoroValue::Map(roots) = doc.get_deep_value() else {
        return found;
    };
    for (root, value) in roots.iter() {
        walk(kind, root, Vec::new(), value, &mut found);
    }
    stored_marks(kind, doc, &mut found);
    found
}

/// The marks over each message body and each amended body, which the deep
/// value drops.
fn stored_marks(kind: DocumentKind, doc: &LoroDoc, found: &mut Vec<Stored>) {
    let list = doc.get_list(MESSAGES);
    for at in 0..list.len() {
        let Some(ValueOrContainer::Container(Container::Map(message))) = list.get(at) else {
            continue;
        };
        body_marks(kind, MESSAGES, Segment::At(at), &message, found);
    }
    doc.get_map(AMENDMENTS).for_each(|key, entry| {
        if let ValueOrContainer::Container(Container::Map(entry)) = entry {
            body_marks(
                kind,
                AMENDMENTS,
                Segment::Key(key.to_owned()),
                &entry,
                found,
            );
        }
    });
}

/// The marks over the `content` body of `entry`, found at `at` under `root`.
fn body_marks(
    kind: DocumentKind,
    root: &str,
    at: Segment,
    entry: &loro::LoroMap,
    found: &mut Vec<Stored>,
) {
    let Some(ValueOrContainer::Container(Container::Text(content))) = entry.get(MESSAGE_CONTENT)
    else {
        return;
    };
    for span in content.to_delta() {
        let TextDelta::Insert {
            attributes: Some(attributes),
            ..
        } = span
        else {
            continue;
        };
        for (mark, value) in attributes {
            found.push(Stored {
                kind,
                root: root.to_owned(),
                segments: vec![
                    at.clone(),
                    Segment::Key(MESSAGE_CONTENT.to_owned()),
                    Segment::Mark(mark),
                ],
                value,
            });
        }
    }
}

fn walk(
    kind: DocumentKind,
    root: &str,
    segments: Vec<Segment>,
    value: &LoroValue,
    found: &mut Vec<Stored>,
) {
    match value {
        LoroValue::Map(entries) => {
            for (key, child) in entries.iter() {
                let mut below = segments.clone();
                below.push(Segment::Key(key.clone()));
                walk(kind, root, below, child, found);
            }
        }
        LoroValue::List(items) => {
            for (at, item) in items.iter().enumerate() {
                let mut below = segments.clone();
                below.push(Segment::At(at));
                walk(kind, root, below, item, found);
            }
        }
        _ => found.push(Stored {
            kind,
            root: root.to_owned(),
            segments,
            value: value.clone(),
        }),
    }
}

/// A stored value in the one spelling the freeze uses. An identifier the
/// engine minted is not the scenario's to fix, so it collapses.
fn stored_text(value: &LoroValue) -> String {
    match value {
        LoroValue::String(text) if Uuid::parse_str(text).is_ok() => "{uuid}".to_owned(),
        LoroValue::String(text) => format!("{:?}", text.as_str()),
        LoroValue::Binary(bytes) => bytes.iter().map(|b| format!("{b:02x}")).collect(),
        LoroValue::I64(n) => n.to_string(),
        LoroValue::Bool(flag) => flag.to_string(),
        other => format!("{other:?}"),
    }
}

/// The path a value is frozen under: identifier keys and list positions
/// collapse, as in the shape.
fn frozen_path(stored: &Stored) -> String {
    let mut path = format!("{}:{}", stored.kind.as_str(), stored.root);
    for segment in &stored.segments {
        let part = match segment {
            Segment::At(_) => ANY.to_owned(),
            Segment::Key(_) if KEYED_BY_ID.contains(&path.as_str()) => ID.to_owned(),
            Segment::Key(key) => key.clone(),
            // A mark reads as in the shape: `#` and its name.
            Segment::Mark(mark) => {
                path.push('#');
                path.push_str(mark);
                continue;
            }
        };
        path.push('/');
        path.push_str(&part);
    }
    path
}

/// The two peers that reopened the scenario's snapshots.
struct Reader<'a> {
    channel: &'a ChannelState,
    space: &'a SpaceState,
}

impl Reader<'_> {
    /// The document of `kind` this reader holds.
    fn doc(&self, kind: DocumentKind) -> &LoroDoc {
        let (segment, amendments) = amended(self.channel);
        match kind {
            DocumentKind::Meta => self.channel.doc(self.channel.meta_id().document_id()),
            DocumentKind::Segment => self.channel.doc(segment.document_id()),
            DocumentKind::Amendments => self.channel.doc(amendments),
            DocumentKind::Thread => self.channel.doc(scenario_thread(self.channel)),
            DocumentKind::Space => Some(self.space.doc()),
        }
        .expect("the reader holds every document of the scenario")
    }
}

/// What `reader` reads a stored value as, or `None` when no reader of this
/// build resolves it.
fn reading(reader: &Reader<'_>, stored: &Stored) -> Option<String> {
    if UNREAD.contains(&frozen_path(stored).as_str()) {
        return Some("read by no reader of this build".to_owned());
    }
    let key = stored.key(0);
    match (stored.root.as_str(), stored.kind) {
        (SCHEMA_META_MAP, _) if key == Some(SCHEMA_VERSION_KEY) => {
            let version = crate::schema::read_version(reader.doc(stored.kind)).ok()?;
            Some(format!(
                "version {version}, {:?}",
                verdict(version, SCHEMA_VERSION)
            ))
        }
        (HEADER, _) => header_reading(reader, stored, key?),
        (PEERS_MAP, _) => {
            let read = reader.doc(stored.kind).get_map(PEERS_MAP).get(key?)?;
            let LoroValue::Binary(bytes) = read.into_value().ok()? else {
                return None;
            };
            let entry = PeerEntry::from_bytes(&bytes)?;
            let who = if entry.user == AUTHOR {
                "the author"
            } else {
                "another user"
            };
            Some(format!("an entry of {who}"))
        }
        (CHANNEL, DocumentKind::Meta) => match key? {
            CHANNEL_NAME => Some(format!("named {:?}", reader.channel.name()?)),
            CHANNEL_TYPE => Some(format!("{:?}", reader.channel.channel_type()?)),
            _ => None,
        },
        (SEGMENTS, DocumentKind::Meta) => segment_reading(reader, stored),
        (MESSAGES, DocumentKind::Segment | DocumentKind::Thread) => message_reading(reader, stored),
        (AMENDMENTS, DocumentKind::Amendments) => amendment_reading(reader, stored),
        (AMENDMENT_THREADS, DocumentKind::Amendments) => recorded_thread_reading(reader, stored),
        (SPACE, DocumentKind::Space) if key == Some(SPACE_NAME) => {
            Some(format!("named {:?}", reader.space.name()?))
        }
        (CATEGORIES, DocumentKind::Space) => category_reading(reader, stored),
        (CATALOGUE, DocumentKind::Space) => catalogue_reading(reader, stored),
        _ => None,
    }
}

/// The reading of a `header` value: the check every document passes on open.
fn header_reading(reader: &Reader<'_>, stored: &Stored, key: &str) -> Option<String> {
    // A fold is read through the check a fold snapshot passes.
    if key == HEADER_FOLDED_SEQ {
        let (_, seq) = header_fold(reader.doc(stored.kind))?;
        return Some(format!("folds its amendments up to delta {}", seq.0));
    }
    let value = stored.string()?;
    match key {
        HEADER_KIND if value == stored.kind.as_str() => {
            Some(format!("admitted as {:?}", stored.kind))
        }
        HEADER_CHANNEL if value == reader.channel.channel_id().to_string() => {
            Some("the channel it opens as".to_owned())
        }
        HEADER_SPACE if value == reader.space.space_id().to_string() => {
            Some("the space it opens as".to_owned())
        }
        HEADER_SEGMENT if value == amended(reader.channel).0.to_string() => {
            Some(match stored.kind {
                DocumentKind::Thread => "the segment of its root".to_owned(),
                _ => "the segment it amends".to_owned(),
            })
        }
        HEADER_ROOT if value == scenario_root(reader.channel)?.to_string() => {
            Some("the message it threads".to_owned())
        }
        HEADER_FOLDED
            if header_fold(reader.doc(stored.kind))?.0 == amended(reader.channel).1
                && value == amended(reader.channel).1.to_string() =>
        {
            Some("folds the amendments of its segment".to_owned())
        }
        _ => None,
    }
}

/// The reading of one `segments` entry field.
fn segment_reading(reader: &Reader<'_>, stored: &Stored) -> Option<String> {
    let entry = stored.at(0)? + 1;
    match stored.key(1)? {
        SEGMENT_DOCUMENT => {
            let id = stored.string()?;
            let which = if id == reader.channel.active_id().to_string() {
                "the active segment"
            } else if reader
                .channel
                .closing_id()
                .is_some_and(|closing| id == closing.to_string())
            {
                "the closing segment"
            } else {
                return None;
            };
            Some(format!("entry {entry} lists {which}"))
        }
        SEGMENT_STATE => Some(format!(
            "entry {entry} is {:?}",
            SegmentState::parse(stored.string()?)?
        )),
        SEGMENT_AMENDMENTS if stored.string()? == amended(reader.channel).1.to_string() => {
            Some(format!("entry {entry} names the amendments of its segment"))
        }
        SEGMENT_FOLDED => {
            let (segment, amendments) = amended(reader.channel);
            let marked = folded_amendments(reader.doc(DocumentKind::Meta), segment)?;
            (marked == amendments && stored.string()? == amendments.to_string())
                .then(|| format!("entry {entry} marks the amendments of its segment folded"))
        }
        _ => None,
    }
}

/// The reading of one message field, through the segment alone: the
/// amendments read over it are the amendment belt's.
fn message_reading(reader: &Reader<'_>, stored: &Stored) -> Option<String> {
    let at = stored.at(0)?;
    let messages = read_messages(reader.doc(stored.kind), reader.channel.channel_id());
    let message = messages.get(at)?;
    let n = at + 1;
    let thread = scenario_thread(reader.channel);
    match stored.key(1)? {
        MESSAGE_THREAD if message.thread.is_some_and(|head| head.document == thread) => {
            Some(format!("message {n} names its thread"))
        }
        MESSAGE_THREAD
            if message.thread.is_some_and(|head| {
                stored.string() == Some(head.document.to_string().as_str())
            }) =>
        {
            Some(format!("message {n} names the thread a fold carried"))
        }
        MESSAGE_ALSO_IN_THREAD if message.also_in_thread == Some(thread) => {
            Some(format!("message {n} is a copy of a reply in its thread"))
        }
        MESSAGE_ID if stored.string()? == message.id.to_string() => Some(format!("message {n}")),
        MESSAGE_AUTHOR if message.author == AUTHOR => Some(format!("message {n} by the author")),
        MESSAGE_TS => Some(format!("message {n} sent at {} ms", message.ts.0)),
        // A mention mark is read through the mentions the view is given.
        MESSAGE_CONTENT if stored.mark(2) == Some(MENTION) => {
            let named = match message.mentions.first()?.target {
                MentionTarget::Channel => "the channel",
                MentionTarget::User(user) if user == AUTHOR => "the author",
                MentionTarget::User(_) => "another user",
            };
            Some(format!("message {n} mentions {named}"))
        }
        MESSAGE_CONTENT if stored.mark(2).is_some() => Some(format!(
            "message {n} marks {}",
            style_reading(stored, &message.styles)?
        )),
        MESSAGE_CONTENT => Some(format!("message {n} reads {:?}", message.text)),
        // The entry is read by its key: the member and the emoji. Its value,
        // the instant, is kept for no reader yet.
        MESSAGE_REACTIONS => {
            let key = stored.key(2)?;
            let reaction = message
                .reactions
                .iter()
                .find(|r| key.ends_with(&format!(" {}", r.emoji.as_str())))?;
            (reaction.users == [AUTHOR] && key.starts_with(&AUTHOR.to_string())).then(|| {
                format!(
                    "message {n} has a reaction of {} bytes by the author",
                    reaction.emoji.as_str().len()
                )
            })
        }
        // A notice is read through the notice the view is given.
        MESSAGE_NOTICE => match message.notice? {
            Notice::MemberAdded { .. } => Some(format!("message {n} announces a member")),
        },
        MESSAGE_MEMBER => match message.notice? {
            Notice::MemberAdded { member } if member == MEMBER => {
                Some(format!("message {n} names the member who joins"))
            }
            Notice::MemberAdded { .. } => None,
        },
        MESSAGE_EDITED if message.edited => Some(format!("message {n} is edited")),
        MESSAGE_DELETED if message.deleted => Some(format!("message {n} is deleted")),
        // A file is read through the attachments the view is given.
        MESSAGE_ATTACHMENTS => {
            let at = stored.at(2)?;
            let file = message.attachments.get(at)?;
            let f = at + 1;
            match stored.key(3)? {
                ATTACHMENT_MEDIA if stored.string()? == file.media_ref.as_str() => {
                    Some(format!("message {n} carries file {f} by its address"))
                }
                ATTACHMENT_NAME => Some(format!(
                    "message {n} carries file {f} named {:?}",
                    file.name
                )),
                ATTACHMENT_TYPE => Some(format!(
                    "message {n} carries file {f} of type {:?}",
                    file.media_type.0
                )),
                ATTACHMENT_SIZE => Some(format!(
                    "message {n} carries file {f} of {} bytes",
                    file.size.0
                )),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The reading of one style mark, through the styles the view is given. It
/// holds the chars of the run under the same key, and what it makes of them.
fn style_reading(stored: &Stored, styles: &[StyleMark]) -> Option<String> {
    let mark = stored.mark(2)?;
    let run = styles.iter().find(|run| style::key(&run.style) == mark)?;
    let words = match &run.style {
        TextStyle::Link(address) if stored.string() == Some(address.as_str()) => {
            format!("a link to {address:?}")
        }
        TextStyle::Link(_) => return None,
        _ if stored.value == LoroValue::Bool(true) => mark.to_owned(),
        _ => return None,
    };
    Some(format!("chars {:?} {words}", run.range.to_usize_range()))
}

/// The reading of one amendment field, through the messages the view is
/// given once the amendments are read over the segment.
fn amendment_reading(reader: &Reader<'_>, stored: &Stored) -> Option<String> {
    let (segment, _) = amended(reader.channel);
    let messages = reader.channel.messages(segment).ok()?;
    let id = stored.key(0)?;
    let at = messages.iter().position(|m| m.id.to_string() == id)?;
    let message = &messages[at];
    let n = at + 1;
    match stored.key(1)? {
        AMENDMENT_CONTENT if stored.mark(2) == Some(MENTION) => {
            match message.mentions.first()?.target {
                MentionTarget::Channel => {
                    Some(format!("message {n} amended to mention the channel"))
                }
                MentionTarget::User(_) => None,
            }
        }
        AMENDMENT_CONTENT if stored.mark(2).is_some() => Some(format!(
            "message {n} amended to mark {}",
            style_reading(stored, &message.styles)?
        )),
        AMENDMENT_CONTENT if message.edited => {
            Some(format!("message {n} amended to read {:?}", message.text))
        }
        AMENDMENT_CONTENT if message.deleted => {
            Some(format!("message {n} amended to read nothing"))
        }
        AMENDMENT_DELETED if message.deleted => Some(format!("message {n} deleted by amendment")),
        AMENDMENT_BY if stored.string()? == "author" && message.author == AUTHOR => {
            Some(format!("message {n} deleted by its author"))
        }
        _ => None,
    }
}

/// The reading of one thread the amendments record, through the head its
/// root shows.
fn recorded_thread_reading(reader: &Reader<'_>, stored: &Stored) -> Option<String> {
    let (segment, _) = amended(reader.channel);
    let messages = reader.channel.messages(segment).ok()?;
    let id = stored.key(0)?;
    let at = messages.iter().position(|m| m.id.to_string() == id)?;
    let head = messages[at].thread.as_ref()?;
    (head.document.to_string() == stored.string()?)
        .then(|| format!("message {} names its thread by amendment", at + 1))
}

/// The reading of one category field, through the list the view is given.
fn category_reading(reader: &Reader<'_>, stored: &Stored) -> Option<String> {
    let id = CategoryId::parse_str(stored.key(0)?).ok()?;
    let categories = reader.space.categories();
    let rank = categories.iter().position(|c| c.id == id)?;
    let category = &categories[rank];
    let n = rank + 1;
    match stored.key(1)? {
        CATEGORY_NAME => Some(format!("category {n} named {:?}", category.name)),
        CATEGORY_POSITION => Some(format!("category {n} at position {}", category.position)),
        _ => None,
    }
}

/// The reading of one catalogue field, through the list the view is given.
fn catalogue_reading(reader: &Reader<'_>, stored: &Stored) -> Option<String> {
    let channel_id = ChannelId::parse_str(stored.key(0)?).ok()?;
    let catalogue = reader.space.catalogue();
    let rank = catalogue.iter().position(|e| e.channel_id == channel_id)?;
    let entry = &catalogue[rank];
    let n = rank + 1;
    match stored.key(1)? {
        ENTRY_NAME => Some(format!("entry {n} named {:?}", entry.name)),
        ENTRY_TOPIC => Some(format!(
            "entry {n} about {:?}",
            entry.topic.as_ref()?.as_str()
        )),
        ENTRY_CATEGORY => {
            let categories = reader.space.categories();
            let rank = categories
                .iter()
                .position(|c| Some(c.id) == entry.category)?;
            Some(format!("entry {n} in category {}", rank + 1))
        }
        ENTRY_POSITION => Some(format!("entry {n} at position {}", entry.position)),
        ENTRY_KEY => {
            let key = reader.space.sealed_key(channel_id).ok()?;
            Some(format!(
                "entry {n} hands over a sealed key of {} bytes",
                key.as_bytes().len()
            ))
        }
        _ => None,
    }
}

/// The scenario's channel, reopened from its snapshots by a second peer.
fn reopened_channel(channel: &ChannelState) -> ChannelState {
    let meta = channel.meta_id();
    let (closing, amendments) = amended(channel);
    let segments: HashMap<_, _> = [channel.active_id(), closing]
        .into_iter()
        .map(|segment| {
            let bytes = channel
                .snapshot(segment.document_id())
                .expect("a segment exports");
            (segment, bytes)
        })
        .collect();
    let mut reopened = ChannelState::open(
        channel.channel_id(),
        meta,
        &channel
            .snapshot(meta.document_id())
            .expect("a meta document exports"),
        &segments,
    )
    .expect("a channel this build wrote reopens");
    // `meta` marks the amendments folded, so no member fetches them. The
    // belt still reads them as one did before the fold.
    assert_eq!(reopened.amendments_to_open(), []);
    reopened.expect_amendments(closing, amendments);
    let bytes = channel.snapshot(amendments).expect("amendments export");
    reopened
        .apply_remote(amendments, &bytes)
        .expect("the bytes wait");
    reopened
        .older_ready(amendments)
        .expect("amendments this build wrote are admitted");
    let thread = scenario_thread(channel);
    // The thread of the locked root holds no reply, so it waits unread.
    assert!(reopened.threads_to_open().contains(&thread));
    let bytes = channel.snapshot(thread).expect("a thread exports");
    reopened
        .apply_remote(thread, &bytes)
        .expect("the bytes wait");
    reopened
        .older_ready(thread)
        .expect("a thread this build wrote is admitted");
    reopened
}

/// The scenario's space, reopened from its snapshot by a second peer.
fn reopened_space(space: &SpaceState) -> SpaceState {
    SpaceState::open(
        space.space_id(),
        &[&space.snapshot().expect("a space exports")],
    )
    .expect("a space this build wrote reopens")
}

/// The scenario, as the writer holds it.
struct Scenario {
    channel: ChannelState,
    space: SpaceState,
}

impl Scenario {
    fn new() -> Self {
        let channel = channel_of_every_key();
        let space = space_of_every_key(&channel);
        Self { channel, space }
    }

    /// Every stored value of the scenario, document by document.
    fn stored(&self) -> Vec<Stored> {
        let (segment, amendments) = amended(&self.channel);
        let docs = [
            (
                DocumentKind::Meta,
                self.channel.doc(self.channel.meta_id().document_id()),
            ),
            (
                DocumentKind::Segment,
                self.channel.doc(segment.document_id()),
            ),
            (DocumentKind::Amendments, self.channel.doc(amendments)),
            (
                DocumentKind::Thread,
                self.channel.doc(scenario_thread(&self.channel)),
            ),
            (DocumentKind::Space, Some(self.space.doc())),
        ];
        docs.into_iter()
            .flat_map(|(kind, doc)| stored_values(kind, doc.expect("held")))
            .collect()
    }
}

/// The scenario's readings, as one reader reads them.
#[derive(Default)]
struct Readings {
    /// The lines the freeze holds.
    lines: BTreeSet<String>,
    /// The stored values no reader resolved.
    unread: Vec<String>,
    /// The roots and the keys above a resolved value, which the coverage
    /// assertion answers from.
    keys: BTreeSet<String>,
}

fn readings_of(scenario: &Scenario, reader: &Reader<'_>) -> Readings {
    let mut readings = Readings::default();
    for stored in scenario.stored() {
        let path = frozen_path(&stored);
        let value = stored_text(&stored.value);
        match reading(reader, &stored) {
            Some(read) => {
                readings.lines.insert(format!("{path} = {value} => {read}"));
                readings.keys.insert(stored.root.clone());
                readings
                    .keys
                    .extend(stored.segments.iter().filter_map(|segment| match segment {
                        Segment::Key(key) | Segment::Mark(key) => Some(key.clone()),
                        Segment::At(_) => None,
                    }));
            }
            None => readings.unread.push(format!("{path} = {value}")),
        }
    }
    readings
}

/// The readings a second peer makes of the scenario.
fn readings_reopened(scenario: &Scenario) -> Readings {
    let channel = reopened_channel(&scenario.channel);
    let space = reopened_space(&scenario.space);
    readings_of(
        scenario,
        &Reader {
            channel: &channel,
            space: &space,
        },
    )
}

fn render(lines: &BTreeSet<String>) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

#[test]
fn the_reading_a_build_makes_is_the_reading_frozen_for_its_version() {
    let written = render(&readings_reopened(&Scenario::new()).lines);
    let Some(frozen) = section_of(FROZEN, SCHEMA_VERSION) else {
        panic!(
            "`schema/readings.txt` carries no `== v{SCHEMA_VERSION}` section. \
             The version rose without a freeze; add the section below:\n{written}"
        );
    };
    assert_eq!(
        written, frozen,
        "what this build stores or reads is not what was frozen for \
         SCHEMA_VERSION {SCHEMA_VERSION}. A value moved its unit or its \
         meaning, and every channel written before reads wrong. Raise \
         SCHEMA_VERSION and freeze the new reading as its own section \
         (`SCHEMA-017`)."
    );
}

#[test]
fn every_stored_value_has_a_reading() {
    let unread = readings_reopened(&Scenario::new()).unread;
    assert!(
        unread.is_empty(),
        "no reader of this build resolves these stored values, so the freeze \
         holds nothing of what they mean. Write their reading in \
         `schema_reading.rs`, or name them in `UNREAD`:\n{}",
        unread.join("\n")
    );
}

#[test]
fn every_declared_key_has_a_reading() {
    let keys = readings_reopened(&Scenario::new()).keys;
    for key in declared_string_consts(SCHEMA_SOURCE) {
        assert!(
            keys.contains(&key),
            "`{key}` is declared as a schema key and no reading carries it"
        );
    }
}

#[test]
fn a_stored_value_moving_moves_the_reading() {
    let scenario = Scenario::new();
    let before = readings_reopened(&scenario).lines;
    // The same key and the same type, another value: only the stored column
    // and the reading can show it.
    let meta = scenario
        .channel
        .doc(scenario.channel.meta_id().document_id())
        .expect("held");
    meta.get_map(CHANNEL)
        .insert(CHANNEL_NAME, "random")
        .expect("the register takes a value");
    meta.commit();
    let after = readings_reopened(&scenario).lines;
    assert_ne!(render(&before), render(&after));
}

#[test]
fn a_reader_moving_moves_the_reading() {
    let scenario = Scenario::new();
    let before = readings_reopened(&scenario).lines;
    // The stored bytes are the same; only what the reader makes of them
    // moved. The name is read from the document, so the reader is moved by
    // rewriting what it reads, on the reader's copy alone.
    let channel = reopened_channel(&scenario.channel);
    let space = reopened_space(&scenario.space);
    let meta = channel.doc(channel.meta_id().document_id()).expect("held");
    meta.get_map(CHANNEL)
        .insert(CHANNEL_NAME, "misread")
        .expect("the register takes a value");
    meta.commit();
    let after = readings_of(
        &scenario,
        &Reader {
            channel: &channel,
            space: &space,
        },
    )
    .lines;
    assert_ne!(
        render(&before),
        render(&after),
        "a reader moving has to move the reading, or the freeze holds only \
         the bytes"
    );
}
