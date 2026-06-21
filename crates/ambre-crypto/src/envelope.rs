//! Generic seal/open over the encrypted envelope.
//!
//! All three wire messages (Update, Snapshot, Ephemeral) share one cryptographic
//! envelope; they differ only by their associated data (`aad` = the serialized
//! public metadata, supplied by the caller). This module composes AEAD, the HMAC
//! commitment tag and the Ed25519 signature into a single [`Sealed`] value.
//!
//! Assembly into `ambre_protocol::{Update,Snapshot,Ephemeral}` and the
//! compression version byte live in the Sync thread, not here.

use ed25519_dalek::SigningKey;

use crate::aead;
use crate::commitment;
use crate::error::CryptoError;
use crate::keys::DocumentKey;
use crate::signature;

/// The cryptographic envelope: nonce, ciphertext, commitment tag and signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    /// XChaCha20 nonce.
    pub nonce: [u8; aead::NONCE_LEN],
    /// AEAD ciphertext (Poly1305 tag appended).
    pub ciphertext: Vec<u8>,
    /// HMAC-SHA256 key-commitment tag.
    pub commitment_tag: [u8; commitment::TAG_LEN],
    /// Ed25519 signature over `nonce || ciphertext || aad || commitment_tag`.
    pub signature: [u8; signature::SIGNATURE_LEN],
}

/// Encrypts and authenticates `plaintext` under `doc_key`, binding it to `aad`
/// and signing with `signing_key`.
pub fn seal(
    plaintext: &[u8],
    aad: &[u8],
    doc_key: &DocumentKey,
    signing_key: &SigningKey,
) -> Sealed {
    let (nonce, ciphertext) = aead::seal(plaintext, aad, doc_key);
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
/// (Sync) drops the message and raises a `SyncWarning`.
pub fn open(
    sealed: &Sealed,
    aad: &[u8],
    doc_key: &DocumentKey,
    author_pub: &[u8; 32],
) -> Result<Vec<u8>, CryptoError> {
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
