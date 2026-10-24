//! Host tests for the crypto primitives and envelope.

use proptest::prelude::*;

use crate::keys::{DocumentKey, FileKey, MasterSeed};
use crate::{CryptoError, aead, commitment, ecdh, envelope, hash, passphrase};
use ambre_core::{Aad, Ciphertext, EntryDigest, FileSalt, MasterKeySalt, Nonce};

/// A passphrase as a prompt hands it over.
fn pass(text: &str) -> ambre_core::Passphrase {
    ambre_core::Passphrase::new(text.to_owned())
}

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
    let author = ambre_core::PublicKey::from_array(signing.verifying_key().to_bytes());
    let aad = &Aad::of(b"public-data");
    let plaintext = b"hello, encrypted world";

    let sealed = envelope::seal(plaintext, aad, &doc_key, &signing).unwrap();
    let opened = envelope::open(&sealed, aad, &doc_key, &author).unwrap();
    assert_eq!(opened, plaintext);
}

/// `ZK-017`: the debug form of an envelope prints its length and none of its
/// bytes, so a `{:?}` in a log carries no payload.
#[test]
fn an_envelope_debug_form_prints_no_byte_of_it() {
    let signing = fixed_seed().derive_signing_key();
    let mut sealed =
        envelope::seal(b"payload", &Aad::of(b"aad"), &fixed_doc_key(), &signing).unwrap();
    // Bytes no length or field name can spell, so a match is a leak.
    sealed.ciphertext = Ciphertext(vec![0xAB; 7]);
    sealed.nonce = Nonce([0xCD; aead::NONCE_LEN]);
    let shown = format!("{sealed:?}");
    assert_eq!(shown, "Sealed { ciphertext_len: 7, .. }");
}

#[test]
fn tampered_ciphertext_fails() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let author = ambre_core::PublicKey::from_array(signing.verifying_key().to_bytes());
    let aad = &Aad::of(b"aad");

    let mut sealed = envelope::seal(b"payload", aad, &doc_key, &signing).unwrap();
    sealed.ciphertext.0[0] ^= 0xFF;
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
    let author = ambre_core::PublicKey::from_array(signing.verifying_key().to_bytes());

    let sealed = envelope::seal(b"payload", &Aad::of(b"document-A"), &doc_key, &signing).unwrap();
    // Re-sign over the altered aad so the signature passes but the HMAC commitment
    // (and the AEAD aad) no longer match -- models inter-document substitution.
    let forged_aad = &Aad::of(b"document-B");
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

/// Every seal draws its own 192-bit nonce (`SYNC-003`). One payload sealed
/// many times under one key never repeats a nonce, nor a ciphertext.
#[test]
fn every_seal_draws_a_fresh_nonce() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let sealed: Vec<_> = (0..64)
        .map(|_| envelope::seal(b"same payload", &Aad::of(b"aad"), &doc_key, &signing).unwrap())
        .collect();
    let nonces: std::collections::HashSet<Nonce> = sealed.iter().map(|s| s.nonce).collect();
    let ciphertexts: std::collections::HashSet<&[u8]> =
        sealed.iter().map(|s| s.ciphertext.0.as_slice()).collect();
    assert_eq!(nonces.len(), sealed.len());
    assert_eq!(ciphertexts.len(), sealed.len());
}

