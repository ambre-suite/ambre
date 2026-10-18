//! The persisted shape of a Murmure document: container names, keys, the
//! values they hold, and the version gate (`ambre-spec-murmure-schema.md`).
//!
//! Every key the engine writes is declared here, so the shape belt can read
//! this file and demand a line for each (`MURMURE-437`).
//!
//! **Where the version lives.** In the foundation's root `meta` map, under
//! `ambre_core::schema::SCHEMA_VERSION_KEY`, as in Trame and Prose
//! (`SCHEMA-004`). The `header` names the kind and the channel only
//! (`MURMURE-001`). The root map and the `meta` *document* share a name and
//! nothing else: one is a container, the other a kind. Absent reads as 0,
//! and the verdict and the walk are `ambre_core::schema`'s own.

use ambre_core::schema::{
    SCHEMA_META_MAP, SCHEMA_VERSION_ABSENT, SCHEMA_VERSION_KEY, SchemaError, SchemaMigration,
    SchemaVerdict, migrate_schema, migration_peer, verdict,
};
use ambre_core::{ChannelId, DocumentId, SpaceId};
use loro::{LoroDoc, LoroMap, LoroValue, ValueOrContainer};

use crate::error::MurmureError;

/// The layout version this build writes and reads (`MURMURE-020`).
///
/// Raise it in the commit that changes a key or a value type, and append the
/// new section to `schema/shapes.txt`.
pub const SCHEMA_VERSION: u32 = 1;

/// The root map every Murmure document carries (`MURMURE-001`).
pub(crate) const HEADER: &str = "header";
pub(crate) const HEADER_KIND: &str = "kind";
pub(crate) const HEADER_CHANNEL: &str = "channel_id";
pub(crate) const HEADER_CREATED_AT: &str = "created_at";
/// The key a space document names its space under, in place of a channel.
pub(crate) const HEADER_SPACE: &str = "space_id";

/// The `space` root of a space document.
pub(crate) const SPACE: &str = "space";
pub(crate) const SPACE_NAME: &str = "name";

/// The `categories` root of a space document, one entry per `CategoryId`.
pub(crate) const CATEGORIES: &str = "categories";
pub(crate) const CATEGORY_NAME: &str = "name";
pub(crate) const CATEGORY_POSITION: &str = "position";

/// The `channels` root of a space document, the catalogue: one entry per
/// public `ChannelId`.
pub(crate) const CATALOGUE: &str = "channels";
pub(crate) const ENTRY_NAME: &str = "name";
pub(crate) const ENTRY_TOPIC: &str = "topic";
pub(crate) const ENTRY_CATEGORY: &str = "category_id";
pub(crate) const ENTRY_POSITION: &str = "position";
pub(crate) const ENTRY_KEY: &str = "key";

/// The `channel` root of a `meta` document.
pub(crate) const CHANNEL: &str = "channel";
pub(crate) const CHANNEL_TYPE: &str = "type";
pub(crate) const CHANNEL_NAME: &str = "name";

/// The `segments` root of a `meta` document, and the fields of one entry.
pub(crate) const SEGMENTS: &str = "segments";
pub(crate) const SEGMENT_DOCUMENT: &str = "document_id";
pub(crate) const SEGMENT_FROM: &str = "from_ts";
pub(crate) const SEGMENT_STATE: &str = "state";

/// The `messages` root of a segment, and the fields of one message.
pub(crate) const MESSAGES: &str = "messages";
pub(crate) const MESSAGE_ID: &str = "id";
pub(crate) const MESSAGE_AUTHOR: &str = "author";
pub(crate) const MESSAGE_TS: &str = "ts";
pub(crate) const MESSAGE_CONTENT: &str = "content";

/// Murmure's migration registry: one row per version bump, appended never
/// rewritten.
///
/// A document with no version reads as 0 (`SCHEMA-005`). No Murmure document
/// was ever written without one, so the walk finds no step from 0 and
/// refuses it. Version 1 is the first layout, so no step exists yet.
pub(crate) const MIGRATIONS: &[SchemaMigration<LoroDoc>] = &[];

