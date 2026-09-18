//! XChaCha20-Poly1305 authenticated encryption with associated data.
//!
//! The 192-bit nonce is generated per message from the CSPRNG; collision over
//! 2^96 messages is cryptographically negligible, so random nonces are safe.
//! The associated data (`aad`) binds the ciphertext to the message's public
//! metadata without encrypting it.

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

use crate::error::CryptoError;
use crate::keys::{DocumentKey, FileKey};

/// Length of the XChaCha20 nonce, in bytes.
pub const NONCE_LEN: usize = 24;

/// Encrypts `plaintext` under `key` with the given `aad`, returning a freshly
/// generated nonce and the ciphertext (Poly1305 tag appended).
///
/// # Errors
///
/// [`CryptoError::EncryptionFailed`] when `plaintext` exceeds the size limit of
/// XChaCha20-Poly1305. The cipher raises nothing else.
pub fn seal(
    plaintext: &[u8],
    aad: &[u8],
    key: &DocumentKey,
) -> Result<([u8; NONCE_LEN], Vec<u8>), CryptoError> {
    seal_under(plaintext, aad, key.as_bytes())
}

/// Decrypts `ciphertext` under `key`, verifying the Poly1305 tag against `aad`.
///
/// # Errors
///
/// [`CryptoError::AeadFailed`] when the tag does not verify: a wrong key,
/// wrong `aad`, or a modified ciphertext. Poly1305 cannot tell a caller which.
pub fn open(
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
    aad: &[u8],
    key: &DocumentKey,
) -> Result<Vec<u8>, CryptoError> {
    open_under(nonce, ciphertext, aad, key.as_bytes())
}

/// Seals one passphrase-sealed `.ambre` file under its own [`FileKey`].
///
/// A separate entry point from [`seal`], not a generic one: the two keys are
/// not interchangeable (`ambre-spec-docfile.md section 5`). Two functions with
/// two key types turn a swap into a compile error rather than a review note.
///
/// # Errors
///
/// [`CryptoError::EncryptionFailed`] when `plaintext` exceeds the size limit of
/// XChaCha20-Poly1305.
pub fn seal_file(
    plaintext: &[u8],
    aad: &[u8],
    key: &FileKey,
) -> Result<([u8; NONCE_LEN], Vec<u8>), CryptoError> {
    seal_under(plaintext, aad, key.as_bytes())
}

/// Opens one passphrase-sealed `.ambre` file under its own [`FileKey`].
///
/// [`CryptoError::AeadFailed`] here means a wrong passphrase **or** a modified
/// file. Poly1305 cannot tell a caller which, and a message that claims one is
/// guessing.
///
/// # Errors
///
/// [`CryptoError::AeadFailed`], for the reason the paragraph above states.
pub fn open_file(
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
    aad: &[u8],
    key: &FileKey,
) -> Result<Vec<u8>, CryptoError> {
    open_under(nonce, ciphertext, aad, key.as_bytes())
}

/// The cipher itself, over raw key bytes. Private: every caller reaches it
/// through one of the typed pairs above, so the key type is never erased at a
/// call site.
fn seal_under(
    plaintext: &[u8],
    aad: &[u8],
    key: &[u8; 32],
) -> Result<([u8; NONCE_LEN], Vec<u8>), CryptoError> {
    let nonce = crate::random::bytes::<NONCE_LEN>();
    let cipher = XChaCha20Poly1305::new(key.into());
    // Encryption is infallible for XChaCha20-Poly1305 except when the plaintext
    // exceeds the AEAD size limit. Surface that as an error instead of panicking
    // the Sync worker.
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| CryptoError::EncryptionFailed)?;
    Ok((nonce, ciphertext))
}

/// The inverse, over raw key bytes. Private for the same reason as
/// [`seal_under`].
fn open_under(
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
    aad: &[u8],
    key: &[u8; 32],
) -> Result<Vec<u8>, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| CryptoError::AeadFailed)
}
