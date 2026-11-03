//! XChaCha20-Poly1305 authenticated encryption with associated data.
//!
//! The 192-bit nonce is generated per message from the CSPRNG; collision over
//! 2^96 messages is cryptographically negligible, so random nonces are safe.
//! The associated data (`aad`) binds the ciphertext to the message's public
//! metadata without encrypting it.

use ambre_core::{
    Ciphertext, Confidential, EntryDigest, FixedBytes, Nonce, Plaintext, SealedRowBytes, Secret,
};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

use crate::error::CryptoError;
use crate::keys::{DocumentKey, FileKey, StoreKey};

/// Length of the XChaCha20 nonce, in bytes: the array length of this crate's
/// own layouts. A crate that frames a nonce reads [`FixedBytes::<Nonce>::SIZE`](FixedBytes::SIZE).
pub(crate) const NONCE_LEN: usize = 24;

const _: () = assert!(FixedBytes::<Nonce>::SIZE.get() == NONCE_LEN as u64);

/// Encrypts `plaintext` under `key` with the given `aad`, returning a freshly
/// generated nonce and the ciphertext (Poly1305 tag appended).
///
/// # Errors
///
/// [`CryptoError::EncryptionFailed`] when `plaintext` exceeds the size limit of
/// XChaCha20-Poly1305. The cipher raises nothing else.
pub fn seal(
    plaintext: &[u8],
    aad: &ambre_core::Aad,
    key: &DocumentKey,
) -> Result<(FixedBytes<Nonce>, ambre_core::Payload<Ciphertext>), CryptoError> {
    seal_under(plaintext, aad, key.as_bytes())
}

/// Decrypts `ciphertext` under `key`, verifying the Poly1305 tag against `aad`.
///
/// The plaintext comes back as [`Confidential<Plaintext>`], so it leaves this crate typed
/// as a secret. A caller reads it in place through
/// `Confidential::<Plaintext>::with_plaintext`, or moves it into a role that keeps it in the
/// clear through `Confidential::<Plaintext>::into_plaintext` (`ARCH-047`).
///
/// # Errors
///
/// [`CryptoError::AeadFailed`] when the tag does not verify: a wrong key,
/// wrong `aad`, or a modified ciphertext. Poly1305 cannot tell a caller which.
pub fn open(
    nonce: &FixedBytes<Nonce>,
    ciphertext: &ambre_core::Payload<Ciphertext>,
    aad: &ambre_core::Aad,
    key: &DocumentKey,
) -> Result<Confidential<Plaintext>, CryptoError> {
    open_under(nonce, ciphertext.as_bytes(), aad, key.as_bytes())
        .map(Confidential::<Plaintext>::new)
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
    plaintext: &Confidential<Plaintext>,
    aad: &ambre_core::Aad,
    key: &FileKey,
) -> Result<(FixedBytes<Nonce>, ambre_core::Payload<Ciphertext>), CryptoError> {
    seal_under(plaintext.secret().expose(), aad, key.as_bytes())
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
    nonce: &FixedBytes<Nonce>,
    ciphertext: &ambre_core::Payload<Ciphertext>,
    aad: &ambre_core::Aad,
    key: &FileKey,
) -> Result<Confidential<Plaintext>, CryptoError> {
    open_under(nonce, ciphertext.as_bytes(), aad, key.as_bytes())
        .map(Confidential::<Plaintext>::new)
}

ambre_core::define_layout_version! {
    /// The first byte of a sealed row of the local store: its layout.
    pub enum RowLayout {
        /// A 24-byte nonce, then the ciphertext and its tag.
        V1 = 1,
    }
    current = V1;
}

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
pub fn seal_row(
    plaintext: &Confidential<Plaintext>,
    context: &ambre_core::Aad,
    key: &StoreKey,
) -> Result<ambre_core::Payload<SealedRowBytes>, CryptoError> {
    let (nonce, ciphertext) = seal_under(plaintext.secret().expose(), context, key.as_bytes())?;
    let mut row = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len().as_len());
    row.push(RowLayout::CURRENT.byte());
    row.extend_from_slice(nonce.as_slice());
    row.extend_from_slice(ciphertext.as_bytes());
    Ok(ambre_core::Payload::new(row))
}

