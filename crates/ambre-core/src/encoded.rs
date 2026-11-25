//! Sealed and signed bytes, one generic type over a role marker.
//!
//! A member row carries a wrapped document key beside a sealed name. An
//! account carries a wrapped master key beside its salt, and a change
//! carries a signed log entry. The relay stores all of them and reads none.
//! A bare `Vec<u8>` lets one stand for another, down to the argument order
//! of an insert. [`Encoded`] keeps them apart, from the JSON body to the
//! column: `Encoded<WrappedDocKey>` and `Encoded<SealedName>` are two types.
//!
//! On the wire each is its URL-safe base64 text, decoded when the body is
//! read. A value that is not base64 refuses the body there, as any field
//! that does not decode does (`SRV-178`). A role whose layout fixes its
//! length refuses any other length there too (`SRV-226`). A sealed name
//! refuses a length past the longest name it holds (`SRV-230`). Each policy
//! is a constant of the marker, and one decoder reads them all. `Debug`
//! prints the length alone.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::base64::{self, Alphabet};
use crate::fixed_bytes::{DebugForm, FixedBytes, FixedRole};
use crate::{CommitmentTag, KeyId, Nonce, PublicKey, Signature};

/// The nonce an AEAD seal opens with, as `ambre_crypto::aead` writes it.
const NONCE_LEN: usize = FixedBytes::<Nonce>::SIZE.get() as usize;
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
const OBJECT_HEADER_LEN: usize = crate::Count::<crate::Octets>::new(1)
    .plus(FixedBytes::<KeyId>::SIZE)
    .plus(FixedBytes::<PublicKey>::SIZE)
    .plus(FixedBytes::<Signature>::SIZE)
    .plus(FixedBytes::<CommitmentTag>::SIZE)
    .plus(FixedBytes::<Nonce>::SIZE)
    .get() as usize;
/// The most bytes one character takes in UTF-8.
const UTF8_MAX_LEN: usize = 4;

/// The version byte a wrapped document key and a wrapped master key open
/// with (audit `P59`, `ZK-047`).
const VERSION_LEN: usize = 1;
/// The lengths of a wrapped document key: its version byte, then the key
/// sealed alone, or beside the anchor of its log. No layout without the
/// version byte is read (audit `P59`).
const WRAPPED_DOC_KEY_LENGTHS: &[usize] = &[
    VERSION_LEN + NONCE_LEN + KEY_LEN + TAG_LEN,
    VERSION_LEN + NONCE_LEN + KEY_LEN + ANCHOR_LEN + TAG_LEN,
];
/// The length of a wrapped master key: the versioned layout every seal
/// writes (`ZK-047`). MVP has not shipped, so no account holds the layout
/// before the version byte, and none is read.
const WRAPPED_MASTER_KEY_LENGTHS: &[usize] = &[VERSION_LEN + NONCE_LEN + KEY_LEN + TAG_LEN];
/// The longest name a sealed name holds, in characters.
const SEALED_NAME_MAX_CHARS: usize = crate::NAME_MAX_CHARS;
/// The most bytes a sealed name holds: its overhead and the longest name.
const SEALED_NAME_MAX_LEN: usize =
    OBJECT_HEADER_LEN + UTF8_MAX_LEN * SEALED_NAME_MAX_CHARS + TAG_LEN;

mod sealed {
    /// Closes [`super::EncodedKind`] to the roles of this module: each length
    /// policy is a rule of the relay (`SRV-226`), read here in one place.
    pub trait Sealed {}
    impl Sealed for super::WrappedDocKey {}
    impl Sealed for super::SealedName {}
    impl Sealed for super::WrappedMasterKey {}
    impl Sealed for super::EscrowedKey {}
    impl Sealed for super::SealedShare {}
    impl Sealed for super::EncodedLogEntry {}
    impl Sealed for super::EncodedKeyLink {}
}

/// The role of a sealed or signed value that travels as base64: its name,
/// and the lengths its layout allows. The trait is sealed to the roles of
/// this module.
pub trait EncodedKind: sealed::Sealed + 'static {
    /// The name the debug form and a refusal print.
    const NAME: crate::StaticKey<crate::DebugNames>;
    /// The lengths the layout of this role allows; a read refuses any other.
    /// `None` when no layout fixes the length.
    const LENGTHS: Option<&'static [usize]> = None;
    /// The most bytes a value of this role holds; a read refuses more.
    const MAX_LEN: Option<usize> = None;
}