/// The open checks the signature first, the tag second and the cipher last
/// (`SYNC-089`). One forged frame fails all three, and each re-signing peels
/// one check away, so the refusal named is always the earliest one.
#[test]
fn the_signature_is_checked_before_the_tag_and_the_cipher() {
    use crate::signature;

    let doc_key = fixed_doc_key();
    let other_key = DocumentKey::from_array([0x33; 32]);
    let signing = fixed_seed().derive_signing_key();
    let author = ambre_core::PublicKey::from_array(signing.verifying_key().to_bytes());
    let aad = &Aad::of(b"aad");
    let mut forged = envelope::seal(b"payload", aad, &other_key, &signing).unwrap();
    forged.ciphertext.0[0] ^= 0xFF;
    let resign = |sealed: &mut envelope::Sealed| {
        sealed.signature = signature::sign(
            &signing,
            &sealed.nonce,
            &sealed.ciphertext,
            aad,
            &sealed.commitment_tag,
        );
    };

    assert_eq!(
        envelope::open(&forged, aad, &doc_key, &author),
        Err(CryptoError::InvalidSignature)
    );
    resign(&mut forged);
    assert_eq!(
        envelope::open(&forged, aad, &doc_key, &author),
        Err(CryptoError::CommitmentMismatch)
    );
    forged.commitment_tag =
        commitment::commitment_tag(&doc_key, &forged.nonce, &forged.ciphertext, aad);
    resign(&mut forged);
    assert_eq!(
        envelope::open(&forged, aad, &doc_key, &author),
        Err(CryptoError::AeadFailed)
    );
}

#[test]
fn invalid_signature_rejected() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let aad = &Aad::of(b"aad");

    let sealed = envelope::seal(b"payload", aad, &doc_key, &signing).unwrap();
    let wrong_author = ambre_core::PublicKey::from_array(
        MasterSeed::from_array([0x99; 32])
            .derive_signing_key()
            .verifying_key()
            .to_bytes(),
    );
    assert_eq!(
        envelope::open(&sealed, aad, &doc_key, &wrong_author),
        Err(CryptoError::InvalidSignature)
    );
}

#[test]
fn wrong_document_key_rejected() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let author = ambre_core::PublicKey::from_array(signing.verifying_key().to_bytes());
    let aad = &Aad::of(b"aad");

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

/// Both derivations of one seed match vectors computed outside this crate:
/// the first 32 bytes of BLAKE2b-512 over `ambre-sign-v1` or `ambre-exch-v1`,
/// then the seed (`ZK-009`). A renamed domain, a reordered input or another
/// hash gives every existing account a different key pair, and fails here.
#[test]
fn derived_keys_match_the_frozen_domains() {
    const SIGN: [u8; 32] = [
        0x9b, 0xe4, 0x65, 0x12, 0x2c, 0x97, 0x8c, 0xfa, 0xee, 0xb6, 0xa6, 0x41, 0x58, 0x1d, 0xe1,
        0x21, 0x29, 0x91, 0xb1, 0x1f, 0x49, 0x56, 0x34, 0x00, 0xa0, 0xb3, 0xa6, 0xf6, 0xb9, 0xf3,
        0xb1, 0xcf,
    ];
    const EXCH: [u8; 32] = [
        0x9d, 0x13, 0x74, 0xad, 0x3e, 0xea, 0xfc, 0x68, 0x88, 0x8b, 0x3f, 0x71, 0x7a, 0x82, 0xeb,
        0x41, 0xde, 0x75, 0x9e, 0xb2, 0xe1, 0xbd, 0xa6, 0x79, 0xae, 0xda, 0xcd, 0x9d, 0x79, 0x68,
        0x05, 0x4d,
    ];
    let seed = MasterSeed::from_array([0x07; 32]);
    assert_eq!(seed.derive_signing_key().to_bytes(), SIGN);
    assert_eq!(seed.derive_exchange_key().to_bytes(), EXCH);
}

// -- The local store -----------------------------------------------------------

/// The store key is the seed's alone: pinned here, so a renamed domain fails
/// this test rather than every copy a device holds.
#[test]
fn the_store_key_matches_its_frozen_domain() {
    const STORE: [u8; 32] = [
        0x44, 0xfc, 0xec, 0x32, 0x4e, 0x29, 0xce, 0x2e, 0x4b, 0xd9, 0x42, 0xac, 0x8f, 0x6b, 0x6a,
        0x5c, 0xdb, 0xde, 0xeb, 0x2b, 0xbd, 0x20, 0xa6, 0xe3, 0x13, 0xc4, 0xe6, 0x85, 0xe8, 0xdd,
        0x10, 0x14,
    ];
    let seed = MasterSeed::from_array([0x07; 32]);
    assert_eq!(seed.derive_store_key().as_bytes(), &STORE);
}

