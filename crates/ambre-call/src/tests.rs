//! The call worker on the host: the seal, the open and the erase it runs in the browser.

use ambre_core::{ClearChunk, KeyIdentifier, Payload, SealedChunk, Seq, Track};
use ambre_crypto::call::{AUDIO_CHUNK_SIZE, ChunkSealer, KeyHandoff, SenderKey};

use super::{CallWorker, Handled, Unsealed};

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
    assert_eq!(alice.hand(seal_handoff(1, 0, &key)), Handled::Kept);
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
    bob.hand(open_handoff(1, 0, &key));
    let opened = bob.open(sealed).unwrap_or_else(|| panic!("no open"));
    assert_eq!(opened.as_bytes(), b"a packet of opus");
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
    assert!(bob.open(sealed_by(&key, 1, 0, b"early")).is_none());
    bob.hand(open_handoff(1, 0, &key));
    assert!(bob.open(sealed_by(&other, 1, 0, b"forged")).is_none());
    let mut changed = sealed_by(&key, 1, 0, b"changed").into_bytes();
    if let Some(byte) = changed.get_mut(20) {
        *byte ^= 1;
    }
    assert!(bob.open(Payload::new(changed)).is_none());
    assert!(bob.open(sealed_by(&key, 2, 0, b"unknown sender")).is_none());
    let opened = bob.open(sealed_by(&key, 1, 0, b"fine"));
    assert_eq!(opened.map(Payload::into_bytes), Some(b"fine".to_vec()));
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
    worker.hand(seal_handoff(1, 0, &own));
    worker.hand(open_handoff(2, 0, &bob));
    worker.hand(open_handoff(3, 0, &carol));
    assert!(!worker.holds_no_key());
    worker.hand(KeyHandoff::Forget {
        sender: Seq::new(3),
    });
    assert!(worker.open(sealed_by(&carol, 3, 0, b"after")).is_none());
    assert!(worker.open(sealed_by(&bob, 2, 0, b"still")).is_some());
    assert_eq!(worker.hand(KeyHandoff::End), Handled::Erased);
    assert!(worker.holds_no_key());
    assert_eq!(worker.seal(&clear(b"after the end")), Err(Unsealed::NoKey));
    assert!(
        worker
            .open(sealed_by(&bob, 2, 0, b"after the end"))
            .is_none()
    );
}

/// A key handed across the port, as bytes, seals what the key of the Sync worker opens.
#[test]
fn a_key_handed_across_the_port_seals_what_the_sync_worker_key_opens() {
    let key = SenderKey::generate();
    let port = seal_handoff(4, 1, &key).to_port();
    let mut worker = CallWorker::default();
    worker.hand(KeyHandoff::from_port(&port).unwrap_or_else(|e| panic!("{e}")));
    let sealed = worker
        .seal(&clear(b"through the port"))
        .unwrap_or_else(|e| panic!("{e:?}"));
    let mut peer = CallWorker::default();
    peer.hand(open_handoff(4, 1, &key));
    assert!(peer.open(sealed).is_some());
}
