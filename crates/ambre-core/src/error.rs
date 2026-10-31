//! Core error types for Ambre.
//!
//! This module defines `Error`, an enum that encompasses all errors that can
//! originate from the `ambre-core` crate.

use thiserror::Error;

/// The main error type for the crate.
#[derive(Error, Debug, Clone)]
pub enum Error {
    /// UUID parsing or validation failed.
    #[error("UUID error: {0}")]
    Uuid(#[from] UuidError),

    /// A fixed-width value, such as a public key, failed to parse.
    #[error("fixed-width value error: {0}")]
    FixedBytes(#[from] crate::FixedBytesError),

    /// Document type validation failed.
    #[error("document type error: {0}")]
    DocType(#[from] DocTypeError),

    /// Version byte validation failed.
    #[error("version error: {0}")]
    Version(#[from] VersionError),

    /// Role validation failed.
    #[error("role error: {0}")]
    Role(#[from] RoleError),
}

/// Errors related to UUID operations.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum UuidError {
    /// The provided string is not a valid UUID.
    #[error("invalid UUID string: {0}")]
    InvalidString(#[from] uuid::Error),

    /// The provided byte slice has incorrect length.
    #[error("invalid UUID length: expected 16 bytes, got {0}")]
    InvalidLength(usize),
}

/// Errors related to document type operations.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DocTypeError {
    /// The provided discriminant is not a valid document type.
    #[error("invalid document type discriminant: {0}")]
    InvalidDiscriminant(u8),
    /// The provided word names no document type.
    #[error("invalid document type: {0}")]
    InvalidString(String),
}

/// Errors related to version byte operations.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    /// The version byte is out of allowed range (0-127).
    #[error("version byte out of range: must be 0-127, got {0}")]
    OutOfRange(u8),
}

/// Errors related to role operations.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum RoleError {
    /// The provided discriminant is not a valid role.
    #[error("invalid role discriminant: {0}")]
    InvalidDiscriminant(u8),

    /// The provided string is not a valid role.
    #[error("invalid role string: {0}")]
    InvalidString(String),
}
