//! X25519 key exchange and document-key wrapping.
//!
//! To share a document, its [`DocumentKey`] is wrapped for each member: an
//! ephemeral-free static ECDH yields a shared secret, a key-encryption key (KEK)
//! is derived from it via domain-separated BLAKE2b, and the document key is
//! sealed under the KEK with XChaCha20-Poly1305. This module provides the
//! primitive only; the share/rotation flow lives in the Sync thread.

use blake2::{Blake2b512, Digest};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::aead;
use crate::error::CryptoError;
use crate::keys::DocumentKey;

/// Domain-separation tag for the KEK derived from an X25519 shared secret.
const KEK_DOMAIN: &[u8] = b"ambre-kek-v1";

/// A document key wrapped for a specific recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedKey {
    /// AEAD nonce used to wrap the key.
    pub nonce: [u8; aead::NONCE_LEN],
    /// Sealed document key (32 bytes + Poly1305 tag).
    pub ciphertext: Vec<u8>,
}

/// Computes the raw X25519 shared secret between `own_secret` and `peer_public`.
pub fn shared_secret(own_secret: &StaticSecret, peer_public: &[u8; 32]) -> [u8; 32] {
    let peer = PublicKey::from(*peer_public);
    own_secret.diffie_hellman(&peer).to_bytes()
}

/// Derives the key-encryption key from a shared secret.
fn derive_kek(shared: &[u8; 32]) -> DocumentKey {
    let mut hasher = Blake2b512::new();
    hasher.update(KEK_DOMAIN);
    hasher.update(shared);
    let digest = hasher.finalize();
    let mut kek = [0u8; 32];
    kek.copy_from_slice(&digest[..32]);
    DocumentKey::from_array(kek)
}

/// Wraps `doc_key` for the holder of `peer_public`, using `own_secret`.
pub fn wrap_document_key(
    own_secret: &StaticSecret,
    peer_public: &[u8; 32],
    doc_key: &DocumentKey,
) -> WrappedKey {
    let shared = shared_secret(own_secret, peer_public);
    let kek = derive_kek(&shared);
    let (nonce, ciphertext) = aead::seal(doc_key.as_bytes(), KEK_DOMAIN, &kek);
    WrappedKey { nonce, ciphertext }
}

/// Unwraps a [`WrappedKey`] sent by the holder of `peer_public`, using `own_secret`.
pub fn unwrap_document_key(
    own_secret: &StaticSecret,
    peer_public: &[u8; 32],
    wrapped: &WrappedKey,
) -> Result<DocumentKey, CryptoError> {
    let shared = shared_secret(own_secret, peer_public);
    let kek = derive_kek(&shared);
    let bytes = aead::open(&wrapped.nonce, &wrapped.ciphertext, KEK_DOMAIN, &kek)?;
    let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| CryptoError::InvalidKeyLength)?;
    Ok(DocumentKey::from_array(arr))
}
