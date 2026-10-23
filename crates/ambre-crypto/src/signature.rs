//! Ed25519 signatures over the encrypted envelope.
//!
//! The signed transcript length-prefixes each field: `len(nonce) || nonce ||
//! len(ciphertext) || ciphertext || len(aad) || aad || len(commitment_tag) ||
//! commitment_tag`, where every `len` is a `u32` little-endian byte count. The
//! prefixes make every field boundary explicit. Two distinct `(ciphertext, aad)`
//! splits can never produce the same transcript, where positional concatenation
//! would let `ciphertext || aad` collide across splits. Signing over the
//! commitment tag prevents a relay from substituting it. The author's public key
//! (in the message's public data) is the verification key.
//!
//! NOTE: `ambre-server` re-implements this exact framing in
//! `ws/handler/message.rs::verify_update_signature` (it must not link
//! `ambre-crypto`); the two layouts MUST stay byte-identical.

use ambre_core::{Ciphertext, CommitmentTag, Nonce, PublicKey, Signature};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};

use crate::error::CryptoError;

/// Length of an Ed25519 signature, in bytes.
pub const SIGNATURE_LEN: usize = 64;

/// Appends `len(field) as u32 LE || field` to `msg`.
#[expect(
    clippy::expect_used,
    reason = "a field past 4 GiB never reaches a signature, and a wrapped length would forge the transcript"
)]
fn push_framed(msg: &mut Vec<u8>, field: &[u8]) {
    // A field longer than u32::MAX is not representable on the wire and never
    // occurs (nonce/tag are fixed-small, ciphertext is bounded by transport
    // limits). Truncating the length would break the transcript, so assert it.
    let len = u32::try_from(field.len()).expect("signed field exceeds u32::MAX");
    msg.extend_from_slice(&len.to_le_bytes());
    msg.extend_from_slice(field);
}

fn message(
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &ambre_core::Aad,
    commitment_tag: &[u8],
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(
        16 + nonce.len() + ciphertext.len() + aad.as_bytes().len() + commitment_tag.len(),
    );
    push_framed(&mut msg, nonce);
    push_framed(&mut msg, ciphertext);
    push_framed(&mut msg, aad.as_bytes());
    push_framed(&mut msg, commitment_tag);
    msg
}

/// Signs `nonce || ciphertext || aad || commitment_tag` with `signing_key`.
pub fn sign(
    signing_key: &SigningKey,
    nonce: &Nonce,
    ciphertext: &Ciphertext,
    aad: &ambre_core::Aad,
    commitment_tag: &CommitmentTag,
) -> Signature {
    let msg = message(&nonce.0, &ciphertext.0, aad, &commitment_tag.0);
    Signature(signing_key.sign(&msg).to_bytes())
}

/// Verifies `signature` over the envelope against the author's 32-byte public key.
///
/// # Errors
///
/// [`CryptoError::InvalidSignature`] when `author_pub` is not a valid Ed25519
/// point, or when strict verification refuses the signature. Ambre rejects a
/// malleable signature, so a re-encoded one fails here too.
pub fn verify(
    author_pub: &PublicKey,
    nonce: &Nonce,
    ciphertext: &Ciphertext,
    aad: &ambre_core::Aad,
    commitment_tag: &CommitmentTag,
    signature: &Signature,
) -> Result<(), CryptoError> {
    let verifying_key = VerifyingKey::from_bytes(author_pub.as_array())
        .map_err(|_| CryptoError::InvalidSignature)?;
    let sig = ed25519_dalek::Signature::from_bytes(&signature.0);
    let msg = message(&nonce.0, &ciphertext.0, aad, &commitment_tag.0);
    // Strict verification rejects non-canonical `S` and small-order `A`, so a
    // valid `(R, S)` cannot be transformed into a distinct valid signature over
    // the same message. Ambre treats "signature identifies the author" as
    // load-bearing on the blind relay, so signature malleability is disallowed.
    verifying_key
        .verify_strict(&msg, &sig)
        .map_err(|_| CryptoError::InvalidSignature)
}

/// Signs `message` whole with `signing_key`. The caller's message opens with
/// its own domain, so the signature serves no other use.
pub fn sign_message(signing_key: &SigningKey, message: &[u8]) -> Signature {
    Signature(signing_key.sign(message).to_bytes())
}

