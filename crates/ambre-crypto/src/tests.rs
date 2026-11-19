//! Host tests for the crypto primitives and envelope.

use proptest::prelude::*;

use crate::keys::{DocumentKey, FileKey, MasterSeed};
use crate::{CryptoError, aead, commitment, ecdh, envelope, hash, passphrase};
use ambre_core::{
    Aad, Ciphertext, EntryDigest, FileSalt, FixedBytes, MasterKeySalt, MediaBytes, Nonce, Payload,
    SealedRowBytes,
};

/// A passphrase as a prompt hands it over.
fn pass(text: &str) -> ambre_core::Confidential<ambre_core::Passphrase> {
    ambre_core::Confidential::<ambre_core::Passphrase>::new(text.to_owned())
}

/// A passphrase the strength floor admits: four words of a diceware list.
const STRONG: &str = "anchovy glider ruckus tadpole";

/// The key [`STRONG`] derives to seal a seed under `salt`.
fn sealing_key(salt: [u8; 32]) -> passphrase::SealingKey {
    let chosen = crate::strength::ChosenPassphrase::check(pass(STRONG)).unwrap();
    passphrase::derive_sealing_key(&chosen, &FixedBytes::<MasterKeySalt>::from_array(salt)).unwrap()
}

/// Plaintext bytes as the envelope takes them.
fn secret(bytes: &[u8]) -> ambre_core::Confidential<ambre_core::Plaintext> {
    ambre_core::Confidential::<ambre_core::Plaintext>::new(bytes.to_vec())
}

/// The ciphertext with the byte at `at` XORed by `mask`, as an attacker
/// tampers with it on the wire. An index past the end, or a zero mask,
/// changes no byte and fails the test. A tamper test never passes on an
/// untouched ciphertext.
fn flipped(ciphertext: &Payload<Ciphertext>, at: usize, mask: u8) -> Payload<Ciphertext> {
    let mut bytes = ciphertext.as_bytes().to_vec();
    if let Some(byte) = bytes.get_mut(at) {
        *byte ^= mask;
    }
    assert_ne!(bytes, ciphertext.as_bytes(), "the flip changed no byte");
    Payload::<Ciphertext>::new(bytes)
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
    let author =
        FixedBytes::<ambre_core::PublicKey>::from_array(signing.verifying_key().to_bytes());
    let aad = &Aad::of(b"public-data");
    let plaintext = b"hello, encrypted world";

    let sealed = envelope::seal(&secret(plaintext), aad, &doc_key, &signing).unwrap();
    let opened = envelope::open(&sealed, aad, &doc_key, &author).unwrap();
    assert_eq!(opened.into_plaintext(), plaintext);
}

/// A media seal is the envelope of any other plaintext: it opens through
/// [`envelope::open`] and fails under another AAD.
#[test]
fn a_media_seal_opens_as_any_envelope() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let author =
        FixedBytes::<ambre_core::PublicKey>::from_array(signing.verifying_key().to_bytes());
    let aad = &Aad::of(b"public-data");
    let media = Payload::<MediaBytes>::new(b"\x89PNG a picture".to_vec());

    let sealed = envelope::seal_media(&media, aad, &doc_key, &signing).unwrap();
    let opened = envelope::open(&sealed, aad, &doc_key, &author).unwrap();
    assert_eq!(opened.into_plaintext(), media.as_bytes());
    assert!(envelope::open(&sealed, &Aad::of(b"other"), &doc_key, &author).is_err());
}

/// `ZK-017`: the debug form of an envelope prints its length and none of its
/// bytes, so a `{:?}` in a log carries no payload.
#[test]
fn an_envelope_debug_form_prints_no_byte_of_it() {
    let signing = fixed_seed().derive_signing_key();
    let mut sealed = envelope::seal(
        &secret(b"payload"),
        &Aad::of(b"aad"),
        &fixed_doc_key(),
        &signing,
    )
    .unwrap();
    // Bytes no length or field name can spell, so a match is a leak.
    sealed.ciphertext = Payload::<Ciphertext>::new(vec![0xAB; 7]);
    sealed.nonce = FixedBytes::from_array([0xCD; aead::NONCE_LEN]);
    let shown = format!("{sealed:?}");
    assert_eq!(shown, "Sealed { ciphertext_len: 7, .. }");
}

#[test]
fn tampered_ciphertext_fails() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let author =
        FixedBytes::<ambre_core::PublicKey>::from_array(signing.verifying_key().to_bytes());
    let aad = &Aad::of(b"aad");

    let mut sealed = envelope::seal(&secret(b"payload"), aad, &doc_key, &signing).unwrap();
    sealed.ciphertext = flipped(&sealed.ciphertext, 0, 0xFF);
    // Signature covers the ciphertext, so this trips the signature check first.
    assert!(matches!(
        envelope::open(&sealed, aad, &doc_key, &author),
        Err(CryptoError::InvalidSignature)
    ));
}

#[test]
fn commitment_tag_wrong_aad_rejected() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let author =
        FixedBytes::<ambre_core::PublicKey>::from_array(signing.verifying_key().to_bytes());

    let sealed = envelope::seal(
        &secret(b"payload"),
        &Aad::of(b"document-A"),
        &doc_key,
        &signing,
    )
    .unwrap();
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
    assert!(matches!(
        envelope::open(&forged, forged_aad, &doc_key, &author),
        Err(CryptoError::CommitmentMismatch)
    ));
}

