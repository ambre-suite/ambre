//! Public (non‑encrypted) metadata attached to updates, snapshots and ephemerals.
//!
//! These fields are visible to the Blind Relay and are used for routing, integrity
//! verification, and conflict resolution without decrypting the payload.

use ambre_core::{PublicKey, Uuid};
use serde::{Deserialize, Serialize};

use crate::messages::{Clock, Counter};

/// Public data of an [`Update`](crate::messages::Update).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePublicData {
    /// Document identifier.
    pub document_id: Uuid,
    /// Ed25519 public key of the author.
    pub pub_key: PublicKey,
    /// Snapshot identifier that this update is based on.
    pub snapshot_id: Uuid,
    /// Monotonic logical clock (Lamport timestamp) of the author for this document.
    pub clock: Clock,
}

/// Public data of a [`Snapshot`](crate::messages::Snapshot).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPublicData {
    /// Document identifier.
    pub document_id: Uuid,
    /// Ed25519 public key of the snapshot creator.
    pub pub_key: PublicKey,
    /// Unique identifier of this snapshot.
    pub snapshot_id: Uuid,
    /// BLAKE2b hash chain proving integrity of the parent snapshot without decrypting it.
    pub parent_snapshot_proof: Vec<u8>,
    /// For each author, the latest clock value included in this snapshot.
    ///
    /// The vector contains `(pub_key, clock)` pairs where `pub_key` is the author's
    /// 32‑byte Ed25519 public key and `clock` is the highest clock value of that author
    /// that is reflected in the snapshot.
    pub parent_snapshot_update_clocks: Vec<(PublicKey, Clock)>,
}

/// Public data of an [`Ephemeral`](crate::messages::Ephemeral).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EphemeralPublicData {
    /// Document identifier.
    pub document_id: Uuid,
    /// Ed25519 public key of the sender.
    pub pub_key: PublicKey,
    /// Monotonic counter for ordering ephemerals from the same author.
    pub counter: Counter,
}