/// A row opens under its own key and its own place, and under nothing else.
#[test]
fn a_store_row_opens_only_where_it_was_sealed() {
    let key = fixed_seed().derive_store_key();
    let row = aead::seal_row(b"a copy", &Aad::of(b"doc:0a"), &key).unwrap();
    assert_eq!(row.first(), Some(&aead::RowLayout::CURRENT.byte()));
    assert!(!row.windows(6).any(|w| w == b"a copy"));
    assert_eq!(
        aead::open_row(&row, &Aad::of(b"doc:0a"), &key).unwrap(),
        b"a copy"
    );

    let stranger = MasterSeed::from_array([0x33; 32]).derive_store_key();
    assert!(aead::open_row(&row, &Aad::of(b"doc:0a"), &stranger).is_err());
    assert!(aead::open_row(&row, &Aad::of(b"doc:0b"), &key).is_err());
    let mut flipped = row.clone();
    if let Some(last) = flipped.last_mut() {
        *last ^= 1;
    }
    assert!(aead::open_row(&flipped, &Aad::of(b"doc:0a"), &key).is_err());
    // A row the store held in the clear, or a stub, opens as nothing.
    assert!(aead::open_row(b"a copy", &Aad::of(b"doc:0a"), &key).is_err());
    assert!(aead::open_row(&[], &Aad::of(b"doc:0a"), &key).is_err());
    assert!(
        aead::open_row(
            &[aead::RowLayout::CURRENT.byte()],
            &Aad::of(b"doc:0a"),
            &key
        )
        .is_err()
    );
}

// -- ECDH --------------------------------------------------------------------

#[test]
fn ecdh_symmetric() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let alice_pub =
        ambre_core::ExchangePublicKey::from_array(x25519_dalek::PublicKey::from(&alice).to_bytes());
    let bob_pub =
        ambre_core::ExchangePublicKey::from_array(x25519_dalek::PublicKey::from(&bob).to_bytes());

    assert_eq!(
        ecdh::shared_secret(&alice, &bob_pub).unwrap().as_bytes(),
        ecdh::shared_secret(&bob, &alice_pub).unwrap().as_bytes()
    );
}

#[test]
fn a_shared_secret_is_wiped_and_never_printed() {
    fn wiped_on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let bob_pub =
        ambre_core::ExchangePublicKey::from_array(x25519_dalek::PublicKey::from(&bob).to_bytes());
    let shared = ecdh::shared_secret(&alice, &bob_pub).unwrap();
    wiped_on_drop(&shared);
    assert_eq!(format!("{shared:?}"), "SharedSecret(32 bytes)");
}

#[test]
fn wrap_unwrap_roundtrip() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let alice_pub =
        ambre_core::ExchangePublicKey::from_array(x25519_dalek::PublicKey::from(&alice).to_bytes());
    let bob_pub =
        ambre_core::ExchangePublicKey::from_array(x25519_dalek::PublicKey::from(&bob).to_bytes());

    let doc_key = DocumentKey::from_array([0x55; 32]);
    let wrapped = ecdh::wrap_document_key(&alice, &bob_pub, &doc_key).unwrap();
    let unwrapped = ecdh::unwrap_document_key(&bob, &alice_pub, &wrapped).unwrap();
    assert_eq!(unwrapped.as_bytes(), doc_key.as_bytes());
}

