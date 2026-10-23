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

/// An HMAC-SHA256 keyed by `key`. The one kept panic site of the module.
#[expect(
    clippy::expect_used,
    reason = "HMAC accepts a key of any length, so the constructor cannot fail"
)]
fn keyed(key: &[u8]) -> HmacSha256 {
    HmacSha256::new_from_slice(key).expect("HMAC accepts a key of any length")
}

fn mac(key: &DocumentKey, nonce: &[u8], ciphertext: &[u8], aad: &[u8]) -> HmacSha256 {
    let mut m = keyed(key.as_bytes());
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

/// Domain-separation tag of the media-address subkey. The subkey keeps the
/// address apart from the commitment tag, which takes the document key directly.
const MEDIA_ADDRESS_DOMAIN: &[u8] = b"ambre-media-address-v1";

/// The address of a media blob: `HMAC(HMAC(key, domain), plaintext)`
/// (`ambre-spec-media.md`, `SYNC-050`).
///
/// Equal bytes under one document key yield one address, so a picture used
/// ten times is one object. Without the key, nobody can compute the address
/// of a file they hold, so the relay cannot ask whether a document holds it.
///
/// # Panics
///
/// Never. HMAC accepts a key of any length, so its constructor cannot fail.
pub fn media_address(key: &DocumentKey, plaintext: &[u8]) -> ambre_core::MediaAddress {
    let subkey: [u8; TAG_LEN] = mac_over(key, MEDIA_ADDRESS_DOMAIN);
    let mut m = keyed(&subkey);
    m.update(plaintext);
    ambre_core::MediaAddress::from_array(m.finalize().into_bytes().into())
}

// A media address is one HMAC output wide.
const _: () = assert!(ambre_core::MEDIA_ADDRESS_LEN == TAG_LEN);

fn mac_over(key: &DocumentKey, data: &[u8]) -> [u8; TAG_LEN] {
    let mut m = keyed(key.as_bytes());
    m.update(data);
    m.finalize().into_bytes().into()
}

/// Verifies `tag` against a freshly computed commitment, in constant time.
///
/// # Errors
///
/// [`CryptoError::CommitmentMismatch`] when `tag` does not match the computed
/// one: a substituted key, or an input the tag does not cover.
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
