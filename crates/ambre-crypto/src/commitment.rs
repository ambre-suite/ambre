//! HMAC-SHA256 key-commitment tag.
//!
//! XChaCha20-Poly1305 is not key-committing: a ciphertext may decrypt under more
//! than one key. The HMAC(document_key, nonce || ciphertext || aad) tag binds a
//! message to a specific key, defeating inter-document substitution by a
//! compromised relay. Distinct from the Poly1305 tag carried inside the AEAD.

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::CryptoError;
use crate::keys::DocumentKey;

/// Length of the commitment tag, in bytes.
pub const TAG_LEN: usize = 32;

type HmacSha256 = Hmac<Sha256>;

fn mac(key: &DocumentKey, nonce: &[u8], ciphertext: &[u8], aad: &[u8]) -> HmacSha256 {
    let mut m =
        HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC accepts a key of any length");
    m.update(nonce);
    m.update(ciphertext);
    m.update(aad);
    m
}

/// Computes the commitment tag over `nonce || ciphertext || aad`.
pub fn commitment_tag(
    key: &DocumentKey,
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
) -> [u8; TAG_LEN] {
    mac(key, nonce, ciphertext, aad)
        .finalize()
        .into_bytes()
        .into()
}

/// Verifies `tag` against a freshly computed commitment, in constant time.
pub fn verify_commitment(
    key: &DocumentKey,
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
    tag: &[u8; TAG_LEN],
) -> Result<(), CryptoError> {
    mac(key, nonce, ciphertext, aad)
        .verify_slice(tag)
        .map_err(|_| CryptoError::CommitmentMismatch)
}