/// Every seal draws its own 192-bit nonce (`SYNC-003`). One payload sealed
/// many times under one key never repeats a nonce, nor a ciphertext.
#[test]
fn every_seal_draws_a_fresh_nonce() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let sealed: Vec<_> = (0..64)
        .map(|_| {
            envelope::seal(
                &secret(b"same payload"),
                &Aad::of(b"aad"),
                &doc_key,
                &signing,
            )
            .unwrap()
        })
        .collect();
    let nonces: std::collections::HashSet<FixedBytes<Nonce>> =
        sealed.iter().map(|s| s.nonce).collect();
    let ciphertexts: std::collections::HashSet<&[u8]> =
        sealed.iter().map(|s| s.ciphertext.as_bytes()).collect();
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
    let author =
        FixedBytes::<ambre_core::PublicKey>::from_array(signing.verifying_key().to_bytes());
    let aad = &Aad::of(b"aad");
    let mut forged = envelope::seal(&secret(b"payload"), aad, &other_key, &signing).unwrap();
    forged.ciphertext = flipped(&forged.ciphertext, 0, 0xFF);
    let resign = |sealed: &mut envelope::Sealed| {
        sealed.signature = signature::sign(
            &signing,
            &sealed.nonce,
            &sealed.ciphertext,
            aad,
            &sealed.commitment_tag,
        );
    };

    assert!(matches!(
        envelope::open(&forged, aad, &doc_key, &author),
        Err(CryptoError::InvalidSignature)
    ));
    resign(&mut forged);
    assert!(matches!(
        envelope::open(&forged, aad, &doc_key, &author),
        Err(CryptoError::CommitmentMismatch)
    ));
    forged.commitment_tag =
        commitment::commitment_tag(&doc_key, &forged.nonce, &forged.ciphertext, aad);
    resign(&mut forged);
    assert!(matches!(
        envelope::open(&forged, aad, &doc_key, &author),
        Err(CryptoError::AeadFailed)
    ));
}

#[test]
fn invalid_signature_rejected() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let aad = &Aad::of(b"aad");

    let sealed = envelope::seal(&secret(b"payload"), aad, &doc_key, &signing).unwrap();
    let wrong_author = FixedBytes::<ambre_core::PublicKey>::from_array(
        MasterSeed::from_array([0x99; 32])
            .derive_signing_key()
            .verifying_key()
            .to_bytes(),
    );
    assert!(matches!(
        envelope::open(&sealed, aad, &doc_key, &wrong_author),
        Err(CryptoError::InvalidSignature)
    ));
}

#[test]
fn wrong_document_key_rejected() {
    let doc_key = fixed_doc_key();
    let signing = fixed_seed().derive_signing_key();
    let author =
        FixedBytes::<ambre_core::PublicKey>::from_array(signing.verifying_key().to_bytes());
    let aad = &Aad::of(b"aad");

    let sealed = envelope::seal(&secret(b"payload"), aad, &doc_key, &signing).unwrap();
    let other_key = DocumentKey::from_array([0x44; 32]);
    assert!(matches!(
        envelope::open(&sealed, aad, &other_key, &author),
        Err(CryptoError::CommitmentMismatch)
    ));
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

/// The recovery phrase spells the seed in BIP39 words, and prints nothing.
#[test]
fn the_recovery_words_spell_the_seed_in_bip39_words() {
    let words = MasterSeed::from_array([0xa7; 32]).recovery_words();
    assert_eq!(words.secret().expose().split(' ').count(), 24);
    assert_eq!(format!("{words:?}"), "RecoveryWords(Secret([REDACTED]))");
}

/// A row opens under its own key and its own place, and under nothing else.
#[test]
fn a_store_row_opens_only_where_it_was_sealed() {
    let key = fixed_seed().derive_store_key();
    let row = aead::seal_row(
        &ambre_core::Confidential::<ambre_core::Plaintext>::new(b"a copy".to_vec()),
        &Aad::of(b"doc:0a"),
        &key,
    )
    .unwrap();
    assert_eq!(
        row.as_bytes().first(),
        Some(&aead::RowLayout::CURRENT.byte())
    );
    assert!(!row.as_bytes().windows(6).any(|w| w == b"a copy"));
    assert_eq!(
        aead::open_row(&row, &Aad::of(b"doc:0a"), &key)
            .unwrap()
            .into_plaintext(),
        b"a copy"
    );

    let stranger = MasterSeed::from_array([0x33; 32]).derive_store_key();
    assert!(aead::open_row(&row, &Aad::of(b"doc:0a"), &stranger).is_err());
    assert!(aead::open_row(&row, &Aad::of(b"doc:0b"), &key).is_err());
    let mut flipped = row.clone().into_bytes();
    *flipped.last_mut().expect("a sealed row holds bytes") ^= 1;
    let flipped = Payload::<SealedRowBytes>::new(flipped);
    assert!(aead::open_row(&flipped, &Aad::of(b"doc:0a"), &key).is_err());
    // A row the store held in the clear, or a stub, opens as nothing.
    for stub in [
        b"a copy".to_vec(),
        Vec::new(),
        vec![aead::RowLayout::CURRENT.byte()],
    ] {
        let stub = Payload::<SealedRowBytes>::new(stub);
        assert!(aead::open_row(&stub, &Aad::of(b"doc:0a"), &key).is_err());
    }
}

// -- ECDH --------------------------------------------------------------------

#[test]
fn ecdh_symmetric() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let alice_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&alice).to_bytes(),
    );
    let bob_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&bob).to_bytes(),
    );

    assert_eq!(
        ecdh::shared_secret(&alice, &bob_pub).unwrap().as_bytes(),
        ecdh::shared_secret(&bob, &alice_pub).unwrap().as_bytes()
    );
}

