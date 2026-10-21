//! Wire messages: updates, snapshots, ephemerals, and sequenced updates.
//!
//! This module defines the core encrypted payloads that travel between client and server.
//! All fields are opaque to the Blind Relay; only the public data is visible.

use crate::error::ValidationError;
use crate::public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
use serde::{Deserialize, Serialize};
use serde_bytes;
use std::collections::HashSet;

// -----------------------------------------------------------------------------
// Newtype wrappers for binary buffers
// -----------------------------------------------------------------------------

/// 24-byte nonce used for XChaCha20-Poly1305 encryption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Nonce(#[serde(with = "serde_bytes")] pub [u8; 24]);

/// 32-byte HMAC-SHA256 commitment tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommitmentTag(#[serde(with = "serde_bytes")] pub [u8; 32]);

/// 64-byte Ed25519 signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Signature(#[serde(with = "serde_bytes")] pub [u8; 64]);

// -----------------------------------------------------------------------------
// Core wire messages
/// Server-assigned monotonic sequence number, shared with the client threads.
pub use ambre_core::Seq;

/// Lamport logical clock (client-side).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Clock(pub u64);

impl Clock {
    /// The clock as a Postgres `BIGINT` stores it, saturating at `i64::MAX`.
    pub fn to_i64_saturating(self) -> i64 {
        i64::try_from(self.0).unwrap_or(i64::MAX)
    }
}

/// A stored clock: a negative one is refused, never clamped.
impl TryFrom<i64> for Clock {
    type Error = std::num::TryFromIntError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        u64::try_from(value).map(Self)
    }
}

impl std::fmt::Display for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Ephemeral counter (client-side).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Counter(pub u64);

/// AEAD ciphertext wrapper. `Debug` prints its length and none of its bytes,
/// so a logged message logs no ciphertext (`ZK-006`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ciphertext(#[serde(with = "serde_bytes")] pub Vec<u8>);

impl Ciphertext {
    /// The length in bytes, the one reading of a ciphertext a log may carry.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether it holds no byte.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Ciphertext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ciphertext({} bytes)", self.0.len())
    }
}

// -----------------------------------------------------------------------------

/// An encrypted, signed update carrying a CRDT delta.
///
/// The update is encrypted with the document's symmetric key (`DocumentKey`).
/// The `commitment_tag` is an HMAC-SHA256 over the ciphertext and public data,
/// binding the message to a specific key (key-commitment).
/// The signature is Ed25519 over the nonce, ciphertext, public data, and commitment tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    /// AEAD ciphertext (XChaCha20-Poly1305).
    pub ciphertext: Ciphertext,
    /// 24-byte nonce used for encryption.
    pub nonce: Nonce,
    /// Public metadata visible to the Blind Relay.
    pub public_data: UpdatePublicData,
    /// HMAC-SHA256(document_key, nonce || ciphertext || public_data_bytes)
    pub commitment_tag: CommitmentTag,
    /// Ed25519 signature of (nonce || ciphertext || public_data_bytes || commitment_tag)
    pub signature: Signature,
}

impl Update {
    /// Checks the semantic invariants a well-formed decode can still violate:
    /// a non-empty ciphertext and a clock that started at 1. Fixed-size fields
    /// (nonce, tags, keys) are already length-checked by postcard. Called on the
    /// relay's decode path before a delta is signed-checked, sequenced, and
    /// persisted, so a structurally valid but meaningless delta is refused early.
    ///
    /// # Errors
    ///
    /// [`ValidationError::EmptyCiphertext`] on an empty ciphertext, and
    /// [`ValidationError::ZeroClock`] on a clock of 0, which no author writes.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.ciphertext.0.is_empty() {
            return Err(ValidationError::EmptyCiphertext);
        }
        if self.public_data.clock.0 == 0 {
            return Err(ValidationError::ZeroClock);
        }
        Ok(())
    }
}

