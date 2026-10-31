//! X25519 key exchange and document-key wrapping.
//!
//! To share a document, its [`DocumentKey`] is wrapped for each member. An
//! ephemeral-free static ECDH yields a shared secret. A key-encryption key (KEK)
//! is derived from it via HKDF-BLAKE2b, bound to both parties' public keys. The
//! document key is then sealed under the KEK with XChaCha20-Poly1305. This module
//! provides the primitive only; the share/rotation flow lives in the Sync thread.

use ambre_core::{Ciphertext, EntryDigest, ExchangePublicKey, FixedBytes, Nonce, Secret};
use blake2::Blake2b512;
use hkdf::SimpleHkdf;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::aead;
use crate::error::CryptoError;
use crate::keys::DocumentKey;

/// Domain-separation tag for the KEK derived from an X25519 shared secret. `v2`
/// binds the transcript to both public keys (see [`derive_kek`]); it is not
/// wire-compatible with the `v1` bare-BLAKE2b derivation.
const KEK_DOMAIN: &[u8] = b"ambre-kek-v2";

/// A document key wrapped for a specific recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedKey {
    /// AEAD nonce used to wrap the key.
    pub nonce: FixedBytes<Nonce>,
    /// Sealed document key (32 bytes + Poly1305 tag).
    pub ciphertext: Ciphertext,
}

/// The raw X25519 output between two parties, the input of the KEK.
///
/// Wiped on drop and never cloned, like the keys of `keys.rs`. Its `Debug`
/// form prints none of the bytes.
#[derive(Debug)]
pub struct SharedSecret(Secret<[u8; 32]>);

impl SharedSecret {
    /// Returns the raw secret bytes, for the KEK derivation and the tests.
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        self.0.expose()
    }
}

/// Computes the raw X25519 shared secret between `own_secret` and `peer_public`.
///
/// Rejects a non-contributory exchange: a low-order `peer_public` drives the
/// shared secret to an all-zero (predictable) value, which we refuse rather than
/// derive a KEK from.
///
/// # Errors
///
/// [`CryptoError::WeakPublicKey`] when `peer_public` is a low-order point, so
/// the exchange is non-contributory.
pub fn shared_secret(
    own_secret: &StaticSecret,
    peer_public: &FixedBytes<ExchangePublicKey>,
) -> Result<SharedSecret, CryptoError> {
    let peer = PublicKey::from(*peer_public.as_array());
    // `x25519_dalek::SharedSecret` wipes itself on drop; its bytes are copied
    // in place into the wiping wrapper.
    let shared = own_secret.diffie_hellman(&peer);
    if !shared.was_contributory() {
        return Err(CryptoError::WeakPublicKey);
    }
    Ok(SharedSecret(Secret::filled(|out: &mut [u8; 32]| {
        out.copy_from_slice(shared.as_bytes());
    })))
}

/// Derives the key-encryption key via HKDF-BLAKE2b, binding the transcript to
/// both parties' public keys (`info = KEK_DOMAIN || sender_pk || recipient_pk`).
/// `sender_pk` is the wrapper's public key, `recipient_pk` the holder's; both
/// sides must agree on the ordering to reconstruct the same KEK.
#[expect(
    clippy::expect_used,
    reason = "HKDF expands up to 255 digests, and 32 bytes is far below that"
)]
fn derive_kek(
    shared: &SharedSecret,
    sender_pk: &FixedBytes<ExchangePublicKey>,
    recipient_pk: &FixedBytes<ExchangePublicKey>,
) -> DocumentKey {
    // `SimpleHkdf` (SimpleHmac-based) is required: BLAKE2b's variable-output core
    // is not the "eager" hash the default `Hkdf<H>` (raw `Hmac`) demands.
    let hk = SimpleHkdf::<Blake2b512>::new(None, shared.as_bytes());
    let mut info = Vec::with_capacity(KEK_DOMAIN.len() + 64);
    info.extend_from_slice(KEK_DOMAIN);
    info.extend_from_slice(sender_pk.as_slice());
    info.extend_from_slice(recipient_pk.as_slice());
    let kek = Secret::filled(|kek: &mut [u8; 32]| {
        hk.expand(&info, kek)
            .expect("HKDF-BLAKE2b expand of 32 bytes is within the output limit");
    });
    DocumentKey::from_secret(kek)
}

