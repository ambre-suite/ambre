//! The persisted shape of a Murmure document: container names, keys, the
//! values they hold, and the version gate (`ambre-spec-murmure-schema.md`).
//!
//! Every name the engine writes is declared here, one closed set per
//! container. The shape belt walks each set and demands a line for each name
//! (`MURMURE-437`). A map is read and written through [`Fields`], typed by its
//! set, so a key of one map does not compile against another.
//!
//! **Where the version lives.** In the foundation's root `meta` map, under
//! `ambre_core::schema::MetaKey::SchemaVersion`, as in Trame and Prose
//! (`SCHEMA-004`). The `header` names the kind and the channel only
//! (`MURMURE-001`). The root map and the `meta` *document* share a name and
//! nothing else: one is a container, the other a kind. Absent reads as 0,
//! and the verdict and the walk are `ambre_core::schema`'s own.

use ambre_core::LoroPayloadRef;
use ambre_core::id::{Channel, Document, Id, Space};
use ambre_core::schema::{
    SCHEMA_VERSION_ABSENT, SchemaError, SchemaMigration, SchemaVerdict, SchemaVersion,
    migrate_schema, migration_peer, verdict,
};
use loro::{LoroDoc, LoroList, LoroMap, LoroValue, ValueOrContainer};

use crate::error::MurmureError;

/// The layout version this build writes and reads (`MURMURE-020`).
///
/// Raise it in the commit that changes a key or a value type, and append the
/// new section to `schema/shapes.txt`.
pub const SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1);

ambre_core::loro_fields!(pub(crate));

ambre_core::crdt_names! {
    /// The root containers of the Murmure documents. A document of each kind
    /// opens the roots its kind holds.
    pub(crate) enum Root: ContainerName {
        /// The root map every Murmure document carries (`MURMURE-001`), read
        /// through [`HeaderKey`].
        Header = "header",
        /// The `space` root of a space document, read through [`SpaceKey`].
        Space = "space",
        /// The `categories` root of a space document, one [`CategoryKey`] map
        /// per `Id<Category>`.
        Categories = "categories",
        /// The `channels` root of a space document, the catalogue: one
        /// [`EntryKey`] map per public `Id<Channel>`.
        Catalogue = "channels",
        /// The `channel` root of a `meta` document, read through
        /// [`ChannelKey`].
        Channel = "channel",
        /// The `segments` list of a `meta` document, one [`SegmentKey`] map per
        /// segment.
        Segments = "segments",
        /// The `amendments` root of an amendments document, one
        /// [`AmendmentKey`] map per `Id<Message>` of its segment.
        Amendments = "amendments",
        /// The `threads` root of an amendments document: the thread document
        /// of each root of its locked segment, keyed by `Id<Message>`
        /// (`MURMURE-484`).
        Threads = "threads",
        /// The `messages` list of a segment, one [`MessageKey`] map per
        /// message.
        Messages = "messages",
    }
}

ambre_core::crdt_names! {
    /// The keys of the `header` root. Public, since a refused header names
    /// the key it lacks.
    pub enum HeaderKey: FieldKey {
        Kind = "kind",
        Channel = "channel_id",
        CreatedAt = "created_at",
        /// The key a space document names its space under, in place of a
        /// channel.
        Space = "space_id",
        /// The key an amendments document names its segment under, beside
        /// its channel (`MURMURE-013`). A thread document names its root's
        /// segment so.
        Segment = "segment_id",
        /// The key a thread document names its root message under
        /// (`MURMURE-009`).
        Root = "root_id",
        /// The amendments document a fold snapshot of a segment folded in,
        /// absent from every other snapshot.
        Folded = "folded",
        /// The last delta number of those amendments the fold holds.
        FoldedSeq = "folded_seq",
    }
}

ambre_core::crdt_names! {
    /// The keys of the `space` root.
    pub(crate) enum SpaceKey: FieldKey {
        Name = "name",
    }
}

