//! Public key wrappers (`[u8; 32]`).
//!
//! One type per role: [`PublicKey`] verifies an Ed25519 signature, and
//! [`ExchangePublicKey`] receives an X25519 key exchange. Both hold 32 bytes.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::error::PublicKeyError;

macro_rules! define_public_key {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name([u8; 32]);

        impl $name {
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

        crate::envelope::fixed_bytes!($name, 32);

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    concat!(stringify!($name), "(0x{}...)"),
                    hex::encode(&self.0[..4])
                )
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "0x{}", self.to_hex())
            }
        }
    };
}

define_public_key! {
    /// A 32-byte Ed25519 public key: the one that verifies a user's signatures.
    ///
    /// Public keys are not secret, so this type deliberately does NOT zeroize on
    /// drop. Doing so would only add a memset to a hot, frequently-cloned type
    /// (every `UpdatePublicData` carries one). It would also imply a
    /// confidentiality the type does not have. Zeroize is reserved for the real
    /// secret-key types.
    PublicKey
}

define_public_key! {
    /// A 32-byte X25519 public key: the one a document key is wrapped to.
    ///
    /// It has the size of a [`PublicKey`] and the same text forms. A distinct
    /// type keeps an exchange key from standing where a signing key is read.
    /// It does not zeroize on drop, for the reason [`PublicKey`] gives.
    ExchangePublicKey
}

impl PublicKey {
    /// The key as the relay's JSON spells it.
    #[must_use]
    pub fn to_key_hex(&self) -> crate::SigningKeyHex {
        crate::SigningKeyHex::encode(self.0)
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
    fn an_exchange_key_reads_its_own_hex() {
        let key = ExchangePublicKey::from_array([0x0f; 32]);
        assert_eq!(ExchangePublicKey::from_hex(&key.to_hex()).unwrap(), key);
        assert_eq!(format!("{key:?}"), "ExchangePublicKey(0x0f0f0f0f...)");
    }

    #[test]
    fn test_serde() {
        let pk = PublicKey::from_array([0xaa; 32]);
        let serialized = serde_json::to_string(&pk).unwrap();
        let deserialized: PublicKey = serde_json::from_str(&serialized).unwrap();
        assert_eq!(pk, deserialized);
    }
}
