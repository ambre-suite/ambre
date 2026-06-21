//! Host tests for the crypto primitives and envelope.

use proptest::prelude::*;

use crate::keys::{DocumentKey, MasterSeed};
use crate::{CryptoError, ecdh, envelope, hash, passphrase};

fn fixed_doc_key() -> DocumentKey {
    DocumentKey::from_array([0x11; 32])
}

fn fixed_seed() -> MasterSeed {
    MasterSeed::from_array([0x22; 32])
}

// ── Envelope ────────────────────────────────────────────────────────────────

#[test]
fn roundtrip_envelope() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let author = signing.verifying_key().to_bytes();
    let aad = b"public-data";
    let plaintext = b"hello, encrypted world";

    let sealed = envelope::seal(plaintext, aad, &doc_key, &signing);
    let opened = envelope::open(&sealed, aad, &doc_key, &author).unwrap();
    assert_eq!(opened, plaintext);
}

#[test]
fn tampered_ciphertext_fails() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let author = signing.verifying_key().to_bytes();
    let aad = b"aad";

    let mut sealed = envelope::seal(b"payload", aad, &doc_key, &signing);
    sealed.ciphertext[0] ^= 0xFF;
    // Signature covers the ciphertext, so this trips the signature check first.
    assert_eq!(
        envelope::open(&sealed, aad, &doc_key, &author),
        Err(CryptoError::InvalidSignature)
    );
}

#[test]
fn commitment_tag_wrong_aad_rejected() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let author = signing.verifying_key().to_bytes();

    let sealed = envelope::seal(b"payload", b"document-A", &doc_key, &signing);
    // Re-sign over the altered aad so the signature passes but the HMAC commitment
    // (and the AEAD aad) no longer match -- models inter-document substitution.
    let forged_aad = b"document-B";
    let signature = crate::signature::sign(
        &signing,
        &sealed.nonce,
        &sealed.ciphertext,
        forged_aad,
        &sealed.commitment_tag,
    );
    let forged = envelope::Sealed {
        signature,
        ..sealed
    };
    assert_eq!(
        envelope::open(&forged, forged_aad, &doc_key, &author),
        Err(CryptoError::CommitmentMismatch)
    );
}

#[test]
fn invalid_signature_rejected() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let aad = b"aad";

    let sealed = envelope::seal(b"payload", aad, &doc_key, &signing);
    let wrong_author = MasterSeed::from_array([0x99; 32])
        .derive_signing_key()
        .verifying_key()
        .to_bytes();
    assert_eq!(
        envelope::open(&sealed, aad, &doc_key, &wrong_author),
        Err(CryptoError::InvalidSignature)
    );
}

#[test]
fn wrong_document_key_rejected() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let author = signing.verifying_key().to_bytes();
    let aad = b"aad";

    let sealed = envelope::seal(b"payload", aad, &doc_key, &signing);
    let other_key = DocumentKey::from_array([0x44; 32]);
    assert_eq!(
        envelope::open(&sealed, aad, &other_key, &author),
        Err(CryptoError::CommitmentMismatch)
    );
}

// ── Key derivation ────────────────────────────────────────────────────────────

#[test]
fn derive_keys_deterministic() {
    let a = MasterSeed::from_array([0x07; 32]);
    let b = MasterSeed::from_array([0x07; 32]);
    assert_eq!(
        a.derive_signing_key().to_bytes(),
        b.derive_signing_key().to_bytes()
    );
    assert_eq!(
        a.derive_exchange_key().to_bytes(),
        b.derive_exchange_key().to_bytes()
    );

    let c = MasterSeed::from_array([0x08; 32]);
    assert_ne!(
        a.derive_signing_key().to_bytes(),
        c.derive_signing_key().to_bytes()
    );
    // Signing and exchange domains must be independent.
    assert_ne!(
        a.derive_signing_key().to_bytes(),
        a.derive_exchange_key().to_bytes()
    );
}

// ── ECDH ──────────────────────────────────────────────────────────────────────

#[test]
fn ecdh_symmetric() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let alice_pub = x25519_dalek::PublicKey::from(&alice).to_bytes();
    let bob_pub = x25519_dalek::PublicKey::from(&bob).to_bytes();

    assert_eq!(
        ecdh::shared_secret(&alice, &bob_pub),
        ecdh::shared_secret(&bob, &alice_pub)
    );
}

#[test]
fn wrap_unwrap_roundtrip() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let alice_pub = x25519_dalek::PublicKey::from(&alice).to_bytes();
    let bob_pub = x25519_dalek::PublicKey::from(&bob).to_bytes();

    let doc_key = DocumentKey::from_array([0x55; 32]);
    let wrapped = ecdh::wrap_document_key(&alice, &bob_pub, &doc_key);
    let unwrapped = ecdh::unwrap_document_key(&bob, &alice_pub, &wrapped).unwrap();
    assert_eq!(unwrapped.as_bytes(), doc_key.as_bytes());
}

// ── Argon2id ──────────────────────────────────────────────────────────────────

#[test]
fn argon2_deterministic() {
    let salt = [0xAB; 16];
    let k1 = passphrase::derive_user_key(b"correct horse", &salt).unwrap();
    let k2 = passphrase::derive_user_key(b"correct horse", &salt).unwrap();
    assert_eq!(k1, k2);

    let other_salt = [0xCD; 16];
    let k3 = passphrase::derive_user_key(b"correct horse", &other_salt).unwrap();
    assert_ne!(k1, k3);

    let k4 = passphrase::derive_user_key(b"wrong passphrase", &salt).unwrap();
    assert_ne!(k1, k4);
}

// ── BLAKE2b chain ─────────────────────────────────────────────────────────────

#[test]
fn blake2_chain_hash() {
    let h1 = hash::chain_hash(b"parent", b"snapshot");
    assert_eq!(h1, hash::chain_hash(b"parent", b"snapshot"));
    // Order-sensitive: the split point matters.
    assert_ne!(h1, hash::chain_hash(b"paren", b"tsnapshot"));
}

// ── Property tests ────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn prop_roundtrip_encrypt_decrypt(data in any::<Vec<u8>>(), key in any::<[u8; 32]>()) {
        let doc_key = DocumentKey::from_array(key);
        let signing = fixed_seed().derive_signing_key();
        let author = signing.verifying_key().to_bytes();
        let aad = b"aad";
        let sealed = envelope::seal(&data, aad, &doc_key, &signing);
        let opened = envelope::open(&sealed, aad, &doc_key, &author).unwrap();
        prop_assert_eq!(opened, data);
    }

    #[test]
    fn prop_single_byte_mutation_fails(
        data in proptest::collection::vec(any::<u8>(), 1..256),
        idx in 0usize..3,
        bit in 0u8..8,
    ) {
        let doc_key = fixed_doc_key();
        let signing = fixed_seed().derive_signing_key();
        let author = signing.verifying_key().to_bytes();
        let aad = b"aad";
        let mut sealed = envelope::seal(&data, aad, &doc_key, &signing);
        let mask = 1u8 << bit;
        match idx {
            0 => sealed.ciphertext[0] ^= mask,
            1 => sealed.commitment_tag[0] ^= mask,
            _ => sealed.signature[0] ^= mask,
        }
        prop_assert!(envelope::open(&sealed, aad, &doc_key, &author).is_err());
    }
}
