//! The call worker on the host: the seal, the open and the erase it runs in the browser.

use ambre_core::index::Pos;
use ambre_core::{
    ClearChunk, Instant, KeyGeneration, KeyIdentifier, MediaSlot, PageLoad, Payload, PictureKind,
    SealedChunk, Seq, SlotSender, Track,
};
use ambre_crypto::call::{AUDIO_CHUNK_SIZE, ChunkSealer, KeyHandoff, SWITCH_DELAY, SenderKey};

use super::{CallWorker, Handled, SlotWatch, Unsealed};

/// The time of every test that waits for no deadline.
const T0: Instant<PageLoad> = Instant::new(0.0);

fn clear(bytes: &[u8]) -> Payload<ClearChunk> {
    Payload::new(bytes.to_vec())
}

fn seal_handoff(sender: u64, generation: u64, key: &SenderKey) -> KeyHandoff {
    KeyHandoff::Seal {
        sender: Seq::new(sender),
        generation: Seq::new(generation),
        key: key.clone(),
    }
}

fn open_handoff(sender: u64, generation: u64, key: &SenderKey) -> KeyHandoff {
    KeyHandoff::Open {
        sender: Seq::new(sender),
        generation: Seq::new(generation),
        key: key.clone(),
    }
}

/// A chunk Bob's sealer seals under `key`, at `generation`.
fn sealed_by(key: &SenderKey, sender: u64, generation: u64, bytes: &[u8]) -> Payload<SealedChunk> {
    let kid = KeyIdentifier {
        track: Track::Audio,
        sender: Seq::new(sender),
        generation: Seq::new(generation),
    };
    ChunkSealer::new(key, kid)
        .and_then(|mut sealer| sealer.seal(&clear(bytes)))
        .unwrap_or_else(|e| panic!("{e}"))
}

/// `CALL-001`: a worker with no key sends nothing. Once the Sync worker hands its key, each
/// chunk leaves sealed at the fixed size, and a holder of the key opens it whole.
#[test]
fn a_chunk_leaves_the_worker_sealed_under_the_sender_key_or_not_at_all() {
    let key = SenderKey::generate();
    let mut alice = CallWorker::default();
    assert_eq!(
        alice.seal(&clear(b"opus before any key")),
        Err(Unsealed::NoKey)
    );
    assert_eq!(alice.hand(seal_handoff(1, 0, &key), T0), Handled::Kept);
    let sealed = alice
        .seal(&clear(b"a packet of opus"))
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(sealed.len(), AUDIO_CHUNK_SIZE);
    assert!(
        !sealed
            .as_bytes()
            .windows(16)
            .any(|w| w == b"a packet of opus"),
        "the clear chunk shows through the seal"
    );
    let mut bob = CallWorker::default();
    bob.hand(open_handoff(1, 0, &key), T0);
    let opened = bob
        .open(Track::Audio, sealed, T0)
        .unwrap_or_else(|| panic!("no open"));
    assert_eq!(opened.clear.as_bytes(), b"a packet of opus");
    assert_eq!(opened.sender, Seq::new(1));
    assert_eq!(alice.seal(&clear(&[0; 161])), Err(Unsealed::TooLarge));
}

/// `CALL-022`: a chunk reaches the decoder only once it opens under the key its identifier
/// names. One of a sender the worker holds no key of, and one with a changed byte, are
/// dropped.
#[test]
fn a_received_chunk_reaches_the_decoder_only_once_it_opens() {
    let key = SenderKey::generate();
    let other = SenderKey::generate();
    let mut bob = CallWorker::default();
    assert!(
        bob.open(Track::Audio, sealed_by(&key, 1, 0, b"early"), T0)
            .is_none()
    );
    bob.hand(open_handoff(1, 0, &key), T0);
    assert!(
        bob.open(Track::Audio, sealed_by(&other, 1, 0, b"forged"), T0)
            .is_none()
    );
    let mut changed = sealed_by(&key, 1, 0, b"changed").into_bytes();
    if let Some(byte) = changed.get_mut(20) {
        *byte ^= 1;
    }
    assert!(bob.open(Track::Audio, Payload::new(changed), T0).is_none());
    assert!(
        bob.open(Track::Audio, sealed_by(&key, 2, 0, b"unknown sender"), T0)
            .is_none()
    );
    let opened = bob.open(Track::Audio, sealed_by(&key, 1, 0, b"fine"), T0);
    assert_eq!(opened.map(|o| o.clear.into_bytes()), Some(b"fine".to_vec()));
}

