//! The chunk format, the replay window, the wraps and the signed join of a call.

use ambre_core::id::{Channel, Id, Meeting, User};
use ambre_core::{
    Authored, ClearChunk, Instant, KeyGeneration, KeyIdentifier, Payload, PublicKey, SealedChunk,
    SenderIndex, Seq, Track,
};
use ed25519_dalek::SigningKey;
use proptest::prelude::*;

use super::{
    AUDIO_CHUNK_SIZE, CallExchangeKey, ChunkError, ChunkHeader, ChunkSealer, JoinClaim,
    ReceiveKeys, SenderKey, SignedJoin, WrapBinding, unwrap_sender_key, wrap_sender_key,
};

fn kid(sender: u64, generation: u64) -> KeyIdentifier {
    KeyIdentifier {
        track: Track::Audio,
        sender: Seq::new(sender),
        generation: Seq::new(generation),
    }
}

fn clear(bytes: &[u8]) -> Payload<ClearChunk> {
    Payload::new(bytes.to_vec())
}

fn sealer(key: &SenderKey, sender: u64, generation: u64) -> ChunkSealer {
    ChunkSealer::new(key, kid(sender, generation)).unwrap_or_else(|e| panic!("{e}"))
}

fn receiver(sender: u64, generation: u64, key: &SenderKey) -> ReceiveKeys {
    let mut keys = ReceiveKeys::default();
    keys.insert(Seq::new(sender), Seq::new(generation), key.clone());
    keys
}

fn seal(sealer: &mut ChunkSealer, bytes: &[u8]) -> Payload<SealedChunk> {
    sealer.seal(&clear(bytes)).unwrap_or_else(|e| panic!("{e}"))
}

fn copy(chunk: &Payload<SealedChunk>) -> Payload<SealedChunk> {
    Payload::new(chunk.as_bytes().to_vec())
}

/// The header spells the key identifier and the counter at fixed places. The SFU and the
/// reviewer of the format read them without a key.
#[test]
fn the_header_of_a_chunk_stays_pinned() {
    let key = SenderKey::generate();
    let mut sealer = sealer(&key, 5, (3 << 32) + 2);
    let chunk = seal(&mut sealer, b"opus");
    let bytes = chunk.as_bytes();
    let expected_kid: u64 = (((3u64 << 32) + 2) << 16) | (5 << 2);
    assert_eq!(bytes.first(), Some(&0xFF));
    assert_eq!(bytes.get(1..9), Some(&expected_kid.to_be_bytes()[..]));
    assert_eq!(bytes.get(9..17), Some(&1u64.to_be_bytes()[..]));
    let header = ChunkHeader::read_audio(&chunk).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(header.kid, kid(5, (3 << 32) + 2));
    assert_eq!(header.counter, Seq::new(1));
}

