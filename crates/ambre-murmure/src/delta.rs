//! The authorship check of a segment delta
//! (`features/ambre-spec-murmure-messages.md section 3`).
//!
//! A segment delta travels as a Loro JSON update (`MURMURE-223`), so its
//! operations read with no document. The check runs on each received delta
//! before import. It refuses the whole delta on the first operation its signer
//! had no right to write. Only the peer registry of the segment is read, to
//! learn which user a PeerID stands for (`ambre-spec-op-authorship.md`).

use ambre_core::authorship::PeerEntry;
use ambre_core::{DocumentId, MessageId, UserId};
use loro::json::{JsonOpContent, JsonSchema, ListOp, MapOp};
use loro::{
    Container, ContainerID, ContainerType, Index, LoroDoc, LoroValue, ValueOrContainer,
    VersionVector,
};
use serde::Deserialize;

use crate::error::MurmureError;
use crate::reaction;
use crate::schema::{self, AmendmentKey, DeletedBy, Fields, MessageKey, Root};
use ambre_core::crdt::{ContainerName, FoundationRoot};

/// The version of Loro's JSON update form, the number a segment delta
/// carries in its `schema_version` field. A space of its own, apart from the
/// document's `SCHEMA_VERSION`: a change of it is a bump of that one
/// (`MURMURE-020`), never the same number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(transparent)]
pub struct JsonSchemaVersion(u8);

impl JsonSchemaVersion {
    /// The version a JSON update carries, as its field holds it.
    #[must_use]
    pub const fn new(version: u8) -> Self {
        Self(version)
    }

    /// The number, as the `schema_version` field writes it.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl std::fmt::Display for JsonSchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// The Loro JSON schema version a segment delta carries (`MURMURE-227`).
pub const JSON_SCHEMA_VERSION: JsonSchemaVersion = JsonSchemaVersion::new(1);

/// Who signed a delta, as the Sync worker verified it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signer {
    /// The only PeerID the operations of the delta may carry (`AUTHOR-012`).
    pub peer: ambre_core::PeerId,
    /// The member the signing key resolves to, `None` when the directory does
    /// not name it (`AUTHOR-013`).
    pub user: Option<UserId>,
}

/// Why the check refused a delta. Every refusal leaves the document as it
/// was, since the check runs before import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// A binary Loro update, where a JSON update is due (`MURMURE-224`).
    #[error("a binary update on a document that takes JSON updates")]
    Binary,
    /// A JSON update of a schema version this build does not compile
    /// (`MURMURE-228`).
    #[error("a JSON update of schema version {0}, where this build reads {JSON_SCHEMA_VERSION}")]
    Version(JsonSchemaVersion),
    /// Bytes that read as no JSON update of the pinned form.
    #[error("bytes that read as no JSON update")]
    Malformed,
    /// A signer the directory does not name, so no message is attributable.
    #[error("a signer the directory does not name")]
    Unattributed,
    /// An operation under another PeerID than its signer's (`AUTHOR-012`).
    #[error("an operation under another PeerID than its signer's")]
    NotSigners,
    /// A message under another author, or with no author (`MURMURE-082`).
    #[error("a message under another author than its signer")]
    ForeignAuthor,
    /// A change to what another user created (`MURMURE-085`, `MURMURE-089`).
    #[error("a change to a message of another author")]
    ForeignMessage,
    /// A registry entry for another PeerID, or naming another user.
    #[error("a registry entry its signer may not write")]
    ForeignEntry,
    /// A message taken out of the list. A delete marks it instead.
    #[error("a message removed from the list")]
    Removed,
    /// A reaction under another member's key, or a `reactions` map replaced
    /// after its message was sent (`MURMURE-094`).
    #[error("a reaction its signer may not write")]
    ForeignReaction,
    /// An amendment of a message its locked segment does not hold
    /// (`MURMURE-013`).
    #[error("an amendment of a message its segment does not hold")]
    NotInSegment,
}

impl Refusal {
    /// Whether the delta came from a newer build, so the user is asked to
    /// update (`MURMURE-228`).
    pub fn asks_update(self) -> bool {
        matches!(self, Self::Version(_))
    }
}

