//! The chunk format, the replay window, the wraps and the signed join of a call.

use ambre_core::id::{Call, Channel, Id, User};
use ambre_core::{
    Authored, ClearChunk, Instant, KeyGeneration, KeyIdentifier, Payload, PictureKind, PublicKey,
    SealedChunk, SenderIndex, Seq, Track,
};
use ed25519_dalek::SigningKey;
use proptest::prelude::*;

use super::{
    AUDIO_CHUNK_SIZE, CallExchangeKey, ChunkError, ChunkHeader, ChunkSealer, JoinClaim, KeyHandoff,
    ReceiveKeys, SenderKey, SignedJoin, VIDEO_OVERHEAD, WrapBinding, unwrap_sender_key,
    wrap_sender_key,
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
            keys.open_video(Payload::new(bytes)),
            Err(ChunkError::Unopened),
            "byte {at} of the prefix"
        );
    }
    let mut keys = receiver(1, 0, &key);
    assert!(keys.open_video(chunk).is_ok());
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
        assert!(keys.open_video(Payload::new(flipped)).is_err());
        for length in [0, 1, 9, 26] {
            let cut = chunk.as_bytes().get(..length).unwrap_or_default().to_vec();
            assert_eq!(
                keys.open_video(Payload::new(cut)),
                Err(ChunkError::Malformed),
                "a chunk cut at {length} bytes"
            );
        }
    }
    let mut keys = receiver(1, 0, &key);
    assert!(keys.open_audio(copy(&delta_chunk)).is_err());
    let mut audio = sealer_of_audio(&key);
    assert!(keys.open_video(seal(&mut audio, b"\x31opus")).is_err());
    assert!(keys.open_video(key_chunk).is_ok());
    assert!(keys.open_video(delta_chunk).is_ok());
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
        let (_, opened) = keys.open_video(low).unwrap_or_else(|e| panic!("{e}"));
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
        let (sender, opened) = keys.open_video(chunk).unwrap_or_else(|e| panic!("{e}"));
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
        call: Id::<Call>::from_bytes([1; 16]),
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

/// A wrap reads back from the 80 bytes the relay forwards and opens for its recipient.
/// Bytes of another length read as no wrap.
#[test]
fn a_wrap_reads_back_from_its_bytes() {
    let alice = CallExchangeKey::generate();
    let bob = CallExchangeKey::generate();
    let key = SenderKey::generate();
    let wrapped = wrap_sender_key(&alice, &bob.public(), binding(1, 2), Seq::new(3), &key)
        .unwrap_or_else(|e| panic!("{e}"));
    let bytes = wrapped.to_bytes();
    assert_eq!(bytes.len().get(), 80);
    let read = super::WrappedSenderKey::from_bytes(&bytes).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(read, wrapped);
    let (_, opened) = unwrap_sender_key(&bob, &alice.public(), binding(1, 2), &read)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(opened.bytes(), key.bytes());
    let short = Payload::new(bytes.as_bytes().get(..79).unwrap_or_default().to_vec());
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
    ];
    let sizes: Vec<usize> = handoffs
        .iter()
        .map(|h| h.to_port().with_plaintext(<[u8]>::len))
        .collect();
    assert_eq!(sizes, [49, 49, 49, 49]);
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
    ] = read.as_slice()
    else {
        panic!("the kinds came back as {read:?}");
    };
    assert_eq!(
        (*sender, *generation, *gone),
        (Seq::new(4), Seq::new(2), Seq::new(4))
    );
    let mut keys = ReceiveKeys::default();
    keys.insert(*sender, *generation, opened_key.clone());
    let (_, clear) = keys.open_audio(chunk).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(clear.as_bytes(), b"opus over the port");
    let mut resealer = ChunkSealer::new(sealing, kid(4, 2)).unwrap_or_else(|e| panic!("{e}"));
    let mut fresh = receiver(4, 2, &key);
    assert!(fresh.open_audio(seal(&mut resealer, b"again")).is_ok());
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
        keys.open_video(copy(&shown)),
        Err(ChunkError::TrackNotCarried)
    );
    assert!(keys.open_audio(copy(&shown)).is_err());
    assert_eq!(
        keys.open_screen(copy(&filmed)),
        Err(ChunkError::TrackNotCarried)
    );
    let (sender, opened) = keys.open_screen(shown).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(sender, Seq::<SenderIndex>::new(1));
    assert_eq!(opened.as_bytes(), keyframe(b"screen").as_slice());
    let delta_chunk = seal_video(&mut screen, &delta(b"more"), PictureKind::Delta);
    assert!(keys.open_screen(delta_chunk).is_ok());
}
