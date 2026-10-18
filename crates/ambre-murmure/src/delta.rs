//! The authorship check of a segment delta
//! (`features/ambre-spec-murmure-messages.md section 3`).
//!
//! A segment delta travels as a Loro JSON update (`MURMURE-223`), so its
//! operations read with no document. The check runs on each received delta
//! before import. It refuses the whole delta on the first operation its signer
//! had no right to write. Only the peer registry of the segment is read, to
//! learn which user a PeerID stands for (`ambre-spec-op-authorship.md`).

use ambre_core::authorship::{PEERS_MAP, PeerEntry, peer_key};
use ambre_core::{DocumentId, UserId};
use loro::json::{JsonOpContent, JsonSchema, ListOp, MapOp};
use loro::{ContainerID, ContainerType, LoroDoc, LoroValue, ValueOrContainer, VersionVector};
use serde::Deserialize;

use crate::error::MurmureError;
use crate::schema::{MESSAGE_AUTHOR, MESSAGES};

/// The Loro JSON schema version a segment delta carries (`MURMURE-227`). A
/// change of it is a bump of `SCHEMA_VERSION` (`MURMURE-020`).
pub const JSON_SCHEMA_VERSION: u8 = 1;

/// Who signed a delta, as the Sync worker verified it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signer {
    /// The only PeerID the operations of the delta may carry (`AUTHOR-012`).
    pub peer: u64,
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
    Version(u8),
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
        schema_version: u8,
    }
    if payload.starts_with(b"loro") || payload.starts_with(b"amsq") {
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
        if !is_root(&op.container, PEERS_MAP, ContainerType::Map) {
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
/// the registry of the segment holds it. The check takes no document, so it
/// copies none (`MURMURE-226`). Every client runs it on the same delta, so
/// every client reaches the same verdict (`MURMURE-079`).
///
/// # Errors
/// The first [`Refusal`] an operation earns.
pub(crate) fn check(
    update: &JsonSchema,
    signer: Signer,
    registered: impl Fn(u64) -> Option<UserId>,
) -> Result<(), Refusal> {
    let user = signer.user.ok_or(Refusal::Unattributed)?;
    if update.changes.iter().any(|c| c.id.peer != signer.peer) {
        return Err(Refusal::NotSigners);
    }
    // A container this session created is the signer's own.
    let owner = |peer: u64| {
        if peer == signer.peer {
            Some(user)
        } else {
            registered(peer)
        }
    };
    let mut created: Vec<&ContainerID> = Vec::new();
    let mut authored: Vec<&ContainerID> = Vec::new();
    for op in update.changes.iter().flat_map(|c| &c.ops) {
        match &op.container {
            ContainerID::Normal { peer, .. } => {
                // A message, its body and every child of it belong to the
                // user whose PeerID created them.
                if owner(*peer) != Some(user) {
                    return Err(Refusal::ForeignMessage);
                }
                if let JsonOpContent::Map(MapOp::Insert { key, value }) = &op.content
                    && key == MESSAGE_AUTHOR
                {
                    if !names(value, user) {
                        return Err(Refusal::ForeignAuthor);
                    }
                    authored.push(&op.container);
                }
            }
            root if is_root(root, MESSAGES, ContainerType::List) => match &op.content {
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
            root if is_root(root, PEERS_MAP, ContainerType::Map) => {
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

/// Whether a registry write is the signer's own: its PeerID's key alone. An
/// entry that reads as one must name the signer. An unreadable one names
/// nobody, so it grants nothing.
fn entry_allowed(content: &JsonOpContent, peer: u64, user: UserId) -> Result<(), Refusal> {
    let JsonOpContent::Map(MapOp::Insert { key, value }) = content else {
        return Err(Refusal::ForeignEntry);
    };
    if *key != peer_key(peer) {
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

/// Whether `value` is the identifier of `user`.
fn names(value: &LoroValue, user: UserId) -> bool {
    matches!(value, LoroValue::String(s) if UserId::parse_str(s).is_ok_and(|u| u == user))
}

fn is_root(id: &ContainerID, root: &str, kind: ContainerType) -> bool {
    matches!(id, ContainerID::Root { name, container_type } if name.as_str() == root && *container_type == kind)
}

/// The user the registry of `doc` names for `peer`, `None` when its entry is
/// absent or unreadable (`AUTHOR-006`).
pub(crate) fn registered(doc: &LoroDoc, peer: u64) -> Option<UserId> {
    let ValueOrContainer::Value(LoroValue::Binary(bytes)) =
        doc.get_map(PEERS_MAP).get(&peer_key(peer))?
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
) -> Result<(), MurmureError> {
    for arrival in arrivals {
        match arrival {
            Arrival::Bytes(bytes) => crate::schema::import_bytes(doc, &bytes, document_id)?,
            Arrival::Signed { update, signer } => {
                if let Err(refusal) = check(&update, signer, |peer| registered(doc, peer)) {
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
pub(crate) fn binary_by(payload: &[u8], peer: u64) -> bool {
    LoroDoc::decode_import_blob_meta(payload, false).is_ok_and(|meta| {
        meta.mode == loro::EncodedBlobMode::Updates
            && meta.partial_end_vv.keys().all(|p| *p == peer)
    })
}
