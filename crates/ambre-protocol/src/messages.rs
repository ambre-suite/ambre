//! Wire messages: updates, snapshots, ephemerals, and sequenced updates.
//!
//! This module defines the core encrypted payloads that travel between client and server.
//! All fields are opaque to the Blind Relay; only the public data is visible.

use crate::public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
use serde::{Deserialize, Serialize};
use serde_bytes;

// ─────────────────────────────────────────────────────────────────────────────
// Newtype wrappers for binary buffers
// ─────────────────────────────────────────────────────────────────────────────

/// 24‑byte nonce used for XChaCha20‑Poly1305 encryption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Nonce(#[serde(with = "serde_bytes")] pub [u8; 24]);

/// 32‑byte HMAC‑SHA256 commitment tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommitmentTag(#[serde(with = "serde_bytes")] pub [u8; 32]);

/// 64‑byte Ed25519 signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Signature(#[serde(with = "serde_bytes")] pub [u8; 64]);

// ─────────────────────────────────────────────────────────────────────────────
// Core wire messages
/// Server‑assigned monotonic sequence number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Seq(pub u64);

/// Lamport logical clock (client‑side).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Clock(pub u64);

/// Ephemeral counter (client‑side).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Counter(pub u64);

/// AEAD ciphertext wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ciphertext(#[serde(with = "serde_bytes")] pub Vec<u8>);

// ─────────────────────────────────────────────────────────────────────────────

/// An encrypted, signed update carrying a CRDT delta.
///
/// The update is encrypted with the document's symmetric key (`DocumentKey`).
/// The `commitment_tag` is an HMAC‑SHA256 over the ciphertext and public data,
/// binding the message to a specific key (key‑commitment).
/// The signature is Ed25519 over the nonce, ciphertext, public data, and commitment tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    /// AEAD ciphertext (XChaCha20‑Poly1305).
    pub ciphertext: Ciphertext,
    /// 24‑byte nonce used for encryption.
    pub nonce: Nonce,
    /// Public metadata visible to the Blind Relay.
    pub public_data: UpdatePublicData,
    /// HMAC‑SHA256(document_key, nonce || ciphertext || public_data_bytes)
    pub commitment_tag: CommitmentTag,
    /// Ed25519 signature of (nonce || ciphertext || public_data_bytes || commitment_tag)
    pub signature: Signature,
}

/// An encrypted, signed snapshot carrying a full CRDT state.
///
/// Snapshots are uploaded/downloaded via S3 presigned URLs, not through WebSocket.
/// The same cryptographic envelope as `Update` is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// AEAD ciphertext (XChaCha20‑Poly1305).
    pub ciphertext: Ciphertext,
    /// 24‑byte nonce used for encryption.
    pub nonce: Nonce,
    /// Public metadata visible to the Blind Relay.
    pub public_data: SnapshotPublicData,
    /// HMAC‑SHA256(document_key, nonce || ciphertext || public_data_bytes)
    pub commitment_tag: CommitmentTag,
    /// Ed25519 signature of (nonce || ciphertext || public_data_bytes || commitment_tag)
    pub signature: Signature,
}

/// An encrypted, signed ephemeral message (cursors, presence, etc.).
///
/// Ephemerals are not stored permanently; they are broadcast to collaborators
/// in real‑time and then discarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ephemeral {
    /// AEAD ciphertext (XChaCha20‑Poly1305).
    pub ciphertext: Ciphertext,
    /// 24‑byte nonce used for encryption.
    pub nonce: Nonce,
    /// Public metadata visible to the Blind Relay.
    pub public_data: EphemeralPublicData,
    /// HMAC‑SHA256(document_key, nonce || ciphertext || public_data_bytes)
    pub commitment_tag: CommitmentTag,
    /// Ed25519 signature of (nonce || ciphertext || public_data_bytes || commitment_tag)
    pub signature: Signature,
}

/// An update paired with its server‑assigned sequence number.
///
/// Used in `MissingUpdates` responses to allow clients to catch up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeqUpdate {
    /// Monotonic sequence number assigned by the server.
    pub seq: Seq,
    /// The update itself.
    pub update: Update,
}