/// `CALL-029`, `CALL-004`: the end of the call drops every key of it. The worker then seals
/// nothing and opens nothing of that call, and a sender that left leaves no key behind.
#[test]
fn the_end_of_a_call_erases_every_key_of_it() {
    let own = SenderKey::generate();
    let bob = SenderKey::generate();
    let carol = SenderKey::generate();
    let mut worker = CallWorker::default();
    assert!(worker.holds_no_key());
    worker.hand(seal_handoff(1, 0, &own), T0);
    worker.hand(open_handoff(2, 0, &bob), T0);
    worker.hand(open_handoff(3, 0, &carol), T0);
    assert!(!worker.holds_no_key());
    worker.hand(
        KeyHandoff::Forget {
            sender: Seq::new(3),
        },
        T0,
    );
    assert!(
        worker
            .open(Track::Audio, sealed_by(&carol, 3, 0, b"after"), T0)
            .is_none()
    );
    assert!(
        worker
            .open(Track::Audio, sealed_by(&bob, 2, 0, b"still"), T0)
            .is_some()
    );
    assert_eq!(worker.hand(KeyHandoff::End, T0), Handled::Erased);
    assert!(worker.holds_no_key());
    assert_eq!(worker.seal(&clear(b"after the end")), Err(Unsealed::NoKey));
    assert_eq!(
        worker.seal_video(
            Track::Camera,
            &clear(b"\x31after the end"),
            PictureKind::Delta
        ),
        Err(Unsealed::NoKey)
    );
    assert!(
        worker
            .open(Track::Audio, sealed_by(&bob, 2, 0, b"after the end"), T0)
            .is_none()
    );
}

/// A key handed across the port, as bytes, seals what the key of the Sync worker opens.
#[test]
fn a_key_handed_across_the_port_seals_what_the_sync_worker_key_opens() {
    let key = SenderKey::generate();
    let port = seal_handoff(4, 1, &key).to_port();
    let mut worker = CallWorker::default();
    worker.hand(
        KeyHandoff::from_port(&port).unwrap_or_else(|e| panic!("{e}")),
        T0,
    );
    let sealed = worker
        .seal(&clear(b"through the port"))
        .unwrap_or_else(|e| panic!("{e:?}"));
    let mut peer = CallWorker::default();
    peer.hand(open_handoff(4, 1, &key), T0);
    assert!(peer.open(Track::Audio, sealed, T0).is_some());
}

/// `CALL-084`: a slot tells the page of its sender at the first chunk of each new sender, and at
/// no other chunk. The sender comes from the key identifier of the chunk the worker opened.
#[test]
fn a_slot_tells_the_page_of_each_new_sender_once() {
    let (bob, carol) = (SenderKey::generate(), SenderKey::generate());
    let mut worker = CallWorker::default();
    worker.hand(open_handoff(2, 0, &bob), T0);
    worker.hand(open_handoff(3, 0, &carol), T0);
    let sealer = |key: &SenderKey, sender| {
        let kid = KeyIdentifier {
            track: Track::Audio,
            sender: Seq::new(sender),
            generation: Seq::new(0),
        };
        ChunkSealer::new(key, kid).unwrap_or_else(|e| panic!("{e}"))
    };
    let (mut from_bob, mut from_carol) = (sealer(&bob, 2), sealer(&carol, 3));
    let mut watch = SlotWatch::new(MediaSlot::Audio(Pos::new(1)));
    let mut notes = Vec::new();
    for speaker in [2, 2, 3, 3, 2] {
        let from = if speaker == 2 {
            &mut from_bob
        } else {
            &mut from_carol
        };
        let sealed = from
            .seal(&clear(b"speech"))
            .unwrap_or_else(|e| panic!("{e}"));
        let opened = worker
            .open(Track::Audio, sealed, T0)
            .unwrap_or_else(|| panic!("no open"));
        notes.push(watch.opened(opened.sender));
    }
    let note = |sender| {
        Some(SlotSender {
            slot: MediaSlot::Audio(Pos::new(1)),
            sender: Seq::new(sender),
        })
    };
    assert_eq!(notes, vec![note(2), None, note(3), None, note(2)]);
}

