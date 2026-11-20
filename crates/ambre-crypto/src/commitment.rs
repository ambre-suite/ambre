//! HMAC-SHA256 key-commitment tag.
//!
//! XChaCha20-Poly1305 is not key-committing: a ciphertext may decrypt under more
//! than one key. The HMAC(document_key, nonce || ciphertext || aad) tag binds a
//! message to a specific key, defeating inter-document substitution by a
//! compromised relay. Distinct from the Poly1305 tag carried inside the AEAD.

use ambre_core::id::{Document, Id};
use ambre_core::{Ciphertext, CommitmentTag, FixedBytes, Nonce, Payload, Secret};
use hmac::digest::FixedOutput;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::CryptoError;
use crate::keys::DocumentKey;

/// Length of the commitment tag, in bytes: the array length of the subkey
/// this module derives. A crate that frames a tag reads
/// [`FixedBytes::<CommitmentTag>::SIZE`](FixedBytes::SIZE).
const TAG_LEN: usize = 32;

const _: () = assert!(FixedBytes::<CommitmentTag>::SIZE.get() == TAG_LEN as u64);

type HmacSha256 = Hmac<Sha256>;

/// An HMAC-SHA256 keyed by `key`. The one kept panic site of the module.
#[expect(
    clippy::expect_used,
    reason = "HMAC accepts a key of any length, so the constructor cannot fail"
)]
fn keyed(key: &[u8]) -> HmacSha256 {
    HmacSha256::new_from_slice(key).expect("HMAC accepts a key of any length")
}

/// The HMAC over `nonce || ciphertext || aad`. Each field takes its own type,
/// so a swap of two fields does not compile.
fn mac(
    key: &DocumentKey,
    nonce: &FixedBytes<Nonce>,
    ciphertext: &Payload<Ciphertext>,
    aad: &ambre_core::Aad,
) -> HmacSha256 {
    let mut m = keyed(key.as_bytes());
    m.update(nonce.as_slice());
    m.update(ciphertext.as_bytes());
    m.update(aad.as_bytes());
    m
}

/// Computes the commitment tag over `nonce || ciphertext || aad`.
pub fn commitment_tag(
    key: &DocumentKey,
    nonce: &FixedBytes<Nonce>,
    ciphertext: &Payload<Ciphertext>,
    aad: &ambre_core::Aad,
) -> FixedBytes<CommitmentTag> {
    FixedBytes::from_array(
        mac(key, nonce, ciphertext, aad)
            .finalize()
            .into_bytes()
            .into(),
    )
}

/// Domain-separation tag of the media-address subkey. The subkey keeps the
/// address apart from the commitment tag, which takes the document key
/// directly. Version 2 binds the document (`SYNC-286`, audit `P20`).
const MEDIA_ADDRESS_DOMAIN: &[u8] = b"ambre-media-address-v2";

/// The address of a media blob of `document_id`:
/// `HMAC(HMAC(key, domain || document_id), plaintext)` (`ambre-spec-media.md`,
/// `SYNC-050`, `SYNC-286`).
///
/// Equal bytes under one document key in one document yield one address, so
/// a picture used ten times is one object. Without the key, nobody can
/// compute the address of a file they hold, so the relay cannot ask whether
/// a document holds it. Two documents under one key give one file two
/// addresses, so the relay cannot tell that both hold it.
///
/// # Panics
///
/// Never. HMAC accepts a key of any length, so its constructor cannot fail.
pub fn media_address(
    key: &DocumentKey,
    document_id: Id<Document>,
    plaintext: &ambre_core::Payload<ambre_core::MediaBytes>,
) -> FixedBytes<ambre_core::MediaAddress> {
    // The subkey is key material, so the HMAC writes it in place under the
    // wiping wrapper, and no bare copy of it leaves this function. An
    // identifier has one length, so the domain and the identifier never
    // read as another pair.
    let subkey: Secret<[u8; TAG_LEN]> = Secret::filled(|out: &mut [u8; TAG_LEN]| {
        let mut m = keyed(key.as_bytes());
        m.update(MEDIA_ADDRESS_DOMAIN);
        m.update(document_id.as_bytes());
        FixedOutput::finalize_into(m, out.into());
    });
    let mut m = keyed(subkey.expose());
    m.update(plaintext.as_bytes());
    FixedBytes::from_array(m.finalize().into_bytes().into())
}

// A media address is one HMAC output wide.
const _: () = assert!(ambre_core::MEDIA_ADDRESS_LEN == TAG_LEN);

/// Verifies `tag` against a freshly computed commitment, in constant time.
///
/// # Errors
///
/// [`CryptoError::CommitmentMismatch`] when `tag` does not match the computed
/// one: a substituted key, or an input the tag does not cover.
pub fn verify_commitment(
    key: &DocumentKey,
    nonce: &FixedBytes<Nonce>,
    ciphertext: &Payload<Ciphertext>,
    aad: &ambre_core::Aad,
    tag: &FixedBytes<CommitmentTag>,
) -> Result<(), CryptoError> {
    mac(key, nonce, ciphertext, aad)
        .verify_slice(tag.as_slice())
        .map_err(|_| CryptoError::CommitmentMismatch)
}
