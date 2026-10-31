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

use crate::fixed_bytes::{DebugForm, FixedBytes, FixedRole};
use crate::{CommitmentTag, KeyId, Nonce, PublicKey, Signature};

/// The nonce an AEAD seal opens with, as `ambre_crypto::aead` writes it.
const NONCE_LEN: usize = FixedBytes::<Nonce>::SIZE.0 as usize;
/// The tag an AEAD seal closes with.
const TAG_LEN: usize = 16;
/// A document key, and a master seed.
const KEY_LEN: usize = 32;
/// The digest of a log's first entry, sealed beside a wrapped key.
const ANCHOR_LEN: usize = 32;
/// The header of a sealed object, as Sync writes it. The layout byte, the key
/// id, the author's key, the signature, the commitment and the nonce, each at
/// the size its own type states. The layout byte's type lives in `ambre-sync`,
/// which this crate cannot see, so its one byte is written here.
const OBJECT_HEADER_LEN: usize = crate::ByteSize(1)
    .saturating_add(FixedBytes::<KeyId>::SIZE)
    .saturating_add(FixedBytes::<PublicKey>::SIZE)
    .saturating_add(FixedBytes::<Signature>::SIZE)
    .saturating_add(FixedBytes::<CommitmentTag>::SIZE)
    .saturating_add(FixedBytes::<Nonce>::SIZE)
    .0 as usize;
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

impl WrappedDocKey {
    /// The wire form of a wrapped key: its nonce, then its ciphertext. The
    /// parts fix the length; nothing checks it against
    /// [`WrappedDocKey::LENGTHS`] here.
    pub fn from_parts(nonce: &FixedBytes<Nonce>, ciphertext: &crate::Ciphertext) -> Self {
        let mut bytes = Vec::with_capacity(NONCE_LEN + ciphertext.0.len());
        nonce.push_to(&mut bytes);
        bytes.extend_from_slice(&ciphertext.0);
        Self(bytes)
    }

    /// The nonce and the ciphertext [`WrappedDocKey::from_parts`] joined.
    /// `None` when the bytes are shorter than a nonce.
    #[must_use]
    pub fn parts(&self) -> Option<(FixedBytes<Nonce>, crate::Ciphertext)> {
        let (nonce, ciphertext) = FixedBytes::<Nonce>::split_from(&self.0)?;
        Some((nonce, crate::Ciphertext(ciphertext.to_vec())))
    }
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

/// The width of a master-key salt, in bytes.
pub const MASTER_KEY_SALT_LEN: usize = 32;

/// The role of the salt the passphrase key of an account is derived with:
/// `FixedBytes<MasterKeySalt>`.
///
/// Fixed at [`MASTER_KEY_SALT_LEN`] bytes, as [`FileSalt`](crate::FileSalt) is
/// at its own width. A value of another width is refused when it is built,
/// whether from the wire, a column or a caller (`SRV-239`). A short salt thus
/// never reaches the derivation at unlock. On the JSON wire it is its
/// hexadecimal text, and `Debug` prints the length alone.
pub enum MasterKeySalt {}

impl FixedRole for MasterKeySalt {
    const NAME: &'static str = "MasterKeySalt";
    const DEBUG: DebugForm = DebugForm::Length;
    type Array = [u8; MASTER_KEY_SALT_LEN];
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
    use super::{
        FixedBytes, KEY_LEN, MASTER_KEY_SALT_LEN, MasterKeySalt, NONCE_LEN, SealedName, TAG_LEN,
        WrappedDocKey, WrappedMasterKey,
    };
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
        assert_eq!(WrappedDocKey::LENGTHS, [72, 104]);
        assert_eq!(WrappedMasterKey::LENGTHS, [72]);
        assert!(reads::<SealedName>(0) && reads::<SealedName>(SealedName::MAX_LEN));
    }

    /// A wrapped key joins its nonce and its ciphertext in that order, at a
    /// length the wire reads back. It splits back into the same parts, and
    /// bytes shorter than a nonce have none.
    #[test]
    fn a_wrapped_key_is_its_nonce_then_its_ciphertext() {
        let nonce = FixedBytes::<crate::Nonce>::from_array([1; NONCE_LEN]);
        let ciphertext = crate::Ciphertext(vec![2; KEY_LEN + TAG_LEN]);
        let key = WrappedDocKey::from_parts(&nonce, &ciphertext);
        assert_eq!(&key.as_bytes()[..NONCE_LEN], &[1; NONCE_LEN]);
        assert_eq!(&key.as_bytes()[NONCE_LEN..], &ciphertext.0[..]);
        assert!(WrappedDocKey::LENGTHS.contains(&key.as_bytes().len()));
        assert_eq!(key.parts(), Some((nonce, ciphertext)));
        assert_eq!(WrappedDocKey::new(vec![0; NONCE_LEN - 1]).parts(), None);
    }

    /// A salt of any width but its own is refused when it is built, from a
    /// slice or from the JSON wire. So a short one never reaches the
    /// passphrase derivation at unlock.
    #[test]
    fn a_master_key_salt_of_another_width_is_refused_when_built() {
        for width in [16, 33, 0] {
            assert!(FixedBytes::<MasterKeySalt>::from_slice(&vec![0xAB; width]).is_err());
        }
        let salt = FixedBytes::<MasterKeySalt>::from_slice(&[0xAB; MASTER_KEY_SALT_LEN]).unwrap();
        assert_eq!(salt, FixedBytes::from_array([0xAB; 32]));
        let json = serde_json::to_string(&salt).unwrap();
        assert_eq!(json, format!("\"{}\"", "ab".repeat(32)));
        let back: FixedBytes<MasterKeySalt> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, salt);
        let short = format!("\"{}\"", "ab".repeat(31));
        assert!(serde_json::from_str::<FixedBytes<MasterKeySalt>>(&short).is_err());
        assert_eq!(format!("{salt:?}"), "MasterKeySalt(32 bytes)");
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