ambre_core::crdt_names! {
    /// The keys of one entry of the `categories` root.
    pub(crate) enum CategoryKey: FieldKey {
        Name = "name",
        Position = "position",
    }
}

ambre_core::crdt_names! {
    /// The keys of one entry of the catalogue.
    pub(crate) enum EntryKey: FieldKey {
        Name = "name",
        Topic = "topic",
        Category = "category_id",
        Position = "position",
        Key = "key",
    }
}

ambre_core::crdt_names! {
    /// The keys of the `channel` root.
    pub(crate) enum ChannelKey: FieldKey {
        Type = "type",
        Name = "name",
    }
}

ambre_core::crdt_names! {
    /// The keys of one entry of the `segments` list.
    pub(crate) enum SegmentKey: FieldKey {
        Document = "document_id",
        From = "from_ts",
        State = "state",
        /// The amendments document of a locked segment, absent while it has
        /// none.
        Amendments = "amendments",
        /// The amendments document the final snapshot folded in, which the
        /// relay erased. An `amendments` that names it names no document.
        Folded = "folded",
    }
}

ambre_core::crdt_names! {
    /// The keys of one entry of the `amendments` root.
    pub(crate) enum AmendmentKey: FieldKey {
        /// The edited body, a char CRDT like the body of a message.
        Content = "content",
        /// `true` once the message is deleted, its edited body emptied.
        Deleted = "deleted",
        /// Who deleted the message, a [`DeletedBy`] word.
        By = "by",
    }
}

ambre_core::crdt_names! {
    /// The keys of one message of the `messages` list.
    pub(crate) enum MessageKey: FieldKey {
        Id = "id",
        Author = "author",
        Ts = "ts",
        Content = "content",
        /// The flat map of a message's reactions, one entry per member and
        /// emoji (`MURMURE-006`). Each entry holds the instant of its
        /// reaction.
        Reactions = "reactions",
        /// `true` once the author edited the body (`MURMURE-084`).
        Edited = "edited",
        /// `true` once the author deleted the message, its body emptied
        /// (`MURMURE-087`).
        Deleted = "deleted",
        /// The kind of a notice, absent from a message a member wrote
        /// (`MURMURE-057`). A notice keeps an empty body.
        Notice = "notice",
        /// The member a `member_added` notice names.
        Member = "member",
        /// The files a message carries, absent from a message that carries
        /// none (`MURMURE-017`). One [`AttachmentKey`] map per file, in the
        /// order its author attached them.
        Attachments = "attachments",
        /// The thread document of a root message, absent while it has no
        /// reply (`MURMURE-107`).
        Thread = "thread",
        /// The thread document a reply also sent to the channel came from
        /// (`MURMURE-109`).
        AlsoInThread = "also_in_thread",
    }
}

ambre_core::crdt_names! {
    /// The keys of one file of a message's `attachments`.
    pub(crate) enum AttachmentKey: FieldKey {
        Media = "media_ref",
        Name = "name",
        Type = "media_type",
        Size = "size",
    }
}

ambre_core::crdt_names! {
    /// The marks over a body. A mark is a key of a text, not of a map.
    pub(crate) enum Mark {
        /// A mention. Its value is a user identifier, or the channel
        /// identifier for an `@channel` (`MURMURE-008`).
        Mention = "mention",
        /// The style marks (`MURMURE-562`). Bold, italic and code hold
        /// `true`, a link its address.
        Bold = "bold",
        Italic = "italic",
        Code = "code",
        Link = "link",
        /// The block marks, which hold `true` over whole lines
        /// (`MURMURE-194`).
        CodeBlock = "code_block",
        Quote = "quote",
        List = "list",
    }
}

/// The map root `root` of `doc`, for a root keyed by identifiers: the
/// categories, the catalogue, the amendments and the threads.
pub(crate) fn root_map(doc: &LoroDoc, root: Root) -> LoroMap {
    doc.get_map(root.as_str())
}

