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
//! length refuses any other length there too (`SRV-226`). A sealed name
//! refuses a length past the longest name it holds (`SRV-230`). `Debug`
//! prints the length alone.

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
/// The header of a sealed object, as Sync writes it. The layout byte, the key
/// id, the author's key, the signature, the commitment and the nonce.
const OBJECT_HEADER_LEN: usize = 1 + 16 + 32 + 64 + 32 + NONCE_LEN;
/// The most bytes one character takes in UTF-8.
const UTF8_MAX_LEN: usize = 4;

macro_rules! define_encoded {
    ($(#[$meta:meta])* $name:ident $(, lengths: [$($len:expr),+])? $(, max: $max:expr)?) => {
        $(#[$meta])*
        #[derive(Clone, Default, PartialEq, Eq, Hash)]
        pub struct $name(Vec<u8>);

        impl $name {
            $(
                /// The lengths the layout of this role allows; a read refuses
                /// any other.
                pub const LENGTHS: &'static [usize] = &[$($len),+];
            )?
            $(
                /// The most bytes a value of this role holds; a read refuses
                /// more.
                pub const MAX_LEN: usize = $max;
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
                $(
                    if bytes.len() > $max {
                        return Err(serde::de::Error::custom(format!(
                            concat!(stringify!($name), " of {} bytes, past {}"),
                            bytes.len(),
                            $max,
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
    /// A document or channel name sealed under its document key: the object
    /// header, the name, the tag.
    SealedName,
    max: OBJECT_HEADER_LEN + UTF8_MAX_LEN * SealedName::MAX_CHARS + TAG_LEN
}

impl SealedName {
    /// The longest name a sealed name holds, in characters.
    pub const MAX_CHARS: usize = crate::NAME_MAX_CHARS;

    /// The bytes a sealed name adds to the name it holds.
    pub const OVERHEAD: usize = OBJECT_HEADER_LEN + TAG_LEN;
}

define_encoded! {
    /// An account's master seed wrapped under the key its passphrase derives:
    /// `nonce || ciphertext`.
    WrappedMasterKey,
    lengths: [NONCE_LEN + KEY_LEN + TAG_LEN]
}

/// The salt the passphrase key of an account is derived with.
///
/// Fixed at [`MasterKeySalt::LEN`] bytes, as [`FileSalt`](crate::FileSalt) is
/// at its own width. A value of another width is refused when it is built,
/// whether from the wire, a column or a caller. A short salt thus never reaches
/// the derivation at unlock. On the wire it is its URL-safe base64 text, and
/// `Debug` prints the length alone.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MasterKeySalt([u8; MasterKeySalt::LEN]);

/// A master-key salt was built from bytes of another width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a master-key salt is {} bytes", MasterKeySalt::LEN)]
pub struct InvalidMasterKeySalt;

impl MasterKeySalt {
    /// The width of the salt, in bytes.
    pub const LEN: usize = 32;

    /// Wraps the salt bytes a CSPRNG drew.
    pub const fn from_array(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// Reads the salt from bytes a column or a body carried.
    ///
    /// # Errors
    ///
    /// [`InvalidMasterKeySalt`] when `bytes` is not [`Self::LEN`] long.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, InvalidMasterKeySalt> {
        bytes.try_into().map(Self).map_err(|_| InvalidMasterKeySalt)
    }

    /// The salt bytes, for the KDF and for the column.
    pub const fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The text the wire carries.
    pub fn to_base64(&self) -> String {
        URL_SAFE.encode(self.0)
    }
}

impl std::fmt::Debug for MasterKeySalt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MasterKeySalt({} bytes)", self.0.len())
    }
}

impl Serialize for MasterKeySalt {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_base64())
    }
}

impl<'de> Deserialize<'de> for MasterKeySalt {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = URL_SAFE
            .decode(text.as_bytes())
            .map_err(|e| serde::de::Error::custom(format!("not base64: {e}")))?;
        Self::from_slice(&bytes).map_err(|_| {
            serde::de::Error::custom(format!(
                "MasterKeySalt of {} bytes, not {}",
                bytes.len(),
                Self::LEN
            ))
        })
    }
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
        only::<MasterKeySalt>(&[MasterKeySalt::LEN]);
        assert_eq!(WrappedDocKey::LENGTHS, [72, 104]);
        assert_eq!(WrappedMasterKey::LENGTHS, [72]);
        assert_eq!(MasterKeySalt::LEN, 32);
        assert!(reads::<SealedName>(0) && reads::<SealedName>(SealedName::MAX_LEN));
    }

    /// A salt of any width but its own is refused when it is built, so a
    /// short one never reaches the passphrase derivation at unlock.
    #[test]
    fn a_master_key_salt_of_another_width_is_refused_when_built() {
        assert!(MasterKeySalt::from_slice(&[0xAB; 16]).is_err());
        assert!(MasterKeySalt::from_slice(&[0xAB; 33]).is_err());
        assert!(MasterKeySalt::from_slice(&[]).is_err());
        let salt = MasterKeySalt::from_slice(&[0xAB; MasterKeySalt::LEN]).unwrap();
        assert_eq!(salt, MasterKeySalt::from_array([0xAB; 32]));
    }

    /// `SRV-230`: a sealed name reads up to the longest name in four-byte
    /// characters, and a byte past it is refused.
    #[test]
    fn a_sealed_name_past_the_longest_name_is_refused() {
        assert_eq!(SealedName::MAX_LEN, 185 + 4 * 128);
        assert!(reads::<SealedName>(SealedName::MAX_LEN));
        assert!(!reads::<SealedName>(SealedName::MAX_LEN + 1));
    }
}
