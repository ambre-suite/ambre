//! UUID v7 wrapper (`[u8; 16]`).
//!
//! Provides a type-safe wrapper around a 128-bit UUID v7 with serde support,
//! generation, and conversion to/from string.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid as ExternalUuid;

use crate::error::UuidError;

/// A 128-bit UUID v7.
///
/// Stored as a `[u8; 16]` in big-endian order (as per RFC 4122).
/// The internal representation is opaque; use `as_bytes()` or `to_string()` for exposure.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Uuid([u8; 16]);

impl Uuid {
    /// Creates a new UUID v7 using the current timestamp and random bytes.
    ///
    /// This uses the `uuid` crate's `Uuid::now_v7()` which guarantees monotonicity
    /// within the same millisecond.
    pub fn new_v7() -> Self {
        let uuid = ExternalUuid::now_v7();
        Self(*uuid.as_bytes())
    }

    /// Creates a UUID from a 16‑byte array.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the underlying 16‑byte array.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Converts the UUID to a standard hexadecimal string with hyphens.
    pub fn to_hyphenated(&self) -> String {
        ExternalUuid::from_bytes(self.0).to_string()
    }

    /// Creates a UUID from a slice of bytes.
    ///
    /// Returns an error if the slice length is not 16.
    pub fn from_slice(slice: &[u8]) -> Result<Self, UuidError> {
        slice
            .try_into()
            .map(Self)
            .map_err(|_| UuidError::InvalidLength(slice.len()))
    }

    /// Parses a UUID from a string representation.
    ///
    /// Accepts both hyphenated and non‑hyphenated forms.
    pub fn parse_str(s: &str) -> Result<Self, UuidError> {
        Ok(Self(*ExternalUuid::parse_str(s)?.as_bytes()))
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uuid({})", self.to_hyphenated())
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hyphenated())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_v7() {
        let uuid = Uuid::new_v7();
        let bytes = uuid.as_bytes();
        // UUID v7 has version bits 0b0111 in byte 6 (index 6)
        assert_eq!(bytes[6] >> 4, 0b0111);
        // Variant bits must be RFC 4122 (0b10xx)
        assert_eq!(bytes[8] >> 6, 0b10);
    }

    #[test]
    fn test_from_bytes() {
        let bytes = [0x00; 16];
        let uuid = Uuid::from_bytes(bytes);
        assert_eq!(uuid.as_bytes(), &bytes);
    }

    #[test]
    fn test_parse_and_to_hyphenated() {
        let s = "12345678-1234-5678-1234-567812345678";
        let uuid = Uuid::parse_str(s).unwrap();
        assert_eq!(uuid.to_hyphenated(), s);
    }

    #[test]
    fn test_serde() {
        let uuid = Uuid::new_v7();
        let serialized = serde_json::to_string(&uuid).unwrap();
        let deserialized: Uuid = serde_json::from_str(&serialized).unwrap();
        assert_eq!(uuid, deserialized);
    }
}