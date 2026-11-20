//! Ties the ProVerif model of the call sender keys to the code (`CALL-142`).
//!
//! The model `proofs/call_sender_keys.pv` names each domain label of the code on a `LABEL`
//! line. Each test reads those lines and recomputes one derivation of the code under the
//! label bytes of the model. The code must derive the same bytes. A label changed on one
//! side alone fails here, so the proof never holds for a design the code left.
//!
//! The model is read at compile time, so the test runs wherever the crate builds. It runs on
//! the host alone, as every test of this crate outside `phone_bench.rs` does.
#![cfg(not(target_arch = "wasm32"))]

use ambre_core::id::{Call, Channel, Id, User};
use ambre_core::{
    Authored, ClearChunk, Confidential, FixedBytes, Instant, KeyIdentifier, Payload, Plaintext,
    SealedChunk, Seq, Track,
};
use ambre_crypto::call::{
    CallExchangeKey, ChunkSealer, JoinClaim, KeyHandoff, SenderKey, SignedJoin, WrapBinding,
    WrappedSenderKey, unwrap_sender_key,
};
use chacha20poly1305::aead::{Aead, AeadInPlace, Payload as AeadPayload};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce, XChaCha20Poly1305, XNonce};
use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey as ExchangePoint, StaticSecret};

/// The model, as CI proves it.
const MODEL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proofs/call_sender_keys.pv"
));

/// The labels the model must name, one for each domain of the code.
const LABELS: [&str; 5] = [
    "label_ratchet",
    "label_join",
    "label_wrap",
    "label_sframe_key",
    "label_sframe_salt",
];

/// The private suite of RFC 9605 that the model names `suite` (`ambre-spec-call-media.md
/// section 3`, the key schedule).
const SUITE: u16 = 0xF001;

/// The plaintext of a sealed audio chunk: the length, the clear chunk, then zeros
/// (`ambre-spec-call-media.md section 3`, the chunk format).
const AUDIO_BODY_LEN: usize = 162;

/// The sender key every test seals under.
const SENDER_KEY: [u8; 32] = [0x2A; 32];

/// The bytes of the `LABEL` line of `name`: the text between its two quotes.
fn label(name: &str) -> Option<Vec<u8>> {
    MODEL.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("LABEL ")?;
        let (named, text) = rest.split_once(' ')?;
        let text = text.trim().strip_prefix('"')?.strip_suffix('"')?;
        (named == name).then(|| text.as_bytes().to_vec())
    })
}

/// The names of every `LABEL` line of the model.
fn named_labels() -> Vec<&'static str> {
    MODEL
        .lines()
        .filter_map(|line| line.trim().strip_prefix("LABEL ")?.split(' ').next())
        .collect()
}

/// The sender key `bytes`, through the one door that builds a key from bytes: the port of
/// the Sync worker.
fn sender_key(bytes: &[u8; 32]) -> Option<SenderKey> {
    let mut port = vec![0u8];
    port.extend_from_slice(&1u64.to_be_bytes());
    port.extend_from_slice(&0u64.to_be_bytes());
    port.extend_from_slice(bytes);
    match KeyHandoff::from_port(&Confidential::<Plaintext>::new(port)).ok()? {
        KeyHandoff::Seal { key, .. } => Some(key),
        _ => None,
    }
}

/// HKDF-SHA256 with an empty salt over `ikm`, expanded under `info` to `N` bytes.
fn expand<const N: usize>(ikm: &[u8], info: &[u8]) -> Option<[u8; N]> {
    let mut out = [0u8; N];
    Hkdf::<Sha256>::new(None, ikm).expand(info, &mut out).ok()?;
    Some(out)
}