/// The secret comes from the exchange: another peer gives another secret. A
/// constant secret would let anyone holding the two public keys derive the
/// key-encryption key, yet every round trip would still pass.
#[test]
fn a_shared_secret_differs_per_peer() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let public = |seed: u8| {
        FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
            x25519_dalek::PublicKey::from(
                &MasterSeed::from_array([seed; 32]).derive_exchange_key(),
            )
            .to_bytes(),
        )
    };
    let with_bob = ecdh::shared_secret(&alice, &public(0x02)).unwrap();
    let with_carol = ecdh::shared_secret(&alice, &public(0x03)).unwrap();
    assert_ne!(with_bob.as_bytes(), with_carol.as_bytes());
}

#[test]
fn a_shared_secret_is_wiped_and_never_printed() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let bob_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&bob).to_bytes(),
    );
    let shared = ecdh::shared_secret(&alice, &bob_pub).unwrap();
    // The bytes sit in an `ambre_core::Secret`, which wipes them on drop.
    assert_eq!(format!("{shared:?}"), "SharedSecret(Secret([REDACTED]))");
}

#[test]
fn wrap_unwrap_roundtrip() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let alice_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&alice).to_bytes(),
    );
    let bob_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&bob).to_bytes(),
    );

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
    let alice_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&alice).to_bytes(),
    );
    let bob_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&bob).to_bytes(),
    );
    let doc_key = DocumentKey::from_array([0x55; 32]);

    let anchored = ecdh::wrap_anchored(
        &alice,
        &bob_pub,
        &doc_key,
        Some(&FixedBytes::<EntryDigest>::from_array([0x66; 32])),
    )
    .unwrap();
    let (key, anchor) = ecdh::unwrap_anchored(&bob, &alice_pub, &anchored).unwrap();
    assert_eq!(key.as_bytes(), doc_key.as_bytes());
    assert_eq!(
        anchor,
        Some(FixedBytes::<EntryDigest>::from_array([0x66; 32]))
    );
    let key = ecdh::unwrap_document_key(&bob, &alice_pub, &anchored).unwrap();
    assert_eq!(key.as_bytes(), doc_key.as_bytes());

    let bare = ecdh::wrap_document_key(&alice, &bob_pub, &doc_key).unwrap();
    let (_, anchor) = ecdh::unwrap_anchored(&bob, &alice_pub, &bare).unwrap();
    assert_eq!(anchor, None);

    // A flipped byte of the sealed anchor fails the tag: nobody swaps it.
    let mut tampered = anchored;
    tampered.ciphertext = flipped(&tampered.ciphertext, 40, 1);
    assert!(ecdh::unwrap_anchored(&bob, &alice_pub, &tampered).is_err());
}

/// Holds `SYNC-264` at the primitive. A wrapper wraps many keys for one
/// recipient from one exchange. Each wrap opens to its key and its anchor
/// under the plain unwrap, as a fresh wrap does.
#[test]
fn a_held_pair_key_wraps_as_a_fresh_exchange_does() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let alice_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&alice).to_bytes(),
    );
    let members: Vec<_> = (2u8..5)
        .map(|byte| MasterSeed::from_array([byte; 32]).derive_exchange_key())
        .collect();
    let anchor = FixedBytes::<EntryDigest>::from_array([0x66; 32]);
    let mut wrapper = ecdh::KeyWrapper::new(&alice);
    assert_eq!(wrapper.own_public(), alice_pub);
    for byte in [0x10u8, 0x20, 0x30] {
        let doc_key = DocumentKey::from_array([byte; 32]);
        for member in &members {
            let member_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
                x25519_dalek::PublicKey::from(member).to_bytes(),
            );
            let wrapped = wrapper.wrap(&member_pub, &doc_key, Some(&anchor)).unwrap();
            let (key, read) = ecdh::unwrap_anchored(member, &alice_pub, &wrapped).unwrap();
            assert_eq!(key.as_bytes(), doc_key.as_bytes());
            assert_eq!(read, Some(anchor));
        }
    }
    // Three keys for three members: one exchange per member.
    assert_eq!(wrapper.recipients().get(), 3);
    assert!(!format!("{wrapper:?}").contains("Secret"));
}

/// The debug form of a wrapper names its own public key and its count of
/// recipients, the two fields it may print.
#[test]
fn the_debug_form_of_a_wrapper_names_its_public_key_and_recipients() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
    let bob_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&bob).to_bytes(),
    );
    let mut wrapper = ecdh::KeyWrapper::new(&alice);
    let doc_key = DocumentKey::from_array([0x10; 32]);
    wrapper.wrap(&bob_pub, &doc_key, None).unwrap();
    assert_eq!(
        format!("{wrapper:?}"),
        format!(
            "KeyWrapper {{ own_pub: {:?}, recipients: 1, .. }}",
            wrapper.own_public()
        )
    );
}

