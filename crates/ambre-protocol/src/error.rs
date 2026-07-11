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

/// Errors related to field validation, enforced by the `validate()` methods on
/// the wire messages (see [`crate::messages`]).
///
/// Length invariants (nonce = 24, commitment tag = 32, signature = 64, public
/// key = 32) are intentionally absent: those fields are fixed-size arrays, so
/// postcard decoding already rejects any other length. Only invariants that a
/// well-formed decode can still violate live here.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Ciphertext is empty (must contain at least one byte).
    #[error("ciphertext cannot be empty")]
    EmptyCiphertext,

    /// Clock value is zero (clocks start at 1).
    #[error("clock must be greater than zero")]
    ZeroClock,

    /// Counter value is zero (counters start at 1).
    #[error("counter must be greater than zero")]
    ZeroCounter,

    /// The same public key appears twice in `parent_snapshot_update_clocks`.
    #[error("duplicate public key in parent snapshot update clocks")]
    DuplicatePublicKey,
}