/// Holds `SYNC-104` at the primitive. The anchor of the membership log rides
/// inside the seal, so it opens with the key and is bound to it. A wrap
/// without one opens with none, and the key alone still unwraps either.
#[test]
fn the_log_anchor_travels_sealed_with_the_key() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let alice_pub =
        ambre_core::ExchangePublicKey::from_array(x25519_dalek::PublicKey::from(&alice).to_bytes());
    let bob_pub =
        ambre_core::ExchangePublicKey::from_array(x25519_dalek::PublicKey::from(&bob).to_bytes());
    let doc_key = DocumentKey::from_array([0x55; 32]);

    let anchored =
        ecdh::wrap_anchored(&alice, &bob_pub, &doc_key, Some(&EntryDigest([0x66; 32]))).unwrap();
    let (key, anchor) = ecdh::unwrap_anchored(&bob, &alice_pub, &anchored).unwrap();
    assert_eq!(key.as_bytes(), doc_key.as_bytes());
    assert_eq!(anchor, Some(EntryDigest([0x66; 32])));
    let key = ecdh::unwrap_document_key(&bob, &alice_pub, &anchored).unwrap();
    assert_eq!(key.as_bytes(), doc_key.as_bytes());

    let bare = ecdh::wrap_document_key(&alice, &bob_pub, &doc_key).unwrap();
    let (_, anchor) = ecdh::unwrap_anchored(&bob, &alice_pub, &bare).unwrap();
    assert_eq!(anchor, None);

    // A flipped byte of the sealed anchor fails the tag: nobody swaps it.
    let mut tampered = anchored;
    if let Some(byte) = tampered.ciphertext.0.get_mut(40) {
        *byte ^= 1;
    }
    assert!(ecdh::unwrap_anchored(&bob, &alice_pub, &tampered).is_err());
}

/// Holds `SRV-226` against the primitives. Each length the wire reads for a
/// wrapped key is the length a wrap writes. A change of layout fails here,
/// rather than at every relay read.
#[test]
fn a_wrap_has_a_length_the_wire_reads() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob_pub = ambre_core::ExchangePublicKey::from_array(
        x25519_dalek::PublicKey::from(&MasterSeed::from_array([0x02; 32]).derive_exchange_key())
            .to_bytes(),
    );
    let doc_key = DocumentKey::from_array([0x55; 32]);
    let length = |wrapped: ecdh::WrappedKey| wrapped.nonce.0.len() + wrapped.ciphertext.len();
    let bare = length(ecdh::wrap_document_key(&alice, &bob_pub, &doc_key).unwrap());
    let anchored = length(
        ecdh::wrap_anchored(&alice, &bob_pub, &doc_key, Some(&EntryDigest([0x66; 32]))).unwrap(),
    );
    assert_eq!(ambre_core::WrappedDocKey::LENGTHS, [bare, anchored]);

    let seed = MasterSeed::from_array([0x03; 32]);
    let wrapped = seed.wrap_for_storage(&doc_key, &Aad::of(b"aad")).unwrap();
    assert_eq!(
        ambre_core::WrappedMasterKey::LENGTHS,
        [wrapped.as_bytes().len()]
    );
}