/// Holds `SYNC-264` on a refusal. A low-order recipient is refused on each
/// wrap, and the wrapper holds no key for it.
#[test]
fn a_wrapper_holds_no_key_for_a_refused_recipient() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let low_order = FixedBytes::<ambre_core::ExchangePublicKey>::from_array([0; 32]);
    let mut wrapper = ecdh::KeyWrapper::new(&alice);
    let doc_key = DocumentKey::from_array([0x55; 32]);
    for _ in 0..2 {
        assert!(matches!(
            wrapper.wrap(&low_order, &doc_key, None),
            Err(CryptoError::WeakPublicKey)
        ));
    }
    assert_eq!(wrapper.recipients().get(), 0);
}

/// Holds `SRV-226` against the primitives. Each length the wire reads for a
/// wrapped key is the length a wrap writes. A change of layout fails here,
/// rather than at every relay read.
#[test]
fn a_wrap_has_a_length_the_wire_reads() {
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let bob_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&MasterSeed::from_array([0x02; 32]).derive_exchange_key())
            .to_bytes(),
    );
    let doc_key = DocumentKey::from_array([0x55; 32]);
    let length = |wrapped: ecdh::WrappedKey| {
        wrapped.nonce.as_slice().len() + wrapped.ciphertext.len().saturating_len().to_usize()
    };
    let bare = length(ecdh::wrap_document_key(&alice, &bob_pub, &doc_key).unwrap());
    let anchored = length(
        ecdh::wrap_anchored(
            &alice,
            &bob_pub,
            &doc_key,
            Some(&FixedBytes::<EntryDigest>::from_array([0x66; 32])),
        )
        .unwrap(),
    );
    assert_eq!(
        ambre_core::Encoded::<ambre_core::WrappedDocKey>::LENGTHS,
        [bare, anchored]
    );

    let seed = MasterSeed::from_array([0x03; 32]);
    let wrapped = seed
        .wrap_for_storage(
            &sealing_key([0xAB; 32]),
            &FixedBytes::<MasterKeySalt>::from_array([0xAB; 32]),
        )
        .unwrap();
    assert_eq!(
        ambre_core::Encoded::<ambre_core::WrappedMasterKey>::LENGTHS,
        [legacy_seal().as_bytes().len(), wrapped.as_bytes().len()]
    );
}

/// The salt the stored-seed tests derive under, and its sealing key.
const SEED_SALT: [u8; 32] = [0xAB; 32];

/// The signing key `seed` derives: the account its seal binds.
fn account_of(seed: &MasterSeed) -> FixedBytes<ambre_core::PublicKey> {
    FixedBytes::from_array(seed.derive_signing_key().verifying_key().to_bytes())
}

/// The seed `MasterSeed::from_array([0x03; 32])` sealed as a client sealed
/// it before the version byte existed: a nonce and a ciphertext under the
/// fixed additional data.
fn legacy_seal() -> ambre_core::Encoded<ambre_core::WrappedMasterKey> {
    let (nonce, ciphertext) = aead::seal(
        &[0x03; 32],
        &Aad::of(crate::keys::LEGACY_SEED_AAD),
        &sealing_key(SEED_SALT).document_key(),
    )
    .unwrap();
    let mut wrapped = nonce.as_slice().to_vec();
    wrapped.extend_from_slice(ciphertext.as_bytes());
    ambre_core::Encoded::new(wrapped)
}

/// `ZK-046`: a stored seed opens under its user key, its salt and its
/// account alone. The login path opens the seed this way, and a wrong
/// passphrase gives the AEAD error, which the login screen reads as
/// "incorrect passphrase".
#[test]
fn a_stored_seed_opens_under_its_user_key_its_salt_and_its_account_alone() {
    let salt = FixedBytes::<MasterKeySalt>::from_array(SEED_SALT);
    let user = passphrase::derive_user_key(&pass(STRONG), &salt).unwrap();
    let seed = MasterSeed::from_array([0x03; 32]);
    let account = account_of(&seed);
    let wrapped = seed
        .wrap_for_storage(&sealing_key(SEED_SALT), &salt)
        .unwrap();
    assert_eq!(
        wrapped.as_bytes().first(),
        Some(&crate::keys::SeedVersion::V1.byte())
    );

    let opened = MasterSeed::open_from_storage(&wrapped, &user, &salt, Some(&account)).unwrap();
    assert_eq!(
        opened.recovery_words().secret().expose(),
        seed.recovery_words().secret().expose()
    );

    let wrong = passphrase::derive_user_key(&pass("wrong passphrase"), &salt).unwrap();
    assert!(matches!(
        MasterSeed::open_from_storage(&wrapped, &wrong, &salt, Some(&account)),
        Err(CryptoError::AeadFailed)
    ));
    // Another salt beside the same blob: the AEAD refuses before any key.
    assert!(matches!(
        MasterSeed::open_from_storage(
            &wrapped,
            &user,
            &FixedBytes::<MasterKeySalt>::from_array([0xCD; 32]),
            Some(&account)
        ),
        Err(CryptoError::AeadFailed)
    ));
    // The seal served as the seed of another account.
    let other = account_of(&MasterSeed::from_array([0x04; 32]));
    assert!(matches!(
        MasterSeed::open_from_storage(&wrapped, &user, &salt, Some(&other)),
        Err(CryptoError::AeadFailed)
    ));
    assert!(matches!(
        MasterSeed::open_from_storage(&wrapped, &user, &salt, None),
        Err(CryptoError::UnboundSeed)
    ));
    assert!(matches!(
        MasterSeed::open_from_storage(
            &ambre_core::Encoded::new(vec![0; 10]),
            &user,
            &salt,
            Some(&account)
        ),
        Err(CryptoError::InvalidKeyLength)
    ));
}