/// `CALL-001`, `CALL-022` and `CALL-097` for the camera: a worker with no key seals no VP8 chunk.
/// Once its key arrives, a keyframe leaves sealed with its prefix in the clear. A holder of the
/// key opens it whole on a receiver of the camera alone. A chunk whose keyframe bit and kind
/// differ leaves nothing.
#[test]
fn a_camera_chunk_leaves_sealed_and_opens_on_a_receiver_of_the_camera_alone() {
    let key = SenderKey::generate();
    let mut alice = CallWorker::default();
    let keyframe = b"\x50\x2a\x00\x9d\x01\x2a\x00\x05\xd0\x02a picture of vp8";
    assert_eq!(
        alice.seal_video(Track::Camera, &clear(keyframe), PictureKind::Keyframe),
        Err(Unsealed::NoKey)
    );
    alice.hand(seal_handoff(1, 0, &key), T0);
    assert_eq!(
        alice.seal_video(Track::Camera, &clear(keyframe), PictureKind::Delta),
        Err(Unsealed::Refused)
    );
    let sealed = alice
        .seal_video(Track::Camera, &clear(keyframe), PictureKind::Keyframe)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(sealed.as_bytes().get(..10), keyframe.get(..10));
    assert!(
        !sealed
            .as_bytes()
            .windows(14)
            .any(|w| w == b"a picture of v")
    );
    let mut bob = CallWorker::default();
    bob.hand(open_handoff(1, 0, &key), T0);
    assert!(bob.open(Track::Audio, copy(&sealed), T0).is_none());
    let opened = bob
        .open(Track::Camera, sealed, T0)
        .unwrap_or_else(|| panic!("no open"));
    assert_eq!(opened.clear.as_bytes(), keyframe);
    assert_eq!(opened.sender, Seq::new(1));
}

/// `CALL-084` for a video slot. The first camera chunk of each new sender in a video slot tells
/// the page that slot and that sender. Its picture then draws in the tile of its member.
#[test]
fn a_video_slot_tells_the_page_of_each_new_sender_once() {
    let (bob, carol) = (SenderKey::generate(), SenderKey::generate());
    let (mut from_bob, mut from_carol) = (CallWorker::default(), CallWorker::default());
    from_bob.hand(seal_handoff(2, 0, &bob), T0);
    from_carol.hand(seal_handoff(3, 0, &carol), T0);
    let mut ada = CallWorker::default();
    ada.hand(open_handoff(2, 0, &bob), T0);
    ada.hand(open_handoff(3, 0, &carol), T0);
    let keyframe = b"\x50\x2a\x00\x9d\x01\x2a\x00\x05\xd0\x02a face";
    let mut watch = SlotWatch::new(MediaSlot::Video(Pos::new(4)));
    let mut notes = Vec::new();
    for speaker in [2, 2, 3] {
        let from = if speaker == 2 {
            &mut from_bob
        } else {
            &mut from_carol
        };
        let sealed = from
            .seal_video(Track::Camera, &clear(keyframe), PictureKind::Keyframe)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let opened = ada
            .open(Track::Camera, sealed, T0)
            .unwrap_or_else(|| panic!("no open"));
        notes.push(watch.opened(opened.sender));
    }
    let note = |sender| {
        Some(SlotSender {
            slot: MediaSlot::Video(Pos::new(4)),
            sender: Seq::new(sender),
        })
    };
    assert_eq!(notes, vec![note(2), None, note(3)]);
}

