//! Public key wrapper (`[u8; 32]`).
//!
//! A type-safe wrapper for 32-byte public keys used in Ed25519 signatures and X25519 key exchange.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::error::PublicKeyError;

/// A 32-byte public key.
///
/// The underlying array is always 32 bytes, as required by Ed25519 and X25519.
///
/// Public keys are not secret, so this type deliberately does NOT zeroize on
/// drop. Doing so would only add a memset to a hot, frequently-cloned type
/// (every `UpdatePublicData` carries one). It would also imply a
/// confidentiality the type does not have. Zeroize is reserved for the real
/// secret-key types.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    /// Creates a public key from a 32-byte array.
    pub const fn from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Creates a public key from a slice, returning an error if the length is not 32.
    ///
    /// # Errors
    ///
    /// [`PublicKeyError::InvalidLength`], carrying the length it received.
    pub fn from_slice(slice: &[u8]) -> Result<Self, PublicKeyError> {
        slice
            .try_into()
            .map(Self)
            .map_err(|_| PublicKeyError::InvalidLength(slice.len()))
    }

    /// Returns the underlying 32-byte array.
    pub const fn as_array(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the key as a slice of 32 bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Returns a hex-encoded string of the public key (lowercase).
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Parses a public key from a hex string (with or without `0x` prefix).
    ///
    /// Returns an error if the string does not contain exactly 64 hex digits.
    ///
    /// # Errors
    ///
    /// [`PublicKeyError::InvalidHex`] on a non-hexadecimal digit, and
    /// [`PublicKeyError::InvalidLength`] on any length other than 32 bytes.
    pub fn from_hex(hex_str: &str) -> Result<Self, PublicKeyError> {
        let stripped = hex_str.strip_prefix("0x").unwrap_or(hex_str);
        let bytes = hex::decode(stripped)?;
        Self::from_slice(&bytes)
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey(0x{}...)", hex::encode(&self.0[..4]))
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_array() {
        let array = [0x42; 32];
        let pk = PublicKey::from_array(array);
        assert_eq!(pk.as_array(), &array);
    }

    #[test]
    fn test_from_slice() {
        let slice = [0x99; 32];
        let pk = PublicKey::from_slice(&slice).unwrap();
        assert_eq!(pk.as_slice(), slice);
        assert!(PublicKey::from_slice(&[0x00; 31]).is_err());
        assert!(PublicKey::from_slice(&[0x00; 33]).is_err());
    }

    #[test]
    fn test_hex() {
        let pk = PublicKey::from_array([0x01; 32]);
        let hex = pk.to_hex();
        assert_eq!(
            hex,
            "0101010101010101010101010101010101010101010101010101010101010101"
        );
        let parsed = PublicKey::from_hex(&hex).unwrap();
        assert_eq!(pk, parsed);
        let parsed_with_prefix = PublicKey::from_hex(&format!("0x{}", hex)).unwrap();
        assert_eq!(pk, parsed_with_prefix);
    }

    #[test]
    fn test_serde() {
        let pk = PublicKey::from_array([0xaa; 32]);
        let serialized = serde_json::to_string(&pk).unwrap();
        let deserialized: PublicKey = serde_json::from_str(&serialized).unwrap();
        assert_eq!(pk, deserialized);
    }
}
