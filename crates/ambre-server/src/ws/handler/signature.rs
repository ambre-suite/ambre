// ambre-server/src/ws/handler/signature.rs
// The signature check of a sealed envelope: the one place the socket path reads
// a ciphertext's bytes, so it holds no log line (`scripts/crate_boundaries.py`).

use crate::error::AppError;

/// Appends `len(field) as u32 LE || field` to `msg`.
///
/// MUST match `ambre_crypto::signature::push_framed` byte-for-byte: the client
/// signs with that framing and the server (which does not link `ambre-crypto`)
/// reconstructs it here to verify.
#[expect(
    clippy::expect_used,
    reason = "a field past 4 GiB never reaches a signature, and a wrapped length would forge the transcript"
)]
pub(crate) fn push_framed(msg: &mut Vec<u8>, field: &[u8]) {
    let len = u32::try_from(field.len()).expect("signed field exceeds u32::MAX");
    msg.extend_from_slice(&len.to_le_bytes());
    msg.extend_from_slice(field);
}

/// Verifies an Ed25519 signature over the length-prefixed transcript
/// `len(nonce)||nonce || len(ciphertext)||ciphertext ||
/// len(public_data_bytes)||public_data_bytes || len(commitment_tag)||commitment_tag`
/// (every `len` a `u32` LE). The framing MUST match
/// `ambre_crypto::signature::message`; positional (unframed) concatenation would
/// let a distinct `(ciphertext, aad)` split forge the same transcript.
///
/// This is the ONLY cryptographic operation performed by the Blind Relay.
/// The server never decrypts content -- it only checks that the sender
/// owns the private key matching the public key in the message.
pub(crate) fn verify_update_signature(
    pubkey: &ambre_core::PublicKey,
    update: &ambre_protocol::Update,
) -> Result<(), AppError> {
    let public_data_bytes = postcard::to_allocvec(&update.public_data).map_err(AppError::Decode)?;
    verify_sealed_signature(
        pubkey,
        &update.nonce.0,
        &update.ciphertext.0,
        &public_data_bytes,
        &update.commitment_tag.0,
        &update.signature.0,
    )
}

/// Verifies the Ed25519 signature over an [`ambre_protocol::Ephemeral`] envelope.
///
/// Presence/cursor blobs are relayed and written to the Redis presence hash. The
/// durable `Update` path is signature-checked. Without the same check here the
/// relay would broadcast an unauthenticated ciphertext under an author key. Every
/// recipient's client would then reject it on open, a presence-display DoS. The
/// transcript matches `verify_update_signature` byte-for-byte, with the `aad`
/// being the postcard of the EPHEMERAL public data.
pub(crate) fn verify_ephemeral_signature(
    pubkey: &ambre_core::PublicKey,
    ephemeral: &ambre_protocol::Ephemeral,
) -> Result<(), AppError> {
    let public_data_bytes =
        postcard::to_allocvec(&ephemeral.public_data).map_err(AppError::Decode)?;
    verify_sealed_signature(
        pubkey,
        &ephemeral.nonce.0,
        &ephemeral.ciphertext.0,
        &public_data_bytes,
        &ephemeral.commitment_tag.0,
        &ephemeral.signature.0,
    )
}

/// Shared Ed25519 verification over the length-prefixed sealed-envelope transcript
/// `len(nonce)||nonce || len(ciphertext)||ciphertext || len(public_data)||public_data
/// || len(commitment_tag)||commitment_tag` (every `len` a `u32` LE), matching
/// `ambre_crypto::signature::message`. Factored out so the `Update` and
/// `Ephemeral` paths cannot drift apart.
pub(crate) fn verify_sealed_signature(
    pubkey: &ambre_core::PublicKey,
    nonce: &[u8],
    ciphertext: &[u8],
    public_data_bytes: &[u8],
    commitment_tag: &[u8],
    signature: &[u8; 64],
) -> Result<(), AppError> {
    use ed25519_dalek::{Signature as DalekSig, VerifyingKey};

    let mut signed_data = Vec::new();
    push_framed(&mut signed_data, nonce);
    push_framed(&mut signed_data, ciphertext);
    push_framed(&mut signed_data, public_data_bytes);
    push_framed(&mut signed_data, commitment_tag);

    let vk = VerifyingKey::from_bytes(pubkey.as_array()).map_err(|_| AppError::InvalidSignature)?;
    let sig = DalekSig::from_slice(signature).map_err(|_| AppError::InvalidSignature)?;

    // Strict verification (canonical S, no small-order A) so a relayed signature
    // cannot be malleated into a second valid signature over the same envelope.
    vk.verify_strict(&signed_data, &sig)
        .map_err(|_| AppError::InvalidSignature)?;

    Ok(())
}