fn copy(chunk: &Payload<SealedChunk>) -> Payload<SealedChunk> {
    Payload::new(chunk.as_bytes().to_vec())
}

/// `CALL-102` and `CALL-110` in the call worker: a worker with no key seals no screen chunk.
/// Once its key arrives, the screen seals under the key identifier of the screen track. A holder
/// of the key opens it on a receiver of the screen alone. The camera door opens none of
/// it, and the end of the call drops the sealer of the screen too.
#[test]
fn a_screen_chunk_leaves_sealed_and_opens_on_a_receiver_of_the_screen_alone() {
    let key = SenderKey::generate();
    let mut alice = CallWorker::default();
    let keyframe = b"\x50\x2a\x00\x9d\x01\x2a\x00\x05\xd0\x02a page of text";
    assert_eq!(
        alice.seal_video(Track::Screen, &clear(keyframe), PictureKind::Keyframe),
        Err(Unsealed::NoKey)
    );
    alice.hand(seal_handoff(2, 0, &key), T0);
    assert_eq!(
        alice.seal_video(Track::Audio, &clear(keyframe), PictureKind::Keyframe),
        Err(Unsealed::NoKey)
    );
    let sealed = alice
        .seal_video(Track::Screen, &clear(keyframe), PictureKind::Keyframe)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(sealed.as_bytes().get(..10), keyframe.get(..10));
    assert!(!sealed.as_bytes().windows(9).any(|w| w == b"page of t"));
    let mut bob = CallWorker::default();
    bob.hand(open_handoff(2, 0, &key), T0);
    assert!(bob.open(Track::Camera, copy(&sealed), T0).is_none());
    assert!(bob.open(Track::Audio, copy(&sealed), T0).is_none());
    let opened = bob
        .open(Track::Screen, sealed, T0)
        .unwrap_or_else(|| panic!("no open"));
    assert_eq!(opened.clear.as_bytes(), keyframe);
    assert_eq!(opened.sender, Seq::new(2));
    assert_eq!(alice.hand(KeyHandoff::End, T0), Handled::Erased);
    assert!(alice.holds_no_key());
    assert_eq!(
        alice.seal_video(Track::Screen, &clear(keyframe), PictureKind::Keyframe),
        Err(Unsealed::NoKey)
    );
}

/// `CALL-135`: Eve leaves, and the Sync worker hands Roger the fresh key of Alice. A malicious
/// SFU forwards no chunk of the fresh epoch, and forwards chunks Eve seals under the older key
/// of Alice. Past the switch delay of the fresh key, none of them reaches the decoder.
#[test]
fn the_worker_closes_an_older_epoch_at_the_switch_delay() {
    let old = SenderKey::generate();
    let fresh = SenderKey::generate();
    let fresh_epoch = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    let mut roger = CallWorker::default();
    roger.hand(open_handoff(1, 0, &old), T0);
    let arrival = Instant::<PageLoad>::new(2_000.0);
    roger.hand(open_handoff(1, fresh_epoch, &fresh), arrival);
    let due = arrival.after(SWITCH_DELAY.to_fractional());
    // One step up, so the forged chunk below carries a counter its key never saw.
    assert!(
        roger
            .open(
                Track::Audio,
                sealed_by(&old.ratcheted(), 1, 1, b"in time"),
                arrival
            )
            .is_some()
    );
    assert!(
        roger
            .open(Track::Audio, sealed_by(&old, 1, 0, b"forged"), due)
            .is_none()
    );
    assert!(
        roger
            .open(
                Track::Audio,
                sealed_by(&fresh, 1, fresh_epoch, b"fresh"),
                due
            )
            .is_some()
    );
}

