//! Generic seal/open over the encrypted envelope.
//!
//! All three wire messages (Update, Snapshot, Ephemeral) share one cryptographic
//! envelope. They differ only by their associated data (`aad` = the serialized
//! public metadata, supplied by the caller). This module composes AEAD, the HMAC
//! commitment tag and the Ed25519 signature into a single [`Sealed`] value.
//!
//! Assembly into `ambre_protocol::{Update,Snapshot,Ephemeral}` and the
//! compression version byte live in the Sync thread, not here.

use ambre_core::{Ciphertext, CommitmentTag, MediaBytes, Nonce, PublicKey, SecretBytes, Signature};
use ed25519_dalek::SigningKey;

use crate::aead;
use crate::commitment;
use crate::error::CryptoError;
use crate::keys::DocumentKey;
use crate::signature;

/// The cryptographic envelope: nonce, ciphertext, commitment tag and signature.
///
/// Its `Debug` form prints the ciphertext length and none of the bytes, so a
/// `{:?}` in a log carries no payload (`ZK-017`).
#[derive(Clone, PartialEq, Eq)]
pub struct Sealed {
    /// XChaCha20 nonce.
    pub nonce: Nonce,
    /// AEAD ciphertext (Poly1305 tag appended).
    pub ciphertext: Ciphertext,
    /// HMAC-SHA256 key-commitment tag.
    pub commitment_tag: CommitmentTag,
    /// Ed25519 signature over `nonce || ciphertext || aad || commitment_tag`.
    pub signature: Signature,
}

impl core::fmt::Debug for Sealed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Sealed")
            .field("ciphertext_len", &self.ciphertext.len())
            .finish_non_exhaustive()
    }
}

/// Encrypts and authenticates `plaintext` under `doc_key`, binding it to `aad`
/// and signing with `signing_key`.
///
/// # Errors
///
/// [`CryptoError::EncryptionFailed`] from the AEAD. The commitment tag and the
/// signature are infallible.
pub fn seal(
    plaintext: &SecretBytes,
    aad: &ambre_core::Aad,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Sealed, CryptoError> {
    let (nonce, ciphertext) = aead::seal(plaintext.secret().expose(), aad, doc_key)?;
    Ok(envelope_of(nonce, ciphertext, aad, doc_key, signing_key))
}

/// Seals a media file as [`seal`] seals any plaintext, borrowed from the
/// [`MediaBytes`] that holds it. A media file is often megabytes long, and
/// this path makes no copy of it before the cipher.
///
/// # Errors
///
/// [`CryptoError::EncryptionFailed`] from the AEAD, as [`seal`].
pub fn seal_media(
    plaintext: &MediaBytes,
    aad: &ambre_core::Aad,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Result<Sealed, CryptoError> {
    let (nonce, ciphertext) = aead::seal(plaintext.as_bytes(), aad, doc_key)?;
    Ok(envelope_of(nonce, ciphertext, aad, doc_key, signing_key))
}

/// Completes the envelope of a ciphertext: the commitment tag, then the
/// signature over all of it.
fn envelope_of(
    nonce: Nonce,
    ciphertext: Ciphertext,
    aad: &ambre_core::Aad,
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Sealed {
    let commitment_tag = commitment::commitment_tag(doc_key, &nonce, &ciphertext, aad);
    let signature = signature::sign(signing_key, &nonce, &ciphertext, aad, &commitment_tag);
    Sealed {
        nonce,
        ciphertext,
        commitment_tag,
        signature,
    }
}

/// Verifies and decrypts a [`Sealed`] envelope in the spec order: signature,
/// then commitment tag, then AEAD. Any failure yields a typed error; the caller
/// (Sync) drops the message and raises a `SyncWarning`. The plaintext comes
/// back as [`SecretBytes`]. A caller reads it in place through
/// `SecretBytes::with_plaintext`, or moves it into a role that keeps it in the
/// clear through `SecretBytes::into_plaintext` (`ARCH-047`).
///
/// # Errors
///
/// The order below is the order of the checks.
///
/// * [`CryptoError::InvalidSignature`] when `author_pub` did not sign it.
/// * [`CryptoError::CommitmentMismatch`] when `doc_key` is not the key the
///   sender committed to.
/// * [`CryptoError::AeadFailed`] when the ciphertext does not verify.
pub fn open(
    sealed: &Sealed,
    aad: &ambre_core::Aad,
    doc_key: &DocumentKey,
    author_pub: &PublicKey,
) -> Result<SecretBytes, CryptoError> {
    signature::verify(
        author_pub,
        &sealed.nonce,
        &sealed.ciphertext,
        aad,
        &sealed.commitment_tag,
        &sealed.signature,
    )?;
    commitment::verify_commitment(
        doc_key,
        &sealed.nonce,
        &sealed.ciphertext,
        aad,
        &sealed.commitment_tag,
    )?;
    aead::open(&sealed.nonce, &sealed.ciphertext, aad, doc_key)
}