/// The audio chunk of `clear` at `counter`, sealed under `sender_key` for `kid`. It follows the
/// key schedule of RFC 9605, under the two SFrame labels of the model.
fn sframe_audio_chunk(
    sender_key: &[u8; 32],
    kid: &[u8],
    counter: u64,
    clear: &[u8],
) -> Option<Vec<u8>> {
    let info = |name: &str| {
        label(name).map(|label| [label.as_slice(), kid, &SUITE.to_be_bytes()].concat())
    };
    let key: [u8; 32] = expand(sender_key, &info("label_sframe_key")?)?;
    let mut nonce: [u8; 12] = expand(sender_key, &info("label_sframe_salt")?)?;
    for (byte, count) in nonce.iter_mut().skip(4).zip(counter.to_be_bytes()) {
        *byte ^= count;
    }
    let mut chunk = vec![0xFF];
    chunk.extend_from_slice(kid);
    chunk.extend_from_slice(&counter.to_be_bytes());
    let header_len = chunk.len();
    chunk.extend_from_slice(&u16::try_from(clear.len()).ok()?.to_be_bytes());
    chunk.extend_from_slice(clear);
    chunk.resize(header_len + AUDIO_BODY_LEN, 0);
    let (aad, body) = chunk.split_at_mut(header_len);
    let tag = ChaCha20Poly1305::new(Key::from_slice(&key))
        .encrypt_in_place_detached(Nonce::from_slice(&nonce), aad, body)
        .ok()?;
    chunk.extend_from_slice(&tag);
    Some(chunk)
}

/// The key identifier of the audio track of sender 3 at `generation`.
fn audio_kid(generation: u64) -> KeyIdentifier {
    KeyIdentifier {
        track: Track::Audio,
        sender: Seq::new(3),
        generation: Seq::new(generation),
    }
}

/// Seals `clear` under `key` for `kid` through the code, and answers the chunk and its
/// counter.
fn sealed_by_the_code(key: &SenderKey, kid: KeyIdentifier, clear: &[u8]) -> Option<(Vec<u8>, u64)> {
    let mut sealer = ChunkSealer::new(key, kid).ok()?;
    let sealed: Payload<SealedChunk> = sealer
        .seal(&Payload::<ClearChunk>::new(clear.to_vec()))
        .ok()?;
    let bytes = sealed.as_bytes().to_vec();
    let counter = u64::from_be_bytes(bytes.get(9..17)?.try_into().ok()?);
    Some((bytes, counter))
}

/// The model names the five labels, declares each as a constant and uses each in a
/// derivation. A label dropped from the model, or never used there, fails.
#[test]
fn the_model_declares_and_uses_each_label_of_the_code() {
    let mut named = named_labels();
    named.sort_unstable();
    let mut expected = LABELS.to_vec();
    expected.sort_unstable();
    assert_eq!(named, expected);
    for name in LABELS {
        let declaration = format!("const {name}: bitstring.");
        assert!(MODEL.contains(&declaration), "{name} is not declared");
        let uses = MODEL
            .lines()
            .filter(|line| !line.contains(&declaration) && !line.trim().starts_with("LABEL "))
            .filter(|line| line.contains(name))
            .count();
        assert!(uses > 0, "{name} is never used");
    }
}

/// A chunk the code seals is the chunk the two SFrame labels of the model derive.
#[test]
fn a_chunk_seals_under_the_sframe_labels_of_the_model() {
    let key = sender_key(&SENDER_KEY).expect("the port builds the key");
    let kid = audio_kid(0);
    let (sealed, counter) = sealed_by_the_code(&key, kid, b"opus").expect("the code seals");
    let spelled = kid.to_bytes().expect("the identifier fits");
    let expected = sframe_audio_chunk(&SENDER_KEY, spelled.as_slice(), counter, b"opus")
        .expect("the model labels derive");
    assert_eq!(sealed, expected);
}

/// A ratchet of the code is the key the ratchet label of the model derives.
#[test]
fn a_ratchet_derives_under_the_ratchet_label_of_the_model() {
    let key = sender_key(&SENDER_KEY).expect("the port builds the key");
    let ratcheted: [u8; 32] = expand(
        &SENDER_KEY,
        &label("label_ratchet").expect("the model names the ratchet label"),
    )
    .expect("HKDF expands 32 bytes");
    let kid = audio_kid(1);
    let (sealed, counter) =
        sealed_by_the_code(&key.ratcheted(), kid, b"opus").expect("the code seals");
    let spelled = kid.to_bytes().expect("the identifier fits");
    let expected = sframe_audio_chunk(&ratcheted, spelled.as_slice(), counter, b"opus")
        .expect("the model labels derive");
    assert_eq!(sealed, expected);
}