/// `ZK-046`: the version byte sits inside the additional data. So a blob
/// whose byte is changed, or stripped to the old layout, does not open.
#[test]
fn a_sealed_seed_whose_version_byte_moves_does_not_open() {
    let salt = FixedBytes::<MasterKeySalt>::from_array(SEED_SALT);
    let user = passphrase::derive_user_key(&pass(STRONG), &salt).unwrap();
    let seed = MasterSeed::from_array([0x03; 32]);
    let account = account_of(&seed);
    let wrapped = seed
        .wrap_for_storage(&sealing_key(SEED_SALT), &salt)
        .unwrap();
    let mut bumped = wrapped.as_bytes().to_vec();
    if let Some(version) = bumped.first_mut() {
        *version = version.wrapping_add(1);
    }
    assert!(
        MasterSeed::open_from_storage(
            &ambre_core::Encoded::new(bumped),
            &user,
            &salt,
            Some(&account)
        )
        .is_err()
    );
    let stripped = wrapped.as_bytes().get(1..).unwrap().to_vec();
    assert!(matches!(
        MasterSeed::open_from_storage(
            &ambre_core::Encoded::new(stripped),
            &user,
            &salt,
            Some(&account)
        ),
        Err(CryptoError::AeadFailed)
    ));
}

/// `ZK-046`: a seed sealed before the version byte existed still opens, with
/// or without the account beside it.
#[test]
fn a_seed_sealed_before_the_version_byte_still_opens() {
    let salt = FixedBytes::<MasterKeySalt>::from_array(SEED_SALT);
    let user = passphrase::derive_user_key(&pass(STRONG), &salt).unwrap();
    let seed = MasterSeed::from_array([0x03; 32]);
    let legacy = legacy_seal();
    for account in [None, Some(account_of(&seed))] {
        let opened =
            MasterSeed::open_from_storage(&legacy, &user, &salt, account.as_ref()).unwrap();
        assert_eq!(
            opened.recovery_words().secret().expose(),
            seed.recovery_words().secret().expose()
        );
    }
    let wrong = passphrase::derive_user_key(&pass("wrong passphrase"), &salt).unwrap();
    assert!(matches!(
        MasterSeed::open_from_storage(&legacy, &wrong, &salt, None),
        Err(CryptoError::AeadFailed)
    ));
}

#[test]
fn weak_peer_public_key_rejected() {
    // The all-zero u-coordinate is a low-order point: the X25519 output is the
    // predictable all-zero secret. Both the raw exchange and the wrap must reject
    // it (M14: reject a non-contributory shared secret).
    let alice = MasterSeed::from_array([0x01; 32]).derive_exchange_key();
    let weak_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array([0u8; 32]);

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
    let alice_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&alice).to_bytes(),
    );
    let bob_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        x25519_dalek::PublicKey::from(&bob).to_bytes(),
    );

    let doc_key = DocumentKey::from_array([0x55; 32]);
    let wrapped = ecdh::wrap_document_key(&alice, &bob_pub, &doc_key).unwrap();

    // Carol tries to unwrap Bob's envelope, claiming Alice as the sender.
    assert!(ecdh::unwrap_document_key(&carol, &alice_pub, &wrapped).is_err());
}

// -- Argon2id ----------------------------------------------------------------

#[test]
fn argon2_deterministic() {
    let salt = FixedBytes::<MasterKeySalt>::from_array([0xAB; 32]);
    let k1 = passphrase::derive_user_key(&pass("correct horse"), &salt).unwrap();
    let k2 = passphrase::derive_user_key(&pass("correct horse"), &salt).unwrap();
    assert_eq!(k1.as_bytes(), k2.as_bytes());

    let other_salt = FixedBytes::<MasterKeySalt>::from_array([0xCD; 32]);
    let k3 = passphrase::derive_user_key(&pass("correct horse"), &other_salt).unwrap();
    assert_ne!(k1.as_bytes(), k3.as_bytes());

    let k4 = passphrase::derive_user_key(&pass("wrong passphrase"), &salt).unwrap();
    assert_ne!(k1.as_bytes(), k4.as_bytes());
}

// -- BLAKE2b chain -----------------------------------------------------------