/// The document kinds this build writes and reads.
///
/// The spec names five. The amendments and thread kinds arrive with the
/// Stories that build them; until then a header naming one reads as foreign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    /// The one document of a space: its name, categories and catalogue.
    Space,
    /// The one `meta` document of a channel.
    Meta,
    /// One time slice of a channel's messages.
    Segment,
}

impl DocumentKind {
    /// The word the `header` stores.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Space => "space",
            Self::Meta => "meta",
            Self::Segment => "segment",
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

/// Where a segment stands in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentState {
    /// The segment new messages go to.
    Active,
    /// A segment being finalized, still open to late deltas.
    Closing,
    /// An immutable segment, loaded on demand only (`MURMURE-023`).
    Final,
}

impl SegmentState {
    /// The word a `segments` entry stores.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Closing => "closing",
            Self::Final => "final",
        }
    }

    /// The state a stored word names, or `None` for a word no build wrote.
    pub(crate) fn parse(word: &str) -> Option<Self> {
        match word {
            "active" => Some(Self::Active),
            "closing" => Some(Self::Closing),
            "final" => Some(Self::Final),
            _ => None,
        }
    }
}

/// A timestamp as Loro stores it. Loro has no unsigned integer, and a
/// millisecond count past `i64::MAX` is 292 million years away, so saturating
/// loses nothing real.
pub(crate) fn ts_value(ms: u64) -> i64 {
    i64::try_from(ms).unwrap_or(i64::MAX)
}

/// Write the `header` of a new document (`MURMURE-001`), and the version
/// in the foundation's `meta` map (`MURMURE-020`).
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
pub(crate) fn write_header(
    doc: &LoroDoc,
    kind: DocumentKind,
    channel_id: ChannelId,
    created_at: u64,
) -> Result<(), MurmureError> {
    let header = doc.get_map(HEADER);
    header.insert(HEADER_KIND, kind.as_str())?;
    header.insert(HEADER_CHANNEL, channel_id.to_string())?;
    header.insert(HEADER_CREATED_AT, ts_value(created_at))?;
    doc.get_map(SCHEMA_META_MAP)
        .insert(SCHEMA_VERSION_KEY, i64::from(SCHEMA_VERSION))?;
    Ok(())
}

/// Write the `header` of a new space document (`MURMURE-001`), and the
/// version in the foundation's `meta` map (`MURMURE-020`).
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
pub(crate) fn write_space_header(
    doc: &LoroDoc,
    space_id: SpaceId,
    created_at: u64,
) -> Result<(), MurmureError> {
    let header = doc.get_map(HEADER);
    header.insert(HEADER_KIND, DocumentKind::Space.as_str())?;
    header.insert(HEADER_SPACE, space_id.to_string())?;
    header.insert(HEADER_CREATED_AT, ts_value(created_at))?;
    doc.get_map(SCHEMA_META_MAP)
        .insert(SCHEMA_VERSION_KEY, i64::from(SCHEMA_VERSION))?;
    Ok(())
}

/// The plain value under `key`, or `None` when the key is absent or holds a
/// container.
pub(crate) fn value(map: &LoroMap, key: &str) -> Option<LoroValue> {
    match map.get(key)? {
        ValueOrContainer::Value(value) => Some(value),
        ValueOrContainer::Container(_) => None,
    }
}

/// The string under `key`, or `None` when it is absent or not a string.
pub(crate) fn string(map: &LoroMap, key: &str) -> Option<String> {
    match value(map, key)? {
        LoroValue::String(s) => Some(s.to_string()),
        _ => None,
    }
}

/// The non-negative integer under `key`, or `None` otherwise.
pub(crate) fn unsigned(map: &LoroMap, key: &str) -> Option<u64> {
    match value(map, key)? {
        LoroValue::I64(n) => u64::try_from(n).ok(),
        _ => None,
    }
}