#[test]
fn weak_peer_public_key_rejected() {
    // The all-zero u-coordinate is a low-order point: the X25519 output is the
    // predictable all-zero secret. Both the raw exchange and the wrap must reject
    // it (M14: reject a non-contributory shared secret).
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let weak_pub = ambre_core::ExchangePublicKey::from_array([0u8; 32]);

    assert_eq!(
        ecdh::shared_secret(&alice, &weak_pub).err(),
        Some(CryptoError::WeakPublicKey)
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
    let alice_pub =
        ambre_core::ExchangePublicKey::from_array(x25519_dalek::PublicKey::from(&alice).to_bytes());
    let bob_pub =
        ambre_core::ExchangePublicKey::from_array(x25519_dalek::PublicKey::from(&bob).to_bytes());

    let doc_key = DocumentKey::from_array([0x55; 32]);
    let wrapped = ecdh::wrap_document_key(&alice, &bob_pub, &doc_key).unwrap();

    // Carol tries to unwrap Bob's envelope, claiming Alice as the sender.
    assert!(ecdh::unwrap_document_key(&carol, &alice_pub, &wrapped).is_err());
}

// -- Argon2id ----------------------------------------------------------------

#[test]
fn argon2_deterministic() {
    let salt = MasterKeySalt::from_array([0xAB; 32]);
    let k1 = passphrase::derive_user_key(&pass("correct horse"), &salt).unwrap();
    let k2 = passphrase::derive_user_key(&pass("correct horse"), &salt).unwrap();
    assert_eq!(k1, k2);

    let other_salt = MasterKeySalt::from_array([0xCD; 32]);
    let k3 = passphrase::derive_user_key(&pass("correct horse"), &other_salt).unwrap();
    assert_ne!(k1, k3);

    let k4 = passphrase::derive_user_key(&pass("wrong passphrase"), &salt).unwrap();
    assert_ne!(k1, k4);
}

// -- BLAKE2b chain -----------------------------------------------------------

#[test]
fn blake2_chain_hash() {
    let snapshot = Ciphertext(b"snapshot".to_vec());
    let genesis = hash::chain_hash(None, &snapshot);
    assert_eq!(genesis, hash::chain_hash(None, &snapshot));
    // The parent is part of the link: a different parent breaks the chain.
    let child = hash::chain_hash(Some(&genesis), &snapshot);
    assert_ne!(child, genesis);
    let other = hash::chain_hash(None, &Ciphertext(b"snapshoT".to_vec()));
    assert_ne!(child, hash::chain_hash(Some(&other), &snapshot));
}

#[test]
fn the_keyless_digests_keep_their_bytes() {
    // The fixture digest keeps the bytes the chain hash gave it, so a
    // committed digest survives the split.
    let mut hasher = <blake2::Blake2b512 as blake2::Digest>::new();
    blake2::Digest::update(&mut hasher, 0u64.to_le_bytes());
    blake2::Digest::update(&mut hasher, b"file");
    let full = blake2::Digest::finalize(hasher);
    assert_eq!(hash::file_digest(b"file")[..], full[..32]);
    assert_eq!(
        hash::chain_hash(None, &Ciphertext(b"file".to_vec())).0,
        hash::file_digest(b"file")
    );
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
    let sealed = next
        .seal_previous(&previous, &Aad::of(b"link"), &author)
        .unwrap();
    let author_pub = ambre_core::PublicKey::from_array(author.verifying_key().to_bytes());

    let opened = next
        .open_previous(&sealed, &Aad::of(b"link"), &author_pub)
        .unwrap();
    assert_eq!(opened.id(), previous.id());
    assert!(
        previous
            .open_previous(&sealed, &Aad::of(b"link"), &author_pub)
            .is_err()
    );
    assert!(
        next.open_previous(&sealed, &Aad::of(b"other"), &author_pub)
            .is_err()
    );
    let stranger = MasterSeed::from_array([0x33; 32]).derive_signing_key();
    let stranger_pub = ambre_core::PublicKey::from_array(stranger.verifying_key().to_bytes());
    assert_eq!(
        next.open_previous(&sealed, &Aad::of(b"link"), &stranger_pub)
            .err(),
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
    assert_ne!(address.as_bytes()[..], plain[..32]);
}

// -- Property tests ----------------------------------------------------------

proptest! {
    #[test]
    fn prop_roundtrip_encrypt_decrypt(data in any::<Vec<u8>>(), key in any::<[u8; 32]>()) {
        let doc_key = DocumentKey::from_array(key);
        let signing = fixed_seed().derive_signing_key();
        let author = ambre_core::PublicKey::from_array(signing.verifying_key().to_bytes());
        let aad = &Aad::of(b"aad");
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
        let author = ambre_core::PublicKey::from_array(signing.verifying_key().to_bytes());
        let aad = &Aad::of(b"aad");
        let mut sealed = envelope::seal(&data, aad, &doc_key, &signing).unwrap();
        let mask = 1u8 << bit;
        match idx {
            0 => sealed.ciphertext.0[0] ^= mask,
            1 => sealed.commitment_tag.0[0] ^= mask,
            _ => sealed.signature.0[0] ^= mask,
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
    let fingerprint = hash::fingerprint(&ambre_core::ExchangePublicKey::from_array([0u8; 32]));
    let fp = fingerprint.as_str();
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
    assert_eq!(
        fp,
        hash::fingerprint(&ambre_core::ExchangePublicKey::from_array([0u8; 32])).as_str()
    );
}

#[test]
fn fingerprint_distinguishes_keys() {
    // A single-bit difference in the key must change the fingerprint (the whole
    // point of pinning it): near-identical substituted keys stay distinguishable.
    let a = hash::fingerprint(&ambre_core::ExchangePublicKey::from_array([0u8; 32]));
    let mut key_b = [0u8; 32];
    key_b[0] = 1;
    let b = hash::fingerprint(&ambre_core::ExchangePublicKey::from_array(key_b));
    assert_ne!(a, b);
}

// -- Passphrase-sealed file --------------------------------------------------

#[test]
fn a_file_key_is_not_the_user_key_of_the_same_passphrase_and_salt() {
    // The two derivations share a passphrase, a salt and a work factor. Only
    // the domain separates them, so this is the one assertion that shows the
    // separation at all. Without it, a file on disk would carry the value that
    // unwraps the account's master key.
    let user = passphrase::derive_user_key(
        &pass("correct horse"),
        &MasterKeySalt::from_array([0xAB; 32]),
    )
    .unwrap();
    let file = FileKey::from_passphrase(&pass("correct horse"), &FileSalt::from_array([0xAB; 16]))
        .unwrap();
    assert_ne!(file.as_bytes(), &user);
}

#[test]
fn a_file_key_is_deterministic_and_per_salt() {
    // Deterministic, or nothing reopens the file; per salt, or two files
    // sealed under one passphrase share a key.
    let salt = FileSalt::from_array([0xAB; 16]);
    let first = FileKey::from_passphrase(&pass("correct horse"), &salt).unwrap();
    let again = FileKey::from_passphrase(&pass("correct horse"), &salt).unwrap();
    assert_eq!(first.as_bytes(), again.as_bytes());

    let elsewhere =
        FileKey::from_passphrase(&pass("correct horse"), &FileSalt::from_array([0xCD; 16]))
            .unwrap();
    assert_ne!(first.as_bytes(), elsewhere.as_bytes());
}

#[test]
fn each_new_file_draws_its_own_salt() {
    // The salt is what keeps two files sealed under one passphrase from sharing
    // a key. So this entry point draws one per file, and the caller cannot
    // supply it here at all.
    let (first, first_salt) = FileKey::for_new_file(&pass("correct horse")).unwrap();
    let (again, again_salt) = FileKey::for_new_file(&pass("correct horse")).unwrap();
    assert_ne!(first_salt, again_salt);
    assert_ne!(first.as_bytes(), again.as_bytes());

    // And the salt it returns is the one that reopens the file. The header
    // carries this value in the clear, and nothing else can re-derive the key.
    let reopened = FileKey::from_passphrase(&pass("correct horse"), &first_salt).unwrap();
    assert_eq!(reopened.as_bytes(), first.as_bytes());
}

#[test]
fn a_sealed_file_opens_under_its_passphrase_and_nothing_else() {
    let salt = FileSalt::from_array([0x01; 16]);
    let aad = &Aad::of(b"the-sealed-header");
    let plaintext = b"a whole plaintext container";
    let key = FileKey::from_passphrase(&pass("open sesame"), &salt).unwrap();
    let (nonce, ciphertext) = aead::seal_file(plaintext, aad, &key).unwrap();
    assert_eq!(
        aead::open_file(&nonce, &ciphertext, aad, &key).unwrap(),
        plaintext
    );

    // A wrong passphrase is a wrong key, and Poly1305 is what says so. Nothing
    // cheaper sits upstream of it: no password verifier, no checksum. The tag
    // is the whole of "that passphrase does not open this file".
    let wrong = FileKey::from_passphrase(&pass("open sesa"), &salt).unwrap();
    assert_eq!(
        aead::open_file(&nonce, &ciphertext, aad, &wrong),
        Err(CryptoError::AeadFailed)
    );

    // And the seal binds the header it covered: an edited salt, nonce or
    // lineage byte fails the tag rather than deriving quietly.
    assert_eq!(
        aead::open_file(&nonce, &ciphertext, &Aad::of(b"the-sealed-headeR"), &key),
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

#[test]
fn a_document_key_sealed_under_another_opens_to_the_same_key() {
    let (inner, outer) = (DocumentKey::generate(), DocumentKey::generate());
    let (nonce, sealed) = aead::seal_document_key(&inner, &Aad::of(b"channel-1"), &outer).unwrap();
    let opened = aead::open_document_key(&nonce, &sealed, &Aad::of(b"channel-1"), &outer).unwrap();
    // The opened key reads what the original sealed.
    let (n, ct) = aead::seal(b"a message", &Aad::of(b""), &inner).unwrap();
    assert_eq!(
        aead::open(&n, &ct, &Aad::of(b""), &opened).unwrap(),
        b"a message"
    );
}

#[test]
fn a_sealed_document_key_opens_under_no_other_key_or_aad() {
    let (inner, outer) = (DocumentKey::generate(), DocumentKey::generate());
    let (nonce, sealed) = aead::seal_document_key(&inner, &Aad::of(b"channel-1"), &outer).unwrap();
    let wrong_key = aead::open_document_key(&nonce, &sealed, &Aad::of(b"channel-1"), &inner);
    assert_eq!(wrong_key.err(), Some(CryptoError::AeadFailed));
    // The aad binds the sealed key to one channel: moved to another, it fails.
    let moved = aead::open_document_key(&nonce, &sealed, &Aad::of(b"channel-2"), &outer);
    assert_eq!(moved.err(), Some(CryptoError::AeadFailed));
}

/// Holds `SYNC-104` for a key sealed under another. The anchor of the log
/// comes back with the key, and a bare seal carries none. A plaintext with a
/// partial anchor is refused rather than read as a key.
#[test]
fn the_log_anchor_travels_sealed_with_a_catalogue_key() {
    let (inner, outer) = (DocumentKey::generate(), DocumentKey::generate());
    let (nonce, sealed) = aead::seal_anchored_key(
        &inner,
        &Aad::of(b"channel-1"),
        &outer,
        Some(&EntryDigest([0x33; 32])),
    )
    .unwrap();
    let (key, anchor) =
        aead::open_anchored_key(&nonce, &sealed, &Aad::of(b"channel-1"), &outer).unwrap();
    assert_eq!(key.as_bytes(), inner.as_bytes());
    assert_eq!(anchor, Some(EntryDigest([0x33; 32])));

    let (nonce, bare) = aead::seal_document_key(&inner, &Aad::of(b"channel-1"), &outer).unwrap();
    let (_, anchor) =
        aead::open_anchored_key(&nonce, &bare, &Aad::of(b"channel-1"), &outer).unwrap();
    assert_eq!(anchor, None);

    let (nonce, partial) = aead::seal(&[7; 48], &Aad::of(b"channel-1"), &outer).unwrap();
    let partial = aead::open_anchored_key(&nonce, &partial, &Aad::of(b"channel-1"), &outer);
    assert_eq!(partial.err(), Some(CryptoError::InvalidKeyLength));
}

#[test]
fn a_sealed_value_that_is_no_key_is_refused() {
    let outer = DocumentKey::generate();
    let (nonce, sealed) = aead::seal(&[7; 16], &Aad::of(b"channel-1"), &outer).unwrap();
    let short = aead::open_document_key(&nonce, &sealed, &Aad::of(b"channel-1"), &outer);
    assert_eq!(short.err(), Some(CryptoError::InvalidKeyLength));
}