/// A sealed or signed value of the role `K`. On the wire it is its URL-safe
/// base64 text, and a read refuses any length its role does not allow.
pub struct Encoded<K: EncodedKind> {
    bytes: Vec<u8>,
    kind: PhantomData<fn() -> K>,
}

impl<K: EncodedKind> Encoded<K> {
    /// Wraps the bytes.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            kind: PhantomData,
        }
    }

    /// The bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The bytes, owned.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Whether there are no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The text the wire carries.
    #[must_use]
    pub fn to_base64(&self) -> String {
        base64::encode(Alphabet::UrlSafe, &self.bytes)
    }

    /// Why `bytes` cannot be a value of this role, or `None` when it can.
    fn refusal(bytes: &[u8]) -> Option<String> {
        let len = bytes.len();
        if let Some(lengths) = K::LENGTHS.filter(|lengths| !lengths.contains(&len)) {
            return Some(format!(
                "{} of {len} bytes, not one of {lengths:?}",
                K::NAME.as_str()
            ));
        }
        K::MAX_LEN
            .filter(|max| len > *max)
            .map(|max| format!("{} of {len} bytes, past {max}", K::NAME.as_str()))
    }
}

impl<K: EncodedKind> From<Vec<u8>> for Encoded<K> {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl<K: EncodedKind> Clone for Encoded<K> {
    fn clone(&self) -> Self {
        Self::new(self.bytes.clone())
    }
}

impl<K: EncodedKind> Default for Encoded<K> {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl<K: EncodedKind> PartialEq for Encoded<K> {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl<K: EncodedKind> Eq for Encoded<K> {}

impl<K: EncodedKind> Hash for Encoded<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl<K: EncodedKind> fmt::Debug for Encoded<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({} bytes)", K::NAME.as_str(), self.bytes.len())
    }
}

impl<K: EncodedKind> Serialize for Encoded<K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_base64())
    }
}

impl<'de, K: EncodedKind> Deserialize<'de> for Encoded<K> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = base64::decode(Alphabet::UrlSafe, &text)
            .map_err(|e| serde::de::Error::custom(format!("not base64: {e}")))?;
        match Self::refusal(&bytes) {
            Some(refusal) => Err(serde::de::Error::custom(refusal)),
            None => Ok(Self::new(bytes)),
        }
    }
}

/// The role of a document key wrapped to one member:
/// `version || nonce || ciphertext`, the key sealed alone or beside the
/// anchor of its log. `ambre_crypto::ecdh` writes and reads the layout.
/// `Encoded<WrappedDocKey>`.
pub enum WrappedDocKey {}

impl EncodedKind for WrappedDocKey {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("WrappedDocKey");
    const LENGTHS: Option<&'static [usize]> = Some(WRAPPED_DOC_KEY_LENGTHS);
}

impl Encoded<WrappedDocKey> {
    /// The lengths the layout of a wrapped key allows; a read refuses any
    /// other.
    pub const LENGTHS: &'static [usize] = WRAPPED_DOC_KEY_LENGTHS;
}

/// The role of a document or channel name sealed under its document key: the
/// object header, the name, the tag. `Encoded<SealedName>`.
pub enum SealedName {}

impl EncodedKind for SealedName {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("SealedName");
    const MAX_LEN: Option<usize> = Some(SEALED_NAME_MAX_LEN);
}

impl Encoded<SealedName> {
    /// The longest name a sealed name holds, in characters.
    pub const MAX_CHARS: usize = SEALED_NAME_MAX_CHARS;

    /// The bytes a sealed name adds to the name it holds.
    pub const OVERHEAD: usize = OBJECT_HEADER_LEN + TAG_LEN;

    /// The most bytes a sealed name holds; a read refuses more.
    pub const MAX_LEN: usize = SEALED_NAME_MAX_LEN;
}

/// The role of an account's master seed wrapped under the key its passphrase
/// derives: `version || nonce || ciphertext`. `Encoded<WrappedMasterKey>`.
pub enum WrappedMasterKey {}

impl EncodedKind for WrappedMasterKey {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("WrappedMasterKey");
    const LENGTHS: Option<&'static [usize]> = Some(WRAPPED_MASTER_KEY_LENGTHS);
}