/// The schema version a document carries.
///
/// Absent reads as [`SCHEMA_VERSION_ABSENT`] (`SCHEMA-005`). A register this
/// build cannot read is refused, never read as absent (`SCHEMA-013`).
///
/// # Errors
/// [`SchemaError::VersionUnreadable`] for a non-integer, a container, or an
/// integer outside `u32`.
pub fn read_version(doc: &LoroDoc) -> Result<u32, SchemaError> {
    let Some(found) = doc.get_map(SCHEMA_META_MAP).get(SCHEMA_VERSION_KEY) else {
        return Ok(SCHEMA_VERSION_ABSENT);
    };
    match found {
        ValueOrContainer::Value(LoroValue::I64(n)) => {
            u32::try_from(n).map_err(|_| SchemaError::VersionUnreadable {
                found: format!("the integer {n}"),
            })
        }
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
    payload: &[u8],
    kind: DocumentKind,
    channel_id: ChannelId,
    document_id: DocumentId,
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
    payloads: &[&[u8]],
    kind: DocumentKind,
    channel_id: ChannelId,
    document_id: DocumentId,
) -> Result<LoroDoc, MurmureError> {
    admit_with(kind, channel_id, document_id, |doc| {
        payloads
            .iter()
            .try_for_each(|payload| import_bytes(doc, payload, document_id))
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
    channel_id: ChannelId,
    document_id: DocumentId,
    fill: impl FnOnce(&LoroDoc) -> Result<(), MurmureError>,
) -> Result<LoroDoc, MurmureError> {
    let doc = import_current(fill)?;
    let header = doc.get_map(HEADER);
    check_kind(&header, kind, document_id)?;
    let found_channel = string(&header, HEADER_CHANNEL)
        .and_then(|s| ChannelId::parse_str(&s).ok())
        .ok_or(MurmureError::HeaderMalformed {
            document_id,
            key: HEADER_CHANNEL,
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
pub(crate) fn admit_space(payloads: &[&[u8]], space_id: SpaceId) -> Result<LoroDoc, MurmureError> {
    let document_id = space_id.document_id();
    let doc = import_current(|doc| {
        payloads
            .iter()
            .try_for_each(|payload| import_bytes(doc, payload, document_id))
    })?;
    let header = doc.get_map(HEADER);
    check_kind(&header, DocumentKind::Space, document_id)?;
    let found = string(&header, HEADER_SPACE)
        .and_then(|s| SpaceId::parse_str(&s).ok())
        .ok_or(MurmureError::HeaderMalformed {
            document_id,
            key: HEADER_SPACE,
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
    payload: &[u8],
    document_id: DocumentId,
) -> Result<(), MurmureError> {
    // A snapshot may carry the relay's numbers before its Loro bytes; the
    // document is the bytes past them.
    doc.import(crate::sequence::loro_part(payload))
        .map_err(|e| {
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
    let doc = LoroDoc::new();
    fill(&doc)?;
    let found = read_version(&doc)?;
    match verdict(found, SCHEMA_VERSION) {
        SchemaVerdict::Current => {}
        SchemaVerdict::Newer => {
            return Err(SchemaError::DocumentTooNew {
                document: found,
                binary: SCHEMA_VERSION,
            }
            .into());
        }
        SchemaVerdict::Older => {
            // Every peer migrates one document under one identity, derived
            // from its heads, so two peers write the same operations
            // (`AUTHOR-024`). No write of this session can land before its
            // own PeerID replaces it (`AUTHOR-029`).
            let heads = doc.oplog_frontiers();
            doc.set_peer_id(migration_peer(heads.iter().map(|id| (id.peer, id.counter))))?;
            migrate_schema(MIGRATIONS, found, SCHEMA_VERSION, &doc)?;
        }
    }
    Ok(doc)
}

/// Refuse a header that names another kind, or none.
fn check_kind(
    header: &LoroMap,
    kind: DocumentKind,
    document_id: DocumentId,
) -> Result<(), MurmureError> {
    let found = string(header, HEADER_KIND).ok_or(MurmureError::HeaderMalformed {
        document_id,
        key: HEADER_KIND,
    })?;
    if found != kind.as_str() {
        return Err(MurmureError::WrongKind {
            document_id,
            expected: kind.as_str(),
            found,
        });
    }
    Ok(())
}