#[test]
fn blake2_chain_hash() {
    let snapshot = Payload::<Ciphertext>::new(b"snapshot".to_vec());
    let genesis = hash::chain_hash(None, &snapshot);
    assert_eq!(genesis, hash::chain_hash(None, &snapshot));
    // The parent is part of the link: a different parent breaks the chain.
    let child = hash::chain_hash(Some(&genesis), &snapshot);
    assert_ne!(child, genesis);
    let other = hash::chain_hash(None, &Payload::<Ciphertext>::new(b"snapshoT".to_vec()));
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
    let file = ambre_core::Payload::<ambre_core::PickedContainer>::new(b"file".to_vec());
    assert_eq!(hash::file_digest(&file).as_array()[..], full[..32]);
    assert_eq!(
        hash::chain_hash(None, &Payload::<Ciphertext>::new(b"file".to_vec())).as_array(),
        hash::file_digest(&file).as_array()
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
        [0x11; 32][..FixedBytes::<ambre_core::KeyId>::SIZE
            .saturating_len()
            .to_usize()]
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
    let author_pub =
        FixedBytes::<ambre_core::PublicKey>::from_array(author.verifying_key().to_bytes());

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
    let stranger_pub =
        FixedBytes::<ambre_core::PublicKey>::from_array(stranger.verifying_key().to_bytes());
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

    let picture = Payload::<MediaBytes>::new(b"the same picture, used twice".to_vec());
    let address = commitment::media_address(&fixed_doc_key(), &picture);
    assert_eq!(
        address,
        commitment::media_address(&fixed_doc_key(), &picture)
    );
    assert_ne!(
        address,
        commitment::media_address(&DocumentKey::from_array([0x12; 32]), &picture)
    );
    let another = Payload::<MediaBytes>::new(b"another picture".to_vec());
    assert_ne!(
        address,
        commitment::media_address(&fixed_doc_key(), &another)
    );
    let plain = Blake2b512::digest(picture.as_bytes());
    assert_ne!(address.as_array()[..], plain[..32]);
}

// -- Property tests ----------------------------------------------------------

proptest! {
    /// `SYNC-264`: for any two keys wrapped in turn for one recipient, the
    /// second wrap, made from the held pair key, opens to its own key.
    #[test]
    fn prop_a_held_pair_key_opens_under_the_plain_unwrap(
        first in any::<[u8; 32]>(),
        second in any::<[u8; 32]>(),
        anchor in any::<[u8; 32]>(),
    ) {
        let alice = fixed_seed().derive_exchange_key();
        let alice_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
            x25519_dalek::PublicKey::from(&alice).to_bytes(),
        );
        let bob = MasterSeed::from_array([0x02; 32]).derive_exchange_key();
        let bob_pub = FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
            x25519_dalek::PublicKey::from(&bob).to_bytes(),
        );
        let anchor = FixedBytes::<EntryDigest>::from_array(anchor);
        let mut wrapper = ecdh::KeyWrapper::new(&alice);
        wrapper.wrap(&bob_pub, &DocumentKey::from_array(first), None).unwrap();
        let wrapped = wrapper
            .wrap(&bob_pub, &DocumentKey::from_array(second), Some(&anchor))
            .unwrap();
        let (key, read) = ecdh::unwrap_anchored(&bob, &alice_pub, &wrapped).unwrap();
        prop_assert_eq!(key.as_bytes(), &second);
        prop_assert_eq!(read, Some(anchor));
    }

    #[test]
    fn prop_roundtrip_encrypt_decrypt(data in any::<Vec<u8>>(), key in any::<[u8; 32]>()) {
        let doc_key = DocumentKey::from_array(key);
        let signing = fixed_seed().derive_signing_key();
        let author = FixedBytes::<ambre_core::PublicKey>::from_array(signing.verifying_key().to_bytes());
        let aad = &Aad::of(b"aad");
        let sealed = envelope::seal(&secret(&data), aad, &doc_key, &signing).unwrap();
        let opened = envelope::open(&sealed, aad, &doc_key, &author).unwrap();
        prop_assert_eq!(opened.into_plaintext(), data);
    }

    #[test]
    fn prop_single_byte_mutation_fails(
        data in proptest::collection::vec(any::<u8>(), 1..256),
        idx in 0usize..3,
        bit in 0u8..8,
    ) {
        let doc_key = fixed_doc_key();
        let signing = fixed_seed().derive_signing_key();
        let author = FixedBytes::<ambre_core::PublicKey>::from_array(signing.verifying_key().to_bytes());
        let aad = &Aad::of(b"aad");
        let mut sealed = envelope::seal(&secret(&data), aad, &doc_key, &signing).unwrap();
        let mask = 1u8 << bit;
        match idx {
            0 => sealed.ciphertext = flipped(&sealed.ciphertext, 0, mask),
            1 => {
                let mut tag = *sealed.commitment_tag.as_array();
                tag[0] ^= mask;
                sealed.commitment_tag = FixedBytes::from_array(tag);
            }
            _ => {
                let mut signature = *sealed.signature.as_array();
                signature[0] ^= mask;
                sealed.signature = FixedBytes::from_array(signature);
            }
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
    let fingerprint = hash::fingerprint(&FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        [0u8; 32],
    ));
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
        hash::fingerprint(&FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
            [0u8; 32]
        ))
        .as_str()
    );
}

proptest! {
    /// The hand-written base32 of the fingerprint spells what the RFC 4648
    /// encoder of `data-encoding` spells, grouped by 4, for any key.
    #[test]
    fn fingerprint_matches_the_reference_base32(key in proptest::array::uniform32(any::<u8>())) {
        use blake2::{Blake2b512, Digest};
        let mut hasher = Blake2b512::new();
        hasher.update(b"ambre-fingerprint-v1");
        hasher.update(key);
        let digest = hasher.finalize();
        let reference = data_encoding::BASE32_NOPAD.encode(digest.get(..20).unwrap_or_default());
        let grouped: Vec<String> = reference
            .as_bytes()
            .chunks(4)
            .map(|group| String::from_utf8_lossy(group).into_owned())
            .collect();
        let fingerprint =
            hash::fingerprint(&FixedBytes::<ambre_core::ExchangePublicKey>::from_array(key));
        prop_assert_eq!(fingerprint.as_str(), grouped.join(" "));
    }
}