/// The bytes of a JSON update, as a segment delta travels.
///
/// The peers stay uncompressed: the check then reads each PeerID where it
/// stands.
///
/// # Errors
/// [`MurmureError::Crdt`] when the update does not serialise.
pub(crate) fn encode(update: &JsonSchema) -> Result<Vec<u8>, MurmureError> {
    serde_json::to_vec(update).map_err(|e| MurmureError::Crdt(e.to_string()))
}

/// Read a segment delta as a JSON update of the pinned version.
///
/// The version is read first, so an update of a newer build is told apart
/// from bytes no build wrote. A registry entry travels as a list of bytes in
/// JSON, and is read back as bytes, the value the registry holds.
///
/// # Errors
/// [`Refusal::Binary`] for a Loro blob, [`Refusal::Version`] for another
/// schema version, and [`Refusal::Malformed`] for anything else unreadable.
pub(crate) fn read(payload: &[u8]) -> Result<JsonSchema, Refusal> {
    #[derive(Deserialize)]
    struct Version {
        schema_version: JsonSchemaVersion,
    }
    if payload.starts_with(b"loro") || payload.starts_with(crate::sequence::FRAME_MAGIC) {
        return Err(Refusal::Binary);
    }
    let version = serde_json::from_slice::<Version>(payload)
        .map_err(|_| Refusal::Malformed)?
        .schema_version;
    if version != JSON_SCHEMA_VERSION {
        return Err(Refusal::Version(version));
    }
    let mut update: JsonSchema = serde_json::from_slice(payload).map_err(|_| Refusal::Malformed)?;
    if update.peers.is_some() {
        return Err(Refusal::Malformed);
    }
    restore_entries(&mut update);
    Ok(update)
}

/// Turn each registry entry back into the bytes it was written as.
fn restore_entries(update: &mut JsonSchema) {
    for op in update.changes.iter_mut().flat_map(|c| c.ops.iter_mut()) {
        if !is_root(&op.container, FoundationRoot::Peers, ContainerType::Map) {
            continue;
        }
        let JsonOpContent::Map(MapOp::Insert { value, .. }) = &mut op.content else {
            continue;
        };
        let LoroValue::List(items) = value else {
            continue;
        };
        let bytes: Option<Vec<u8>> = items
            .iter()
            .map(|item| match item {
                LoroValue::I64(b) => u8::try_from(*b).ok(),
                _ => None,
            })
            .collect();
        if let Some(bytes) = bytes {
            *value = LoroValue::Binary(bytes.into());
        }
    }
}

