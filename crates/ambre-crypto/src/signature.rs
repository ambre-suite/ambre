//! Ed25519 signatures over the encrypted envelope.
//!
//! The signed message is `nonce || ciphertext || aad || commitment_tag`. Signing
//! over the commitment tag prevents a relay from substituting it. The author's
//! public key (in the message's public data) is the verification key.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::error::CryptoError;

/// Length of an Ed25519 signature, in bytes.
pub const SIGNATURE_LEN: usize = 64;

fn message(nonce: &[u8], ciphertext: &[u8], aad: &[u8], commitment_tag: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(nonce.len() + ciphertext.len() + aad.len() + commitment_tag.len());
    msg.extend_from_slice(nonce);
    msg.extend_from_slice(ciphertext);
    msg.extend_from_slice(aad);
    msg.extend_from_slice(commitment_tag);
    msg
}

/// Signs `nonce || ciphertext || aad || commitment_tag` with `signing_key`.
pub fn sign(
    signing_key: &SigningKey,
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
    commitment_tag: &[u8],
) -> [u8; SIGNATURE_LEN] {
    let msg = message(nonce, ciphertext, aad, commitment_tag);
    signing_key.sign(&msg).to_bytes()
}

/// Verifies `signature` over the envelope against the author's 32-byte public key.
pub fn verify(
    author_pub: &[u8; 32],
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
    commitment_tag: &[u8],
    signature: &[u8; SIGNATURE_LEN],
) -> Result<(), CryptoError> {
    let verifying_key =
        VerifyingKey::from_bytes(author_pub).map_err(|_| CryptoError::InvalidSignature)?;
    let sig = Signature::from_bytes(signature);
    let msg = message(nonce, ciphertext, aad, commitment_tag);
    verifying_key
        .verify(&msg, &sig)
        .map_err(|_| CryptoError::InvalidSignature)
}