/// The list root `root` of `doc`: the segments or the messages.
pub(crate) fn root_list(doc: &LoroDoc, root: Root) -> LoroList {
    doc.get_list(root.as_str())
}

/// The `header` of `doc`.
pub(crate) fn header(doc: &LoroDoc) -> Fields<HeaderKey> {
    Fields::of(root_map(doc, Root::Header))
}

/// The `channel` root of a `meta` document.
pub(crate) fn channel(doc: &LoroDoc) -> Fields<ChannelKey> {
    Fields::of(root_map(doc, Root::Channel))
}

/// The `space` root of a space document.
pub(crate) fn space(doc: &LoroDoc) -> Fields<SpaceKey> {
    Fields::of(root_map(doc, Root::Space))
}

/// The map `item` holds, read through the keys of the schema `K`, or `None`
/// for an item of another kind.
pub(crate) fn fields_of<K: ambre_core::crdt::FieldKey>(
    item: ValueOrContainer,
) -> Option<Fields<K>> {
    match item {
        ValueOrContainer::Container(loro::Container::Map(map)) => Some(Fields::of(map)),
        _ => None,
    }
}

/// Murmure's migration registry: one row per version bump, appended never
/// rewritten.
///
/// A document with no version reads as 0 (`SCHEMA-005`). No Murmure document
/// was ever written without one, so the walk finds no step from 0 and
/// refuses it. Version 1 is the first layout, and no build before it shipped,
/// so the list is empty.
///
/// A step writes no operation (`MURMURE-566`). Each member signs its own
/// writes, and the check refuses a change under any other identity, so a
/// migration's writes would never travel. A later step therefore reads the
/// document as it is and keeps its stamp.
pub(crate) const MIGRATIONS: &[SchemaMigration<LoroDoc>] = &[];

/// The document kinds this build writes and reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    /// The one document of a space: its name, categories and catalogue.
    Space,
    /// The one `meta` document of a channel.
    Meta,
    /// One time slice of a channel's messages.
    Segment,
    /// The late edits and deletes of one locked segment (`MURMURE-013`).
    Amendments,
    /// The replies to one root message (`MURMURE-105`).
    Thread,
}

impl DocumentKind {
    /// The word the `header` stores.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Space => "space",
            Self::Meta => "meta",
            Self::Segment => "segment",
            Self::Amendments => "amendments",
            Self::Thread => "thread",
        }
    }
}

/// Who deleted a message of a locked segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeletedBy {
    /// Its author (`MURMURE-088`).
    Author,
}

impl DeletedBy {
    /// The word an amendment stores under `by`.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Author => "author",
        }
    }
}

/// Who may read a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    /// Two or more members, found by their member set.
    Direct,
    /// Members on invitation.
    Private,
    /// Listed in the space catalogue.
    Public,
}

impl ChannelType {
    /// The word `meta.channel.type` stores.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Private => "private",
            Self::Public => "public",
        }
    }

    /// The type a stored word names, or `None` for a word no build wrote.
    pub(crate) fn parse(word: &str) -> Option<Self> {
        match word {
            "direct" => Some(Self::Direct),
            "private" => Some(Self::Private),
            "public" => Some(Self::Public),
            _ => None,
        }
    }
}

/// What a notice announces (`MURMURE-057`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeKind {
    /// A member joins; the message names them under `member`.
    MemberAdded,
}

impl NoticeKind {
    /// The word a message stores under `notice`.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MemberAdded => "member_added",
        }
    }

    /// The kind a stored word names, or `None` for a word no build wrote.
    pub(crate) fn parse(word: &str) -> Option<Self> {
        match word {
            "member_added" => Some(Self::MemberAdded),
            _ => None,
        }
    }
}

pub use ambre_core::SegmentState;

/// A timestamp as Loro stores it. Loro has no unsigned integer, and a
/// millisecond count past `i64::MAX` is 292 million years away, so saturating
/// loses nothing real.
pub(crate) fn ts_value(ts: ambre_core::Instant<ambre_core::Authored>) -> i64 {
    ambre_core::StoredAsI64::to_i64_saturating(ts)
}