/// `CALL-154`: Eve is removed, and the relay withholds the fresh key of Alice from Roger. The
/// Sync worker of Roger hands the close of the removal alone. Past its delay, no chunk Eve
/// seals under the older key of Alice reaches the decoder, and the fresh key still opens once
/// it arrives.
#[test]
fn the_worker_closes_an_older_epoch_at_a_removal_with_no_fresh_key() {
    let old = SenderKey::generate();
    let mut roger = CallWorker::default();
    roger.hand(open_handoff(1, 0, &old), T0);
    let removal = Instant::<PageLoad>::new(2_000.0);
    let close = KeyHandoff::Close {
        sender: Seq::new(1),
        generation: Seq::ZERO,
        within: SWITCH_DELAY,
    };
    assert_eq!(roger.hand(close, removal), Handled::Kept);
    let due = removal.after(SWITCH_DELAY.to_fractional());
    // One step up, so the forged chunk below carries a counter its key never saw.
    assert!(
        roger
            .open(
                Track::Audio,
                sealed_by(&old.ratcheted(), 1, 1, b"in time"),
                removal
            )
            .is_some()
    );
    assert!(
        roger
            .open(Track::Audio, sealed_by(&old, 1, 0, b"forged"), due)
            .is_none()
    );
    let fresh = SenderKey::generate();
    let fresh_epoch = Seq::<KeyGeneration>::ZERO.next_epoch().map_or(0, Seq::get);
    roger.hand(open_handoff(1, fresh_epoch, &fresh), due);
    assert!(
        roger
            .open(
                Track::Audio,
                sealed_by(&fresh, 1, fresh_epoch, b"fresh"),
                due
            )
            .is_some()
    );
}

/// `CALL-147`: a `Seal` at the generation the worker seals under, or below it, restarts no
/// counter. The worker keeps its sealer, so the next chunk carries the next counter. A higher
/// generation then takes over from counter 1, under its own key identifier.
#[test]
fn a_seal_at_or_below_the_held_generation_restarts_no_counter() {
    let key = SenderKey::generate();
    let mut alice = CallWorker::default();
    assert_eq!(alice.hand(seal_handoff(4, 3, &key), T0), Handled::Kept);
    let header = |sealed: &Payload<SealedChunk>| {
        ambre_crypto::call::ChunkHeader::read_audio(sealed).unwrap_or_else(|e| panic!("{e}"))
    };
    let first = alice
        .seal(&clear(b"one"))
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(header(&first).counter, Seq::new(1));
    // The same key again, then an older generation: neither is taken.
    assert_eq!(alice.hand(seal_handoff(4, 3, &key), T0), Handled::Refused);
    assert_eq!(alice.hand(seal_handoff(4, 2, &key), T0), Handled::Refused);
    let second = alice
        .seal(&clear(b"two"))
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(header(&second).kid, header(&first).kid);
    assert_eq!(
        header(&second).counter,
        Seq::new(2),
        "a counter started again"
    );
    // A generation past the width of the header seals nothing, and a later `Seal` of the
    // generation sealed under before is still refused.
    let wide = 1u64 << 48;
    alice.hand(seal_handoff(4, wide, &key), T0);
    assert_eq!(alice.seal(&clear(b"none")), Err(Unsealed::NoKey));
    assert_eq!(alice.hand(seal_handoff(4, 3, &key), T0), Handled::Refused);
    // The next generation seals from counter 1 under its own key identifier.
    let mut fresh = CallWorker::default();
    fresh.hand(seal_handoff(4, 3, &key), T0);
    assert_eq!(
        fresh.hand(seal_handoff(4, 4, &key.ratcheted()), T0),
        Handled::Kept
    );
    let next = fresh
        .seal(&clear(b"three"))
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(header(&next).kid.generation, Seq::new(4));
    assert_eq!(header(&next).counter, Seq::new(1));
}