/// Check every operation of `update` against what its signer may write.
///
/// `registered` names the user a PeerID of an earlier session stands for, as
/// the registry of the segment holds it. `reactions` says whether a container
/// is the `reactions` map of a message the segment holds. `thread_of` reads
/// the `thread` field of a message map the segment holds, `None` for any
/// other container. The check takes no document, so it copies none
/// (`MURMURE-226`). Every client runs it on the same delta, so every client
/// reaches the same verdict (`MURMURE-079`).
///
/// # Errors
/// The first [`Refusal`] an operation earns.
pub(crate) fn check(
    update: &JsonSchema,
    signer: Signer,
    registered: impl Fn(ambre_core::PeerId) -> Option<UserId>,
    reactions: impl Fn(&ContainerID) -> bool,
    thread_of: impl Fn(&ContainerID) -> Option<Option<String>>,
) -> Result<(), Refusal> {
    let user = signer.user.ok_or(Refusal::Unattributed)?;
    if update.changes.iter().any(|c| c.id.peer != signer.peer.0) {
        return Err(Refusal::NotSigners);
    }
    // A container this session created is the signer's own.
    let owner = |peer: ambre_core::PeerId| {
        if peer == signer.peer {
            Some(user)
        } else {
            registered(peer)
        }
    };
    let mut created: Vec<&ContainerID> = Vec::new();
    let mut authored: Vec<&ContainerID> = Vec::new();
    // The `reactions` maps of the messages this delta creates.
    let mut fresh: Vec<&ContainerID> = Vec::new();
    for op in update.changes.iter().flat_map(|c| &c.ops) {
        match &op.container {
            id @ ContainerID::Normal { peer, .. } => {
                // A reactions map is open to every member, each under their
                // own key (`MURMURE-094`).
                if fresh.contains(&id) || reactions(id) {
                    reaction_allowed(&op.content, user)?;
                    continue;
                }
                // A message, its body and every child of it belong to the
                // user whose PeerID created them. Its `thread` field is the
                // one exception (`MURMURE-255`).
                if owner(ambre_core::PeerId(*peer)) != Some(user) {
                    if thread_set(&op.content, thread_of(id)) {
                        continue;
                    }
                    return Err(Refusal::ForeignMessage);
                }
                match &op.content {
                    JsonOpContent::Map(MapOp::Insert { key, value })
                        if MessageKey::parse(key) == Some(MessageKey::Author) =>
                    {
                        if !names(value, user) {
                            return Err(Refusal::ForeignAuthor);
                        }
                        authored.push(id);
                    }
                    // The map comes with its message, and is never replaced:
                    // a new one would drop every member's reaction.
                    JsonOpContent::Map(MapOp::Insert { key, value })
                        if MessageKey::parse(key) == Some(MessageKey::Reactions) =>
                    {
                        match value {
                            LoroValue::Container(map) if created.contains(&id) => fresh.push(map),
                            _ => return Err(Refusal::ForeignReaction),
                        }
                    }
                    JsonOpContent::Map(MapOp::Delete { key })
                        if MessageKey::parse(key) == Some(MessageKey::Reactions) =>
                    {
                        return Err(Refusal::ForeignReaction);
                    }
                    _ => {}
                }
            }
            root if is_root(root, Root::Messages, ContainerType::List) => match &op.content {
                JsonOpContent::List(ListOp::Insert { value, .. }) => {
                    for item in value {
                        let LoroValue::Container(id) = item else {
                            return Err(Refusal::ForeignAuthor);
                        };
                        created.push(id);
                    }
                }
                _ => return Err(Refusal::Removed),
            },
            root if is_root(root, FoundationRoot::Peers, ContainerType::Map) => {
                entry_allowed(&op.content, signer.peer, user)?;
            }
            // The header and the version register hold no message.
            ContainerID::Root { .. } => {}
        }
    }
    // A message this delta creates carries its signer as author.
    if created.iter().any(|id| !authored.contains(id)) {
        return Err(Refusal::ForeignAuthor);
    }
    Ok(())
}

