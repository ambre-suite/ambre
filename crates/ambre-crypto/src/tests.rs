//! Host tests for the crypto primitives and envelope.

use proptest::prelude::*;

use crate::keys::{DocumentKey, FileKey, MasterSeed};
use crate::{CryptoError, aead, commitment, ecdh, envelope, hash, passphrase};

fn fixed_doc_key() -> DocumentKey {
    DocumentKey::from_array([0x11; 32])
}

fn fixed_seed() -> MasterSeed {
    MasterSeed::from_array([0x22; 32])
}

// -- Envelope ----------------------------------------------------------------

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

// -- Key derivation ----------------------------------------------------------

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

// -- ECDH --------------------------------------------------------------------

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
    // The KEK is derived over `sender_pk || recipient_pk`. A key wrapped by
    // Alice for Bob cannot be unwrapped by Carol, even if Carol somehow
    // presents Alice's public key as the sender. The recipient half of the
    // transcript differs, so the KEK (and thus the AEAD open) fails.
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

// -- Argon2id ----------------------------------------------------------------

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

// -- BLAKE2b chain -----------------------------------------------------------

#[test]
fn blake2_chain_hash() {
    let h1 = hash::chain_hash(b"parent", b"snapshot");
    assert_eq!(h1, hash::chain_hash(b"parent", b"snapshot"));
    // Order-sensitive: the split point matters.
    assert_ne!(h1, hash::chain_hash(b"paren", b"tsnapshot"));
}

// -- Key identifier ----------------------------------------------------------

/// A key's identifier is derived from the key alone. The same key always gives
/// the same one, another key gives another, and the key's bytes are not in it
/// (`SYNC-115`).
#[test]
fn a_key_id_is_derived_from_the_key() {
    let key = fixed_doc_key();
    assert_eq!(key.id(), DocumentKey::from_array([0x11; 32]).id());
    assert_ne!(key.id(), DocumentKey::from_array([0x12; 32]).id());
    assert_ne!(
        key.id().as_array()[..],
        [0x11; 32][..ambre_core::KEY_ID_LEN]
    );
}

/// A previous key sealed under the next one opens under that key alone, for
/// its author alone, and comes back as the same key (`SYNC-116`).
#[test]
fn a_previous_key_opens_under_the_next_one_and_no_other() {
    let previous = DocumentKey::from_array([0x21; 32]);
    let next = fixed_doc_key();
    let author = fixed_seed().derive_signing_key();
    let sealed = next.seal_previous(&previous, b"link", &author).unwrap();
    let author_pub = author.verifying_key().to_bytes();

    let opened = next.open_previous(&sealed, b"link", &author_pub).unwrap();
    assert_eq!(opened.id(), previous.id());
    assert!(
        previous
            .open_previous(&sealed, b"link", &author_pub)
            .is_err()
    );
    assert!(next.open_previous(&sealed, b"other", &author_pub).is_err());
    let stranger = MasterSeed::from_array([0x33; 32]).derive_signing_key();
    let stranger_pub = stranger.verifying_key().to_bytes();
    assert_eq!(
        next.open_previous(&sealed, b"link", &stranger_pub).err(),
        Some(CryptoError::InvalidSignature)
    );
}

// -- Media address -----------------------------------------------------------

/// One address per plaintext under one key, and another under any other key.
/// It is never the bare hash of the bytes, which the relay could compute
/// itself (`SYNC-050`).
#[test]
fn a_media_address_needs_the_document_key() {
    use blake2::{Blake2b512, Digest};

    let picture = b"the same picture, used twice";
    let address = commitment::media_address(&fixed_doc_key(), picture);
    assert_eq!(
        address,
        commitment::media_address(&fixed_doc_key(), picture)
    );
    assert_ne!(
        address,
        commitment::media_address(&DocumentKey::from_array([0x12; 32]), picture)
    );
    assert_ne!(
        address,
        commitment::media_address(&fixed_doc_key(), b"another picture")
    );
    let plain = Blake2b512::digest(picture);
    assert_ne!(address[..], plain[..32]);
}

// -- Property tests ----------------------------------------------------------

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

// -- TOFU fingerprint (`ambre-spec-key-verification.md`) ---------------------