/// Write the `header` of a new document (`MURMURE-001`), and the version
/// in the foundation's `meta` map (`MURMURE-020`).
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
pub(crate) fn write_header(
    doc: &LoroDoc,
    kind: DocumentKind,
    channel_id: Id<Channel>,
    created_at: ambre_core::Instant<ambre_core::Authored>,
) -> Result<(), MurmureError> {
    let header = header(doc);
    header.insert(HeaderKey::Kind, kind.as_str())?;
    header.insert(HeaderKey::Channel, channel_id.to_string())?;
    header.insert(HeaderKey::CreatedAt, ts_value(created_at))?;
    doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .insert(
            ambre_core::schema::MetaKey::SchemaVersion.as_str(),
            i64::from(SCHEMA_VERSION.get()),
        )?;
    Ok(())
}

/// Write the `header` of a new space document (`MURMURE-001`), and the
/// version in the foundation's `meta` map (`MURMURE-020`).
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
pub(crate) fn write_space_header(
    doc: &LoroDoc,
    space_id: Id<Space>,
    created_at: ambre_core::Instant<ambre_core::Authored>,
) -> Result<(), MurmureError> {
    let header = header(doc);
    header.insert(HeaderKey::Kind, DocumentKind::Space.as_str())?;
    header.insert(HeaderKey::Space, space_id.to_string())?;
    header.insert(HeaderKey::CreatedAt, ts_value(created_at))?;
    doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .insert(
            ambre_core::schema::MetaKey::SchemaVersion.as_str(),
            i64::from(SCHEMA_VERSION.get()),
        )?;
    Ok(())
}

/// The plain value under `key`, or `None` when the key is absent or holds a
/// container.
pub(crate) fn value<K: ambre_core::crdt::FieldKey>(map: &Fields<K>, key: K) -> Option<LoroValue> {
    map.value(key)
}

/// The string under `key`, or `None` when it is absent or not a string.
pub(crate) fn string<K: ambre_core::crdt::FieldKey>(map: &Fields<K>, key: K) -> Option<String> {
    string_value(map.get(key))
}

/// The name under `key`, or `None` when it is absent, not a string, or past
/// the cap. A peer may write a name past the cap, and no view shows one.
pub(crate) fn name<K: ambre_core::crdt::FieldKey>(
    map: &Fields<K>,
    key: K,
) -> Option<ambre_core::Name> {
    string(map, key).and_then(|text| ambre_core::Name::new(text).ok())
}

