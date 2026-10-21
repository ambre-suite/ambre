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
//! that does not decode does (`SRV-178`). A role whose layout fixes its
//! length refuses any other length there too (`SRV-226`). `Debug` prints the
//! length alone.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The nonce an AEAD seal opens with, as `ambre_crypto::aead` writes it.
const NONCE_LEN: usize = 24;
/// The tag an AEAD seal closes with.
const TAG_LEN: usize = 16;
/// A document key, and a master seed.
const KEY_LEN: usize = 32;
/// The digest of a log's first entry, sealed beside a wrapped key.
const ANCHOR_LEN: usize = 32;

macro_rules! define_encoded {
    ($(#[$meta:meta])* $name:ident $(, lengths: [$($len:expr),+])?) => {
        $(#[$meta])*
        #[derive(Clone, Default, PartialEq, Eq, Hash)]
        pub struct $name(Vec<u8>);

        impl $name {
            $(
                /// The lengths the layout of this role allows; a read refuses
                /// any other.
                pub const LENGTHS: &'static [usize] = &[$($len),+];
            )?

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

            /// Whether there are no bytes.
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
                let bytes = URL_SAFE
                    .decode(text.as_bytes())
                    .map_err(|e| serde::de::Error::custom(format!("not base64: {e}")))?;
                $(
                    let lengths: &[usize] = &[$($len),+];
                    if !lengths.contains(&bytes.len()) {
                        return Err(serde::de::Error::custom(format!(
                            concat!(stringify!($name), " of {} bytes, not one of {:?}"),
                            bytes.len(),
                            lengths,
                        )));
                    }
                )?
                Ok(Self(bytes))
            }
        }
    };
}

define_encoded! {
    /// A document key wrapped to one member: `nonce || ciphertext`, the key
    /// sealed alone or beside the anchor of its log.
    WrappedDocKey,
    lengths: [NONCE_LEN + KEY_LEN + TAG_LEN, NONCE_LEN + KEY_LEN + ANCHOR_LEN + TAG_LEN]
}

define_encoded! {
    /// A document or channel name sealed under its document key.
    SealedName
}

define_encoded! {
    /// An account's master seed wrapped under the key its passphrase derives:
    /// `nonce || ciphertext`.
    WrappedMasterKey,
    lengths: [NONCE_LEN + KEY_LEN + TAG_LEN]
}

define_encoded! {
    /// The salt the passphrase key of an account is derived with.
    MasterKeySalt,
    lengths: [32]
}

define_encoded! {
    /// A member's master seed sealed to their organization, for a recovery
    /// (`SRV-067`). No client seals one yet, so no layout fixes its length.
    EscrowedKey
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

#[cfg(test)]
mod tests {
    use super::{MasterKeySalt, SealedName, WrappedDocKey, WrappedMasterKey};
    use serde::de::DeserializeOwned;

    fn wire(len: usize) -> String {
        serde_json::to_string(&SealedName::new(vec![7; len])).unwrap()
    }

    fn reads<T: DeserializeOwned>(len: usize) -> bool {
        serde_json::from_str::<T>(&wire(len)).is_ok()
    }

    #[test]
    fn an_encoded_value_travels_as_its_url_safe_base64() {
        let name = SealedName::new(vec![0xab, 0xcd, 0xef]);
        assert_eq!(serde_json::to_string(&name).unwrap(), "\"q83v\"");
        let parsed: SealedName = serde_json::from_str("\"q83v\"").unwrap();
        assert_eq!(parsed.as_bytes(), &[0xab, 0xcd, 0xef]);
        assert_eq!(format!("{parsed:?}"), "SealedName(3 bytes)");
    }

    #[test]
    fn a_value_that_is_not_base64_refuses_the_read() {
        assert!(serde_json::from_str::<SealedName>("\"not base64!\"").is_err());
        assert!(serde_json::from_str::<SealedName>("\"q83\"").is_err());
    }

    /// `SRV-226`: each role whose layout fixes its length reads that length
    /// alone. An empty value, one byte short and one byte past are refused.
    #[test]
    fn a_fixed_role_refuses_any_other_length() {
        fn only<T: DeserializeOwned>(lengths: &[usize]) {
            for &len in lengths {
                assert!(reads::<T>(len), "{len}");
                assert!(!reads::<T>(len - 1), "{}", len - 1);
                assert!(!reads::<T>(len + 1), "{}", len + 1);
            }
            assert!(!reads::<T>(0));
        }
        only::<WrappedDocKey>(WrappedDocKey::LENGTHS);
        only::<WrappedMasterKey>(WrappedMasterKey::LENGTHS);
        only::<MasterKeySalt>(MasterKeySalt::LENGTHS);
        assert_eq!(WrappedDocKey::LENGTHS, [72, 104]);
        assert_eq!(WrappedMasterKey::LENGTHS, [72]);
        assert_eq!(MasterKeySalt::LENGTHS, [32]);
        assert!(reads::<SealedName>(0) && reads::<SealedName>(4000));
    }
}
