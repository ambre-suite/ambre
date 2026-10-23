//! The byte newtypes of the encrypted envelope: nonce, ciphertext, commitment
//! tag and signature.
//!
//! `ambre-crypto` computes them and `ambre-protocol` carries them on the wire.
//! Both depend on this crate alone, so the types live here. They hold bytes
//! only: no primitive of this module encrypts, hashes or signs.

use serde::{Deserialize, Serialize};

/// 24-byte nonce used for XChaCha20-Poly1305 encryption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Nonce(#[serde(with = "serde_bytes")] pub [u8; 24]);

/// 32-byte HMAC-SHA256 commitment tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommitmentTag(#[serde(with = "serde_bytes")] pub [u8; 32]);

/// 64-byte Ed25519 signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Signature(#[serde(with = "serde_bytes")] pub [u8; 64]);

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

/// The digest of one membership-log entry, which the next entry names. A
/// wrapped document key also seals the digest of the first entry, as the
/// anchor of the log (`SYNC-104`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntryDigest(pub [u8; 32]);
