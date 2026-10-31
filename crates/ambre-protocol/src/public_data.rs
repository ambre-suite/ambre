//! Public (non-encrypted) metadata attached to updates, snapshots and ephemerals.
//!
//! These fields are visible to the Blind Relay and are used for routing, integrity
//! verification, and conflict resolution without decrypting the payload.

use ambre_core::id::{Document, Id, Snapshot};
use ambre_core::{ChainHash, FixedBytes, KeyId, PublicKey};
use serde::{Deserialize, Serialize};

use crate::messages::{Clock, Counter};

/// Public data of an [`Update`](crate::messages::Update).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePublicData {
    /// Document identifier.
    pub document_id: Id<Document>,
    /// Ed25519 public key of the author.
    pub pub_key: FixedBytes<PublicKey>,
    /// Identifier of the document key that sealed the update (`SYNC-125`).
    pub key_id: FixedBytes<KeyId>,
    /// Snapshot identifier that this update is based on.
    pub snapshot_id: Id<Snapshot>,
    /// Monotonic logical clock (Lamport timestamp) of the author for this document.
    pub clock: Clock,
}

/// Public data of a [`Snapshot`](crate::messages::Snapshot).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPublicData {
    /// Document identifier.
    pub document_id: Id<Document>,
    /// Ed25519 public key of the snapshot creator.
    pub pub_key: FixedBytes<PublicKey>,
    /// Identifier of the document key that sealed the snapshot (`SYNC-125`).
    pub key_id: FixedBytes<KeyId>,
    /// Unique identifier of this snapshot.
    pub snapshot_id: Id<Snapshot>,
    /// BLAKE2b hash chain proving integrity of the parent snapshot without
    /// decrypting it. `None` on the first snapshot, whose parent is the
    /// genesis base.
    pub parent_snapshot_proof: Option<FixedBytes<ChainHash>>,
    /// For each author, the latest clock value included in this snapshot.
    ///
    /// The vector contains `(pub_key, clock)` pairs. `pub_key` is the author's 32-byte
    /// Ed25519 public key, and `clock` is the highest clock value of that author
    /// reflected in the snapshot.
    pub parent_snapshot_update_clocks: Vec<(FixedBytes<PublicKey>, Clock)>,
}

/// Public data of an [`Ephemeral`](crate::messages::Ephemeral).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EphemeralPublicData {
    /// Document identifier.
    pub document_id: Id<Document>,
    /// Ed25519 public key of the sender.
    pub pub_key: FixedBytes<PublicKey>,
    /// Identifier of the document key that sealed the frame (`SYNC-125`).
    pub key_id: FixedBytes<KeyId>,
    /// Monotonic counter for ordering ephemerals from the same author.
    pub counter: Counter,
}
