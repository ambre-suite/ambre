//! XChaCha20-Poly1305 authenticated encryption with associated data.
//!
//! The 192-bit nonce is generated per message from the CSPRNG; collision over
//! 2^96 messages is cryptographically negligible, so random nonces are safe.
//! The associated data (`aad`) binds the ciphertext to the message's public
//! metadata without encrypting it.

use ambre_core::EntryDigest;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

use crate::error::CryptoError;
use crate::keys::{DocumentKey, FileKey, StoreKey};

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

/// The first byte of a sealed row of the local store: the layout below.
pub const ROW_VERSION: u8 = 1;

/// Seals one row of the local store under the user's [`StoreKey`]: the
/// version byte, a fresh nonce, then the ciphertext and its tag.
///
/// `context` names the row's place in the store, a document or a draft. It is
/// the associated data, so a row moved under another name stops opening.
///
/// # Errors
///
/// [`CryptoError::EncryptionFailed`] when `plaintext` exceeds the size limit of
/// XChaCha20-Poly1305.
pub fn seal_row(plaintext: &[u8], context: &[u8], key: &StoreKey) -> Result<Vec<u8>, CryptoError> {
    let (nonce, ciphertext) = seal_under(plaintext, context, key.as_bytes())?;
    let mut row = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
    row.push(ROW_VERSION);
    row.extend_from_slice(&nonce);
    row.extend_from_slice(&ciphertext);
    Ok(row)
}

/// Opens a row [`seal_row`] sealed under the same `context`.
///
/// # Errors
///
/// [`CryptoError::AeadFailed`] for a row of another layout, a row too short to
/// hold a nonce, a wrong key, another `context` or a modified row. The caller
/// treats them alike: the row is not one this session can read.
pub fn open_row(row: &[u8], context: &[u8], key: &StoreKey) -> Result<Vec<u8>, CryptoError> {
    let Some((&ROW_VERSION, rest)) = row.split_first() else {
        return Err(CryptoError::AeadFailed);
    };
    let (Some(nonce), Some(ciphertext)) = (rest.get(..NONCE_LEN), rest.get(NONCE_LEN..)) else {
        return Err(CryptoError::AeadFailed);
    };
    let nonce: &[u8; NONCE_LEN] = nonce.try_into().map_err(|_| CryptoError::AeadFailed)?;
    open_under(nonce, ciphertext, context, key.as_bytes())
}

/// Seals one document key under another, as a catalogue holds the key of a
/// public channel under the key of its space.
///
/// A pair of its own rather than [`seal`] over the key bytes. Those bytes
/// stay private to this crate, so no caller holds a key as a plain buffer.
///
/// # Errors
///
/// [`CryptoError::EncryptionFailed`], which 32 bytes never reach.
pub fn seal_document_key(
    inner: &DocumentKey,
    aad: &[u8],
    outer: &DocumentKey,
) -> Result<([u8; NONCE_LEN], Vec<u8>), CryptoError> {
    seal_anchored_key(inner, aad, outer, None)
}

/// Seals `inner` as [`seal_document_key`] does, with the anchor of the
/// document's membership log sealed beside it, as a wrap carries it
/// (`SYNC-104`). A member who takes a channel key from its catalogue learns
/// where the channel's log starts from the member who sealed it. The relay
/// alone never tells it.
///
/// # Errors
///
/// [`CryptoError::EncryptionFailed`], which 64 bytes never reach.
pub fn seal_anchored_key(
    inner: &DocumentKey,
    aad: &[u8],
    outer: &DocumentKey,
    anchor: Option<&EntryDigest>,
) -> Result<([u8; NONCE_LEN], Vec<u8>), CryptoError> {
    let mut plain = zeroize::Zeroizing::new(inner.as_bytes().to_vec());
    if let Some(anchor) = anchor {
        plain.extend_from_slice(&anchor.0);
    }
    seal_under(&plain, aad, outer.as_bytes())
}

/// Opens a document key sealed by [`seal_document_key`].
///
/// # Errors
///
/// [`CryptoError::AeadFailed`] for a wrong key, a wrong `aad` or a modified
/// ciphertext. [`CryptoError::InvalidKeyLength`] for a plaintext that is
/// neither a key nor a key and an anchor.
pub fn open_document_key(
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
    aad: &[u8],
    outer: &DocumentKey,
) -> Result<DocumentKey, CryptoError> {
    open_anchored_key(nonce, ciphertext, aad, outer).map(|(key, _)| key)
}

/// Opens what [`seal_anchored_key`] sealed: the key, and the anchor of the
/// membership log when the sealer put one beside it.
///
/// # Errors
///
/// Every error of [`open_document_key`].
pub fn open_anchored_key(
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
    aad: &[u8],
    outer: &DocumentKey,
) -> Result<(DocumentKey, Option<EntryDigest>), CryptoError> {
    let plain = zeroize::Zeroizing::new(open_under(nonce, ciphertext, aad, outer.as_bytes())?);
    let (key, anchor) = match plain.len() {
        32 => (plain.get(..32), None),
        64 => (plain.get(..32), plain.get(32..)),
        _ => (None, None),
    };
    let key: [u8; 32] = key
        .and_then(|k| k.try_into().ok())
        .ok_or(CryptoError::InvalidKeyLength)?;
    let anchor = anchor
        .and_then(|a| <[u8; 32]>::try_from(a).ok())
        .map(EntryDigest);
    Ok((DocumentKey::from_array(key), anchor))
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
