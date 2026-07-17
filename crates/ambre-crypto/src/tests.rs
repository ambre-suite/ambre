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

    let sealed = envelope::seal(plaintext, aad, &doc_key, &signing).unwrap();
    let opened = envelope::open(&sealed, aad, &doc_key, &author).unwrap();
    assert_eq!(opened, plaintext);
}

#[test]
fn tampered_ciphertext_fails() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let author = signing.verifying_key().to_bytes();
    let aad = b"aad";

    let mut sealed = envelope::seal(b"payload", aad, &doc_key, &signing).unwrap();
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

    let sealed = envelope::seal(b"payload", b"document-A", &doc_key, &signing).unwrap();
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

    let sealed = envelope::seal(b"payload", aad, &doc_key, &signing).unwrap();
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

    let sealed = envelope::seal(b"payload", aad, &doc_key, &signing).unwrap();
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
        ecdh::shared_secret(&alice, &bob_pub).unwrap(),
        ecdh::shared_secret(&bob, &alice_pub).unwrap()
    );
}

#[test]
fn wrap_unwrap_roundtrip() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let alice_pub = x25519_dalek::PublicKey::from(&alice).to_bytes();
    let bob_pub = x25519_dalek::PublicKey::from(&bob).to_bytes();

    let doc_key = DocumentKey::from_array([0x55; 32]);
    let wrapped = ecdh::wrap_document_key(&alice, &bob_pub, &doc_key).unwrap();
    let unwrapped = ecdh::unwrap_document_key(&bob, &alice_pub, &wrapped).unwrap();
    assert_eq!(unwrapped.as_bytes(), doc_key.as_bytes());
}

#[test]
fn weak_peer_public_key_rejected() {
    // The all-zero u-coordinate is a low-order point: the X25519 output is the
    // predictable all-zero secret. Both the raw exchange and the wrap must reject
    // it (M14: reject a non-contributory shared secret).
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let weak_pub = [0u8; 32];

    assert_eq!(
        ecdh::shared_secret(&alice, &weak_pub),
        Err(CryptoError::WeakPublicKey)
    );

    let doc_key = DocumentKey::from_array([0x55; 32]);
    assert_eq!(
        ecdh::wrap_document_key(&alice, &weak_pub, &doc_key).err(),
        Some(CryptoError::WeakPublicKey)
    );
}

#[test]
fn kek_binds_peer_identity() {
    // The KEK is derived over `sender_pk || recipient_pk`, so a key wrapped by
    // Alice for Bob cannot be unwrapped by Carol even if Carol somehow presents
    // Alice's public key as the sender: the recipient half of the transcript
    // differs, so the KEK (and thus the AEAD open) fails.
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let carol = MasterSeed::from_array([0x03; 32]).derive_exchange_key();
    let alice_pub = x25519_dalek::PublicKey::from(&alice).to_bytes();
    let bob_pub = x25519_dalek::PublicKey::from(&bob).to_bytes();

    let doc_key = DocumentKey::from_array([0x55; 32]);
    let wrapped = ecdh::wrap_document_key(&alice, &bob_pub, &doc_key).unwrap();

    // Carol tries to unwrap Bob's envelope, claiming Alice as the sender.
    assert!(ecdh::unwrap_document_key(&carol, &alice_pub, &wrapped).is_err());
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
        let sealed = envelope::seal(&data, aad, &doc_key, &signing).unwrap();
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
        let mut sealed = envelope::seal(&data, aad, &doc_key, &signing).unwrap();
        let mask = 1u8 << bit;
        match idx {
            0 => sealed.ciphertext[0] ^= mask,
            1 => sealed.commitment_tag[0] ^= mask,
            _ => sealed.signature[0] ^= mask,
        }
        prop_assert!(envelope::open(&sealed, aad, &doc_key, &author).is_err());
    }
}

// ── TOFU fingerprint (ambre-spec-chrome.md §2.H) ─────────────────────────────

#[test]
fn fingerprint_is_stable_grouped_base32() {
    // Golden vector: the all-zero key. This is the string a user reads aloud for
    // out-of-band verification, so it is frozen -- any change to the domain tag,
    // truncation length, alphabet or grouping must break this test on purpose,
    // not silently shift what two people compare over the phone.
    let fp = hash::fingerprint(&[0u8; 32]);
    assert_eq!(fp, "2HBA RR6Y FDOA EIJF ZZFT YWQZ MKRW N5FK");

    // 20 bytes -> 32 base32 chars -> 8 groups of 4, space-separated.
    let groups: Vec<&str> = fp.split(' ').collect();
    assert_eq!(groups.len(), 8, "expected 8 groups, got: {fp}");
    assert!(groups.iter().all(|g| g.len() == 4), "ragged grouping: {fp}");
    // No padding, uppercase RFC 4648 alphabet only.
    assert!(
        fp.chars()
            .all(|c| c == ' ' || c.is_ascii_uppercase() || ('2'..='7').contains(&c)),
        "unexpected characters: {fp}"
    );
    // Deterministic: recomputing yields the identical string.
    assert_eq!(fp, hash::fingerprint(&[0u8; 32]));
}

#[test]
fn fingerprint_distinguishes_keys() {
    // A single-bit difference in the key must change the fingerprint (the whole
    // point of pinning it): near-identical substituted keys stay distinguishable.
    let a = hash::fingerprint(&[0u8; 32]);
    let mut key_b = [0u8; 32];
    key_b[0] = 1;
    let b = hash::fingerprint(&key_b);
    assert_ne!(a, b);
}
