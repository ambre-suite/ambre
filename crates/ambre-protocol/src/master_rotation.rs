//! What a re-seal of the Master Seed sends, and what its proof signs
//! (`SRV-278`, `SRV-279`, `SYNC-254`).
//!
//! A re-seal replaces the sealed seed the relay holds. A session alone does not
//! authorize it: a stolen session would lock the owner out of the seed. The
//! client signs the new sealed seed and the salt of the one it replaces with
//! the signing key of the account. The relay verifies the signature under the
//! key it registered, and writes only over the sealed seed the proof names.

use ambre_core::encoded::Encoded;
use ambre_core::{FixedBytes, MasterKeySalt, Signature, WrappedMasterKey};

use crate::transcript::SignedTranscript;

/// The domain the transcript opens with, so the signature serves no other use.
pub const ROTATION_DOMAIN: &[u8] = b"ambre-master-rotation-v1";

/// The Master Seed sealed under a passphrase, and the salt that derives the
/// sealing key: the whole of what the relay stores of the seed.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SealedSeed {
    pub master_key_enc: Encoded<WrappedMasterKey>,
    pub master_key_salt: FixedBytes<MasterKeySalt>,
}

/// The body of a re-seal. It holds the new sealed seed and the salt of the
/// sealed seed it replaces. The account signs the two.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct MasterRotation {
    /// The sealed seed the relay stores in place of the current one.
    pub sealed: SealedSeed,
    /// The salt of the sealed seed the client read and replaces. Each seal
    /// draws a fresh salt, so the salt names one sealed seed.
    pub replaces: FixedBytes<MasterKeySalt>,
    /// The Ed25519 signature of the account's signing key over
    /// [`rotation_transcript`], hex.
    pub signature: FixedBytes<Signature>,
}

impl MasterRotation {
    /// The bytes [`MasterRotation::signature`] covers.
    #[must_use]
    pub fn transcript(&self) -> SignedTranscript {
        rotation_transcript(&self.replaces, &self.sealed)
    }
}

/// The bytes a re-seal signs: the domain, the salt of the replaced sealed seed,
/// then the new salt and the new sealed seed.
///
/// Each part has a fixed length: the salt its width, and a wrapped master key
/// the one length its layout allows. So no two inputs share a transcript.
#[must_use]
pub fn rotation_transcript(
    replaces: &FixedBytes<MasterKeySalt>,
    sealed: &SealedSeed,
) -> SignedTranscript {
    let enc = sealed.master_key_enc.as_bytes();
    // The domain and two salts. The compiler sums the constants, so the sum
    // cannot overflow at run time.
    let fixed = const { ROTATION_DOMAIN.len() + 2 * ambre_core::MASTER_KEY_SALT_LEN };
    let mut transcript = Vec::with_capacity(fixed.saturating_add(enc.len()));
    transcript.extend_from_slice(ROTATION_DOMAIN);
    transcript.extend_from_slice(replaces.as_slice());
    transcript.extend_from_slice(sealed.master_key_salt.as_slice());
    transcript.extend_from_slice(enc);
    SignedTranscript::new(transcript)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sealed(enc: u8, salt: u8) -> SealedSeed {
        SealedSeed {
            master_key_enc: Encoded::new(vec![enc; Encoded::<WrappedMasterKey>::LENGTHS[0]]),
            master_key_salt: FixedBytes::from_array([salt; 32]),
        }
    }

    /// `SRV-278`: a proof names the replaced seal, the new salt and the new
    /// sealed seed. Changing any of the three changes what is signed, and the
    /// domain sets it apart from every other signature of the account.
    #[test]
    fn the_transcript_names_the_replaced_salt_and_the_new_sealed_seed() {
        let replaces = FixedBytes::<MasterKeySalt>::from_array([9; 32]);
        let base = rotation_transcript(&replaces, &sealed(1, 2));
        assert!(base.as_bytes().starts_with(ROTATION_DOMAIN));
        assert_ne!(
            base,
            rotation_transcript(&FixedBytes::from_array([8; 32]), &sealed(1, 2))
        );
        assert_ne!(base, rotation_transcript(&replaces, &sealed(1, 3)));
        assert_ne!(base, rotation_transcript(&replaces, &sealed(4, 2)));
        // The replaced salt and the new salt sit at fixed offsets: swapping
        // them is another transcript.
        assert_ne!(
            rotation_transcript(&FixedBytes::from_array([2; 32]), &sealed(1, 9)),
            base
        );
    }

    /// The body reads back from the JSON it writes, field for field.
    #[test]
    fn a_rotation_reads_back_from_its_json() {
        let sent = MasterRotation {
            sealed: sealed(1, 2),
            replaces: FixedBytes::from_array([3; 32]),
            signature: FixedBytes::from_array([4; 64]),
        };
        let json = serde_json::to_value(&sent).expect("json");
        let read: MasterRotation = serde_json::from_value(json).expect("reads");
        assert_eq!(read.transcript(), sent.transcript());
        assert_eq!(read.signature, sent.signature);
    }
}
