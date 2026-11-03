//! Wire messages: updates, snapshots, ephemerals, and sequenced updates.
//!
//! This module defines the core encrypted payloads that travel between client and server.
//! All fields are opaque to the Blind Relay; only the public data is visible.

use crate::error::ValidationError;
use crate::public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
use ambre_core::{FixedBytes, Payload};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// -----------------------------------------------------------------------------
// Newtype wrappers for binary buffers
// -----------------------------------------------------------------------------

/// The envelope byte roles and fixed-size types: `Payload<Ciphertext>`,
/// `FixedBytes<Nonce>` and the others. `ambre-crypto` builds them too, so
/// they live in `ambre-core`, the one crate both depend on.
pub use ambre_core::envelope::{Ciphertext, CommitmentTag, Nonce, Signature};

// -----------------------------------------------------------------------------
// Core wire messages
use ambre_core::{Before, SeqColumn, SeqKind, Wire};
/// The series number and the relay's marker: `Seq<Relay>` is the number the
/// sequencer assigns, shared with the client threads.
pub use ambre_core::{Relay, Seq};

/// The author key that numbers its own deltas (`ambre-spec-delta-order.md`).
/// `Seq<AuthorKey>` is the clock a delta carries. Every session of one author
/// key shares the series, and each update takes a clock above every one the
/// author used for the document (`SYNC-024`). It is not a Lamport clock: no
/// peer's clock moves it (`SYNC-025`).
pub enum AuthorKey {}

impl SeqKind for AuthorKey {
    type IssueRight = ();
    const NAME: &'static str = "Clock";
}

/// The clock crosses the wire in every update.
impl Wire for AuthorKey {}

/// A rotation hands back the clock before the one a refused frame took.
impl Before for AuthorKey {}

/// The relay stores the clock of each delta in Postgres.
impl SeqColumn for AuthorKey {}

/// The session that numbers its own ephemerals, client-side. `Seq<Ephemerals>`
/// is the counter an ephemeral carries, which a receiver checks against a
/// replay. The marker is plural, since `Ephemeral` names the message.
pub enum Ephemerals {}

impl SeqKind for Ephemerals {
    type IssueRight = ();
    const NAME: &'static str = "Counter";
}

/// The counter crosses the wire in every ephemeral.
impl Wire for Ephemerals {}

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
    pub ciphertext: Payload<Ciphertext>,
    /// 24-byte nonce used for encryption.
    pub nonce: FixedBytes<Nonce>,
    /// Public metadata visible to the Blind Relay.
    pub public_data: UpdatePublicData,
    /// HMAC-SHA256(document_key, nonce || ciphertext || public_data_bytes)
    pub commitment_tag: FixedBytes<CommitmentTag>,
    /// Ed25519 signature of (nonce || ciphertext || public_data_bytes || commitment_tag)
    pub signature: FixedBytes<Signature>,
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
        if self.ciphertext.as_bytes().is_empty() {
            return Err(ValidationError::EmptyCiphertext);
        }
        if self.public_data.clock == Seq::<AuthorKey>::ZERO {
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
    pub ciphertext: Payload<Ciphertext>,
    /// 24-byte nonce used for encryption.
    pub nonce: FixedBytes<Nonce>,
    /// Public metadata visible to the Blind Relay.
    pub public_data: SnapshotPublicData,
    /// HMAC-SHA256(document_key, nonce || ciphertext || public_data_bytes)
    pub commitment_tag: FixedBytes<CommitmentTag>,
    /// Ed25519 signature of (nonce || ciphertext || public_data_bytes || commitment_tag)
    pub signature: FixedBytes<Signature>,
}

impl Snapshot {
    /// Checks the semantic invariants of a decoded snapshot: a non-empty ciphertext
    /// and no author appearing twice in `parent_snapshot_update_clocks`. Called by
    /// the client before upload (snapshots travel client-to-S3 directly, so the
    /// Blind Relay never sees this struct to enforce it).
    ///
    /// The parent proof is deliberately NOT required to be non-empty: the first
    /// snapshot of a document has the genesis base as its parent. That first
    /// snapshot carries no proof (see [`ambre_core::Id::genesis`]).
    ///
    /// # Errors
    ///
    /// [`ValidationError::EmptyCiphertext`] on an empty ciphertext, and
    /// [`ValidationError::DuplicatePublicKey`] when an author appears twice in
    /// `parent_snapshot_update_clocks`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.ciphertext.as_bytes().is_empty() {
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
    pub ciphertext: Payload<Ciphertext>,
    /// 24-byte nonce used for encryption.
    pub nonce: FixedBytes<Nonce>,
    /// Public metadata visible to the Blind Relay.
    pub public_data: EphemeralPublicData,
    /// HMAC-SHA256(document_key, nonce || ciphertext || public_data_bytes)
    pub commitment_tag: FixedBytes<CommitmentTag>,
    /// Ed25519 signature of (nonce || ciphertext || public_data_bytes || commitment_tag)
    pub signature: FixedBytes<Signature>,
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
        if self.ciphertext.as_bytes().is_empty() {
            return Err(ValidationError::EmptyCiphertext);
        }
        if self.public_data.counter == Seq::<Ephemerals>::ZERO {
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
    pub seq: Seq<Relay>,
    /// The update itself.
    pub update: Update,
}

#[cfg(test)]
mod validate_tests {
    use super::*;
    use crate::public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
    use ambre_core::Id;
    use ambre_core::PublicKey;