/// A wrap built under the wrap label of the model, as the key of the KDF and as the
/// additional data, opens through the code.
#[test]
fn a_wrap_opens_under_the_wrap_label_of_the_model() {
    let wrap_label = label("label_wrap").expect("the model names the wrap label");
    let recipient = CallExchangeKey::generate();
    let recipient_public = recipient.public();
    let sender = StaticSecret::from([7u8; 32]);
    let sender_public = ExchangePoint::from(&sender);
    let shared = sender.diffie_hellman(&ExchangePoint::from(*recipient_public.as_array()));
    let binding = WrapBinding {
        call: Id::<Call>::from_bytes([1; 16]),
        sender: Seq::new(2),
        recipient: Seq::new(3),
    };
    let info = [
        wrap_label.as_slice(),
        binding.call.as_bytes(),
        &2u64.to_be_bytes(),
        &3u64.to_be_bytes(),
        sender_public.as_bytes(),
        recipient_public.as_slice(),
    ]
    .concat();
    let kek: [u8; 32] = expand(shared.as_bytes(), &info).expect("HKDF expands 32 bytes");
    // The generation, the key, then no departure in sixteen zero slots (`CALL-157`).
    let plaintext = [&9u64.to_be_bytes()[..], &SENDER_KEY, &[0u8; 1 + 8 * 16]].concat();
    let nonce = [5u8; 24];
    let ciphertext = XChaCha20Poly1305::new(Key::from_slice(&kek))
        .encrypt(
            XNonce::from_slice(&nonce),
            AeadPayload {
                msg: &plaintext,
                aad: &wrap_label,
            },
        )
        .expect("the wrap seals");
    let wrapped =
        WrappedSenderKey::from_bytes(&Payload::new([&nonce[..], ciphertext.as_slice()].concat()))
            .expect("a wrap is 209 bytes");
    let opened = unwrap_sender_key(
        &recipient,
        &FixedBytes::from_array(sender_public.to_bytes()),
        binding,
        &wrapped,
    )
    .expect("the code opens the wrap of the model");
    let key = opened.key;
    assert_eq!(opened.generation.get(), 9);
    let kid = audio_kid(9);
    let (sealed, counter) = sealed_by_the_code(&key, kid, b"opus").expect("the code seals");
    let spelled = kid.to_bytes().expect("the identifier fits");
    let expected = sframe_audio_chunk(&SENDER_KEY, spelled.as_slice(), counter, b"opus")
        .expect("the model labels derive");
    assert_eq!(sealed, expected);
}

/// The signature of a join the code signs covers the join label of the model, then the
/// fields of the claim.
#[test]
fn a_signed_join_covers_the_join_label_of_the_model() {
    let join_label = label("label_join").expect("the model names the join label");
    let identity = SigningKey::from_bytes(&[4u8; 32]);
    let claim = JoinClaim {
        call: Id::<Call>::from_bytes([1; 16]),
        channel: Id::<Channel>::from_bytes([2; 16]),
        member: Id::<User>::from_bytes([3; 16]),
        identity: FixedBytes::from_array(identity.verifying_key().to_bytes()),
        at: Instant::<Authored>::new(1_700_000_000_000),
        sender: Seq::new(4),
        exchange: CallExchangeKey::generate().public(),
    };
    let signed = SignedJoin::sign(&identity, claim).expect("the identity signs its claim");
    let bytes = signed.to_bytes();
    let bytes = bytes.as_bytes();
    let (fields, signature) = bytes
        .split_at_checked(bytes.len() - 64)
        .expect("a signed join ends on its signature");
    let signature = ed25519_dalek::Signature::from_slice(signature).expect("64 bytes");
    identity
        .verifying_key()
        .verify_strict(&[join_label.as_slice(), fields].concat(), &signature)
        .expect("the signature covers the join label of the model");
}