/// Opens a row [`seal_row`] sealed under the same `context`.
///
/// # Errors
///
/// [`CryptoError::AeadFailed`] for a row of another layout, a row too short to
/// hold a nonce, a wrong key, another `context` or a modified row. The caller
/// treats them alike: the row is not one this session can read.
pub fn open_row(
    row: &ambre_core::Payload<SealedRowBytes>,
    context: &ambre_core::Aad,
    key: &StoreKey,
) -> Result<Confidential<Plaintext>, CryptoError> {
    let Some((&first, rest)) = row.as_bytes().split_first() else {
        return Err(CryptoError::AeadFailed);
    };
    let Some(RowLayout::V1) = RowLayout::from_byte(first) else {
        return Err(CryptoError::AeadFailed);
    };
    let (Some(nonce), Some(ciphertext)) = (rest.get(..NONCE_LEN), rest.get(NONCE_LEN..)) else {
        return Err(CryptoError::AeadFailed);
    };
    let nonce =
        FixedBytes::<Nonce>::from_array(nonce.try_into().map_err(|_| CryptoError::AeadFailed)?);
    open_under(&nonce, ciphertext, context, key.as_bytes()).map(Confidential::<Plaintext>::new)
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
    aad: &ambre_core::Aad,
    outer: &DocumentKey,
) -> Result<(FixedBytes<Nonce>, ambre_core::Payload<Ciphertext>), CryptoError> {
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
    aad: &ambre_core::Aad,
    outer: &DocumentKey,
    anchor: Option<&FixedBytes<EntryDigest>>,
) -> Result<(FixedBytes<Nonce>, ambre_core::Payload<Ciphertext>), CryptoError> {
    let plain = Secret::filled(|plain: &mut Vec<u8>| {
        plain.reserve_exact(64);
        plain.extend_from_slice(inner.as_bytes());
        if let Some(anchor) = anchor {
            plain.extend_from_slice(anchor.as_slice());
        }
    });
    seal_under(plain.expose(), aad, outer.as_bytes())
}

/// Opens a document key sealed by [`seal_document_key`].
///
/// # Errors
///
/// [`CryptoError::AeadFailed`] for a wrong key, a wrong `aad` or a modified
/// ciphertext. [`CryptoError::InvalidKeyLength`] for a plaintext that is
/// neither a key nor a key and an anchor.
pub fn open_document_key(
    nonce: &FixedBytes<Nonce>,
    ciphertext: &ambre_core::Payload<Ciphertext>,
    aad: &ambre_core::Aad,
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
    nonce: &FixedBytes<Nonce>,
    ciphertext: &ambre_core::Payload<Ciphertext>,
    aad: &ambre_core::Aad,
    outer: &DocumentKey,
) -> Result<(DocumentKey, Option<FixedBytes<EntryDigest>>), CryptoError> {
    let plain = Secret::new(open_under(
        nonce,
        ciphertext.as_bytes(),
        aad,
        outer.as_bytes(),
    )?);
    crate::keys::split_anchored(plain.expose())
}

/// The cipher itself, over raw key bytes. Private: every caller reaches it
/// through one of the typed pairs above, so the key type is never erased at a
/// call site.
fn seal_under(
    plaintext: &[u8],
    aad: &ambre_core::Aad,
    key: &[u8; 32],
) -> Result<(FixedBytes<Nonce>, ambre_core::Payload<Ciphertext>), CryptoError> {
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
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| CryptoError::EncryptionFailed)?;
    Ok((
        FixedBytes::from_array(nonce),
        ambre_core::Payload::<Ciphertext>::new(ciphertext),
    ))
}

/// The inverse, over raw key bytes. Private for the same reason as
/// [`seal_under`].
fn open_under(
    nonce: &FixedBytes<Nonce>,
    ciphertext: &[u8],
    aad: &ambre_core::Aad,
    key: &[u8; 32],
) -> Result<Vec<u8>, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(
            XNonce::from_slice(nonce.as_slice()),
            Payload {
                msg: ciphertext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| CryptoError::AeadFailed)
}