/// Check every operation of an amendments delta against what its signer may
/// write (`features/ambre-spec-murmure-messages.md section 3`, step 6).
///
/// `author_of` names the author the locked segment holds for a message key,
/// `None` when it holds no such message. An entry is created under the key of
/// one of the signer's own messages, and only its creator's user writes in it.
/// `thread_of` reads the thread a message key already names, in the segment or
/// in the `threads` root. Any member names an absent one (`MURMURE-484`).
///
/// # Errors
/// The first [`Refusal`] an operation earns. [`Refusal::NotInSegment`] marks an
/// entry of a message the segment does not hold (`MURMURE-013`).
/// [`Refusal::ForeignMessage`] marks one of another author (`MURMURE-253`).
pub(crate) fn check_amendments(
    update: &JsonSchema,
    signer: Signer,
    registered: impl Fn(ambre_core::PeerId) -> Option<UserId>,
    author_of: impl Fn(MessageId) -> Option<UserId>,
    thread_of: impl Fn(MessageId) -> Option<String>,
) -> Result<(), Refusal> {
    let user = signer.user.ok_or(Refusal::Unattributed)?;
    if update.changes.iter().any(|c| c.id.peer != signer.peer.0) {
        return Err(Refusal::NotSigners);
    }
    for op in update.changes.iter().flat_map(|c| &c.ops) {
        match &op.container {
            ContainerID::Normal { peer, .. } => {
                let owner = if *peer == signer.peer.0 {
                    Some(user)
                } else {
                    registered(ambre_core::PeerId(*peer))
                };
                if owner != Some(user) {
                    return Err(Refusal::ForeignMessage);
                }
                // An author deletes under their own name. A removal by a
                // moderator is the moderation feature's, and no build writes
                // it yet.
                if let JsonOpContent::Map(MapOp::Insert { key, value }) = &op.content
                    && AmendmentKey::parse(key) == Some(AmendmentKey::By)
                    && !matches!(value, LoroValue::String(s) if s.as_str() == DeletedBy::Author.as_str())
                {
                    return Err(Refusal::ForeignMessage);
                }
            }
            root if is_root(root, Root::Amendments, ContainerType::Map) => match &op.content {
                JsonOpContent::Map(MapOp::Insert {
                    key,
                    value: LoroValue::Container(_),
                }) => match message_key(key).and_then(&author_of) {
                    None => return Err(Refusal::NotInSegment),
                    Some(author) if author != user => return Err(Refusal::ForeignMessage),
                    Some(_) => {}
                },
                _ => return Err(Refusal::Removed),
            },
            root if is_root(root, Root::Threads, ContainerType::Map) => match &op.content {
                JsonOpContent::Map(MapOp::Insert {
                    key,
                    value: LoroValue::String(value),
                }) => {
                    let Some(message) = message_key(key).filter(|m| author_of(*m).is_some()) else {
                        return Err(Refusal::NotInSegment);
                    };
                    let named = thread_of(message);
                    if DocumentId::parse_str(value).is_err()
                        || named.is_some_and(|set| set.as_str() != value.as_str())
                    {
                        return Err(Refusal::ForeignMessage);
                    }
                }
                _ => return Err(Refusal::Removed),
            },
            root if is_root(root, FoundationRoot::Peers, ContainerType::Map) => {
                entry_allowed(&op.content, signer.peer, user)?;
            }
            // The header and the version register hold no amendment.
            ContainerID::Root { .. } => {}
        }
    }
    Ok(())
}

/// The message an amendment or a thread key names, spelled as the engine
/// writes an identifier. Any other spelling names no message: a reader looks
/// the key up as written, and would never find it.
fn message_key(key: &str) -> Option<MessageId> {
    MessageId::parse_str(key)
        .ok()
        .filter(|id| id.to_string() == key)
}

/// The author `segment` holds for the message `key` names, `None` when it
/// holds no such message.
pub(crate) fn author_in(segment: &LoroDoc, key: MessageId) -> Option<UserId> {
    let key = key.to_string();
    let key = key.as_str();
    let list = crate::schema::root_list(segment, Root::Messages);
    (0..list.len()).find_map(|index| {
        let map = list.get(index).and_then(schema::fields_of::<MessageKey>)?;
        if schema::string(&map, MessageKey::Id).as_deref() != Some(key) {
            return None;
        }
        UserId::parse_str(&schema::string(&map, MessageKey::Author)?).ok()
    })
}

/// The `thread` field `segment` holds for the message `key` names, `None`
/// when it holds no such message or no thread.
fn thread_in(segment: &LoroDoc, key: MessageId) -> Option<String> {
    let key = key.to_string();
    let key = key.as_str();
    let list = crate::schema::root_list(segment, Root::Messages);
    (0..list.len()).find_map(|index| {
        let map = list.get(index).and_then(schema::fields_of::<MessageKey>)?;
        if schema::string(&map, MessageKey::Id).as_deref() != Some(key) {
            return None;
        }
        schema::string(&map, MessageKey::Thread)
    })
}

/// Whether a write to another author's message sets its `thread` field to a
/// thread document, where `current` is what the field holds. An absent field
/// takes one, and a set one takes the same again (`MURMURE-108`).
///
/// The verdict reads the field as the receiver holds it. A member who sets a
/// first value thus refuses every other one, the relay's included. The relay
/// links one document per root (`SRV-214`), so only one value is due.
fn thread_set(content: &JsonOpContent, current: Option<Option<String>>) -> bool {
    let JsonOpContent::Map(MapOp::Insert {
        key,
        value: LoroValue::String(value),
    }) = content
    else {
        return false;
    };
    let Some(current) = current else {
        return false;
    };
    MessageKey::parse(key) == Some(MessageKey::Thread)
        && DocumentId::parse_str(value).is_ok()
        && current.is_none_or(|set| set.as_str() == value.as_str())
}

