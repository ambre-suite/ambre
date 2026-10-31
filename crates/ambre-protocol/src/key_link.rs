//! One link of a document's key chain (`ambre-spec-key-rotation.md`).
//!
//! A rotation leaves the previous key sealed under the new one and signed by
//! the rotator (`SYNC-116`). The relay stores the link and indexes it by the two
//! identifiers it names, and opens nothing: the sealed key is a client object.

use ambre_core::id::{Document, Id};
use ambre_core::{Encoded, EncodedKeyLink, FixedBytes, KeyId};
use serde::{Deserialize, Serialize};

use crate::error::{SerializationError, ValidationError};

/// The most bytes a sealed previous key may take. One 32-byte key in the full
/// envelope fills about 220; the bound refuses a link padded to fill the
/// relay's table.
pub const MAX_SEALED_KEY_LEN: usize = 256;

/// The previous key of `document_id`, sealed under the key `key` names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyLink {
    /// The document whose keys the link chains.
    pub document_id: Id<Document>,
    /// The key the link starts from: the one installed by the rotation.
    pub key: FixedBytes<KeyId>,
    /// The key it seals: the one the rotation replaced.
    pub previous: FixedBytes<KeyId>,
    /// `previous`, sealed under `key` and signed by the rotator. Its header
    /// names `key` and the rotator's signing key.
    pub sealed: ambre_core::Payload<ambre_core::SealedPreviousKey>,
}

impl KeyLink {
    /// Refuses a link the relay must not store.
    ///
    /// # Errors
    ///
    /// [`ValidationError::EmptyCiphertext`] for an empty seal,
    /// [`ValidationError::OversizedKeyLink`] past [`MAX_SEALED_KEY_LEN`], and
    /// [`ValidationError::SelfLink`] for a link that seals its own key.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.sealed.is_empty() {
            return Err(ValidationError::EmptyCiphertext);
        }
        if self.sealed.as_bytes().len() > MAX_SEALED_KEY_LEN {
            return Err(ValidationError::OversizedKeyLink);
        }
        if self.key == self.previous {
            return Err(ValidationError::SelfLink);
        }
        Ok(())
    }

    /// The bytes a request carries.
    ///
    /// # Errors
    ///
    /// [`SerializationError::Postcard`] from the encoder.
    pub fn encode(&self) -> Result<Encoded<EncodedKeyLink>, SerializationError> {
        Ok(Encoded::new(postcard::to_allocvec(self)?))
    }

    /// Reads back what [`Self::encode`] wrote.
    ///
    /// # Errors
    ///
    /// [`SerializationError::Postcard`] for bytes that are not a link.
    pub fn decode(encoded: &Encoded<EncodedKeyLink>) -> Result<Self, SerializationError> {
        Ok(postcard::from_bytes(encoded.as_bytes())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(sealed: usize, previous: u8) -> KeyLink {
        KeyLink {
            document_id: Id::from_bytes([1; 16]),
            key: FixedBytes::from_array([2; 16]),
            previous: FixedBytes::from_array([previous; 16]),
            sealed: ambre_core::Payload::new(vec![7; sealed]),
        }
    }

    /// The relay stores a link of the size one key needs. It refuses an empty
    /// one, a padded one and one that seals its own key.
    #[test]
    fn a_link_the_relay_stores_seals_one_other_key() {
        assert_eq!(link(220, 3).validate(), Ok(()));
        assert_eq!(link(0, 3).validate(), Err(ValidationError::EmptyCiphertext));
        assert_eq!(
            link(MAX_SEALED_KEY_LEN + 1, 3).validate(),
            Err(ValidationError::OversizedKeyLink)
        );
        assert_eq!(link(220, 2).validate(), Err(ValidationError::SelfLink));
        let sent = link(220, 3);
        assert_eq!(KeyLink::decode(&sent.encode().unwrap()).unwrap(), sent);
    }

    /// The seal travels as the byte string it always did, last in the link:
    /// its length as a varint, then the bytes.
    #[test]
    fn a_link_encodes_its_seal_as_a_plain_byte_string() {
        let link = link(220, 3).encode().unwrap();
        let encoded = link.as_bytes();
        let seal = [[0xdc, 0x01].as_slice(), &[7; 220]].concat();
        assert!(
            encoded.ends_with(&seal),
            "220 as a postcard varint, then the bytes"
        );
    }
}