/// An encrypted, signed snapshot carrying a full CRDT state.
///
/// Snapshots are uploaded/downloaded via S3 presigned URLs, not through WebSocket.
/// The same cryptographic envelope as `Update` is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// AEAD ciphertext (XChaCha20-Poly1305).
    pub ciphertext: Ciphertext,
    /// 24-byte nonce used for encryption.
    pub nonce: Nonce,
    /// Public metadata visible to the Blind Relay.
    pub public_data: SnapshotPublicData,
    /// HMAC-SHA256(document_key, nonce || ciphertext || public_data_bytes)
    pub commitment_tag: CommitmentTag,
    /// Ed25519 signature of (nonce || ciphertext || public_data_bytes || commitment_tag)
    pub signature: Signature,
}

impl Snapshot {
    /// Checks the semantic invariants of a decoded snapshot: a non-empty ciphertext
    /// and no author appearing twice in `parent_snapshot_update_clocks`. Called by
    /// the client before upload (snapshots travel client-to-S3 directly, so the
    /// Blind Relay never sees this struct to enforce it).
    ///
    /// The parent proof is deliberately NOT required to be non-empty: the first
    /// snapshot of a document has the genesis base as its parent. That first
    /// snapshot carries no proof (see [`ambre_core::SnapshotId::genesis`]).
    ///
    /// # Errors
    ///
    /// [`ValidationError::EmptyCiphertext`] on an empty ciphertext, and
    /// [`ValidationError::DuplicatePublicKey`] when an author appears twice in
    /// `parent_snapshot_update_clocks`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.ciphertext.0.is_empty() {
            return Err(ValidationError::EmptyCiphertext);
        }
        let mut seen = HashSet::new();
        for (pub_key, _) in &self.public_data.parent_snapshot_update_clocks {
            if !seen.insert(pub_key) {
                return Err(ValidationError::DuplicatePublicKey);
            }
        }
        Ok(())
    }
}

/// An encrypted, signed ephemeral message, such as a cursor or a presence beat.
///
/// Ephemerals are not stored permanently; they are broadcast to collaborators
/// in real-time and then dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ephemeral {
    /// AEAD ciphertext (XChaCha20-Poly1305).
    pub ciphertext: Ciphertext,
    /// 24-byte nonce used for encryption.
    pub nonce: Nonce,
    /// Public metadata visible to the Blind Relay.
    pub public_data: EphemeralPublicData,
    /// HMAC-SHA256(document_key, nonce || ciphertext || public_data_bytes)
    pub commitment_tag: CommitmentTag,
    /// Ed25519 signature of (nonce || ciphertext || public_data_bytes || commitment_tag)
    pub signature: Signature,
}

impl Ephemeral {
    /// Checks the semantic invariants a well-formed decode can still violate:
    /// a non-empty ciphertext and a counter that started at 1. Called on the
    /// relay's decode path before the presence blob is broadcast and written to
    /// the presence hash.
    ///
    /// # Errors
    ///
    /// [`ValidationError::EmptyCiphertext`] on an empty ciphertext, and
    /// [`ValidationError::ZeroCounter`] on a counter of 0.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.ciphertext.0.is_empty() {
            return Err(ValidationError::EmptyCiphertext);
        }
        if self.public_data.counter.0 == 0 {
            return Err(ValidationError::ZeroCounter);
        }
        Ok(())
    }
}

/// An update paired with its server-assigned sequence number.
///
/// Used in `MissingUpdates` responses to allow clients to catch up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeqUpdate {
    /// Monotonic sequence number assigned by the server.
    pub seq: Seq,
    /// The update itself.
    pub update: Update,
}

#[cfg(test)]
mod validate_tests {
    use super::*;
    use crate::public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
    use ambre_core::{DocumentId, KeyId, PublicKey, SnapshotId};