/// The string a read found, or `None` when it found nothing or no string.
/// A root keyed by identifiers is read so: its key is an identifier, not a
/// name of the schema.
pub(crate) fn string_value(found: Option<ValueOrContainer>) -> Option<String> {
    match found? {
        ValueOrContainer::Value(LoroValue::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// The non-negative integer under `key`, or `None` otherwise.
pub(crate) fn unsigned<K: ambre_core::crdt::FieldKey>(map: &Fields<K>, key: K) -> Option<u64> {
    match value(map, key)? {
        LoroValue::I64(n) => u64::try_from(n).ok(),
        _ => None,
    }
}

/// Whether the boolean under `key` is `true`. Absent, or of another type,
/// reads as `false`.
pub(crate) fn flag<K: ambre_core::crdt::FieldKey>(map: &Fields<K>, key: K) -> bool {
    matches!(value(map, key), Some(LoroValue::Bool(true)))
}

/// The schema version a document carries.
///
/// Absent reads as [`SCHEMA_VERSION_ABSENT`] (`SCHEMA-005`). A register this
/// build cannot read is refused, never read as absent (`SCHEMA-013`).
///
/// # Errors
/// [`SchemaError::VersionUnreadable`] for a non-integer, a container, or an
/// integer outside `u32`.
pub fn read_version(doc: &LoroDoc) -> Result<SchemaVersion, SchemaError> {
    let Some(found) = doc
        .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
        .get(ambre_core::schema::MetaKey::SchemaVersion.as_str())
    else {
        return Ok(SCHEMA_VERSION_ABSENT);
    };
    match found {
        ValueOrContainer::Value(LoroValue::I64(n)) => u32::try_from(n)
            .map(SchemaVersion::new)
            .map_err(|_| SchemaError::VersionUnreadable {
                found: format!("the integer {n}"),
            }),
        ValueOrContainer::Value(other) => Err(SchemaError::VersionUnreadable {
            found: format!("{other:?}"),
        }),
        ValueOrContainer::Container(_) => Err(SchemaError::VersionUnreadable {
            found: "a container".to_owned(),
        }),
    }
}

/// Import `payload` into a fresh document and check its header, before the
/// engine holds it. The one-payload case of [`admit_all`].
///
/// # Errors
/// Every refusal of [`admit_all`].
pub(crate) fn admit(
    payload: LoroPayloadRef<'_>,
    kind: DocumentKind,
    channel_id: Id<Channel>,
    document_id: Id<Document>,
) -> Result<LoroDoc, MurmureError> {
    admit_all(&[payload], kind, channel_id, document_id)
}

/// Import every payload into a fresh document, in order, and check its header
/// before the engine holds it.
///
/// A document opened over the relay arrives as a snapshot and a replay, so the
/// check runs once on the whole. The order is the foundation's. The CRDT
/// engine reads the bytes first (`SCHEMA-014`). The version then decides
/// whether the rest is readable (`MURMURE-019`). The kind and the channel come
/// last (`MURMURE-027`). The document is returned only on success, so a
/// refusal leaves the channel as it was.
///
/// # Errors
/// [`SchemaError::PayloadRejected`] for bytes Loro refuses, the version
/// refusals of [`SchemaError`], [`MurmureError::WrongKind`],
/// [`MurmureError::HeaderMalformed`], and [`MurmureError::ForeignChannel`].
pub(crate) fn admit_all(
    payloads: &[LoroPayloadRef<'_>],
    kind: DocumentKind,
    channel_id: Id<Channel>,
    document_id: Id<Document>,
) -> Result<LoroDoc, MurmureError> {
    admit_with(kind, channel_id, document_id, |doc| {
        payloads
            .iter()
            .try_for_each(|&payload| import_bytes(doc, payload, document_id))
    })
}

/// [`admit_all`], with the document filled by `fill` rather than from bytes.
/// A segment's signed deltas pass the authorship check on the way in (the
/// Murmure messages spec).
///
/// # Errors
/// Every refusal of `fill` and of [`admit_all`].
pub(crate) fn admit_with(
    kind: DocumentKind,
    channel_id: Id<Channel>,
    document_id: Id<Document>,
    fill: impl FnOnce(&LoroDoc) -> Result<(), MurmureError>,
) -> Result<LoroDoc, MurmureError> {
    let doc = import_current(fill)?;
    let header = header(&doc);
    check_kind(&header, kind, document_id)?;
    let found_channel = string(&header, HeaderKey::Channel)
        .and_then(|s| Id::<Channel>::parse_str(&s).ok())
        .ok_or(MurmureError::HeaderMalformed {
            document_id,
            key: HeaderKey::Channel,
        })?;
    if found_channel != channel_id {
        // A document of another channel would read as a valid one of this
        // channel. The warn is the one trace a misrouted document leaves.
        tracing::warn!(
            "[Murmure] document {document_id} names channel {found_channel}, not {channel_id}; refused"
        );
        return Err(MurmureError::ForeignChannel {
            document_id,
            expected: channel_id,
            found: found_channel,
        });
    }
    Ok(doc)
}

/// Import every payload of a space document, in order, and check its header
/// before the engine holds it. The same order as [`admit_all`]: the bytes,
/// the version, the kind, then the space the header names.
///
/// # Errors
/// Every refusal of [`admit_all`], with [`MurmureError::ForeignSpace`] in
/// place of the foreign channel.
pub(crate) fn admit_space(
    payloads: &[LoroPayloadRef<'_>],
    space_id: Id<Space>,
) -> Result<LoroDoc, MurmureError> {
    let document_id = space_id.document_id();
    let doc = import_current(|doc| {
        payloads
            .iter()
            .try_for_each(|&payload| import_bytes(doc, payload, document_id))
    })?;
    let header = header(&doc);
    check_kind(&header, DocumentKind::Space, document_id)?;
    let found = string(&header, HeaderKey::Space)
        .and_then(|s| Id::<Space>::parse_str(&s).ok())
        .ok_or(MurmureError::HeaderMalformed {
            document_id,
            key: HeaderKey::Space,
        })?;
    if found != space_id {
        tracing::warn!("[Murmure] document {document_id} names space {found}; refused");
        return Err(MurmureError::ForeignSpace { document_id, found });
    }
    Ok(doc)
}

/// Import one payload of Loro bytes into `doc`.
///
/// # Errors
/// [`SchemaError::PayloadRejected`] for bytes the CRDT engine refuses.
pub(crate) fn import_bytes(
    doc: &LoroDoc,
    payload: LoroPayloadRef<'_>,
    document_id: Id<Document>,
) -> Result<(), MurmureError> {
    // A snapshot may carry the relay's numbers before its Loro bytes; the
    // document is the bytes past them.
    crate::sequence::import(doc, payload).map_err(|e| {
        tracing::warn!("[Murmure] document {document_id} rejected by the CRDT engine: {e:?}");
        SchemaError::PayloadRejected
    })?;
    Ok(())
}

/// Fill a fresh document with `fill`, and bring it to the compiled version or
/// refuse it (`SCHEMA-014`, `MURMURE-019`).
fn import_current(
    fill: impl FnOnce(&LoroDoc) -> Result<(), MurmureError>,
) -> Result<LoroDoc, MurmureError> {
    import_through(MIGRATIONS, SCHEMA_VERSION, fill)
}

/// [`import_current`] over the registry `steps`, up to `target`. A test hands
/// it a step and a target of its own, so the walk stays proved while
/// [`MIGRATIONS`] holds none.
pub(crate) fn import_through(
    steps: &[SchemaMigration<LoroDoc>],
    target: SchemaVersion,
    fill: impl FnOnce(&LoroDoc) -> Result<(), MurmureError>,
) -> Result<LoroDoc, MurmureError> {
    let doc = LoroDoc::new();
    fill(&doc)?;
    let found = read_version(&doc)?;
    match verdict(found, target) {
        SchemaVerdict::Current => {}
        SchemaVerdict::Newer => {
            return Err(SchemaError::DocumentTooNew {
                document: found,
                binary: target,
            }
            .into());
        }
        SchemaVerdict::Older => {
            // Every peer migrates one document under one identity, derived
            // from its heads, so two peers write the same operations
            // (`AUTHOR-024`). No write of this session can land before its
            // own PeerID replaces it (`AUTHOR-029`).
            let heads = doc.oplog_frontiers();
            doc.set_peer_id(
                migration_peer(heads.iter().map(|id| {
                    (
                        ambre_core::PeerId::new(id.peer),
                        ambre_core::OpCounter::new(id.counter),
                    )
                }))
                .get(),
            )?;
            migrate_schema(steps, found, target, &doc)?;
        }
    }
    Ok(doc)
}

/// Refuse a header that names another kind, or none.
fn check_kind(
    header: &Fields<HeaderKey>,
    kind: DocumentKind,
    document_id: Id<Document>,
) -> Result<(), MurmureError> {
    let found = string(header, HeaderKey::Kind).ok_or(MurmureError::HeaderMalformed {
        document_id,
        key: HeaderKey::Kind,
    })?;
    if found != kind.as_str() {
        return Err(MurmureError::WrongKind {
            document_id,
            expected: kind,
            found,
        });
    }
    Ok(())
}
