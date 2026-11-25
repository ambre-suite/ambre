//! The authorship check of a segment delta
//! (`features/ambre-spec-murmure-messages.md section 3`).
//!
//! A segment delta travels as a Loro JSON update (`MURMURE-223`), so its
//! operations read with no document. The check runs on each received delta
//! before import. It refuses the whole delta on the first operation its signer
//! had no right to write. Only the peer registry of the segment is read, to
//! learn which user a PeerID stands for (`ambre-spec-op-authorship.md`).

use ambre_core::authorship::PeerEntry;
use ambre_core::crdt::CrdtName;
use ambre_core::id::{Document, Id, Message, User};
use ambre_core::version::{Current, OpenRange, Version, VersionKind};
use ambre_core::{DeltaBytes, LoroPayload, Payload};
use loro::json::{JsonOpContent, JsonSchema, ListOp, MapOp};
use loro::{
    Container, ContainerID, ContainerType, ID, Index, LoroDoc, LoroValue, ValueOrContainer,
    VersionVector,
};
use serde::Deserialize;

use crate::logic::error::MurmureError;
use crate::logic::reaction;
use crate::logic::schema::{self, AmendmentKey, DeletedBy, Fields, MessageKey, Root};
use ambre_core::crdt::{ContainerName, FoundationRoot};

/// The space of Loro's JSON update form, the number a segment delta
/// carries in its `schema_version` field. A space of its own, apart from the
/// document's `SCHEMA_VERSION`: a change of it is a bump of that one
/// (`MURMURE-020`), never the same number.
pub enum JsonUpdate {}

impl VersionKind for JsonUpdate {
    type Repr = u8;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("JsonSchemaVersion");
}

impl OpenRange for JsonUpdate {}

/// One JSON form this build writes and reads.
impl Current for JsonUpdate {
    const CURRENT: Version<Self> = Version::new(1);
}

/// The Loro JSON schema version a segment delta carries (`MURMURE-227`).
pub const JSON_SCHEMA_VERSION: Version<JsonUpdate> = Version::CURRENT;

/// Who signed a delta, as the Sync worker verified it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signer {
    /// The only PeerID the operations of the delta may carry (`AUTHOR-012`).
    pub peer: ambre_core::PeerId,
    /// The member the signing key resolves to, `None` when the directory does
    /// not name it (`AUTHOR-013`).
    pub user: Option<Id<User>>,
    /// The session nonce `peer` derives from. A registry entry of `peer` that
    /// another session wrote claims the PeerID (`AUTHOR-033`).
    pub nonce: ambre_core::FixedBytes<ambre_core::SessionNonce>,
}