    fn update(cipher: Vec<u8>, clock: u64) -> Update {
        Update {
            ciphertext: Payload::<Ciphertext>::new(cipher),
            nonce: FixedBytes::from_array([0u8; 24]),
            public_data: UpdatePublicData {
                document_id: Id::from_bytes([1u8; 16]),
                pub_key: FixedBytes::from_array([2u8; 32]),
                key_id: FixedBytes::from_array([4u8; 16]),
                snapshot_id: Id::from_bytes([3u8; 16]),
                clock: Seq::<AuthorKey>::new(clock),
            },
            commitment_tag: FixedBytes::from_array([0u8; 32]),
            signature: FixedBytes::from_array([0u8; 64]),
        }
    }

    fn ephemeral(cipher: Vec<u8>, counter: u64) -> Ephemeral {
        Ephemeral {
            ciphertext: Payload::<Ciphertext>::new(cipher),
            nonce: FixedBytes::from_array([0u8; 24]),
            public_data: EphemeralPublicData {
                document_id: Id::from_bytes([1u8; 16]),
                pub_key: FixedBytes::from_array([2u8; 32]),
                key_id: FixedBytes::from_array([4u8; 16]),
                counter: Seq::<Ephemerals>::new(counter),
            },
            commitment_tag: FixedBytes::from_array([0u8; 32]),
            signature: FixedBytes::from_array([0u8; 64]),
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

    fn snapshot(
        cipher: Vec<u8>,
        proof: Option<FixedBytes<ambre_core::ChainHash>>,
        clocks: Vec<(FixedBytes<PublicKey>, Seq<AuthorKey>)>,
    ) -> Snapshot {
        Snapshot {
            ciphertext: Payload::<Ciphertext>::new(cipher),
            nonce: FixedBytes::from_array([0u8; 24]),
            public_data: SnapshotPublicData {
                document_id: Id::from_bytes([1u8; 16]),
                pub_key: FixedBytes::from_array([2u8; 32]),
                key_id: FixedBytes::from_array([4u8; 16]),
                snapshot_id: Id::from_bytes([3u8; 16]),
                parent_snapshot_proof: proof,
                parent_snapshot_update_clocks: clocks,
            },
            commitment_tag: FixedBytes::from_array([0u8; 32]),
            signature: FixedBytes::from_array([0u8; 64]),
        }
    }

    #[test]
    fn snapshot_valid() {
        let clocks = vec![
            (FixedBytes::from_array([9u8; 32]), Seq::<AuthorKey>::new(1)),
            (FixedBytes::from_array([8u8; 32]), Seq::<AuthorKey>::new(2)),
        ];
        assert!(
            snapshot(vec![1], Some(FixedBytes::from_array([1u8; 32])), clocks)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn snapshot_valid_with_empty_genesis_proof() {
        // The first snapshot has the genesis base as parent and no proof.
        assert!(snapshot(vec![1], None, vec![]).validate().is_ok());
    }

    #[test]
    fn snapshot_rejects_empty_ciphertext() {
        assert_eq!(
            snapshot(vec![], Some(FixedBytes::from_array([1u8; 32])), vec![]).validate(),
            Err(ValidationError::EmptyCiphertext)
        );
    }

    #[test]
    fn snapshot_rejects_duplicate_author() {
        let dup = FixedBytes::<PublicKey>::from_array([7u8; 32]);
        let clocks = vec![
            (dup, Seq::<AuthorKey>::new(1)),
            (dup, Seq::<AuthorKey>::new(2)),
        ];
        assert_eq!(
            snapshot(vec![1], Some(FixedBytes::from_array([1u8; 32])), clocks).validate(),
            Err(ValidationError::DuplicatePublicKey)
        );
    }
}