/// A chunk the sender sealed opens at the receiver, whole and attributed to its sender.
#[test]
fn a_sealed_audio_chunk_opens_whole_under_its_sender_key() {
    let key = SenderKey::generate();
    let mut sealer = sealer(&key, 1, 0);
    let mut keys = receiver(1, 0, &key);
    let (sender, opened) = keys
        .open_audio(seal(&mut sealer, b"a packet of opus"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(sender, Seq::<SenderIndex>::new(1));
    assert_eq!(opened.as_bytes(), b"a packet of opus");
}

/// A video track seals nothing until the clear prefix rule lands with the video story.
#[test]
fn a_video_track_is_not_sealed_yet() {
    let key = SenderKey::generate();
    let camera = KeyIdentifier {
        track: Track::Camera,
        ..kid(1, 0)
    };
    assert!(matches!(
        ChunkSealer::new(&key, camera),
        Err(ChunkError::TrackNotCarried)
    ));
}

/// A clear chunk past the fixed size is refused, so no chunk of another size leaves.
#[test]
fn a_clear_chunk_past_the_fixed_size_is_refused() {
    let key = SenderKey::generate();
    let mut sealer = sealer(&key, 1, 0);
    assert_eq!(
        sealer.seal(&clear(&[7; 161])),
        Err(ChunkError::ChunkTooLarge)
    );
}

/// Two calls of the generator never give one key, and a ratchet gives another key. The
/// receiver of the newer key opens nothing of the older.
#[test]
fn a_sender_key_is_drawn_fresh_and_a_ratchet_hides_the_key_before_it() {
    let first = SenderKey::generate();
    let second = SenderKey::generate();
    assert_ne!(first.bytes(), second.bytes());
    let next = first.ratcheted();
    assert_ne!(next.bytes(), first.bytes());
    let mut sealer = sealer(&first, 1, 0);
    let mut keys = receiver(1, 1, &next);
    assert_eq!(
        keys.open_audio(seal(&mut sealer, b"before")),
        Err(ChunkError::UnknownKey)
    );
}

/// A receiver holding one generation derives the next ones of the same epoch, and no key
/// of the next epoch.
#[test]
fn a_receiver_ratchets_up_inside_an_epoch_alone() {
    let key = SenderKey::generate();
    let later = key.ratcheted().ratcheted().ratcheted();
    let mut keys = receiver(1, 0, &key);
    let mut ahead = sealer(&later, 1, 3);
    assert!(keys.open_audio(seal(&mut ahead, b"three up")).is_ok());
    let fresh_epoch = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    let mut fresh = sealer(&key, 1, fresh_epoch);
    assert_eq!(
        keys.open_audio(seal(&mut fresh, b"next epoch")),
        Err(ChunkError::UnknownKey)
    );
}

/// The same chunk twice, and a chunk behind the window, are dropped; a late chunk inside
/// the window still opens (`CALL-026`).
#[test]
fn a_chunk_outside_the_replay_window_is_dropped() {
    let key = SenderKey::generate();
    let mut sealer = sealer(&key, 1, 0);
    let mut keys = receiver(1, 0, &key);
    let first = seal(&mut sealer, b"first");
    let late = seal(&mut sealer, b"late");
    assert!(keys.open_audio(copy(&first)).is_ok());
    assert_eq!(keys.open_audio(copy(&first)), Err(ChunkError::Replayed));
    let chunks: Vec<_> = (0..200).map(|_| seal(&mut sealer, b"x")).collect();
    for chunk in chunks.into_iter().skip(10) {
        assert!(keys.open_audio(chunk).is_ok());
    }
    assert_eq!(keys.open_audio(late), Err(ChunkError::Replayed));
    let mut inside = Vec::new();
    for _ in 0..4 {
        inside.push(seal(&mut sealer, b"y"));
    }
    inside.reverse();
    for chunk in inside {
        assert!(keys.open_audio(chunk).is_ok());
    }
}

/// A forged chunk moves no window: the genuine chunk of that counter still opens.
#[test]
fn a_chunk_that_does_not_open_moves_no_window() {
    let key = SenderKey::generate();
    let mut sealer = sealer(&key, 1, 0);
    let mut keys = receiver(1, 0, &key);
    let genuine = seal(&mut sealer, b"genuine");
    let mut forged = genuine.as_bytes().to_vec();
    if let Some(last) = forged.last_mut() {
        *last ^= 1;
    }
    assert_eq!(
        keys.open_audio(Payload::new(forged)),
        Err(ChunkError::Unopened)
    );
    assert!(keys.open_audio(genuine).is_ok());
}

/// Mallory seals under her own key a chunk whose identifier names Bob, or relabels a
/// chunk of hers as Bob's: the receiver drops both (`CALL-025`).
#[test]
fn a_chunk_that_names_another_sender_is_dropped() {
    let bob = SenderKey::generate();
    let mallory = SenderKey::generate();
    let mut keys = ReceiveKeys::default();
    keys.insert(Seq::new(1), Seq::ZERO, bob.clone());
    keys.insert(Seq::new(2), Seq::ZERO, mallory.clone());
    let mut as_bob = sealer(&mallory, 1, 0);
    assert_eq!(
        keys.open_audio(seal(&mut as_bob, b"forged")),
        Err(ChunkError::Unopened)
    );
    let mut own = sealer(&mallory, 2, 0);
    let mut relabelled = seal(&mut own, b"relabelled").into_bytes();
    let bob_kid: u64 = 1 << 2;
    if let Some(field) = relabelled.get_mut(1..9) {
        field.copy_from_slice(&bob_kid.to_be_bytes());
    }
    assert_eq!(
        keys.open_audio(Payload::new(relabelled)),
        Err(ChunkError::Unopened)
    );
    let mut genuine = sealer(&bob, 1, 0);
    assert_eq!(
        keys.open_audio(seal(&mut genuine, b"bob")).map(|(s, _)| s),
        Ok(Seq::new(1))
    );
}

proptest! {
    /// Every audio chunk has one size, whatever the length of its clear chunk (`CALL-007`).
    #[test]
    fn every_audio_chunk_has_one_size(lengths in prop::collection::vec(0usize..=160, 1..40)) {
        let key = SenderKey::generate();
        let mut sealer = sealer(&key, 3, 7);
        let mut keys = receiver(3, 7, &key);
        for length in lengths {
            let bytes = vec![0xA5; length];
            let chunk = seal(&mut sealer, &bytes);
            prop_assert_eq!(chunk.len(), AUDIO_CHUNK_SIZE);
            let (_, opened) = keys.open_audio(chunk).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(opened.as_bytes(), &bytes[..]);
        }
    }

    /// One sealer never writes one counter twice, and its counters go up (`CALL-005`).
    #[test]
    fn one_key_never_seals_two_chunks_under_one_counter(count in 1usize..300) {
        let key = SenderKey::generate();
        let mut sealer = sealer(&key, 1, 0);
        let mut last = Seq::ZERO;
        for _ in 0..count {
            let chunk = seal(&mut sealer, b"x");
            let header = ChunkHeader::read_audio(&chunk).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert!(header.counter > last);
            last = header.counter;
        }
    }

    /// A change of any byte of a chunk drops it (`CALL-024`).
    #[test]
    fn a_changed_byte_drops_the_chunk(position in 0usize..195, flip in 1u8..=255) {
        let key = SenderKey::generate();
        let mut sealer = sealer(&key, 1, 0);
        let mut keys = receiver(1, 0, &key);
        let mut bytes = seal(&mut sealer, b"some opus").into_bytes();
        if let Some(byte) = bytes.get_mut(position) {
            *byte ^= flip;
        }
        prop_assert!(keys.open_audio(Payload::new(bytes)).is_err());
    }
}

fn binding(sender: u64, recipient: u64) -> WrapBinding {
    WrapBinding {
        call: Id::<Meeting>::from_bytes([1; 16]),
        sender: Seq::new(sender),
        recipient: Seq::new(recipient),
    }
}

/// A wrap opens for its recipient alone, under its own binding alone, and carries the
/// generation and the key.
#[test]
fn a_wrap_opens_for_its_recipient_and_binding_alone() {
    let alice = CallExchangeKey::generate();
    let bob = CallExchangeKey::generate();
    let carol = CallExchangeKey::generate();
    let key = SenderKey::generate();
    let generation = Seq::<KeyGeneration>::new(9);
    let wrapped = wrap_sender_key(&alice, &bob.public(), binding(1, 2), generation, &key)
        .unwrap_or_else(|e| panic!("{e}"));
    let (opened_generation, opened) =
        unwrap_sender_key(&bob, &alice.public(), binding(1, 2), &wrapped)
            .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(opened_generation, generation);
    assert_eq!(opened.bytes(), key.bytes());
    assert!(unwrap_sender_key(&carol, &alice.public(), binding(1, 2), &wrapped).is_err());
    assert!(unwrap_sender_key(&bob, &alice.public(), binding(1, 3), &wrapped).is_err());
    assert!(unwrap_sender_key(&bob, &alice.public(), binding(4, 2), &wrapped).is_err());
}

fn claim(identity: &SigningKey) -> JoinClaim {
    JoinClaim {
        call: Id::<Meeting>::from_bytes([1; 16]),
        channel: Id::<Channel>::from_bytes([2; 16]),
        member: Id::<User>::from_bytes([3; 16]),
        identity: ambre_core::FixedBytes::<PublicKey>::from_array(
            identity.verifying_key().to_bytes(),
        ),
        at: Instant::<Authored>::new(1_700_000_000_000),
        sender: Seq::new(4),
        exchange: CallExchangeKey::generate().public(),
    }
}

/// A join verifies under the identity it names, and a changed field or another signer
/// refuses it.
#[test]
fn a_signed_join_verifies_under_its_identity_alone() {
    let identity = SigningKey::from_bytes(&[5; 32]);
    let signed = SignedJoin::sign(&identity, claim(&identity)).unwrap_or_else(|e| panic!("{e}"));
    assert!(signed.verified().is_ok());
    let moved = signed.with_claim(JoinClaim {
        sender: Seq::new(5),
        ..claim(&identity)
    });
    assert!(moved.verified().is_err());
    let other = SigningKey::from_bytes(&[6; 32]);
    assert!(SignedJoin::sign(&other, claim(&identity)).is_err());
}

/// The bytes one known key seals match a second path written from RFC 9605 alone, and
/// stay pinned. A change of the key schedule, the nonce, the padding or the additional
/// data changes every chunk a client can open.
#[test]
fn the_sealed_bytes_of_a_known_key_follow_rfc_9605_and_stay_pinned() {
    use chacha20poly1305::aead::{Aead, Payload as Parts};
    use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
    use hkdf::Hkdf;
    use sha2::Sha256;

    let key = SenderKey(std::sync::Arc::new(ambre_core::Secret::new([7; 32])));
    let mut sealer = sealer(&key, 5, 2);
    let chunk = seal(&mut sealer, b"opus");

    let kid: u64 = (2 << 16) | (5 << 2);
    let counter: u64 = 1;
    let hk = Hkdf::<Sha256>::new(Some(&[]), &[7; 32]);
    let info = |label: &[u8]| [label, &kid.to_be_bytes(), &0xF001u16.to_be_bytes()].concat();
    let mut sframe_key = [0u8; 32];
    let mut sframe_salt = [0u8; 12];
    assert!(
        hk.expand(&info(b"SFrame 1.0 Secret key "), &mut sframe_key)
            .is_ok()
    );
    assert!(
        hk.expand(&info(b"SFrame 1.0 Secret salt "), &mut sframe_salt)
            .is_ok()
    );
    let mut nonce = sframe_salt;
    for (n, c) in nonce.iter_mut().skip(4).zip(counter.to_be_bytes()) {
        *n ^= c;
    }
    let header = [&[0xFFu8][..], &kid.to_be_bytes(), &counter.to_be_bytes()].concat();
    let mut plaintext = [&4u16.to_be_bytes()[..], b"opus"].concat();
    plaintext.resize(162, 0);
    let sealed = ChaCha20Poly1305::new(&sframe_key.into())
        .encrypt(
            &nonce.into(),
            Parts {
                msg: &plaintext,
                aad: &header,
            },
        )
        .unwrap_or_default();
    assert_eq!(chunk.as_bytes(), [header, sealed].concat());

    let hex: String = chunk
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(hex, PINNED);
}

/// The chunk of the test above, as the format first sealed it on 2026-11-09.
const PINNED: &str = "ff000000000002001400000000000000018d1bc4980b3910326d7f20e7ed68a8191c363f375c15501c96fce349eb85596f834b8c636462bb34493d51e700e1e6efd3b3dc364ef2a300b2c4488f9ba0c66d7d9d5a37764a5e8b47d8319bcfc623a2674813fc6756ca57ca0251d2cbc97205e425e6f8d820f34afa025256f96c3d032e88d62f03018ef7f93571a26915c99cab9703a9535261c64b3a9aef442071bbe79077915e635ccd7c0bd55dbc72d3284d30cc39431e7a10f58fac8a89559621e842";
