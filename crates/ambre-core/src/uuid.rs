//! UUID v7 wrapper (`[u8; 16]`).
//!
//! Provides a type-safe wrapper around a 128-bit UUID v7 with serde support,
//! generation, and conversion to/from string.

use serde::de::{Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid as ExternalUuid;

use crate::error::UuidError;

/// A 128-bit UUID v7.
///
/// Stored as a `[u8; 16]` in big-endian order (as per RFC 4122).
/// The internal representation is opaque; use `as_bytes()` or `to_string()` for exposure.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Uuid([u8; 16]);

impl Uuid {
    /// Creates a new UUID v7 using the current timestamp and random bytes.
    ///
    /// This uses the `uuid` crate's `Uuid::now_v7()` which guarantees monotonicity
    /// within the same millisecond.
    pub fn new() -> Self {
        let uuid = ExternalUuid::now_v7();
        Self(*uuid.as_bytes())
    }

    /// Creates a UUID from a 16-byte array.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the underlying 16-byte array.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Converts the UUID to a standard hexadecimal string with hyphens.
    pub fn to_hyphenated(&self) -> String {
        ExternalUuid::from_bytes(self.0).to_string()
    }

    /// Converts the UUID to its 32 lowercase hexadecimal digits, without
    /// hyphens.
    pub fn to_simple(&self) -> String {
        ExternalUuid::from_bytes(self.0).simple().to_string()
    }

    /// Creates a UUID from a slice of bytes.
    ///
    /// Returns an error if the slice length is not 16.
    ///
    /// # Errors
    ///
    /// [`UuidError::InvalidLength`], carrying the length it received.
    pub fn from_slice(slice: &[u8]) -> Result<Self, UuidError> {
        slice
            .try_into()
            .map(Self)
            .map_err(|_| UuidError::InvalidLength(slice.len()))
    }

    /// Parses a UUID from a string representation.
    ///
    /// Accepts both hyphenated and non-hyphenated forms.
    ///
    /// # Errors
    ///
    /// [`UuidError::InvalidString`] when `s` is not a UUID in either form.
    pub fn parse_str(s: &str) -> Result<Self, UuidError> {
        Ok(Self(*ExternalUuid::parse_str(s)?.as_bytes()))
    }
}

/// Reads the 16 bytes as the derive of `[u8; 16]` reads them: a tuple of
/// 16 elements, in any format. serde unrolls that read into 16 calls, about
/// 1 KB of the Sync worker. A loop over the elements reads the same bytes
/// (`ambre-spec-budgets.md` section 5).
impl<'de> Deserialize<'de> for Uuid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_tuple(16, UuidVisitor)
    }
}

/// The visitor of [`Uuid`]'s tuple of bytes.
struct UuidVisitor;

impl<'de> Visitor<'de> for UuidVisitor {
    type Value = Uuid;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of length 16")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Uuid, A::Error> {
        let mut bytes = [0; 16];
        crate::fixed_bytes::read_tuple(&mut seq, &mut bytes, &self)?;
        Ok(Uuid(bytes))
    }
}

impl Default for Uuid {
    fn default() -> Self {
        Self::new()
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
    fn the_simple_form_is_the_hyphenated_one_without_hyphens() {
        let uuid = Uuid::from_bytes([0xab; 16]);
        assert_eq!(uuid.to_simple(), uuid.to_hyphenated().replace('-', ""));
        assert_eq!(Uuid::parse_str(&uuid.to_simple()).unwrap(), uuid);
    }

    #[test]
    fn test_new() {
        let uuid = Uuid::new();
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

    proptest::proptest! {
        /// The tuple read reads the bytes the derive of `[u8; 16]` reads,
        /// and refuses what it refuses, in postcard and in JSON.
        #[test]
        fn the_tuple_read_reads_as_the_array_derive(
            bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..24),
            json in proptest::collection::vec(0_u16..300, 0..20),
        ) {
            proptest::prop_assert_eq!(
                postcard::from_bytes::<Uuid>(&bytes).ok().map(|uuid| uuid.0),
                postcard::from_bytes::<[u8; 16]>(&bytes).ok()
            );
            let text = serde_json::to_string(&json).unwrap();
            proptest::prop_assert_eq!(
                serde_json::from_str::<Uuid>(&text).ok().map(|uuid| uuid.0),
                serde_json::from_str::<[u8; 16]>(&text).ok()
            );
        }

        /// A uuid writes and reads back the bytes of the array.
        #[test]
        fn a_uuid_travels_as_its_array(bytes in proptest::prelude::any::<[u8; 16]>()) {
            let wire = postcard::to_allocvec(&Uuid(bytes)).unwrap();
            proptest::prop_assert_eq!(&wire, &postcard::to_allocvec(&bytes).unwrap());
            proptest::prop_assert_eq!(postcard::from_bytes::<Uuid>(&wire).unwrap(), Uuid(bytes));
        }
    }

    #[test]
    fn test_serde() {
        let uuid = Uuid::new();
        let serialized = serde_json::to_string(&uuid).unwrap();
        let deserialized: Uuid = serde_json::from_str(&serialized).unwrap();
        assert_eq!(uuid, deserialized);
    }
}