#[test]
fn fingerprint_is_stable_grouped_base32() {
    // Golden vector: the all-zero key. This is the string a user reads aloud for
    // out-of-band verification, so it is frozen. Any change to the domain tag,
    // truncation length, alphabet or grouping must break this test on purpose.
    // It must not silently shift what two people compare over the phone.
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

// -- Passphrase-sealed file --------------------------------------------------

#[test]
fn a_file_key_is_not_the_user_key_of_the_same_passphrase_and_salt() {
    // The two derivations share a passphrase, a salt and a work factor. Only
    // the domain separates them, so this is the one assertion that shows the
    // separation at all. Without it, a file on disk would carry the value that
    // unwraps the account's master key.
    let salt = [0xAB; 16];
    let user = passphrase::derive_user_key(b"correct horse", &salt).unwrap();
    let file = FileKey::from_passphrase(b"correct horse", &salt).unwrap();
    assert_ne!(file.as_bytes(), &user);
}

#[test]
fn a_file_key_is_deterministic_and_per_salt() {
    // Deterministic, or nothing reopens the file; per salt, or two files
    // sealed under one passphrase share a key.
    let salt = [0xAB; 16];
    let first = FileKey::from_passphrase(b"correct horse", &salt).unwrap();
    let again = FileKey::from_passphrase(b"correct horse", &salt).unwrap();
    assert_eq!(first.as_bytes(), again.as_bytes());

    let elsewhere = FileKey::from_passphrase(b"correct horse", &[0xCD; 16]).unwrap();
    assert_ne!(first.as_bytes(), elsewhere.as_bytes());
}

#[test]
fn each_new_file_draws_its_own_salt() {
    // The salt is what keeps two files sealed under one passphrase from sharing
    // a key. So this entry point draws one per file, and the caller cannot
    // supply it here at all.
    let (first, first_salt) = FileKey::for_new_file(b"correct horse").unwrap();
    let (again, again_salt) = FileKey::for_new_file(b"correct horse").unwrap();
    assert_ne!(first_salt, again_salt);
    assert_ne!(first.as_bytes(), again.as_bytes());

    // And the salt it returns is the one that reopens the file. The header
    // carries this value in the clear, and nothing else can re-derive the key.
    let reopened = FileKey::from_passphrase(b"correct horse", &first_salt).unwrap();
    assert_eq!(reopened.as_bytes(), first.as_bytes());
}

#[test]
fn a_sealed_file_opens_under_its_passphrase_and_nothing_else() {
    let salt = [0x01; 16];
    let aad = b"the-sealed-header";
    let plaintext = b"a whole plaintext container";
    let key = FileKey::from_passphrase(b"open sesame", &salt).unwrap();
    let (nonce, ciphertext) = aead::seal_file(plaintext, aad, &key).unwrap();
    assert_eq!(
        aead::open_file(&nonce, &ciphertext, aad, &key).unwrap(),
        plaintext
    );

    // A wrong passphrase is a wrong key, and Poly1305 is what says so. Nothing
    // cheaper sits upstream of it: no password verifier, no checksum. The tag
    // is the whole of "that passphrase does not open this file".
    let wrong = FileKey::from_passphrase(b"open sesa", &salt).unwrap();
    assert_eq!(
        aead::open_file(&nonce, &ciphertext, aad, &wrong),
        Err(CryptoError::AeadFailed)
    );

    // And the seal binds the header it covered: an edited salt, nonce or
    // lineage byte fails the tag rather than deriving quietly.
    assert_eq!(
        aead::open_file(&nonce, &ciphertext, b"the-sealed-headeR", &key),
        Err(CryptoError::AeadFailed)
    );
}

#[test]
fn the_sealed_header_widths_match_the_primitives() {
    // `ambre-core` lays the sealed header out and names no primitive
    // (`DOCFILE-006`), so it declares its two widths independently of the
    // cipher and the KDF. This is the seam where a mismatch
    // would surface, and it is the only place both sides are nameable at once.
    assert_eq!(ambre_core::docfile::SEAL_NONCE_LEN, aead::NONCE_LEN);
    assert_eq!(
        ambre_core::docfile::SALT_LEN,
        16,
        "Argon2id requires at least 8 bytes of salt; the header carries 16"
    );
}