/// Wraps `doc_key` for the holder of `peer_public`, using `own_secret`. Here the
/// caller is the sender, `peer_public` the recipient.
///
/// # Errors
///
/// [`CryptoError::WeakPublicKey`] from [`shared_secret`], and
/// [`CryptoError::EncryptionFailed`] from the AEAD.
pub fn wrap_document_key(
    own_secret: &StaticSecret,
    peer_public: &FixedBytes<ExchangePublicKey>,
    doc_key: &DocumentKey,
) -> Result<WrappedKey, CryptoError> {
    wrap_anchored(own_secret, peer_public, doc_key, None)
}

/// Wraps `doc_key` as [`wrap_document_key`] does, with the digest of the
/// document's first membership entry sealed beside it (`SYNC-104`). So the
/// recipient learns where the log starts from the member who wrapped, never
/// from the relay alone.
///
/// # Errors
///
/// Every error of [`wrap_document_key`].
pub fn wrap_anchored(
    own_secret: &StaticSecret,
    peer_public: &FixedBytes<ExchangePublicKey>,
    doc_key: &DocumentKey,
    anchor: Option<&FixedBytes<EntryDigest>>,
) -> Result<WrappedKey, CryptoError> {
    let shared = shared_secret(own_secret, peer_public)?;
    let own_pub =
        FixedBytes::<ExchangePublicKey>::from_array(PublicKey::from(own_secret).to_bytes());
    let kek = derive_kek(&shared, &own_pub, peer_public);
    let plaintext = Secret::filled(|plain: &mut Vec<u8>| {
        plain.reserve_exact(64);
        plain.extend_from_slice(doc_key.as_bytes());
        if let Some(anchor) = anchor {
            plain.extend_from_slice(anchor.as_slice());
        }
    });
    let (nonce, ciphertext) =
        aead::seal(plaintext.expose(), &ambre_core::Aad::of(KEK_DOMAIN), &kek)?;
    Ok(WrappedKey { nonce, ciphertext })
}

/// Unwraps a [`WrappedKey`] sent by the holder of `peer_public`, using
/// `own_secret`. Here `peer_public` is the sender and the caller the recipient,
/// so the identity binding is fed to [`derive_kek`] in that order.
///
/// # Errors
///
/// * [`CryptoError::WeakPublicKey`] from [`shared_secret`].
/// * [`CryptoError::AeadFailed`] when the wrapped key does not verify under the
///   derived key-encryption key.
/// * [`CryptoError::InvalidKeyLength`] when the plaintext inside is neither a
///   key nor a key and an anchor.
pub fn unwrap_document_key(
    own_secret: &StaticSecret,
    peer_public: &FixedBytes<ExchangePublicKey>,
    wrapped: &WrappedKey,
) -> Result<DocumentKey, CryptoError> {
    unwrap_anchored(own_secret, peer_public, wrapped).map(|(key, _)| key)
}

/// Unwraps what [`wrap_anchored`] sealed: the key, and the anchor of the
/// membership log when the wrapper sealed one.
///
/// # Errors
///
/// Every error of [`unwrap_document_key`].
pub fn unwrap_anchored(
    own_secret: &StaticSecret,
    peer_public: &FixedBytes<ExchangePublicKey>,
    wrapped: &WrappedKey,
) -> Result<(DocumentKey, Option<FixedBytes<EntryDigest>>), CryptoError> {
    let shared = shared_secret(own_secret, peer_public)?;
    let own_pub =
        FixedBytes::<ExchangePublicKey>::from_array(PublicKey::from(own_secret).to_bytes());
    let kek = derive_kek(&shared, peer_public, &own_pub);
    let bytes = aead::open(
        &wrapped.nonce,
        &wrapped.ciphertext,
        &ambre_core::Aad::of(KEK_DOMAIN),
        &kek,
    )?;
    crate::keys::split_anchored(bytes.secret().expose())
}