#[test]
fn fingerprint_distinguishes_keys() {
    // A single-bit difference in the key must change the fingerprint (the whole
    // point of pinning it): near-identical substituted keys stay distinguishable.
    let a = hash::fingerprint(&FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        [0u8; 32],
    ));
    let mut key_b = [0u8; 32];
    key_b[0] = 1;
    let b = hash::fingerprint(&FixedBytes::<ambre_core::ExchangePublicKey>::from_array(
        key_b,
    ));
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
        &FixedBytes::<MasterKeySalt>::from_array([0xAB; 32]),
    )
    .unwrap();
    let file = FileKey::from_passphrase(
        &pass("correct horse"),
        &FixedBytes::<FileSalt>::from_array([0xAB; 16]),
    )
    .unwrap();
    assert_ne!(file.as_bytes(), user.as_bytes());
}

#[test]
fn a_file_key_is_deterministic_and_per_salt() {
    // Deterministic, or nothing reopens the file; per salt, or two files
    // sealed under one passphrase share a key.
    let salt = FixedBytes::<FileSalt>::from_array([0xAB; 16]);
    let first = FileKey::from_passphrase(&pass("correct horse"), &salt).unwrap();
    let again = FileKey::from_passphrase(&pass("correct horse"), &salt).unwrap();
    assert_eq!(first.as_bytes(), again.as_bytes());

    let elsewhere = FileKey::from_passphrase(
        &pass("correct horse"),
        &FixedBytes::<FileSalt>::from_array([0xCD; 16]),
    )
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
    let salt = FixedBytes::<FileSalt>::from_array([0x01; 16]);
    let aad = &Aad::of(b"the-sealed-header");
    let plaintext = b"a whole plaintext container";
    let key = FileKey::from_passphrase(&pass("open sesame"), &salt).unwrap();
    let sealed = ambre_core::Confidential::<ambre_core::Plaintext>::new(plaintext.to_vec());
    let (nonce, ciphertext) = aead::seal_file(&sealed, aad, &key).unwrap();
    assert_eq!(
        aead::open_file(&nonce, &ciphertext, aad, &key)
            .unwrap()
            .into_plaintext(),
        plaintext
    );

    // A wrong passphrase is a wrong key, and Poly1305 is what says so. Nothing
    // cheaper sits upstream of it: no password verifier, no checksum. The tag
    // is the whole of "that passphrase does not open this file".
    let wrong = FileKey::from_passphrase(&pass("open sesa"), &salt).unwrap();
    assert!(matches!(
        aead::open_file(&nonce, &ciphertext, aad, &wrong),
        Err(CryptoError::AeadFailed)
    ));

    // And the seal binds the header it covered: an edited salt, nonce or
    // lineage byte fails the tag rather than deriving quietly.
    assert!(matches!(
        aead::open_file(&nonce, &ciphertext, &Aad::of(b"the-sealed-headeR"), &key),
        Err(CryptoError::AeadFailed)
    ));
}