    fn update(cipher: Vec<u8>, clock: u64) -> Update {
        Update {
            ciphertext: Ciphertext(cipher),
            nonce: Nonce([0u8; 24]),
            public_data: UpdatePublicData {
                document_id: DocumentId::from_bytes([1u8; 16]),
                pub_key: PublicKey::from_array([2u8; 32]),
                key_id: KeyId::from_array([4u8; 16]),
                snapshot_id: SnapshotId::from_bytes([3u8; 16]),
                clock: Clock(clock),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        }
    }

    fn ephemeral(cipher: Vec<u8>, counter: u64) -> Ephemeral {
        Ephemeral {
            ciphertext: Ciphertext(cipher),
            nonce: Nonce([0u8; 24]),
            public_data: EphemeralPublicData {
                document_id: DocumentId::from_bytes([1u8; 16]),
                pub_key: PublicKey::from_array([2u8; 32]),
                key_id: KeyId::from_array([4u8; 16]),
                counter: Counter(counter),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        }
    }

    #[test]
    fn update_valid() {
        assert!(update(vec![1], 1).validate().is_ok());
    }

    /// Holds `ZK-006` for a decoded message: a ciphertext prints its length
    /// alone, so the debug form of anything carrying one prints no byte of it.
    #[test]
    fn a_message_debug_form_prints_no_byte_of_its_ciphertext() {
        let printed = format!(
            "{:?} {:?}",
            update(vec![0xAB; 3], 1),
            ephemeral(vec![0xAB; 5], 1)
        );
        assert!(printed.contains("Ciphertext(3 bytes)"), "{printed}");
        assert!(printed.contains("Ciphertext(5 bytes)"), "{printed}");
        assert!(!printed.contains("171"), "{printed}");
    }

    #[test]
    fn update_rejects_empty_ciphertext() {
        assert_eq!(
            update(vec![], 1).validate(),
            Err(ValidationError::EmptyCiphertext)
        );
    }

    #[test]
    fn update_rejects_zero_clock() {
        assert_eq!(
            update(vec![1], 0).validate(),
            Err(ValidationError::ZeroClock)
        );
    }

    #[test]
    fn ephemeral_valid() {
        assert!(ephemeral(vec![1], 1).validate().is_ok());
    }

    #[test]
    fn ephemeral_rejects_empty_ciphertext() {
        assert_eq!(
            ephemeral(vec![], 1).validate(),
            Err(ValidationError::EmptyCiphertext)
        );
    }

    #[test]
    fn ephemeral_rejects_zero_counter() {
        assert_eq!(
            ephemeral(vec![1], 0).validate(),
            Err(ValidationError::ZeroCounter)
        );
    }

    fn snapshot(cipher: Vec<u8>, proof: Vec<u8>, clocks: Vec<(PublicKey, Clock)>) -> Snapshot {
        Snapshot {
            ciphertext: Ciphertext(cipher),
            nonce: Nonce([0u8; 24]),
            public_data: SnapshotPublicData {
                document_id: DocumentId::from_bytes([1u8; 16]),
                pub_key: PublicKey::from_array([2u8; 32]),
                key_id: KeyId::from_array([4u8; 16]),
                snapshot_id: SnapshotId::from_bytes([3u8; 16]),
                parent_snapshot_proof: proof,
                parent_snapshot_update_clocks: clocks,
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        }
    }

    #[test]
    fn snapshot_valid() {
        let clocks = vec![
            (PublicKey::from_array([9u8; 32]), Clock(1)),
            (PublicKey::from_array([8u8; 32]), Clock(2)),
        ];
        assert!(snapshot(vec![1], vec![1], clocks).validate().is_ok());
    }

    #[test]
    fn snapshot_valid_with_empty_genesis_proof() {
        // The first snapshot has the genesis base as parent and no proof.
        assert!(snapshot(vec![1], vec![], vec![]).validate().is_ok());
    }

    #[test]
    fn snapshot_rejects_empty_ciphertext() {
        assert_eq!(
            snapshot(vec![], vec![1], vec![]).validate(),
            Err(ValidationError::EmptyCiphertext)
        );
    }

    #[test]
    fn snapshot_rejects_duplicate_author() {
        let dup = PublicKey::from_array([7u8; 32]);
        let clocks = vec![(dup.clone(), Clock(1)), (dup, Clock(2))];
        assert_eq!(
            snapshot(vec![1], vec![1], clocks).validate(),
            Err(ValidationError::DuplicatePublicKey)
        );
    }
}