/// The `thread` field of `id` when it is a message map of `doc`, `None` for
/// any other container.
pub(crate) fn thread_field(doc: &LoroDoc, id: &ContainerID) -> Option<Option<String>> {
    let path = doc.get_path_to_container(id)?;
    let [(list, _), (_, Index::Seq(_))] = path.as_slice() else {
        return None;
    };
    if !is_root(list, Root::Messages, ContainerType::List) {
        return None;
    }
    let Some(Container::Map(map)) = doc.get_container(id.clone()) else {
        return None;
    };
    Some(schema::string(&Fields::of(map), MessageKey::Thread))
}

/// Whether a registry write is the signer's own: its PeerID's key alone. An
/// entry that reads as one must name the signer. An unreadable one names
/// nobody, so it grants nothing.
fn entry_allowed(
    content: &JsonOpContent,
    peer: ambre_core::PeerId,
    user: UserId,
) -> Result<(), Refusal> {
    let JsonOpContent::Map(MapOp::Insert { key, value }) = content else {
        return Err(Refusal::ForeignEntry);
    };
    if *key != ambre_core::PeerKey::of(peer).as_str() {
        return Err(Refusal::ForeignEntry);
    }
    let claimed = match value {
        LoroValue::Binary(bytes) => PeerEntry::from_bytes(bytes),
        _ => None,
    };
    match claimed {
        Some(entry) if entry.user != user => Err(Refusal::ForeignEntry),
        _ => Ok(()),
    }
}

/// Whether a write to a `reactions` map is the signer's own: an entry under
/// their key, set to a plain value or removed.
fn reaction_allowed(content: &JsonOpContent, user: UserId) -> Result<(), Refusal> {
    let own = |key: &str| reaction::ReactionKey::read(key).map(|(writer, _)| writer) == Some(user);
    match content {
        JsonOpContent::Map(MapOp::Insert { key, value })
            if own(key) && !matches!(value, LoroValue::Container(_)) =>
        {
            Ok(())
        }
        JsonOpContent::Map(MapOp::Delete { key }) if own(key) => Ok(()),
        _ => Err(Refusal::ForeignReaction),
    }
}

/// The messages a JSON update creates, in operation order. A message takes
/// its identity once, when it is created, so a later write never names it.
pub(crate) fn created_messages(update: &JsonSchema) -> Vec<MessageId> {
    update
        .changes
        .iter()
        .flat_map(|c| &c.ops)
        .filter_map(|op| match &op.content {
            JsonOpContent::Map(MapOp::Insert {
                key,
                value: LoroValue::String(id),
            }) if MessageKey::parse(key) == Some(MessageKey::Id) => MessageId::parse_str(id).ok(),
            _ => None,
        })
        .collect()
}

/// Whether `value` is the identifier of `user`.
fn names(value: &LoroValue, user: UserId) -> bool {
    matches!(value, LoroValue::String(s) if UserId::parse_str(s).is_ok_and(|u| u == user))
}

fn is_root(id: &ContainerID, root: impl ContainerName, kind: ContainerType) -> bool {
    matches!(id, ContainerID::Root { name, container_type } if name.as_str() == root.as_str() && *container_type == kind)
}

/// The user the registry of `doc` names for `peer`, `None` when its entry is
/// absent or unreadable (`AUTHOR-006`).
pub(crate) fn registered(doc: &LoroDoc, peer: ambre_core::PeerId) -> Option<UserId> {
    let ValueOrContainer::Value(LoroValue::Binary(bytes)) = doc
        .get_map(ambre_core::crdt::FoundationRoot::Peers.as_str())
        .get(ambre_core::PeerKey::of(peer).as_str())?
    else {
        return None;
    };
    PeerEntry::from_bytes(&bytes).map(|entry| entry.user)
}