#[test]
fn the_sealed_header_widths_match_the_primitives() {
    // `ambre-core` lays the sealed header out and names no primitive
    // (`DOCFILE-006`), so it declares its two widths independently of the
    // cipher and the KDF. This is the seam where a mismatch
    // would surface, and it is the only place both sides are nameable at once.
    assert_eq!(ambre_client_core::docfile::SEAL_NONCE_LEN, aead::NONCE_LEN);
    assert_eq!(
        ambre_client_core::docfile::SALT_LEN,
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
        aead::open(&n, &ct, &Aad::of(b""), &opened)
            .unwrap()
            .into_plaintext(),
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
        Some(&FixedBytes::<EntryDigest>::from_array([0x33; 32])),
    )
    .unwrap();
    let (key, anchor) =
        aead::open_anchored_key(&nonce, &sealed, &Aad::of(b"channel-1"), &outer).unwrap();
    assert_eq!(key.as_bytes(), inner.as_bytes());
    assert_eq!(
        anchor,
        Some(FixedBytes::<EntryDigest>::from_array([0x33; 32]))
    );

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

// -- Byte goldens ------------------------------------------------------------
//
// Each value below comes from an independent implementation: Python's `hmac`,
// `hashlib.blake2b` and the `cryptography` Ed25519 signer, over the inputs
// written here. A refactor of a construction keeps every byte it writes, or
// one of these fails. The relay re-implements the signed transcript, so its
// bytes are pinned by value as well as by round trip.

/// Decodes a lowercase hex golden.
fn golden(hex: &str) -> Vec<u8> {
    data_encoding::HEXLOWER.decode(hex.as_bytes()).unwrap()
}

/// The inputs every golden of the envelope shares.
fn golden_envelope() -> (FixedBytes<Nonce>, Payload<Ciphertext>, Aad) {
    (
        FixedBytes::from_array([0x07; 24]),
        Payload::<Ciphertext>::new(b"a sealed body".to_vec()),
        Aad::of(b"public-data"),
    )
}

#[test]
fn the_commitment_tag_keeps_its_bytes() {
    let (nonce, ciphertext, aad) = golden_envelope();
    let tag = commitment::commitment_tag(&fixed_doc_key(), &nonce, &ciphertext, &aad);
    assert_eq!(
        tag.as_slice().to_vec(),
        golden("bbb7a3f12f49c5ac3bb02be16a06d657ce026587d7c01053d69444d2f30ce622")
    );
}

#[test]
fn the_media_address_keeps_its_bytes() {
    let picture = Payload::<MediaBytes>::new(b"a picture".to_vec());
    let address = commitment::media_address(&fixed_doc_key(), &picture);
    let expected: [u8; 32] =
        golden("c72a97882edaf9922202a868bba98c10b9d8391391ee833ce679124602fd0f98")
            .try_into()
            .unwrap();
    assert_eq!(
        address,
        FixedBytes::<ambre_core::MediaAddress>::from_array(expected)
    );
}

#[test]
fn the_envelope_signature_keeps_its_bytes() {
    let (nonce, ciphertext, aad) = golden_envelope();
    let tag = commitment::commitment_tag(&fixed_doc_key(), &nonce, &ciphertext, &aad);
    let signing = ed25519_dalek::SigningKey::from_bytes(&[0x33; 32]);
    let signature = crate::signature::sign(&signing, &nonce, &ciphertext, &aad, &tag);
    assert_eq!(
        signature.as_slice().to_vec(),
        golden(
            "ceb6340011299611cb720434c2ccbc24e44497db42b827870a1876a1110be99f\
             6100b0be0749510adc17b1b1fae731151e64e813b05fedb8a105a1be3bab9909"
        )
    );
}

#[test]
fn the_chain_hash_and_the_keyless_digests_keep_their_bytes() {
    let (_, snapshot, _) = golden_envelope();
    let parent = FixedBytes::<ambre_core::ChainHash>::from_array([0x44; 32]);
    assert_eq!(
        hash::chain_hash(Some(&parent), &snapshot)
            .as_slice()
            .to_vec(),
        golden("e02ef1c5d7a3f1836f76be11227c66f52f0fc2f9d5beef621ea870e716d53186")
    );
    let headless = golden("997889dc0af01e9390d3155a446f283600e018a8cccc32c15085956fe8ef4a79");
    assert_eq!(
        hash::chain_hash(None, &snapshot).as_slice().to_vec(),
        headless
    );
    let file =
        ambre_core::Payload::<ambre_core::PickedContainer>::new(snapshot.as_bytes().to_vec());
    assert_eq!(hash::file_digest(&file).as_array().to_vec(), headless);
}

#[test]
fn the_dev_seed_keeps_its_bytes() {
    let seed: [u8; 32] = golden("31824b2cddf4df243951a9a2e3e4f6ae90373fc78543354bc52f6643631426f2")
        .try_into()
        .unwrap();
    assert_eq!(
        hash::dev_seed(&ambre_core::Validated::<ambre_core::DisplayName>::parse("alice").unwrap())
            .derive_signing_key()
            .verifying_key(),
        MasterSeed::from_array(seed)
            .derive_signing_key()
            .verifying_key()
    );
}

#[test]
fn an_old_passphrase_derives_the_key_it_always_did() {
    // Pinned against the Argon2id v1 parameters before they took a type. A
    // cost that moved by one byte would lock every account out, and every
    // sealed file.
    let user = passphrase::derive_user_key(
        &pass("correct horse"),
        &FixedBytes::<MasterKeySalt>::from_array([0xAB; 32]),
    )
    .unwrap();
    assert_eq!(
        user.as_bytes().to_vec(),
        golden("2d10bdb7ee0406ce92f33836f189f2527051e4ccd9666dec855d4dfa3bcebdfd")
    );
    let file = FileKey::from_passphrase(
        &pass("correct horse"),
        &FixedBytes::<FileSalt>::from_array([0xAB; 16]),
    )
    .unwrap();
    assert_eq!(
        file.as_bytes().to_vec(),
        golden("3aa4da2953c8cdf77f2c64d721d4926ec230466b9bf2e9bd670d8ac793ce5ebe")
    );
}

#[test]
fn a_row_the_old_code_sealed_still_opens() {
    // A row sealed by the build that wrote `RowLayout::V1` as a bare byte:
    // the layout byte, the nonce, then the ciphertext. A store a user holds
    // today keeps rows of this shape, so every later build opens them.
    let row = Payload::<SealedRowBytes>::new(golden(
        "01a11182bb03cf28fbbeb25b10c25f4d53019867ee3101ff1ca1d3e4b463358f16a8df15edbd570d3cfd0c5ac5fdcb",
    ));
    let key = fixed_seed().derive_store_key();
    assert_eq!(
        aead::open_row(&row, &Aad::of(b"doc:0a"), &key)
            .unwrap()
            .into_plaintext(),
        b"a copy"
    );
}

/// The module digest is BLAKE2b with a 32-byte output: the value Python's
/// `hashlib.blake2b(data, digest_size=32)` gives, which the build script
/// writes (`scripts/converter_build.py`, `ARCH-080`).
#[test]
fn the_module_digest_is_blake2b_256() {
    assert_eq!(
        crate::hash::module_digest(&ambre_core::Payload::<ambre_client_core::FileBytes>::new(
            b"abc".to_vec()
        ))
        .to_hex(),
        "bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319"
    );
}

// -- The pin an invite link carries (`SYNC-205`) -----------------------------

#[test]
fn an_inviter_pin_is_stable_and_differs_per_key() {
    let key = |first: u8| {
        let mut bytes = [0u8; 32];
        bytes[0] = first;
        FixedBytes::<ambre_core::PublicKey>::from_array(bytes)
    };
    let pin = hash::inviter_pin(&key(0));
    // A link sent yesterday must still check today: the value is frozen.
    assert_eq!(pin, hash::inviter_pin(&key(0)));
    assert_ne!(pin, hash::inviter_pin(&key(1)));
    // The domain tag keeps a pin apart from a bare digest of the same key.
    let bare = <blake2::Blake2b<blake2::digest::consts::U32> as blake2::Digest>::digest([0u8; 32]);
    assert_ne!(pin.as_slice(), bare.as_slice());
}