impl Encoded<WrappedMasterKey> {
    /// The length the one layout of a wrapped master key allows.
    pub const LENGTHS: &'static [usize] = WRAPPED_MASTER_KEY_LENGTHS;
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
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("MasterKeySalt");
    const DEBUG: DebugForm = DebugForm::Length;
    type Array = [u8; MASTER_KEY_SALT_LEN];
}

/// The role of a member's master seed sealed to their organization, for a
/// recovery (`SRV-067`). No client seals one yet, so no layout fixes its
/// length. `Encoded<EscrowedKey>`.
pub enum EscrowedKey {}

impl EncodedKind for EscrowedKey {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("EscrowedKey");
}

/// The role of one share of an organization's recovery secret, sealed to its
/// holder. `Encoded<SealedShare>`.
pub enum SealedShare {}

impl EncodedKind for SealedShare {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("SealedShare");
}

/// The role of a signed entry of a document's membership log.
/// `Encoded<EncodedLogEntry>`.
pub enum EncodedLogEntry {}

impl EncodedKind for EncodedLogEntry {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("EncodedLogEntry");
}

/// The role of a signed link from one document key to the next.
/// `Encoded<EncodedKeyLink>`.
pub enum EncodedKeyLink {}

impl EncodedKind for EncodedKeyLink {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("EncodedKeyLink");
}

#[cfg(test)]
mod tests {
    use super::{
        Encoded, EncodedKind, FixedBytes, MASTER_KEY_SALT_LEN, MasterKeySalt, SealedName,
        WrappedDocKey, WrappedMasterKey,
    };

    fn wire(len: usize) -> String {
        serde_json::to_string(&Encoded::<SealedName>::new(vec![7; len])).unwrap()
    }

    fn reads<K: EncodedKind>(len: usize) -> bool {
        serde_json::from_str::<Encoded<K>>(&wire(len)).is_ok()
    }

    #[test]
    fn an_encoded_value_travels_as_its_url_safe_base64() {
        let name = Encoded::<SealedName>::new(vec![0xab, 0xcd, 0xef]);
        assert_eq!(serde_json::to_string(&name).unwrap(), "\"q83v\"");
        let parsed: Encoded<SealedName> = serde_json::from_str("\"q83v\"").unwrap();
        assert_eq!(parsed.as_bytes(), &[0xab, 0xcd, 0xef]);
        assert_eq!(format!("{parsed:?}"), "SealedName(3 bytes)");
    }

    #[test]
    fn a_value_that_is_not_base64_refuses_the_read() {
        assert!(serde_json::from_str::<Encoded<SealedName>>("\"not base64!\"").is_err());
        assert!(serde_json::from_str::<Encoded<SealedName>>("\"q83\"").is_err());
    }

    /// `SRV-226`: each role whose layout fixes its length reads that length
    /// alone. An empty value, one byte short and one byte past are refused.
    #[test]
    fn a_fixed_role_refuses_any_other_length() {
        fn only<K: EncodedKind>(lengths: &[usize]) {
            for &len in lengths {
                assert!(reads::<K>(len), "{len}");
                for near in [len - 1, len + 1] {
                    assert_eq!(reads::<K>(near), lengths.contains(&near), "{near}");
                }
            }
            assert!(!reads::<K>(0));
        }
        only::<WrappedDocKey>(Encoded::<WrappedDocKey>::LENGTHS);
        only::<WrappedMasterKey>(Encoded::<WrappedMasterKey>::LENGTHS);
        // Audit `P59`: no unversioned layout of either is read.
        assert_eq!(Encoded::<WrappedDocKey>::LENGTHS, [73, 105]);
        assert_eq!(Encoded::<WrappedMasterKey>::LENGTHS, [73]);
        for unversioned in [72, 104] {
            assert!(!reads::<WrappedDocKey>(unversioned), "{unversioned}");
        }
        assert!(!reads::<WrappedMasterKey>(72));
        assert!(reads::<SealedName>(0) && reads::<SealedName>(Encoded::<SealedName>::MAX_LEN));
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
        assert_eq!(Encoded::<SealedName>::MAX_LEN, 185 + 4 * 128);
        assert!(reads::<SealedName>(Encoded::<SealedName>::MAX_LEN));
        assert!(!reads::<SealedName>(Encoded::<SealedName>::MAX_LEN + 1));
    }
}
