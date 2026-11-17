//! Public (non-encrypted) metadata attached to updates, snapshots and ephemerals.
//!
//! These fields are visible to the Blind Relay and are used for routing, integrity
//! verification, and conflict resolution without decrypting the payload.

use ambre_core::id::{Document, Id, Snapshot};
use ambre_core::{ChainHash, FixedBytes, KeyId, PublicKey};
use serde::{Deserialize, Serialize};

use crate::membership::{LogHead, MembershipEntry};
use crate::messages::{DeltaAuthor, Ephemerals, Relay, Seq};

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
    pub clock: Seq<DeltaAuthor>,
    /// The head of the membership log the author last read (`SYNC-206`). The
    /// signature covers it, so the relay can drop the update and never
    /// rewrite the head.
    pub log_head: LogHead,
    /// The entry `log_head` names: the last entry of the log the author
    /// read, absent on an empty log (`SYNC-218`). Its own author signed it,
    /// so a peer can check that a member the log allows wrote the entry. A
    /// head with no such entry blames nobody. The update signature covers
    /// it, so the relay cannot strip it and keep the head. Boxed, so an
    /// update stays a small value on the stack and in each frame `enum`.
    pub log_entry: Option<Box<MembershipEntry>>,
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
    /// The horizon: the last relay number of the deltas the snapshot folds
    /// in. The signature covers it, so a relay that names another horizon
    /// for the snapshot is caught at the open (`SYNC-259`).
    pub covered_seq: Seq<Relay>,
    /// BLAKE2b hash chain proving integrity of the parent snapshot without
    /// decrypting it. `None` on the first snapshot, whose parent is the
    /// genesis base.
    pub parent_snapshot_proof: Option<FixedBytes<ChainHash>>,
    /// For each author, the latest clock value included in this snapshot.
    ///
    /// The vector contains `(pub_key, clock)` pairs. `pub_key` is the author's 32-byte
    /// Ed25519 public key, and `clock` is the highest clock value of that author
    /// reflected in the snapshot.
    pub parent_snapshot_update_clocks: Vec<(FixedBytes<PublicKey>, Seq<DeltaAuthor>)>,
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
    pub counter: Seq<Ephemerals>,
    /// The head of the membership log the sender last read (`SYNC-262`).
    /// Presence flows while nobody writes, so a relay that drops every
    /// update towards one member still lets this head through. It stays
    /// public: the sample a hidden rotation brings is sealed under a key
    /// that member lacks (`SYNC-263`).
    pub log_head: LogHead,
    /// The entry `log_head` names, absent on an empty log. A head counts
    /// only when this entry proves it, as on an update (`SYNC-219`). Boxed
    /// for the reason the update gives.
    pub log_entry: Option<Box<MembershipEntry>>,
}
