//! The chunk format, the replay window, the wraps and the signed join of a call.

use ambre_core::id::{Call, Channel, Id, User};
use ambre_core::{
    Authored, ChunkCounter, ClearChunk, Instant, KeyGeneration, KeyIdentifier, PageLoad, Payload,
    PictureKind, PublicKey, SealedChunk, SenderIndex, Seq, TimeSpan, Track,
};
use ed25519_dalek::SigningKey;
use proptest::prelude::*;

use super::{
    AUDIO_CHUNK_SIZE, CONFIG_BYTE, CallExchangeKey, ChunkError, ChunkHeader, ChunkSealer,
    DEPARTURE_SLOTS, Departures, HEADER_LEN, JoinClaim, KeyHandoff, MAX_RATCHETS,
    OPENERS_PER_SENDER, ReceiveKeys, ReplayWindow, SWITCH_DELAY, SenderKey, SignedJoin, TAG_LEN,
    VIDEO_OVERHEAD, WrapBinding, unwrap_sender_key, wrap_sender_key,
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

/// The time of every receiver test that waits for no deadline.
const T0: Instant<PageLoad> = Instant::new(0.0);

fn receiver(sender: u64, generation: u64, key: &SenderKey) -> ReceiveKeys {
    let mut keys = ReceiveKeys::default();
    keys.insert(Seq::new(sender), Seq::new(generation), key.clone(), T0);
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
        .open_audio(seal(&mut sealer, b"a packet of opus"), T0)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(sender, Seq::<SenderIndex>::new(1));
    assert_eq!(opened.as_bytes(), b"a packet of opus");
}

fn camera(sender: u64, generation: u64) -> KeyIdentifier {
    KeyIdentifier {
        track: Track::Camera,
        ..kid(sender, generation)
    }
}

fn camera_sealer(key: &SenderKey, sender: u64) -> ChunkSealer {
    ChunkSealer::new(key, camera(sender, 0)).unwrap_or_else(|e| panic!("{e}"))
}

/// A VP8 keyframe: a clear keyframe bit, the start code, a size of 1280x720, then a body.
fn keyframe(body: &[u8]) -> Vec<u8> {
    [
        &[0x50, 0x2a, 0x00, 0x9d, 0x01, 0x2a, 0x00, 0x05, 0xd0, 0x02][..],
        body,
    ]
    .concat()
}

/// A VP8 delta chunk: a set keyframe bit, then a body.
fn delta(body: &[u8]) -> Vec<u8> {
    [&[0x31][..], body].concat()
}

fn seal_video(sealer: &mut ChunkSealer, bytes: &[u8], kind: PictureKind) -> Payload<SealedChunk> {
    sealer
        .seal_video(&clear(bytes), kind)
        .unwrap_or_else(|e| panic!("{e}"))
}

/// `CALL-006`: each byte of the clear prefix is covered by the tag. A change that keeps the
/// keyframe bit keeps the length of the prefix, so the chunk parses and the tag refuses it.
#[test]
fn a_changed_byte_of_the_clear_prefix_drops_the_video_chunk() {
    let key = SenderKey::generate();
    let mut sealer = camera_sealer(&key, 1);
    let chunk = seal_video(&mut sealer, &keyframe(b"vp8 body"), PictureKind::Keyframe);
    for at in 0..10 {
        let mut bytes = chunk.as_bytes().to_vec();
        if let Some(byte) = bytes.get_mut(at) {
            // Bit 0 of the first byte is the keyframe bit, which `CALL-098` covers.
            *byte ^= if at == 0 { 0x80 } else { 0x01 };
        }
        let mut keys = receiver(1, 0, &key);
        assert_eq!(
            keys.open_video(Payload::new(bytes), T0),
            Err(ChunkError::Unopened),
            "byte {at} of the prefix"
        );
    }
    let mut keys = receiver(1, 0, &key);
    assert!(keys.open_video(chunk, T0).is_ok());
}

/// `CALL-097`: the call worker seals nothing of a chunk whose keyframe bit and kind differ,
/// nor of a keyframe shorter than its prefix. Neither spends a counter.
#[test]
fn a_vp8_chunk_whose_bit_and_kind_differ_or_shorter_than_its_prefix_seals_nothing() {
    let key = SenderKey::generate();
    let mut sealer = camera_sealer(&key, 1);
    for (bytes, kind) in [
        (keyframe(b"body"), PictureKind::Delta),
        (delta(b"body"), PictureKind::Keyframe),
    ] {
        assert_eq!(
            sealer.seal_video(&clear(&bytes), kind),
            Err(ChunkError::KindMismatch)
        );
    }
    let short = keyframe(b"");
    for length in 0..10 {
        let cut = short.get(..length).unwrap_or_default();
        assert_eq!(
            sealer.seal_video(&clear(cut), PictureKind::Keyframe),
            Err(ChunkError::Malformed),
            "a keyframe of {length} bytes"
        );
    }
    let first = seal_video(&mut sealer, &delta(b""), PictureKind::Delta);
    assert_eq!(first.as_bytes().get(10..18), Some(&1u64.to_be_bytes()[..]));
    // An audio sealer takes no video chunk, and a camera sealer no audio chunk.
    assert_eq!(
        sealer.seal(&clear(b"opus")),
        Err(ChunkError::TrackNotCarried)
    );
    let mut audio = sealer_of_audio(&key);
    assert_eq!(
        audio.seal_video(&clear(&delta(b"")), PictureKind::Delta),
        Err(ChunkError::TrackNotCarried)
    );
}

fn sealer_of_audio(key: &SenderKey) -> ChunkSealer {
    sealer(key, 1, 0)
}

/// `CALL-098`: a receiver reads the length of the prefix from the keyframe bit. A bit flipped
/// on the way moves the header, and a chunk cut inside its prefix or its header is malformed.
/// The audio door and the video door each refuse the chunk of the other.
#[test]
fn a_sealed_vp8_chunk_whose_prefix_is_another_length_is_dropped() {
    let key = SenderKey::generate();
    let mut sealer = camera_sealer(&key, 1);
    let key_chunk = seal_video(&mut sealer, &keyframe(b"body"), PictureKind::Keyframe);
    let delta_chunk = seal_video(&mut sealer, &delta(b"body"), PictureKind::Delta);
    for chunk in [&key_chunk, &delta_chunk] {
        let mut flipped = chunk.as_bytes().to_vec();
        if let Some(first) = flipped.first_mut() {
            *first ^= 1;
        }
        let mut keys = receiver(1, 0, &key);
        assert!(keys.open_video(Payload::new(flipped), T0).is_err());
        for length in [0, 1, 9, 26] {
            let cut = chunk.as_bytes().get(..length).unwrap_or_default().to_vec();
            assert_eq!(
                keys.open_video(Payload::new(cut), T0),
                Err(ChunkError::Malformed),
                "a chunk cut at {length} bytes"
            );
        }
    }
    let mut keys = receiver(1, 0, &key);
    assert!(keys.open_audio(copy(&delta_chunk), T0).is_err());
    let mut audio = sealer_of_audio(&key);
    assert!(keys.open_video(seal(&mut audio, b"\x31opus"), T0).is_err());
    assert!(keys.open_video(key_chunk, T0).is_ok());
    assert!(keys.open_video(delta_chunk, T0).is_ok());
}

/// The bytes one known key seals of a VP8 keyframe match a second path written from RFC 9605
/// alone. The clear prefix leads the header in the additional data (`CALL-006`).
#[test]
fn the_sealed_bytes_of_a_vp8_keyframe_follow_rfc_9605_with_the_prefix_in_the_additional_data() {
    use chacha20poly1305::aead::{Aead, Payload as Parts};
    use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
    use hkdf::Hkdf;
    use sha2::Sha256;

    let key = SenderKey(std::sync::Arc::new(ambre_core::Secret::new([7; 32])));
    let mut sealer = ChunkSealer::new(&key, camera(5, 2)).unwrap_or_else(|e| panic!("{e}"));
    let clear = keyframe(b"vp8");
    let chunk = seal_video(&mut sealer, &clear, PictureKind::Keyframe);

    let kid: u64 = (2 << 16) | (5 << 2) | 1;
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
    let (prefix, body) = clear.split_at(10);
    let aad = [
        prefix,
        &[0xFFu8][..],
        &kid.to_be_bytes(),
        &counter.to_be_bytes(),
    ]
    .concat();
    let sealed = ChaCha20Poly1305::new(&sframe_key.into())
        .encrypt(
            &nonce.into(),
            Parts {
                msg: body,
                aad: &aad,
            },
        )
        .unwrap_or_default();
    assert_eq!(chunk.as_bytes(), [aad, sealed].concat());
}

/// The chunks of three encodings share one sealer. A receiver of one encoding opens each of
/// its chunks across the gaps the others leave in the counters.
#[test]
fn a_receiver_of_one_encoding_opens_its_chunks_across_the_counters_of_the_others() {
    let key = SenderKey::generate();
    let mut sealer = camera_sealer(&key, 1);
    let mut keys = receiver(1, 0, &key);
    for picture in 0..60u8 {
        let mut sealed: Vec<_> = (0..3u8)
            .map(|layer| seal_video(&mut sealer, &delta(&[layer, picture]), PictureKind::Delta))
            .collect();
        let low = sealed.swap_remove(0);
        let (_, opened) = keys.open_video(low, T0).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(opened.as_bytes(), delta(&[0, picture]));
    }
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
        keys.open_audio(seal(&mut sealer, b"before"), T0),
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
    assert!(keys.open_audio(seal(&mut ahead, b"three up"), T0).is_ok());
    let fresh_epoch = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    let mut fresh = sealer(&key, 1, fresh_epoch);
    assert_eq!(
        keys.open_audio(seal(&mut fresh, b"next epoch"), T0),
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
    assert!(keys.open_audio(copy(&first), T0).is_ok());
    assert_eq!(keys.open_audio(copy(&first), T0), Err(ChunkError::Replayed));
    let chunks: Vec<_> = (0..200).map(|_| seal(&mut sealer, b"x")).collect();
    for chunk in chunks.into_iter().skip(10) {
        assert!(keys.open_audio(chunk, T0).is_ok());
    }
    assert_eq!(keys.open_audio(late, T0), Err(ChunkError::Replayed));
    let mut inside = Vec::new();
    for _ in 0..4 {
        inside.push(seal(&mut sealer, b"y"));
    }
    inside.reverse();
    for chunk in inside {
        assert!(keys.open_audio(chunk, T0).is_ok());
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
        keys.open_audio(Payload::new(forged), T0),
        Err(ChunkError::Unopened)
    );
    assert!(keys.open_audio(genuine, T0).is_ok());
}

/// Mallory seals under her own key a chunk whose identifier names Bob, or relabels a
/// chunk of hers as Bob's: the receiver drops both (`CALL-025`).
#[test]
fn a_chunk_that_names_another_sender_is_dropped() {
    let bob = SenderKey::generate();
    let mallory = SenderKey::generate();
    let mut keys = ReceiveKeys::default();
    keys.insert(Seq::new(1), Seq::ZERO, bob.clone(), T0);
    keys.insert(Seq::new(2), Seq::ZERO, mallory.clone(), T0);
    let mut as_bob = sealer(&mallory, 1, 0);
    assert_eq!(
        keys.open_audio(seal(&mut as_bob, b"forged"), T0),
        Err(ChunkError::Unopened)
    );
    let mut own = sealer(&mallory, 2, 0);
    let mut relabelled = seal(&mut own, b"relabelled").into_bytes();
    let bob_kid: u64 = 1 << 2;
    if let Some(field) = relabelled.get_mut(1..9) {
        field.copy_from_slice(&bob_kid.to_be_bytes());
    }
    assert_eq!(
        keys.open_audio(Payload::new(relabelled), T0),
        Err(ChunkError::Unopened)
    );
    let mut genuine = sealer(&bob, 1, 0);
    assert_eq!(
        keys.open_audio(seal(&mut genuine, b"bob"), T0)
            .map(|(s, _)| s),
        Ok(Seq::new(1))
    );
}

proptest! {
    /// `CALL-023` and `ZK-028`: the seal leaves the clear prefix of any VP8 chunk unchanged, adds
    /// a fixed overhead alone, and the chunk opens back whole.
    #[test]
    fn the_seal_of_a_vp8_chunk_keeps_its_prefix_and_adds_a_fixed_overhead(
        first in any::<u8>(),
        rest in prop::collection::vec(any::<u8>(), 9..2000),
    ) {
        let key = SenderKey::generate();
        let mut sealer = camera_sealer(&key, 1);
        let mut keys = receiver(1, 0, &key);
        let bytes = [&[first][..], &rest].concat();
        let (kind, prefix) = if first & 1 == 0 {
            (PictureKind::Keyframe, 10)
        } else {
            (PictureKind::Delta, 1)
        };
        let chunk = seal_video(&mut sealer, &bytes, kind);
        prop_assert_eq!(chunk.as_bytes().get(..prefix), bytes.get(..prefix));
        let overhead = usize::try_from(VIDEO_OVERHEAD.get()).unwrap_or(usize::MAX);
        prop_assert_eq!(chunk.as_bytes().len(), bytes.len() + overhead);
        let (sender, opened) = keys.open_video(chunk, T0).unwrap_or_else(|e| panic!("{e}"));
        prop_assert_eq!(sender, Seq::<SenderIndex>::new(1));
        prop_assert_eq!(opened.as_bytes(), &bytes[..]);
    }

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
            let (_, opened) = keys.open_audio(chunk, T0).map_err(|e| TestCaseError::fail(e.to_string()))?;
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
        prop_assert!(keys.open_audio(Payload::new(bytes), T0).is_err());
    }
}

fn binding(sender: u64, recipient: u64) -> WrapBinding {
    WrapBinding {
        call: Id::<Call>::from_bytes([1; 16]),
        sender: Seq::new(sender),
        recipient: Seq::new(recipient),
    }
}

/// A wrap opens for its recipient alone, under its own binding alone, and carries the
/// generation, the departures, the place of the log its sender heeded and the key.
#[test]
fn a_wrap_opens_for_its_recipient_and_binding_alone() {
    let alice = CallExchangeKey::generate();
    let bob = CallExchangeKey::generate();
    let carol = CallExchangeKey::generate();
    let key = SenderKey::generate();
    let generation = Seq::<KeyGeneration>::new(9);
    let mut departures = Departures::default();
    departures.depart(Seq::new(7));
    let wrapped = wrap_sender_key(
        &alice,
        &bob.public(),
        binding(1, 2),
        generation,
        &departures,
        Seq::new(11),
        &key,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let opened = unwrap_sender_key(&bob, &alice.public(), binding(1, 2), &wrapped)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(opened.generation, generation);
    assert_eq!(opened.departures, departures);
    assert_eq!(opened.heeded, Seq::new(11));
    assert_eq!(opened.key.bytes(), key.bytes());
    assert!(unwrap_sender_key(&carol, &alice.public(), binding(1, 2), &wrapped).is_err());
    assert!(unwrap_sender_key(&bob, &alice.public(), binding(1, 3), &wrapped).is_err());
    assert!(unwrap_sender_key(&bob, &alice.public(), binding(4, 2), &wrapped).is_err());
}

fn claim(identity: &SigningKey) -> JoinClaim {
    JoinClaim {
        call: Id::<Call>::from_bytes([1; 16]),
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

/// A signed join reads back whole from the bytes the relay carries, at one size, and still
/// verifies. A changed byte refuses it, and bytes of another length read as no join.
#[test]
fn a_signed_join_reads_back_from_its_bytes_and_a_changed_byte_refuses_it() {
    let identity = SigningKey::from_bytes(&[5; 32]);
    let signed = SignedJoin::sign(&identity, claim(&identity)).unwrap_or_else(|e| panic!("{e}"));
    let bytes = signed.to_bytes();
    assert_eq!(bytes.len().get(), 192);
    let read = SignedJoin::from_bytes(&bytes).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(read, signed);
    assert!(read.verified().is_ok());
    for position in [0, 60, 100, 191] {
        let mut changed = bytes.as_bytes().to_vec();
        if let Some(byte) = changed.get_mut(position) {
            *byte ^= 1;
        }
        let changed = SignedJoin::from_bytes(&Payload::new(changed));
        assert!(changed.is_ok_and(|join| join.verified().is_err()));
    }
    let mut long = bytes.into_bytes();
    long.push(0);
    assert!(SignedJoin::from_bytes(&Payload::new(long)).is_err());
}

/// A wrap reads back from the 217 bytes the relay forwards and opens for its recipient.
/// Bytes of another length read as no wrap.
#[test]
fn a_wrap_reads_back_from_its_bytes() {
    let alice = CallExchangeKey::generate();
    let bob = CallExchangeKey::generate();
    let key = SenderKey::generate();
    let none = Departures::default();
    let wrapped = wrap_sender_key(
        &alice,
        &bob.public(),
        binding(1, 2),
        Seq::new(3),
        &none,
        Seq::ZERO,
        &key,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let bytes = wrapped.to_bytes();
    assert_eq!(bytes.len().get(), 217);
    assert_eq!(wrapped.size().get(), 217);
    let read = super::WrappedSenderKey::from_bytes(&bytes).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(read, wrapped);
    let opened = unwrap_sender_key(&bob, &alice.public(), binding(1, 2), &read)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(opened.key.bytes(), key.bytes());
    let short = Payload::new(bytes.as_bytes().get(..216).unwrap_or_default().to_vec());
    assert!(super::WrappedSenderKey::from_bytes(&short).is_err());
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

/// `CALL-064`: a handoff crosses the port as bytes of one size, and the key it carries opens
/// what the key of the Sync worker sealed. The call worker reads back each kind.
#[test]
fn a_handoff_crosses_the_port_whole() {
    let key = SenderKey::generate();
    let mut sealer = sealer(&key, 4, 2);
    let chunk = seal(&mut sealer, b"opus over the port");
    let handoffs = [
        KeyHandoff::Open {
            sender: Seq::new(4),
            generation: Seq::new(2),
            key: key.clone(),
        },
        KeyHandoff::Seal {
            sender: Seq::new(4),
            generation: Seq::new(2),
            key: key.clone(),
        },
        KeyHandoff::Forget {
            sender: Seq::new(4),
        },
        KeyHandoff::End,
        KeyHandoff::Close {
            sender: Seq::new(4),
            generation: Seq::new(2),
            within: SWITCH_DELAY,
        },
    ];
    let sizes: Vec<usize> = handoffs
        .iter()
        .map(|h| h.to_port().with_plaintext(<[u8]>::len))
        .collect();
    assert_eq!(sizes, [49, 49, 49, 49, 49]);
    let read: Vec<KeyHandoff> = handoffs
        .iter()
        .map(|h| KeyHandoff::from_port(&h.to_port()).unwrap_or_else(|e| panic!("{e}")))
        .collect();
    let [
        KeyHandoff::Open {
            sender,
            generation,
            key: opened_key,
        },
        KeyHandoff::Seal { key: sealing, .. },
        KeyHandoff::Forget { sender: gone },
        KeyHandoff::End,
        KeyHandoff::Close {
            sender: closed,
            generation: through,
            within,
        },
    ] = read.as_slice()
    else {
        panic!("the kinds came back as {read:?}");
    };
    assert_eq!(
        (*sender, *generation, *gone, *closed, *through),
        (
            Seq::new(4),
            Seq::new(2),
            Seq::new(4),
            Seq::new(4),
            Seq::new(2)
        )
    );
    assert_eq!(*within, SWITCH_DELAY);
    let mut keys = ReceiveKeys::default();
    keys.insert(*sender, *generation, opened_key.clone(), T0);
    let (_, clear) = keys.open_audio(chunk, T0).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(clear.as_bytes(), b"opus over the port");
    let mut resealer = ChunkSealer::new(sealing, kid(4, 2)).unwrap_or_else(|e| panic!("{e}"));
    let mut fresh = receiver(4, 2, &key);
    assert!(fresh.open_audio(seal(&mut resealer, b"again"), T0).is_ok());
}

/// Bytes of another size, or of a kind no handoff holds, build no key.
#[test]
fn a_malformed_handoff_builds_no_key() {
    let end = KeyHandoff::End.to_port().into_plaintext();
    let mut unknown = end.clone();
    if let Some(kind) = unknown.first_mut() {
        *kind = 9;
    }
    for bytes in [unknown, end[..48].to_vec(), [end.clone(), vec![0]].concat()] {
        let port = ambre_core::Confidential::<ambre_core::Plaintext>::new(bytes);
        assert!(KeyHandoff::from_port(&port).is_err());
    }
}

/// `CALL-067`: a 20 ms Opus frame at the constant bitrate of a call fits the clear part of a
/// sealed audio chunk. So the seal drops none at that bitrate.
#[test]
fn a_frame_at_the_constant_audio_bitrate_fits_a_chunk() {
    let frame = u64::from(ambre_core::AUDIO_BITRATE.get()) * 20 / 1000 / 8;
    assert!(
        frame <= super::AUDIO_CLEAR_CAP as u64,
        "{frame} bytes per 20 ms"
    );
}

fn screen_sealer(key: &SenderKey, sender: u64) -> ChunkSealer {
    let screen = KeyIdentifier {
        track: Track::Screen,
        ..kid(sender, 0)
    };
    ChunkSealer::new(key, screen).unwrap_or_else(|e| panic!("{e}"))
}

/// `CALL-102`: a chunk of the screen is sealed as a camera chunk, under the key identifier of
/// the screen track. The header names track 2, and the clear prefix leaves unchanged. The
/// screen seals under a key of its own: one clear chunk under one counter seals to other bytes
/// on the camera.
#[test]
fn a_screen_chunk_is_sealed_under_the_key_identifier_of_the_screen() {
    let key = SenderKey::generate();
    let mut screen = screen_sealer(&key, 3);
    let mut camera = camera_sealer(&key, 3);
    let picture = keyframe(b"a screen of text");
    let sealed = seal_video(&mut screen, &picture, PictureKind::Keyframe);
    let on_camera = seal_video(&mut camera, &picture, PictureKind::Keyframe);
    let (prefix, header) = ChunkHeader::read_video(&sealed).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(prefix.get(), 10);
    assert_eq!(header.kid.track, Track::Screen);
    assert_eq!(header.kid.sender, Seq::<SenderIndex>::new(3));
    assert_eq!(header.counter, Seq::new(1));
    assert_eq!(sealed.as_bytes().get(..10), picture.get(..10));
    assert_eq!(
        u64::try_from(sealed.as_bytes().len()).ok(),
        u64::try_from(picture.len())
            .ok()
            .map(|n| n + VIDEO_OVERHEAD.get())
    );
    assert_ne!(
        sealed.as_bytes().get(19..),
        on_camera.as_bytes().get(19..),
        "the screen seals under the key of the camera"
    );
    // A screen sealer refuses an audio chunk, as a camera sealer does.
    assert_eq!(
        screen.seal(&clear(b"opus")),
        Err(ChunkError::TrackNotCarried)
    );
}

/// `CALL-110`: a chunk of the screen opens at the door of the screen alone, and a chunk of the
/// camera does not open there. The audio door refuses the screen too.
#[test]
fn a_screen_chunk_opens_through_the_door_of_the_screen_alone() {
    let key = SenderKey::generate();
    let mut screen = screen_sealer(&key, 1);
    let mut camera = camera_sealer(&key, 1);
    let shown = seal_video(&mut screen, &keyframe(b"screen"), PictureKind::Keyframe);
    let filmed = seal_video(&mut camera, &keyframe(b"camera"), PictureKind::Keyframe);
    let mut keys = receiver(1, 0, &key);
    assert_eq!(
        keys.open_video(copy(&shown), T0),
        Err(ChunkError::TrackNotCarried)
    );
    assert!(keys.open_audio(copy(&shown), T0).is_err());
    assert_eq!(
        keys.open_screen(copy(&filmed), T0),
        Err(ChunkError::TrackNotCarried)
    );
    let (sender, opened) = keys
        .open_screen(shown, T0)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(sender, Seq::<SenderIndex>::new(1));
    assert_eq!(opened.as_bytes(), keyframe(b"screen").as_slice());
    let delta_chunk = seal_video(&mut screen, &delta(b"more"), PictureKind::Delta);
    assert!(keys.open_screen(delta_chunk, T0).is_ok());
}

fn counter(value: u64) -> Seq<ChunkCounter> {
    Seq::new(value)
}

/// The replay window as a second path: the set of the counters opened, and the highest one.
/// A counter opens above the highest, or less than 128 behind it and never opened.
fn model_admits(opened: &std::collections::BTreeSet<u64>, highest: u64, value: u64) -> bool {
    value > highest || (highest - value < 128 && !opened.contains(&value))
}

/// The window reaches 127 counters behind the highest and no further, and a
/// counter opened behind the highest stays refused.
#[test]
fn the_replay_window_reaches_127_counters_behind_the_highest() {
    let mut window = ReplayWindow::default();
    window.mark(counter(200));
    assert!(!window.admits(counter(72)));
    assert!(window.admits(counter(73)));
    window.mark(counter(73));
    assert!(!window.admits(counter(73)));
    assert!(!window.admits(counter(200)));
    assert!(window.admits(counter(199)));
}

/// A move of the highest counter carries each counter opened before it. The
/// move is one step, or a jump of exactly the width of the window.
#[test]
fn the_replay_window_carries_its_counters_when_the_highest_moves() {
    let mut window = ReplayWindow::default();
    window.mark(counter(5));
    window.mark(counter(6));
    assert!(!window.admits(counter(5)));
    window.mark(counter(10));
    window.mark(counter(8));
    assert!(!window.admits(counter(8)));
    assert!(window.admits(counter(9)));
    assert!(window.admits(counter(7)));
    let mut jumped = ReplayWindow::default();
    jumped.mark(counter(5));
    jumped.mark(counter(133));
    assert!(!jumped.admits(counter(5)));
    assert!(jumped.admits(counter(6)));
    assert!(!jumped.admits(counter(133)));
    jumped.mark(counter(134));
    assert!(!jumped.admits(counter(133)));
    assert!(jumped.admits(counter(132)));
}

proptest! {
    /// `CALL-026`: the window admits what the model admits, over any order of counters, and
    /// a counter that fails to open moves neither of them.
    #[test]
    fn the_replay_window_agrees_with_a_model_of_the_counters_opened(
        steps in prop::collection::vec(
            (prop_oneof![0u64..300, 300u64..700], any::<bool>()),
            1..300,
        ),
    ) {
        let mut window = ReplayWindow::default();
        let mut opened = std::collections::BTreeSet::new();
        let mut highest = 0;
        for (value, opens) in steps {
            let admitted = model_admits(&opened, highest, value);
            prop_assert_eq!(window.admits(counter(value)), admitted, "counter {}", value);
            if admitted && opens {
                window.mark(counter(value));
                opened.insert(value);
                highest = highest.max(value);
            }
        }
    }
}

/// The header of a chunk reads past exactly one tag of room, and no less. Another config byte
/// refuses the chunk before any key is looked up.
#[test]
fn a_header_needs_a_tag_of_room_and_the_config_byte() {
    let key = SenderKey::generate();
    let mut sealer = sealer(&key, 1, 0);
    let sealed = seal(&mut sealer, b"opus").into_bytes();
    let header = sealed.get(..HEADER_LEN).unwrap_or_default().to_vec();
    let with_room = |room: usize| Payload::new([header.clone(), vec![0; room]].concat());
    assert!(ChunkHeader::read_audio(&with_room(TAG_LEN)).is_ok());
    assert_eq!(
        ChunkHeader::read_audio(&with_room(TAG_LEN - 1)).map(|_| ()),
        Err(ChunkError::Malformed)
    );
    let mut other = sealed.clone();
    if let Some(config) = other.first_mut() {
        *config = CONFIG_BYTE ^ 1;
    }
    assert_eq!(
        ChunkHeader::read_audio(&Payload::new(other)).map(|_| ()),
        Err(ChunkError::Malformed)
    );
}

/// A receiver forgets one sender alone, and holds no key once it forgot the last one.
#[test]
fn a_receiver_forgets_one_sender_alone() {
    let key = SenderKey::generate();
    let mut keys = ReceiveKeys::default();
    assert!(keys.is_empty());
    keys.insert(Seq::new(1), Seq::ZERO, key.clone(), T0);
    assert!(!keys.is_empty());
    keys.forget(Seq::new(2));
    assert!(!keys.is_empty());
    keys.forget(Seq::new(1));
    assert!(keys.is_empty());
    let mut sealer = sealer(&key, 1, 0);
    assert_eq!(
        keys.open_audio(seal(&mut sealer, b"gone"), T0)
            .map(|(s, _)| s),
        Err(ChunkError::UnknownKey)
    );
}

/// A receiver ratchets up exactly `MAX_RATCHETS` times from a key it holds, and no more.
#[test]
fn a_receiver_ratchets_up_to_the_bound_and_no_further() {
    let key = SenderKey::generate();
    let reach = MAX_RATCHETS.get();
    let at = |steps: u64| (0..steps).fold(key.clone(), |key, _| key.ratcheted());
    let mut keys = receiver(1, 0, &key);
    let mut past = sealer(&at(reach + 1), 1, reach + 1);
    assert_eq!(
        keys.open_audio(seal(&mut past, b"past"), T0)
            .map(|(s, _)| s),
        Err(ChunkError::UnknownKey)
    );
    let mut bound = sealer(&at(reach), 1, reach);
    assert!(keys.open_audio(seal(&mut bound, b"bound"), T0).is_ok());
}

/// A receiver keeps the openers of the newest generations of a sender. The oldest one drops
/// with its window once one more opens, and no chunk of it opens again.
#[test]
fn a_receiver_keeps_the_openers_of_the_newest_generations_alone() {
    let key = SenderKey::generate();
    let kept = OPENERS_PER_SENDER.get();
    let mut sealers: Vec<_> = (0..kept + 2)
        .scan(key.clone(), |key, generation| {
            let current = key.clone();
            *key = key.ratcheted();
            Some(sealer(&current, 1, generation))
        })
        .collect();
    let mut keys = receiver(1, 0, &key);
    let mut open = |keys: &mut ReceiveKeys, generation: u64| {
        let sealer = usize::try_from(generation)
            .ok()
            .and_then(|at| sealers.get_mut(at))
            .unwrap_or_else(|| panic!("no sealer at {generation}"));
        keys.open_audio(seal(sealer, b"x"), T0).map(|(s, _)| s)
    };
    for generation in 0..kept {
        assert_eq!(open(&mut keys, generation), Ok(Seq::new(1)));
    }
    assert_eq!(open(&mut keys, 0), Ok(Seq::new(1)));
    assert_eq!(open(&mut keys, kept), Ok(Seq::new(1)));
    assert_eq!(open(&mut keys, 0), Err(ChunkError::Replayed));
    assert_eq!(open(&mut keys, 1), Ok(Seq::new(1)));
    assert_eq!(open(&mut keys, kept + 1), Ok(Seq::new(1)));
    assert_eq!(open(&mut keys, 1), Err(ChunkError::Replayed));
}

/// The debug form of an exchange key names its type and no byte of it.
#[test]
fn the_debug_form_of_an_exchange_key_shows_no_byte() {
    assert_eq!(
        format!("{:?}", CallExchangeKey::generate()),
        "CallExchangeKey([REDACTED])"
    );
}

/// Regression (`CALL-026`): once the opener of a generation dropped, a fresh opener with an
/// empty window opened a copy of a chunk again. So the SFU could replay that chunk.
#[test]
fn a_chunk_of_a_dropped_generation_does_not_open_again() {
    let key = SenderKey::generate();
    let mut keys = receiver(1, 0, &key);
    let mut oldest = sealer(&key, 1, 0);
    let first = seal(&mut oldest, b"first");
    assert!(keys.open_audio(copy(&first), T0).is_ok());
    let mut current = key.clone();
    for generation in 1..=OPENERS_PER_SENDER.get() {
        current = current.ratcheted();
        let mut next = sealer(&current, 1, generation);
        assert!(keys.open_audio(seal(&mut next, b"next"), T0).is_ok());
    }
    assert_eq!(
        keys.open_audio(first, T0).map(|(s, _)| s),
        Err(ChunkError::Replayed)
    );
}

/// `CALL-125`: once a chunk of Bob opens under his fresh key, no chunk under his older keys
/// opens. Mallory left and still holds the older key. A malicious SFU forwards her chunks,
/// sealed after the switch, and a chunk of Bob that arrives late.
#[test]
fn a_chunk_of_an_older_epoch_does_not_open_once_a_newer_one_opened() {
    let old = SenderKey::generate();
    let fresh = SenderKey::generate();
    let fresh_epoch = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    let mut keys = receiver(1, 0, &old);
    keys.insert(Seq::new(1), Seq::new(fresh_epoch), fresh.clone(), T0);
    let mut bob_old = sealer(&old, 1, 0);
    let late = seal(&mut bob_old, b"late");
    // The fresh key waits out the switch delay: the older epoch still opens until then.
    assert!(keys.open_audio(seal(&mut bob_old, b"before"), T0).is_ok());
    let mut bob_fresh = sealer(&fresh, 1, fresh_epoch);
    assert!(
        keys.open_audio(seal(&mut bob_fresh, b"switched"), T0)
            .is_ok()
    );

    let mut mallory_same = sealer(&old, 1, 0);
    let mut mallory_ahead = sealer(&old.ratcheted(), 1, 1);
    let mut mallory_camera = ChunkSealer::new(&old, camera(1, 0)).unwrap_or_else(|e| panic!("{e}"));
    for forged in [
        keys.open_audio(late, T0),
        keys.open_audio(seal(&mut mallory_same, b"forged"), T0),
        keys.open_audio(seal(&mut mallory_ahead, b"forged"), T0),
        keys.open_video(
            seal_video(
                &mut mallory_camera,
                &keyframe(b"forged"),
                PictureKind::Keyframe,
            ),
            T0,
        ),
    ] {
        assert_eq!(forged.map(|(s, _)| s), Err(ChunkError::Retired));
    }
    assert!(keys.open_audio(seal(&mut bob_fresh, b"after"), T0).is_ok());
    let mut bob_step = sealer(&fresh.ratcheted(), 1, fresh_epoch + 1);
    assert!(keys.open_audio(seal(&mut bob_step, b"one up"), T0).is_ok());
}

/// `CALL-135`: Alice draws a fresh key once Eve leaves, and no chunk of the fresh epoch reaches
/// Roger. A malicious SFU forwards chunks Eve seals under the older key of Alice. They open
/// within the switch delay alone, and the camera and a step up close with the audio.
#[test]
fn an_older_epoch_closes_at_the_switch_delay_with_no_fresh_chunk() {
    let old = SenderKey::generate();
    let fresh = SenderKey::generate();
    let fresh_epoch = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    let mut roger = receiver(1, 0, &old);
    let arrival = Instant::<PageLoad>::new(1_000.0);
    roger.insert(Seq::new(1), Seq::new(fresh_epoch), fresh.clone(), arrival);
    let delay = SWITCH_DELAY.to_fractional();
    let just_before = arrival.after(delay).before(TimeSpan::new(1.0));
    let mut eve = sealer(&old, 1, 0);
    assert!(
        roger
            .open_audio(seal(&mut eve, b"in time"), just_before)
            .is_ok()
    );

    let due = arrival.after(delay);
    let mut eve_ahead = sealer(&old.ratcheted(), 1, 1);
    let mut eve_camera = ChunkSealer::new(&old, camera(1, 0)).unwrap_or_else(|e| panic!("{e}"));
    for forged in [
        roger.open_audio(seal(&mut eve, b"forged"), due),
        roger.open_audio(seal(&mut eve_ahead, b"forged"), due),
        roger.open_video(
            seal_video(&mut eve_camera, &keyframe(b"forged"), PictureKind::Keyframe),
            due,
        ),
    ] {
        assert_eq!(forged.map(|(s, _)| s), Err(ChunkError::Retired));
    }
    let mut alice = sealer(&fresh, 1, fresh_epoch);
    assert!(roger.open_audio(seal(&mut alice, b"fresh"), due).is_ok());
}

/// `CALL-135`: a second fresh key inside the delay keeps the time of the first, as the sender
/// does. The deadline then closes both older epochs at once.
#[test]
fn a_second_fresh_key_keeps_the_deadline_of_the_first() {
    let old = SenderKey::generate();
    let first = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    let second = Seq::<KeyGeneration>::new(first)
        .next_epoch()
        .map_or(0, Seq::get);
    let mut roger = receiver(1, 0, &old);
    let arrival = Instant::<PageLoad>::new(1_000.0);
    let fresh = SenderKey::generate();
    roger.insert(Seq::new(1), Seq::new(first), fresh.clone(), arrival);
    let later = arrival.after(TimeSpan::new(4_000.0));
    roger.insert(Seq::new(1), Seq::new(second), SenderKey::generate(), later);
    let due = arrival.after(SWITCH_DELAY.to_fractional());
    let mut eve = sealer(&old, 1, 0);
    let mut first_epoch = sealer(&fresh, 1, first);
    for forged in [
        roger.open_audio(seal(&mut eve, b"forged"), due),
        roger.open_audio(seal(&mut first_epoch, b"forged"), due),
    ] {
        assert_eq!(forged.map(|(s, _)| s), Err(ChunkError::Retired));
    }
}

/// `CALL-135`: the wraps of a join while a key waits may arrive in either order. A fresh key
/// that reaches the receiver before the older one still sets the deadline.
#[test]
fn an_older_key_after_a_fresh_one_still_closes_at_the_deadline() {
    let old = SenderKey::generate();
    let fresh = SenderKey::generate();
    let fresh_epoch = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    let mut roger = ReceiveKeys::default();
    roger.insert(Seq::new(1), Seq::new(fresh_epoch), fresh, T0);
    roger.insert(Seq::new(1), Seq::ZERO, old.clone(), T0);
    let mut eve = sealer(&old, 1, 0);
    assert!(roger.open_audio(seal(&mut eve, b"in time"), T0).is_ok());
    let due = T0.after(SWITCH_DELAY.to_fractional());
    assert_eq!(
        roger
            .open_audio(seal(&mut eve, b"forged"), due)
            .map(|(s, _)| s),
        Err(ChunkError::Retired)
    );
}

/// `CALL-026` across a switch: the first chunk under a fresh epoch retires the older ones and
/// keeps the opener it just made. A copy of that chunk the SFU replays is dropped.
#[test]
fn a_replayed_chunk_of_a_fresh_epoch_is_dropped() {
    let old = SenderKey::generate();
    let fresh = SenderKey::generate();
    let fresh_epoch = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    let mut keys = receiver(1, 0, &old);
    keys.insert(Seq::new(1), Seq::new(fresh_epoch), fresh.clone(), T0);
    let mut bob = sealer(&fresh, 1, fresh_epoch);
    let first = seal(&mut bob, b"switched");
    assert!(keys.open_audio(copy(&first), T0).is_ok());
    assert_eq!(
        keys.open_audio(first, T0).map(|(s, _)| s),
        Err(ChunkError::Replayed)
    );
}

/// `CALL-135`: the first chunk of a fresh epoch spends the deadline that epoch set. A newer
/// fresh key that arrives after that deadline waits a whole switch delay of its own, so the
/// current epoch keeps opening until then.
#[test]
fn a_fresh_key_after_a_spent_deadline_waits_a_whole_delay() {
    let old = SenderKey::generate();
    let fresh = SenderKey::generate();
    let first = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    let second = Seq::<KeyGeneration>::new(first)
        .next_epoch()
        .map_or(0, Seq::get);
    let mut roger = receiver(1, 0, &old);
    roger.insert(Seq::new(1), Seq::new(first), fresh.clone(), T0);
    let mut alice = sealer(&fresh, 1, first);
    assert!(roger.open_audio(seal(&mut alice, b"switched"), T0).is_ok());

    let delay = SWITCH_DELAY.to_fractional();
    let arrival = T0.after(delay).after(TimeSpan::new(1.0));
    roger.insert(
        Seq::new(1),
        Seq::new(second),
        SenderKey::generate(),
        arrival,
    );
    let just_before = arrival.after(delay).before(TimeSpan::new(1.0));
    assert!(
        roger
            .open_audio(seal(&mut alice, b"in time"), just_before)
            .is_ok()
    );
    assert_eq!(
        roger
            .open_audio(seal(&mut alice, b"late"), arrival.after(delay))
            .map(|(s, _)| s),
        Err(ChunkError::Retired)
    );
}

/// `CALL-135`: an older key that arrives after the switch sets a deadline again. A chunk of
/// the current epoch under a fresh step leaves that deadline set. A newer fresh key then
/// keeps its time, as the sender does.
#[test]
fn a_step_of_the_current_epoch_keeps_the_deadline_a_late_key_set() {
    let old = SenderKey::generate();
    let fresh = SenderKey::generate();
    let first = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    let second = Seq::<KeyGeneration>::new(first)
        .next_epoch()
        .map_or(0, Seq::get);
    let mut roger = receiver(1, first, &fresh);
    let mut alice = sealer(&fresh, 1, first);
    assert!(roger.open_audio(seal(&mut alice, b"switched"), T0).is_ok());
    roger.insert(Seq::new(1), Seq::ZERO, old, T0);
    let mut alice_step = sealer(&fresh.ratcheted(), 1, first + 1);
    assert!(
        roger
            .open_audio(seal(&mut alice_step, b"one up"), T0)
            .is_ok()
    );

    let later = T0.after(TimeSpan::new(4_000.0));
    roger.insert(Seq::new(1), Seq::new(second), SenderKey::generate(), later);
    let due = T0.after(SWITCH_DELAY.to_fractional());
    assert_eq!(
        roger
            .open_audio(seal(&mut alice, b"late"), due)
            .map(|(s, _)| s),
        Err(ChunkError::Retired)
    );
}

/// The Project Wycheproof vectors of ChaCha20-Poly1305 with a 96-bit nonce, vendored from
/// `testvectors_v1/chacha20_poly1305_test.json` (`fixtures/THIRD_PARTY.md`). The file holds
/// 87 of the 316 vectors of that group, under the 64 KiB cap of a fixture. The cut takes, for
/// each flag in turn, the shortest vector left, until the next would pass the cap. Every flag
/// of the group is there, the RFC 8439 vector and 16 forged tags included.
const WYCHEPROOF: &str = include_str!("../tests/fixtures/wycheproof/chacha20_poly1305_test.json");

/// The bytes of one hex field of a vector.
fn hex_field(test: &serde_json::Value, field: &str) -> Vec<u8> {
    let text = test[field]
        .as_str()
        .unwrap_or_else(|| panic!("a vector with no `{field}`"));
    data_encoding::HEXLOWER
        .decode(text.as_bytes())
        .unwrap_or_else(|e| panic!("`{field}` is no hex: {e}"))
}

/// `CALL-148`: the key and the nonce a chunk seals under drive the AEAD as a reference does.
/// Each vector of Project Wycheproof goes through the key and the nonce of the chunk format,
/// at counter 0, where the nonce is the salt. A valid vector seals to its ciphertext and its
/// tag, and opens back. An invalid one opens to nothing.
#[test]
fn the_chunk_aead_passes_the_wycheproof_vectors() {
    use chacha20poly1305::aead::AeadInPlace;

    let suite: serde_json::Value =
        serde_json::from_str(WYCHEPROOF).unwrap_or_else(|e| panic!("the vectors parse: {e}"));
    assert_eq!(suite["algorithm"], "CHACHA20-POLY1305");
    let mut counted = (0, 0);
    for group in suite["testGroups"].as_array().into_iter().flatten() {
        assert_eq!(group["ivSize"], 96, "a group of another nonce size");
        for test in group["tests"].as_array().into_iter().flatten() {
            let id = &test["tcId"];
            let key: [u8; 32] = hex_field(test, "key")
                .try_into()
                .unwrap_or_else(|_| panic!("vector {id}: a key of another size"));
            let salt: [u8; 12] = hex_field(test, "iv")
                .try_into()
                .unwrap_or_else(|_| panic!("vector {id}: a nonce of another size"));
            let (aad, msg, ct, tag) = (
                hex_field(test, "aad"),
                hex_field(test, "msg"),
                hex_field(test, "ct"),
                hex_field(test, "tag"),
            );
            let chunk_key = super::ChunkKey::from_parts(key, salt);
            let nonce = chunk_key.nonce(Seq::<ChunkCounter>::new(0));
            assert_eq!(
                nonce.as_slice(),
                &salt[..],
                "vector {id}: the nonce at counter 0"
            );
            let cipher = chunk_key.cipher();
            let mut opened = ct.clone();
            let open = cipher.decrypt_in_place_detached(
                &nonce,
                &aad,
                &mut opened,
                chacha20poly1305::Tag::from_slice(&tag),
            );
            match test["result"].as_str() {
                Some("valid") => {
                    let mut sealed = msg.clone();
                    let got = cipher
                        .encrypt_in_place_detached(&nonce, &aad, &mut sealed)
                        .unwrap_or_else(|e| panic!("vector {id}: {e}"));
                    assert_eq!(sealed, ct, "vector {id}: the ciphertext");
                    assert_eq!(got.as_slice(), tag.as_slice(), "vector {id}: the tag");
                    assert!(open.is_ok(), "vector {id}: a valid vector does not open");
                    assert_eq!(opened, msg, "vector {id}: the opened text");
                    counted.0 += 1;
                }
                Some("invalid") => {
                    assert!(open.is_err(), "vector {id}: an invalid vector opens");
                    counted.1 += 1;
                }
                other => panic!("vector {id}: a result of {other:?}"),
            }
        }
    }
    assert_eq!(
        counted,
        (71, 16),
        "the vendored file holds 71 valid and 16 invalid vectors"
    );
}

/// `CALL-157`: Eve is removed while Alice talks under her key of epoch zero. The relay
/// withholds the wrap of the fresh key of Alice from Roger. No fresh key reaches him, and no
/// deadline of a fresh key starts. Once the switch delay passed, the close of the removal still
/// shuts every chunk Eve seals under the older key. That holds for the same step, a step up
/// and a camera chunk. A late handoff of that key opens nothing either.
#[test]
fn a_removal_closes_the_older_epochs_with_no_fresh_key() {
    let old = SenderKey::generate();
    let mut roger = receiver(1, 0, &old);
    let removal = Instant::<PageLoad>::new(1_000.0);
    roger.close(Seq::new(1), Seq::ZERO, SWITCH_DELAY, removal);
    let delay = SWITCH_DELAY.to_fractional();
    let just_before = removal.after(delay).before(TimeSpan::new(1.0));
    let mut eve = sealer(&old, 1, 0);
    assert!(
        roger
            .open_audio(seal(&mut eve, b"in time"), just_before)
            .is_ok()
    );

    let due = removal.after(delay);
    let mut eve_ahead = sealer(&old.ratcheted(), 1, 1);
    let mut eve_camera = ChunkSealer::new(&old, camera(1, 0)).unwrap_or_else(|e| panic!("{e}"));
    for forged in [
        roger.open_audio(seal(&mut eve, b"forged"), due),
        roger.open_audio(seal(&mut eve_ahead, b"forged"), due),
        roger.open_video(
            seal_video(&mut eve_camera, &keyframe(b"forged"), PictureKind::Keyframe),
            due,
        ),
    ] {
        assert!(forged.is_err(), "a chunk under the older key opened");
    }
    roger.insert(Seq::new(1), Seq::ZERO, old.clone(), due);
    assert_eq!(
        roger
            .open_audio(seal(&mut eve, b"replayed key"), due)
            .map(|(s, _)| s),
        Err(ChunkError::Retired)
    );

    let fresh = SenderKey::generate();
    let fresh_epoch = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    roger.insert(Seq::new(1), Seq::new(fresh_epoch), fresh.clone(), due);
    let mut alice = sealer(&fresh, 1, fresh_epoch);
    assert!(roger.open_audio(seal(&mut alice, b"fresh"), due).is_ok());
}

/// `CALL-157`: a second removal inside the delay keeps the time of the first close, and moves
/// it to the newer epoch. A key of Alice drawn between the two removals closes at that time
/// too.
#[test]
fn a_second_removal_keeps_the_time_of_the_first_close() {
    let old = SenderKey::generate();
    let middle = SenderKey::generate();
    let middle_epoch = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    let mut roger = receiver(1, 0, &old);
    let first = Instant::<PageLoad>::new(1_000.0);
    roger.close(Seq::new(1), Seq::ZERO, SWITCH_DELAY, first);
    let second = Instant::<PageLoad>::new(3_000.0);
    roger.insert(Seq::new(1), Seq::new(middle_epoch), middle.clone(), second);
    roger.close(Seq::new(1), Seq::new(middle_epoch), SWITCH_DELAY, second);
    let due = first.after(SWITCH_DELAY.to_fractional());
    let mut eve_old = sealer(&old, 1, 0);
    let mut eve_middle = sealer(&middle, 1, middle_epoch);
    assert!(
        roger
            .open_audio(seal(&mut eve_old, b"forged"), due)
            .is_err()
    );
    assert!(
        roger
            .open_audio(seal(&mut eve_middle, b"forged"), due)
            .is_err()
    );
}

/// `CALL-157`: the departures of a wrap keep the newest indexes past the slots, and name an
/// index alone once it was departed.
#[test]
fn the_departures_keep_the_newest_indexes() {
    let mut departures = Departures::default();
    let slots = u64::try_from(DEPARTURE_SLOTS).unwrap_or(0);
    for index in 1..=slots + 1 {
        departures.depart(Seq::new(index));
    }
    assert!(!departures.names_any(&[Seq::new(1)]));
    assert!(departures.names_any(&[Seq::new(2), Seq::new(99)]));
    assert!(departures.names_any(&[Seq::new(slots + 1)]));
    assert!(!departures.names_any(&[]));
}
