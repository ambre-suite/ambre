//! Error types for cryptographic operations.
//!
//! Variants are intentionally coarse and free of sensitive detail: a caller on
//! the Sync thread maps any failure to a DROP + `SyncWarning`, never surfacing
//! which check failed to a potential attacker.

use thiserror::Error;

/// Failure of a cryptographic operation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptoError {
    /// Ed25519 signature verification failed.
    #[error("invalid signature")]
    InvalidSignature,

    /// HMAC-SHA256 key-commitment tag did not match (wrong key or substitution).
    #[error("commitment tag mismatch")]
    CommitmentMismatch,

    /// XChaCha20-Poly1305 authentication/decryption failed (tampering or corruption).
    #[error("AEAD authentication failed")]
    AeadFailed,

    /// A key or buffer had an unexpected length.
    #[error("invalid key length")]
    InvalidKeyLength,

    /// Argon2id key derivation failed.
    #[error("key derivation failed")]
    Argon2Failed,

    /// XChaCha20-Poly1305 encryption failed (plaintext exceeds the AEAD limit).
    #[error("AEAD encryption failed")]
    EncryptionFailed,

    /// The X25519 shared secret was non-contributory (peer sent a low-order
    /// public key), yielding a predictable all-zero secret. Rejected.
    #[error("weak public key")]
    WeakPublicKey,
}