/// The nonce the test sessions register under, with their signers.
#[cfg(test)]
pub(crate) const TEST_NONCE: ambre_core::FixedBytes<ambre_core::SessionNonce> =
    ambre_core::FixedBytes::from_array([7; 16]);

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
    Version(Version<JsonUpdate>),
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
    /// An end of a call under another member's key, an end removed, or a
    /// `call_end` map replaced after its message was sent (`MURMURE-593`).
    #[error("an end of a call its signer may not write")]
    ForeignCallEnd,
    /// An amendment of a message its locked segment does not hold
    /// (`MURMURE-013`).
    #[error("an amendment of a message its segment does not hold")]
    NotInSegment,
    /// A message identifier written on a map the delta did not create, written
    /// twice, or already held by another message (`MURMURE-002`, `MURMURE-678`).
    #[error("a message identifier its signer may not write")]
    MessageId,
    /// A delta under a PeerID whose registry entry another session wrote
    /// (`AUTHOR-033`). A second preimage of a member's PeerID buys this.
    #[error("a PeerID another session registered")]
    PeerClaimed,
    /// A write to a root container that members may not write. The `header`
    /// and the `meta` roots close once the document is created. A root the
    /// kind of the document does not hold never opens (`MURMURE-679`).
    #[error("a write to a root container its signer may not write")]
    ForeignRoot,
    /// A fresh link receipt under another member's key, of a thread the
    /// document names nowhere, that is no signature, or removed
    /// (`MURMURE-760`).
    #[error("a fresh link receipt its signer may not write")]
    ForeignReceipt,
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
pub(crate) fn encode(update: &JsonSchema) -> Result<Payload<DeltaBytes>, MurmureError> {
    serde_json::to_vec(update)
        .map(Payload::<DeltaBytes>::new)
        .map_err(|e| MurmureError::Crdt(e.to_string()))
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
pub(crate) fn read(payload: &Payload<DeltaBytes>) -> Result<JsonSchema, Refusal> {
    #[derive(Deserialize)]
    struct Header {
        schema_version: Version<JsonUpdate>,
    }
    let payload = payload.as_bytes();
    if payload.starts_with(b"loro") || payload.starts_with(crate::logic::sequence::FRAME_MAGIC) {
        return Err(Refusal::Binary);
    }
    let version = serde_json::from_slice::<Header>(payload)
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
/// other container. `held` reads what the segment holds already: its header,
/// its operations and its message identifiers. The check takes no document,
/// so it copies none (`MURMURE-226`). Every client runs it on the same delta,
/// so every client reaches the same verdict (`MURMURE-079`).
///
/// # Errors
/// The first [`Refusal`] an operation earns.
pub(crate) fn check(
    update: &JsonSchema,
    signer: Signer,
    registered: impl Fn(ambre_core::PeerId) -> Option<Id<User>>,
    reactions: impl Fn(&ContainerID) -> bool,
    call_ends: impl Fn(&ContainerID) -> bool,
    thread_of: impl Fn(&ContainerID) -> Option<ThreadField>,
    held: Held<'_>,
) -> Result<(), Refusal> {
    let user = signer.user.ok_or(Refusal::Unattributed)?;
    if update
        .changes
        .iter()
        .any(|c| c.id.peer != signer.peer.get())
    {
        return Err(Refusal::NotSigners);
    }
    // The identifiers this delta gives its new messages, each once.
    let mut identities: Vec<Id<Message>> = Vec::new();
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
    // The `call_end` maps of the call messages this delta creates.
    let mut fresh_ends: Vec<&ContainerID> = Vec::new();
    for op in update.changes.iter().flat_map(|c| &c.ops) {
        match &op.container {
            id @ ContainerID::Normal { peer, .. } => {
                // A reactions map is open to every member, each under their
                // own key (`MURMURE-094`).
                if fresh.contains(&id) || reactions(id) {
                    reaction_allowed(&op.content, user)?;
                    continue;
                }
                // The end of a call is open to every member, each under their own
                // key, and is never removed (`MURMURE-593`).
                if fresh_ends.contains(&id) || call_ends(id) {
                    end_allowed(&op.content, user)?;
                    continue;
                }
                // A message, its body and every child of it belong to the
                // user whose PeerID created them. Its `thread` field is the
                // one exception (`MURMURE-255`), with the link receipt the
                // delta that sets it carries (`MURMURE-733`).
                if owner(ambre_core::PeerId::new(*peer)) != Some(user) {
                    let current = thread_of(id);
                    if thread_set(&op.content, current) || receipt_set(&op.content, current) {
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
                    // A message takes its identifier once, with its map. An
                    // amendment and a fold name a message by it, so no other
                    // map may hold it (`MURMURE-678`).
                    JsonOpContent::Map(MapOp::Insert { key, value })
                        if MessageKey::parse(key) == Some(MessageKey::Id) =>
                    {
                        let message = match value {
                            LoroValue::String(text) => message_key(text),
                            _ => None,
                        };
                        let Some(message) = message.filter(|_| created.contains(&id)) else {
                            return Err(Refusal::MessageId);
                        };
                        if identities.contains(&message) || (held.elsewhere)(message, id) {
                            return Err(Refusal::MessageId);
                        }
                        identities.push(message);
                    }
                    JsonOpContent::Map(MapOp::Delete { key })
                        if MessageKey::parse(key) == Some(MessageKey::Id) =>
                    {
                        return Err(Refusal::MessageId);
                    }
                    // The map comes with its message, and is never replaced:
                    // a new one would drop every member's reaction. It is the
                    // container its own operation creates, never one held
                    // already, or the rule of reactions would reach it.
                    JsonOpContent::Map(MapOp::Insert { key, value })
                        if MessageKey::parse(key) == Some(MessageKey::Reactions) =>
                    {
                        match value {
                            LoroValue::Container(map)
                                if created.contains(&id)
                                    && creates(map, ID::new(signer.peer.get(), op.counter)) =>
                            {
                                fresh.push(map);
                            }
                            _ => return Err(Refusal::ForeignReaction),
                        }
                    }
                    JsonOpContent::Map(MapOp::Delete { key })
                        if MessageKey::parse(key) == Some(MessageKey::Reactions) =>
                    {
                        return Err(Refusal::ForeignReaction);
                    }
                    // The map comes with its call message, and is never replaced: a
                    // new one would drop the end another member wrote.
                    JsonOpContent::Map(MapOp::Insert { key, value })
                        if MessageKey::parse(key) == Some(MessageKey::CallEnd) =>
                    {
                        match value {
                            LoroValue::Container(map)
                                if created.contains(&id)
                                    && creates(map, ID::new(signer.peer.get(), op.counter)) =>
                            {
                                fresh_ends.push(map);
                            }
                            _ => return Err(Refusal::ForeignCallEnd),
                        }
                    }
                    JsonOpContent::Map(MapOp::Delete { key })
                        if MessageKey::parse(key) == Some(MessageKey::CallEnd) =>
                    {
                        return Err(Refusal::ForeignCallEnd);
                    }
                    _ => {}
                }
            }
            root if is_root(root, Root::Messages, ContainerType::List) => match &op.content {
                JsonOpContent::List(ListOp::Insert { value, .. }) => {
                    for (offset, item) in value.iter().enumerate() {
                        let LoroValue::Container(id) = item else {
                            return Err(Refusal::ForeignAuthor);
                        };
                        // A message is the map its own operation creates.
                        let counter = i32::try_from(offset)
                            .ok()
                            .and_then(|offset| op.counter.checked_add(offset))
                            .ok_or(Refusal::Malformed)?;
                        if !creates(id, ID::new(signer.peer.get(), counter)) {
                            return Err(Refusal::Malformed);
                        }
                        created.push(id);
                    }
                }
                _ => return Err(Refusal::Removed),
            },
            root if is_root(root, FoundationRoot::Peers, ContainerType::Map) => {
                entry_allowed(&op.content, signer.peer, user)?;
            }
            root if is_root(root, Root::ThreadReceipts, ContainerType::Map) => {
                fresh_allowed(&op.content, user, held.named)?;
            }
            // The header and the version register are the creating delta's
            // alone, and no other root is a segment's (`MURMURE-679`).
            root => founding_allowed(root, op, signer.peer, held)?,
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
/// [`Refusal::ForeignRoot`] marks a write to the header after the creating
/// delta (`MURMURE-679`).
pub(crate) fn check_amendments(
    update: &JsonSchema,
    signer: Signer,
    registered: impl Fn(ambre_core::PeerId) -> Option<Id<User>>,
    author_of: impl Fn(Id<Message>) -> Option<Id<User>>,
    thread_of: impl Fn(Id<Message>) -> ThreadField,
    held: Held<'_>,
) -> Result<(), Refusal> {
    let user = signer.user.ok_or(Refusal::Unattributed)?;
    if update
        .changes
        .iter()
        .any(|c| c.id.peer != signer.peer.get())
    {
        return Err(Refusal::NotSigners);
    }
    for op in update.changes.iter().flat_map(|c| &c.ops) {
        match &op.container {
            ContainerID::Normal { peer, .. } => {
                let owner = if *peer == signer.peer.get() {
                    Some(user)
                } else {
                    registered(ambre_core::PeerId::new(*peer))
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
                    value: LoroValue::Container(entry),
                }) => match message_key(key).and_then(&author_of) {
                    // An entry is the map its own operation creates.
                    _ if !creates(entry, ID::new(signer.peer.get(), op.counter)) => {
                        return Err(Refusal::Malformed);
                    }
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
                    let takes = Id::<Document>::parse_str(value)
                        .is_ok_and(|thread| thread_of(message).takes(thread));
                    if !takes {
                        return Err(Refusal::ForeignMessage);
                    }
                }
                _ => return Err(Refusal::Removed),
            },
            // The link receipt of a recorded thread rides in the delta that
            // records it, while no thread is named (`MURMURE-733`).
            root if is_root(root, Root::Receipts, ContainerType::Map) => match &op.content {
                JsonOpContent::Map(MapOp::Insert {
                    key,
                    value: LoroValue::String(value),
                }) => {
                    let Some(message) = message_key(key).filter(|m| author_of(*m).is_some()) else {
                        return Err(Refusal::NotInSegment);
                    };
                    let takes = thread_of(message) == ThreadField::Unset
                        && ambre_core::FixedBytes::<ambre_core::Signature>::from_hex(value).is_ok();
                    if !takes {
                        return Err(Refusal::ForeignMessage);
                    }
                }
                _ => return Err(Refusal::Removed),
            },
            root if is_root(root, FoundationRoot::Peers, ContainerType::Map) => {
                entry_allowed(&op.content, signer.peer, user)?;
            }
            root if is_root(root, Root::ThreadReceipts, ContainerType::Map) => {
                fresh_allowed(&op.content, user, held.named)?;
            }
            // The header and the version register are the creating delta's
            // alone, and no other root is an amendments document's
            // (`MURMURE-679`).
            root => founding_allowed(root, op, signer.peer, held)?,
        }
    }
    Ok(())
}

/// The message an amendment or a thread key names, spelled as the engine
/// writes an identifier. Any other spelling names no message: a reader looks
/// the key up as written, and would never find it.
fn message_key(key: &str) -> Option<Id<Message>> {
    Id::<Message>::parse_str(key)
        .ok()
        .filter(|id| id.to_string() == key)
}

/// Whether a document holds its header yet, as the check reads it before the
/// first operation of a delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Founding {
    /// The `header` and the `meta` roots are empty: the next delta creates
    /// the document.
    Blank,
    /// The document holds a header, written once by its creating delta.
    Founded,
}

impl Founding {
    /// What `doc` holds now.
    pub(crate) fn of(doc: &LoroDoc) -> Self {
        let empty = |name: &str| schema::held_root_map(doc, name).is_none_or(|map| map.is_empty());
        if empty(Root::Header.as_str()) && empty(FoundationRoot::Meta.as_str()) {
            Self::Blank
        } else {
            Self::Founded
        }
    }
}

/// What the document a delta goes to holds already, as the check of a message
/// identifier and of a root write reads it. Lookups alone, never the document
/// (`MURMURE-226`).
#[derive(Clone, Copy)]
pub(crate) struct Held<'a> {
    /// Whether the document held a header before the delta. Read once, so
    /// the creating delta that writes every key of it passes whole.
    founding: Founding,
    /// The operations the document holds already. An operation it holds is
    /// an echo or a replay, which Loro drops on import.
    version: &'a VersionVector,
    /// Whether a message map other than the given container holds the
    /// identifier.
    elsewhere: &'a dyn Fn(Id<Message>, &ContainerID) -> bool,
    /// Whether a root the document holds names the thread, or records it.
    /// A fresh link receipt of any other thread is refused (`MURMURE-760`).
    named: &'a dyn Fn(Id<Document>) -> bool,
}

/// Whether `content` adds a fresh link receipt that `user` may write
/// (`MURMURE-760`). The key names a thread the document names and `user`
/// alone, and the value reads as a signature. A member thus rewrites only
/// their own entry, and erases or shadows no other member's. A receipt that
/// counts stays beside every one that does not. No entry is removed.
fn fresh_allowed(
    content: &JsonOpContent,
    user: Id<User>,
    named: &dyn Fn(Id<Document>) -> bool,
) -> Result<(), Refusal> {
    let JsonOpContent::Map(MapOp::Insert {
        key,
        value: LoroValue::String(value),
    }) = content
    else {
        return Err(Refusal::ForeignReceipt);
    };
    let takes = schema::FreshKey::parse(key).is_some_and(|key| key.by == user && named(key.thread))
        && ambre_core::FixedBytes::<ambre_core::Signature>::from_hex(value).is_ok();
    if takes {
        Ok(())
    } else {
        Err(Refusal::ForeignReceipt)
    }
}

/// Whether `container` is the one the operation `op` creates. Loro names a
/// created container after its operation, and refuses an update that names
/// another one. A container held already would let a delta reach it through
/// the rule of a fresh one (`MURMURE-681`).
fn creates(container: &ContainerID, op: ID) -> bool {
    matches!(container, ContainerID::Normal { peer, counter, .. } if *peer == op.peer && *counter == op.counter)
}

/// Whether a write to the root `root` is allowed. The creating delta writes
/// plain values in the `header` and the `meta` roots. An operation the
/// document holds already is an echo. Every other root write is refused
/// (`MURMURE-679`).
fn founding_allowed(
    root: &ContainerID,
    op: &loro::json::JsonOp,
    peer: ambre_core::PeerId,
    held: Held<'_>,
) -> Result<(), Refusal> {
    let founding_root = is_root(root, Root::Header, ContainerType::Map)
        || is_root(root, FoundationRoot::Meta, ContainerType::Map);
    if !founding_root {
        return Err(Refusal::ForeignRoot);
    }
    if held.version.includes_id(ID::new(peer.get(), op.counter)) {
        return Ok(());
    }
    match (&op.content, held.founding) {
        (JsonOpContent::Map(MapOp::Insert { value, .. }), Founding::Blank)
            if !matches!(value, LoroValue::Container(_)) =>
        {
            Ok(())
        }
        _ => Err(Refusal::ForeignRoot),
    }
}

/// The one message map of `segment` that holds the identifier `key`. `None`
/// when no map holds it, or when two do: a check then names no author, and
/// refuses rather than pick one (`MURMURE-678`).
fn sole_message(segment: &LoroDoc, key: Id<Message>) -> Option<Fields<MessageKey>> {
    let key = key.to_string();
    let key = key.as_str();
    let list = schema::held_root_list(segment, Root::Messages)?;
    let mut found = (0..list.len())
        .filter_map(|index| list.get(index).and_then(schema::fields_of::<MessageKey>))
        .filter(|map| schema::string(map, MessageKey::Id).as_deref() == Some(key));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// Whether a message map of `doc` other than `container` holds the
/// identifier `key`.
fn held_elsewhere(doc: &LoroDoc, key: Id<Message>, container: &ContainerID) -> bool {
    let key = key.to_string();
    let key = key.as_str();
    let Some(list) = schema::held_root_list(doc, Root::Messages) else {
        return false;
    };
    (0..list.len()).any(|index| match list.get(index) {
        Some(ValueOrContainer::Container(Container::Map(map))) => {
            loro::ContainerTrait::id(&map) != *container
                && schema::string(&Fields::<MessageKey>::new(map), MessageKey::Id).as_deref()
                    == Some(key)
        }
        _ => false,
    })
}

/// The author `segment` holds for the message `key` names, `None` when it
/// holds no such message, or holds the identifier twice.
pub(crate) fn author_in(segment: &LoroDoc, key: Id<Message>) -> Option<Id<User>> {
    let map = sole_message(segment, key)?;
    Id::<User>::parse_str(&schema::string(&map, MessageKey::Author)?).ok()
}

/// What a message's `thread` field holds, as the check reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThreadField {
    /// No thread is named yet.
    Unset,
    /// The thread document the field names.
    Names(Id<Document>),
    /// A value no reader takes for a document. It is set, so it takes no
    /// other.
    Unreadable,
}

impl ThreadField {
    /// The field as `stored` holds it, `None` for an absent one.
    fn read(stored: Option<String>) -> Self {
        stored.map_or(Self::Unset, |stored| {
            Id::<Document>::parse_str(&stored).map_or(Self::Unreadable, Self::Names)
        })
    }

    /// Whether the field takes `thread`: an absent field takes any, and a
    /// set one the same again (`MURMURE-108`).
    fn takes(self, thread: Id<Document>) -> bool {
        match self {
            Self::Unset => true,
            Self::Names(set) => set == thread,
            Self::Unreadable => false,
        }
    }
}

/// The `thread` field `segment` holds for the message `key` names, unset
/// when it holds no such message or no thread. The check of a thread asks
/// [`author_in`] first, so an identifier held twice never reaches here.
fn thread_in(segment: &LoroDoc, key: Id<Message>) -> ThreadField {
    ThreadField::read(
        sole_message(segment, key).and_then(|map| schema::string(&map, MessageKey::Thread)),
    )
}

/// Whether a message of the segment `doc` names `thread` in its `thread`
/// field.
fn names_thread(doc: &LoroDoc, thread: Id<Document>) -> bool {
    let Some(list) = schema::held_root_list(doc, Root::Messages) else {
        return false;
    };
    let wanted = thread.to_string();
    (0..list.len()).any(|index| match list.get(index) {
        Some(ValueOrContainer::Container(Container::Map(map))) => {
            schema::string(&Fields::<MessageKey>::new(map), MessageKey::Thread).as_deref()
                == Some(wanted.as_str())
        }
        _ => false,
    })
}

/// Whether a write to another author's message sets its `thread` field to a
/// thread document, where `current` is what the field holds. An absent field
/// takes one, and a set one takes the same again (`MURMURE-108`).
///
/// The verdict reads the field as the receiver holds it. A member who sets a
/// first value thus refuses every other one, the relay's included. The relay
/// links one document per root (`SRV-214`), so only one value is due.
fn thread_set(content: &JsonOpContent, current: Option<ThreadField>) -> bool {
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
        && Id::<Document>::parse_str(value).is_ok_and(|thread| current.takes(thread))
}

/// Whether a write to another author's message sets its link receipt while
/// its `thread` field is absent, as the receiver holds it (`MURMURE-733`).
/// The delta that sets the field thus carries the receipt, and no later
/// delta of another member replaces it.
fn receipt_set(content: &JsonOpContent, current: Option<ThreadField>) -> bool {
    let JsonOpContent::Map(MapOp::Insert {
        key,
        value: LoroValue::String(value),
    }) = content
    else {
        return false;
    };
    current == Some(ThreadField::Unset)
        && MessageKey::parse(key) == Some(MessageKey::Receipt)
        && ambre_core::FixedBytes::<ambre_core::Signature>::from_hex(value).is_ok()
}

/// The `thread` field of `id` when it is a message map of `doc`, `None` for
/// any other container.
pub(crate) fn thread_field(doc: &LoroDoc, id: &ContainerID) -> Option<ThreadField> {
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
    Some(ThreadField::read(schema::string(
        &Fields::new(map),
        MessageKey::Thread,
    )))
}

/// Whether a registry write is the signer's own: its PeerID's key alone. An
/// entry that reads as one must name the signer. An unreadable one names
/// nobody, so it grants nothing.
fn entry_allowed(
    content: &JsonOpContent,
    peer: ambre_core::PeerId,
    user: Id<User>,
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
fn reaction_allowed(content: &JsonOpContent, user: Id<User>) -> Result<(), Refusal> {
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

/// Whether `content` writes the end of `user` alone: an insert of a plain value under
/// their own key (`MURMURE-593`). An end is never deleted.
fn end_allowed(content: &JsonOpContent, user: Id<User>) -> Result<(), Refusal> {
    match content {
        JsonOpContent::Map(MapOp::Insert { key, value })
            if crate::logic::call::is_own_end(key, user)
                && !matches!(value, LoroValue::Container(_)) =>
        {
            Ok(())
        }
        _ => Err(Refusal::ForeignCallEnd),
    }
}

/// The messages a JSON update creates, in operation order. A message takes
/// its identity once, when it is created, so a later write never names it.
pub(crate) fn created_messages(update: &JsonSchema) -> Vec<Id<Message>> {
    update
        .changes
        .iter()
        .flat_map(|c| &c.ops)
        .filter_map(|op| match &op.content {
            JsonOpContent::Map(MapOp::Insert {
                key,
                value: LoroValue::String(id),
            }) if MessageKey::parse(key) == Some(MessageKey::Id) => {
                Id::<Message>::parse_str(id).ok()
            }
            _ => None,
        })
        .collect()
}

/// Each message `update` creates, with the operation that created its map:
/// the map whose `id` the update writes (`MURMURE-674`). The engine numbers
/// these alone, with no walk of the segment.
pub(crate) fn created_with_ops(update: &JsonSchema) -> Vec<(Id<Message>, ID)> {
    update
        .changes
        .iter()
        .flat_map(|c| &c.ops)
        .filter_map(|op| match (&op.container, &op.content) {
            (
                ContainerID::Normal { peer, counter, .. },
                JsonOpContent::Map(MapOp::Insert {
                    key,
                    value: LoroValue::String(id),
                }),
            ) if MessageKey::parse(key) == Some(MessageKey::Id) => Id::<Message>::parse_str(id)
                .ok()
                .map(|id| (id, ID::new(*peer, *counter))),
            _ => None,
        })
        .collect()
}

/// Whether `value` is the identifier of `user`.
fn names(value: &LoroValue, user: Id<User>) -> bool {
    matches!(value, LoroValue::String(s) if Id::<User>::parse_str(s).is_ok_and(|u| u == user))
}

fn is_root(id: &ContainerID, root: impl ContainerName, kind: ContainerType) -> bool {
    matches!(id, ContainerID::Root { name, container_type } if name.as_str() == root.as_str() && *container_type == kind)
}

/// The user the registry of `doc` names for `peer`, `None` when its entry is
/// absent or unreadable (`AUTHOR-006`).
pub(crate) fn registered(doc: &LoroDoc, peer: ambre_core::PeerId) -> Option<Id<User>> {
    let ValueOrContainer::Value(LoroValue::Binary(bytes)) =
        schema::held_root_map(doc, FoundationRoot::Peers.as_str())?
            .get(ambre_core::PeerKey::of(peer).as_str())?
    else {
        return None;
    };
    PeerEntry::from_bytes(&bytes).map(|entry| entry.user)
}

/// Whether the registry of `doc` leaves the PeerID of `signer` free for its
/// session (`AUTHOR-033`). A blank document holds no registry, and claims
/// nothing.
pub(crate) fn peer_unclaimed(doc: &LoroDoc, signer: Signer) -> bool {
    schema::held_root_map(doc, FoundationRoot::Peers.as_str()).is_none_or(|registry| {
        ambre_crdt::session::peer_unclaimed(&registry, signer.peer, &signer.nonce)
    })
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
    Bytes(LoroPayload),
    /// A delta a member signed, read and not checked yet.
    Signed { update: JsonSchema, signer: Signer },
}

/// One payload of a binary document, a channel `meta` or a space, kept until
/// the document opens. The signer of a delta stays with its bytes, so the
/// open checks its PeerID (`AUTHOR-037`).
#[derive(Debug, Clone)]
pub(crate) enum Kept {
    /// A snapshot, or a delta read back from the local store: Loro bytes,
    /// checked where they came from.
    Bytes(LoroPayload),
    /// A delta a member signed, whose PeerID is not checked yet.
    Signed {
        payload: Payload<DeltaBytes>,
        signer: Signer,
    },
}

/// Import `kept` into `doc`, in order. A signed delta passes the PeerID check
/// first, against the registry `doc` holds so far. A refused one is left out
/// and changes nothing (`AUTHOR-037`). So the first session that registers a
/// PeerID keeps it, as in an open document.
///
/// # Errors
/// [`MurmureError::Schema`] for Loro bytes the engine refuses.
pub(crate) fn import_kept(
    doc: &LoroDoc,
    kept: &[Kept],
    document_id: Id<Document>,
) -> Result<(), MurmureError> {
    for payload in kept {
        match payload {
            Kept::Bytes(bytes) => {
                crate::logic::schema::import_bytes(doc, bytes.borrowed(), document_id)?;
            }
            Kept::Signed { payload, signer } => {
                if !peer_unclaimed(doc, *signer) {
                    tracing::warn!(
                        "[Murmure] a delta for {document_id} refused: {}",
                        Refusal::PeerClaimed
                    );
                    continue;
                }
                crate::logic::schema::import_bytes(doc, payload.into(), document_id)?;
            }
        }
    }
    Ok(())
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
    if !peer_unclaimed(doc, signer) {
        return Err(Refusal::PeerClaimed);
    }
    let version = doc.oplog_vv();
    let elsewhere = |key: Id<Message>, container: &ContainerID| held_elsewhere(doc, key, container);
    let named = |thread: Id<Document>| match rules {
        Rules::Segment => names_thread(doc, thread),
        Rules::Thread => false,
        Rules::Amendments { segment } => {
            names_thread(segment, thread)
                || schema::held_root_map(doc, Root::Threads.as_str()).is_some_and(|threads| {
                    let wanted = thread.to_string();
                    let mut found = false;
                    threads.for_each(|_, value| {
                        found |= matches!(
                            value,
                            ValueOrContainer::Value(LoroValue::String(held)) if *held == *wanted
                        );
                    });
                    found
                })
        }
    };
    let held = Held {
        founding: Founding::of(doc),
        version: &version,
        elsewhere: &elsewhere,
        named: &named,
    };
    match rules {
        Rules::Segment => check(
            update,
            signer,
            |peer| registered(doc, peer),
            |id| reaction::is_map(doc, id),
            |id| crate::logic::call::is_end_map(doc, id),
            |id| thread_field(doc, id),
            held,
        ),
        Rules::Thread => check(
            update,
            signer,
            |peer| registered(doc, peer),
            |id| reaction::is_map(doc, id),
            |_| false,
            |_| None,
            held,
        ),
        Rules::Amendments { segment } => check_amendments(
            update,
            signer,
            |peer| registered(doc, peer),
            |key| author_in(segment, key),
            |key| {
                schema::string_value(
                    schema::held_root_map(doc, Root::Threads.as_str())
                        .and_then(|threads| threads.get(&key.to_string())),
                )
                .map_or_else(
                    || thread_in(segment, key),
                    |named| ThreadField::read(Some(named)),
                )
            },
            held,
        ),
    }
}

/// Import `arrivals` into `doc`, in order. A signed delta passes the check
/// first, against the registry `doc` holds so far. A refused one is left out,
/// and changes nothing (`MURMURE-030`). So is a checked one Loro refuses: one
/// member's delta never makes the document unopenable (`MURMURE-680`).
///
/// # Errors
/// [`MurmureError::Schema`] for Loro bytes the engine refuses.
pub(crate) fn import_arrivals(
    doc: &LoroDoc,
    arrivals: Vec<Arrival>,
    document_id: Id<Document>,
    rules: Rules<'_>,
) -> Result<(), MurmureError> {
    for arrival in arrivals {
        match arrival {
            Arrival::Bytes(bytes) => {
                crate::logic::schema::import_bytes(doc, bytes.borrowed(), document_id)?
            }
            Arrival::Signed { update, signer } => {
                let verdict = verdict(doc, rules, &update, signer);
                if let Err(refusal) = verdict {
                    tracing::warn!("[Murmure] a delta for {document_id} refused: {refusal}");
                    continue;
                }
                // Loro decodes the whole update before it applies any of it,
                // so a refused update changes nothing.
                if let Err(error) = doc.import_json_updates(update) {
                    tracing::warn!("[Murmure] a delta for {document_id} Loro refused: {error}");
                }
            }
        }
    }
    Ok(())
}

/// Whether every operation of a binary update was written under `peer`
/// (`AUTHOR-012`). The `meta` document travels binary, and is checked so.
pub(crate) fn binary_by(payload: &Payload<DeltaBytes>, peer: ambre_core::PeerId) -> bool {
    LoroDoc::decode_import_blob_meta(payload.as_bytes(), false).is_ok_and(|meta| {
        meta.mode == loro::EncodedBlobMode::Updates
            && meta.partial_end_vv.keys().all(|p| *p == peer.get())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absent thread field takes any thread, a set one the same again,
    /// and one no reader takes for a document takes none.
    #[test]
    fn a_thread_field_takes_a_thread_only_when_absent_or_the_same() {
        let (thread, other) = (Id::<Document>::mint(), Id::<Document>::mint());
        assert!(ThreadField::read(None).takes(thread));
        let set = ThreadField::read(Some(thread.to_string()));
        assert_eq!(set, ThreadField::Names(thread));
        assert!(set.takes(thread));
        assert!(!set.takes(other));
        let unreadable = ThreadField::read(Some("not a document".to_owned()));
        assert_eq!(unreadable, ThreadField::Unreadable);
        assert!(!unreadable.takes(thread));
    }

    /// A segment that holds one message under an identifier names its
    /// author. A segment that holds the identifier twice names none, so an
    /// amendment of it is refused rather than bound to the wrong message
    /// (`MURMURE-678`).
    #[test]
    fn an_identifier_held_twice_names_no_author() {
        let (message, alice, mallory) = (
            Id::<Message>::mint(),
            Id::<User>::mint(),
            Id::<User>::mint(),
        );
        let segment = LoroDoc::new();
        let push = |author: Id<User>| {
            let map = Fields::<MessageKey>::new(
                crate::logic::schema::root_list(&segment, Root::Messages)
                    .push_container(loro::LoroMap::new())
                    .expect("push"),
            );
            map.insert(MessageKey::Id, message.to_string()).expect("id");
            map.insert(MessageKey::Author, author.to_string())
                .expect("author");
            map.insert(MessageKey::Thread, Id::<Document>::mint().to_string())
                .expect("thread");
        };
        push(alice);
        assert_eq!(author_in(&segment, message), Some(alice));
        assert!(matches!(
            thread_in(&segment, message),
            ThreadField::Names(_)
        ));
        push(mallory);
        assert_eq!(author_in(&segment, message), None);
        assert_eq!(thread_in(&segment, message), ThreadField::Unset);
    }
}
