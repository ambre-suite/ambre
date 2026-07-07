//! XChaCha20-Poly1305 authenticated encryption with associated data.
//!
//! The 192-bit nonce is generated per message from the CSPRNG; collision over
//! 2^96 messages is cryptographically negligible, so random nonces are safe.
//! The associated data (`aad`) binds the ciphertext to the message's public
//! metadata without encrypting it.

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

use crate::error::CryptoError;
use crate::keys::DocumentKey;

/// Length of the XChaCha20 nonce, in bytes.
pub const NONCE_LEN: usize = 24;

/// Encrypts `plaintext` under `key` with the given `aad`, returning a freshly
/// generated nonce and the ciphertext (Poly1305 tag appended).
pub fn seal(
    plaintext: &[u8],
    aad: &[u8],
    key: &DocumentKey,
) -> Result<([u8; NONCE_LEN], Vec<u8>), CryptoError> {
    let nonce = crate::random::bytes::<NONCE_LEN>();
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
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

/// Decrypts `ciphertext` under `key`, verifying the Poly1305 tag against `aad`.
pub fn open(
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
    aad: &[u8],
    key: &DocumentKey,
) -> Result<Vec<u8>, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
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
