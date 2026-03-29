//! Wire protocol error types.
//!
//! This module defines `Error`, an enum that encompasses all errors that can
//! originate from the `ambre‑protocol` crate (serialization, validation, version
//! mismatches, etc.).

use thiserror::Error;

use ambre_core::error::VersionError;

/// The main error type for the wire protocol.
#[derive(Error, Debug, Clone)]
pub enum Error {
    /// Version byte validation failed.
    #[error("version error: {0}")]
    Version(#[from] VersionError),

    /// Serialization or deserialization failed.
    #[error("serialization error: {0}")]
    Serialization(#[from] SerializationError),

    /// A field validation failed (length, range, etc.).
    #[error("validation error: {0}")]
    Validation(#[from] ValidationError),
}

/// Errors related to serialization/deserialization.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum SerializationError {
    /// Postcard serialization failed.
    #[error("postcard serialization failed: {0}")]
    Postcard(#[from] postcard::Error),
}

/// Errors related to field validation.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Nonce length is incorrect (expected 24 bytes).
    #[error("invalid nonce length: expected 24 bytes, got {0}")]
    InvalidNonceLength(usize),

    /// Commitment tag length is incorrect (expected 32 bytes).
    #[error("invalid commitment tag length: expected 32 bytes, got {0}")]
    InvalidCommitmentTagLength(usize),

    /// Signature length is incorrect (expected 64 bytes).
    #[error("invalid signature length: expected 64 bytes, got {0}")]
    InvalidSignatureLength(usize),

    /// Public key length is incorrect (expected 32 bytes).
    #[error("invalid public key length: expected 32 bytes, got {0}")]
    InvalidPublicKeyLength(usize),

    /// Ciphertext is empty (must contain at least one byte).
    #[error("ciphertext cannot be empty")]
    EmptyCiphertext,

    /// Snapshot proof is empty (must contain at least one byte).
    #[error("snapshot proof cannot be empty")]
    EmptySnapshotProof,

    /// Clock value is zero (clocks start at 1).
    #[error("clock must be greater than zero")]
    ZeroClock,

    /// Counter value is zero (counters start at 1).
    #[error("counter must be greater than zero")]
    ZeroCounter,

    /// The same public key appears twice in `parent_snapshot_update_clocks`.
    #[error("duplicate public key in parent snapshot update clocks")]
    DuplicatePublicKey,

    /// The snapshot ID equals the parent snapshot ID (no self‑reference).
    #[error("snapshot ID cannot be equal to its parent snapshot ID")]
    SelfReferentialSnapshot,
}