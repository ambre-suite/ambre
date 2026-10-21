//! Sealed and signed bytes, one type per role.
//!
//! A member row carries a wrapped document key beside a sealed name. An
//! account carries a wrapped master key beside its salt, and a change
//! carries a signed log entry. The relay stores all of them and reads none.
//! A bare `Vec<u8>` lets one stand for another, down to the argument order
//! of an insert. One type per role keeps them apart, from the JSON body to
//! the column.
//!
//! On the wire each is its URL-safe base64 text, decoded when the body is
//! read. A value that is not base64 refuses the body there, as any field
//! that does not decode does (`SRV-178`). `Debug` prints the length alone.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! define_encoded {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Default, PartialEq, Eq, Hash)]
        pub struct $name(Vec<u8>);

        impl $name {
            /// Wraps the bytes.
            pub fn new(bytes: Vec<u8>) -> Self {
                Self(bytes)
            }

            /// The bytes.
            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }

            /// The bytes, owned.
            pub fn into_bytes(self) -> Vec<u8> {
                self.0
            }

            /// Whether there are no bytes: a field the sender left out.
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }

            /// The text the wire carries.
            pub fn to_base64(&self) -> String {
                URL_SAFE.encode(&self.0)
            }
        }

        impl From<Vec<u8>> for $name {
            fn from(bytes: Vec<u8>) -> Self {
                Self(bytes)
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!(stringify!($name), "({} bytes)"), self.0.len())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_base64())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                URL_SAFE
                    .decode(text.as_bytes())
                    .map(Self)
                    .map_err(|e| serde::de::Error::custom(format!("not base64: {e}")))
            }
        }
    };
}

define_encoded! {
    /// A document key wrapped to one member: `nonce || ciphertext`.
    WrappedDocKey
}

define_encoded! {
    /// A document or channel name sealed under its document key.
    SealedName
}

define_encoded! {
    /// An account's master seed wrapped under the key its passphrase derives.
    WrappedMasterKey
}

define_encoded! {
    /// The salt the passphrase key of an account is derived with.
    MasterKeySalt
}

define_encoded! {
    /// One share of an organization's recovery secret, sealed to its holder.
    SealedShare
}

define_encoded! {
    /// A signed entry of a document's membership log.
    EncodedLogEntry
}

define_encoded! {
    /// A signed link from one document key to the next.
    EncodedKeyLink
}

define_encoded! {
    /// The proof a snapshot carries of the snapshot it replaces.
    EncodedProof
}

#[cfg(test)]
mod tests {
    use super::WrappedDocKey;

    #[test]
    fn an_encoded_value_travels_as_its_url_safe_base64() {
        let key = WrappedDocKey::new(vec![0xab, 0xcd, 0xef]);
        assert_eq!(serde_json::to_string(&key).unwrap(), "\"q83v\"");
        let parsed: WrappedDocKey = serde_json::from_str("\"q83v\"").unwrap();
        assert_eq!(parsed.as_bytes(), &[0xab, 0xcd, 0xef]);
        assert_eq!(format!("{parsed:?}"), "WrappedDocKey(3 bytes)");
    }

    #[test]
    fn a_value_that_is_not_base64_refuses_the_read() {
        assert!(serde_json::from_str::<WrappedDocKey>("\"not base64!\"").is_err());
        assert!(serde_json::from_str::<WrappedDocKey>("\"q83\"").is_err());
    }
}