/// The operations a JSON update holds, per PeerID: where they start, and
/// where they end.
pub(crate) fn range(update: &JsonSchema) -> (VersionVector, VersionVector) {
    let mut start = VersionVector::new();
    let mut end = VersionVector::new();
    for change in &update.changes {
        let peer = change.id.peer;
        let from = change.id.counter;
        let to = from.saturating_add(i32::try_from(change.op_len()).unwrap_or(i32::MAX));
        if start.get(&peer).is_none_or(|s| *s > from) {
            start.insert(peer, from);
        }
        if end.get(&peer).is_none_or(|e| *e < to) {
            end.insert(peer, to);
        }
    }
    (start, end)
}

/// One payload of a segment, kept until the segment opens.
#[derive(Debug, Clone)]
pub(crate) enum Arrival {
    /// A snapshot, or a delta read back from the local store: Loro bytes,
    /// checked where they came from.
    Bytes(Vec<u8>),
    /// A delta a member signed, read and not checked yet.
    Signed { update: JsonSchema, signer: Signer },
}

/// What the deltas of a document are checked against.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Rules<'a> {
    /// A segment: its own registry and its own messages, whose `thread`
    /// field any member sets once (`MURMURE-255`).
    Segment,
    /// A thread document: its own registry and its own replies. A reply
    /// starts no thread, so no other member writes in it.
    Thread,
    /// An amendments document: its own registry, and the locked segment that
    /// names the author of each message.
    Amendments { segment: &'a LoroDoc },
}

/// Check `update` against what its signer may write in `doc`, under `rules`.
///
/// # Errors
/// The first [`Refusal`] an operation earns.
pub(crate) fn verdict(
    doc: &LoroDoc,
    rules: Rules<'_>,
    update: &JsonSchema,
    signer: Signer,
) -> Result<(), Refusal> {
    match rules {
        Rules::Segment => check(
            update,
            signer,
            |peer| registered(doc, peer),
            |id| reaction::is_map(doc, id),
            |id| thread_field(doc, id),
        ),
        Rules::Thread => check(
            update,
            signer,
            |peer| registered(doc, peer),
            |id| reaction::is_map(doc, id),
            |_| None,
        ),
        Rules::Amendments { segment } => check_amendments(
            update,
            signer,
            |peer| registered(doc, peer),
            |key| author_in(segment, key),
            |key| {
                schema::string_value(
                    crate::schema::root_map(doc, Root::Threads).get(&key.to_string()),
                )
                .or_else(|| thread_in(segment, key))
            },
        ),
    }
}

/// Import `arrivals` into `doc`, in order. A signed delta passes the check
/// first, against the registry `doc` holds so far. A refused one is left out,
/// and changes nothing (`MURMURE-030`).
///
/// # Errors
/// [`MurmureError::Schema`] for Loro bytes the engine refuses, and
/// [`MurmureError::Crdt`] for a checked update Loro refuses.
pub(crate) fn import_arrivals(
    doc: &LoroDoc,
    arrivals: Vec<Arrival>,
    document_id: DocumentId,
    rules: Rules<'_>,
) -> Result<(), MurmureError> {
    for arrival in arrivals {
        match arrival {
            Arrival::Bytes(bytes) => crate::schema::import_bytes(doc, &bytes, document_id)?,
            Arrival::Signed { update, signer } => {
                let verdict = verdict(doc, rules, &update, signer);
                if let Err(refusal) = verdict {
                    tracing::warn!("[Murmure] a delta for {document_id} refused: {refusal}");
                    continue;
                }
                doc.import_json_updates(update)?;
            }
        }
    }
    Ok(())
}

/// Whether every operation of a binary update was written under `peer`
/// (`AUTHOR-012`). The `meta` document travels binary, and is checked so.
pub(crate) fn binary_by(payload: &[u8], peer: ambre_core::PeerId) -> bool {
    LoroDoc::decode_import_blob_meta(payload, false).is_ok_and(|meta| {
        meta.mode == loro::EncodedBlobMode::Updates
            && meta.partial_end_vv.keys().all(|p| *p == peer.0)
    })
}