/// Verifies a signature [`sign_message`] made, strictly, as [`verify`] does.
///
/// # Errors
///
/// [`CryptoError::InvalidSignature`] for a key that is not a point, or a
/// signature that does not verify. A signature of the wrong length is not a
/// [`Signature`], so it never reaches here.
pub fn verify_message(
    author_pub: &PublicKey,
    message: &[u8],
    signature: &Signature,
) -> Result<(), CryptoError> {
    let verifying_key = VerifyingKey::from_bytes(author_pub.as_array())
        .map_err(|_| CryptoError::InvalidSignature)?;
    let sig = ed25519_dalek::Signature::from_bytes(&signature.0);
    verifying_key
        .verify_strict(message, &sig)
        .map_err(|_| CryptoError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    /// A whole-message signature checks under its author's key alone, and
    /// over its own message alone.
    #[test]
    fn a_message_signature_checks_under_its_author_alone() {
        let sk = test_key();
        let pk = ambre_core::PublicKey::from_array(sk.verifying_key().to_bytes());
        let sig = sign_message(&sk, b"entry");
        assert!(verify_message(&pk, b"entry", &sig).is_ok());
        assert!(verify_message(&pk, b"entrY", &sig).is_err());
        let other = PublicKey::from_array(
            SigningKey::from_bytes(&[8u8; 32])
                .verifying_key()
                .to_bytes(),
        );
        assert!(verify_message(&other, b"entry", &sig).is_err());
    }

    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn nonce() -> Nonce {
        Nonce([1; 24])
    }

    fn ct(bytes: &[u8]) -> Ciphertext {
        Ciphertext(bytes.to_vec())
    }

    fn tag() -> CommitmentTag {
        CommitmentTag([2; 32])
    }

    #[test]
    fn sign_then_verify_roundtrips() {
        let sk = test_key();
        let pk = ambre_core::PublicKey::from_array(sk.verifying_key().to_bytes());
        let sig = sign(
            &sk,
            &nonce(),
            &ct(b"cipher"),
            &ambre_core::Aad::of(b"aad"),
            &tag(),
        );
        assert!(
            verify(
                &pk,
                &nonce(),
                &ct(b"cipher"),
                &ambre_core::Aad::of(b"aad"),
                &tag(),
                &sig
            )
            .is_ok()
        );
    }

    #[test]
    fn tampered_field_rejected() {
        let sk = test_key();
        let pk = ambre_core::PublicKey::from_array(sk.verifying_key().to_bytes());
        let sig = sign(
            &sk,
            &nonce(),
            &ct(b"cipher"),
            &ambre_core::Aad::of(b"aad"),
            &tag(),
        );
        assert!(
            verify(
                &pk,
                &nonce(),
                &ct(b"CIPHER"),
                &ambre_core::Aad::of(b"aad"),
                &tag(),
                &sig
            )
            .is_err()
        );
    }

    /// The order of the Ed25519 group, little-endian.
    const GROUP_ORDER: [u8; 32] = [
        0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde,
        0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
    ];

    /// The same signature with its scalar raised by the group order: a second
    /// encoding that a lax verifier accepts over the same message.
    fn malleated(sig: &Signature) -> Signature {
        let mut out = *sig;
        let mut carry = 0u16;
        for (s, l) in out.0[32..].iter_mut().zip(GROUP_ORDER) {
            let sum = u16::from(*s) + u16::from(l) + carry;
            *s = sum as u8;
            carry = sum >> 8;
        }
        out
    }

    /// Holds `SYNC-072` on the client: the non-canonical second form of a
    /// valid signature is refused. So is a small-order key, under which one
    /// forged signature would verify over any message.
    #[test]
    fn only_the_canonical_signature_is_accepted() {
        let sk = test_key();
        let pk = ambre_core::PublicKey::from_array(sk.verifying_key().to_bytes());
        let sig = sign(&sk, &nonce(), &ct(b"c"), &ambre_core::Aad::of(b"a"), &tag());
        assert!(
            verify(
                &pk,
                &nonce(),
                &ct(b"c"),
                &ambre_core::Aad::of(b"a"),
                &tag(),
                &sig
            )
            .is_ok()
        );
        assert!(
            verify(
                &pk,
                &nonce(),
                &ct(b"c"),
                &ambre_core::Aad::of(b"a"),
                &tag(),
                &malleated(&sig)
            )
            .is_err()
        );

        // The identity point, and the signature (R = identity, S = 0).
        let mut identity = [0u8; 32];
        identity[0] = 1;
        let mut forged = Signature([0u8; SIGNATURE_LEN]);
        forged.0[..32].copy_from_slice(&identity);
        let identity_key = PublicKey::from_array(identity);
        assert!(
            verify(
                &identity_key,
                &nonce(),
                &ct(b"c"),
                &ambre_core::Aad::of(b"a"),
                &tag(),
                &forged
            )
            .is_err()
        );
    }

    /// The length prefixes make the ciphertext|aad boundary unambiguous. Two
    /// distinct splits that share the same concatenation must not verify under a
    /// signature made for the other split. Without framing, `("ab","c")` and
    /// `("a","bc")` would produce an identical transcript.
    #[test]
    fn ciphertext_aad_boundary_is_unambiguous() {
        let sk = test_key();
        let pk = ambre_core::PublicKey::from_array(sk.verifying_key().to_bytes());
        let sig = sign(
            &sk,
            &nonce(),
            &ct(b"ab"),
            &ambre_core::Aad::of(b"c"),
            &tag(),
        );
        // Same concatenation, different split -> must be rejected.
        assert!(
            verify(
                &pk,
                &nonce(),
                &ct(b"a"),
                &ambre_core::Aad::of(b"bc"),
                &tag(),
                &sig
            )
            .is_err()
        );
        // Exact split -> accepted.
        assert!(
            verify(
                &pk,
                &nonce(),
                &ct(b"ab"),
                &ambre_core::Aad::of(b"c"),
                &tag(),
                &sig
            )
            .is_ok()
        );
    }
}
