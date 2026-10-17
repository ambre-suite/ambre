//! Unit tests for the wire protocol types.

use super::*;
use crate::messages::{Ciphertext, Clock, Counter, Seq};
use ambre_core::{DocumentId, KeyId, PublicKey, SnapshotId, UserId};

#[test]
fn update_roundtrip() {
    let update = Update {
        ciphertext: Ciphertext(vec![1, 2, 3, 4]),
        nonce: Nonce([0u8; 24]),
        public_data: UpdatePublicData {
            document_id: DocumentId::new(),
            pub_key: PublicKey::from_array([5u8; 32]),
            key_id: KeyId::from_array([8u8; 16]),
            snapshot_id: SnapshotId::new(),
            clock: Clock(42),
        },
        commitment_tag: CommitmentTag([6u8; 32]),
        signature: Signature([7u8; 64]),
    };

    let bytes = postcard::to_allocvec(&update).expect("serialization failed");
    let decoded: Update = postcard::from_bytes(&bytes).expect("deserialization failed");

    assert_eq!(update.ciphertext, decoded.ciphertext);
    assert_eq!(update.nonce, decoded.nonce);
    assert_eq!(
        update.public_data.document_id,
        decoded.public_data.document_id
    );
    assert_eq!(update.public_data.pub_key, decoded.public_data.pub_key);
    assert_eq!(
        update.public_data.snapshot_id,
        decoded.public_data.snapshot_id
    );
    assert_eq!(update.public_data.clock, decoded.public_data.clock);
    assert_eq!(update.commitment_tag, decoded.commitment_tag);
    assert_eq!(update.signature, decoded.signature);
}

#[test]
fn snapshot_roundtrip() {
    let snapshot = Snapshot {
        ciphertext: Ciphertext(vec![8, 9, 10]),
        nonce: Nonce([11u8; 24]),
        public_data: SnapshotPublicData {
            document_id: DocumentId::new(),
            pub_key: PublicKey::from_array([12u8; 32]),
            key_id: KeyId::from_array([8u8; 16]),
            snapshot_id: SnapshotId::new(),
            parent_snapshot_proof: vec![13, 14],
            parent_snapshot_update_clocks: vec![
                (PublicKey::from_array([15u8; 32]), Clock(16)),
                (PublicKey::from_array([17u8; 32]), Clock(18)),
            ],
        },
        commitment_tag: CommitmentTag([19u8; 32]),
        signature: Signature([20u8; 64]),
    };

    let bytes = postcard::to_allocvec(&snapshot).unwrap();
    let decoded: Snapshot = postcard::from_bytes(&bytes).unwrap();

    assert_eq!(snapshot.ciphertext, decoded.ciphertext);
    assert_eq!(snapshot.nonce, decoded.nonce);
    assert_eq!(
        snapshot.public_data.document_id,
        decoded.public_data.document_id
    );
    assert_eq!(snapshot.public_data.pub_key, decoded.public_data.pub_key);
    assert_eq!(
        snapshot.public_data.snapshot_id,
        decoded.public_data.snapshot_id
    );
    assert_eq!(
        snapshot.public_data.parent_snapshot_proof,
        decoded.public_data.parent_snapshot_proof
    );
    assert_eq!(
        snapshot.public_data.parent_snapshot_update_clocks,
        decoded.public_data.parent_snapshot_update_clocks
    );
    assert_eq!(snapshot.commitment_tag, decoded.commitment_tag);
    assert_eq!(snapshot.signature, decoded.signature);
}

#[test]
fn ephemeral_roundtrip() {
    let ephemeral = Ephemeral {
        ciphertext: Ciphertext(vec![21, 22, 23]),
        nonce: Nonce([24u8; 24]),
        public_data: EphemeralPublicData {
            document_id: DocumentId::new(),
            pub_key: PublicKey::from_array([25u8; 32]),
            key_id: KeyId::from_array([8u8; 16]),
            counter: Counter(26),
        },
        commitment_tag: CommitmentTag([27u8; 32]),
        signature: Signature([28u8; 64]),
    };

    let bytes = postcard::to_allocvec(&ephemeral).unwrap();
    let decoded: Ephemeral = postcard::from_bytes(&bytes).unwrap();

    assert_eq!(ephemeral.ciphertext, decoded.ciphertext);
    assert_eq!(ephemeral.nonce, decoded.nonce);
    assert_eq!(
        ephemeral.public_data.document_id,
        decoded.public_data.document_id
    );
    assert_eq!(ephemeral.public_data.pub_key, decoded.public_data.pub_key);
    assert_eq!(ephemeral.public_data.counter, decoded.public_data.counter);
    assert_eq!(ephemeral.commitment_tag, decoded.commitment_tag);
    assert_eq!(ephemeral.signature, decoded.signature);
}

#[test]
fn ws_messages_roundtrip() {
    let update = Update {
        ciphertext: Ciphertext(vec![1]),
        nonce: Nonce([2u8; 24]),
        public_data: UpdatePublicData {
            document_id: DocumentId::new(),
            pub_key: PublicKey::from_array([3u8; 32]),
            key_id: KeyId::from_array([8u8; 16]),
            snapshot_id: SnapshotId::new(),
            clock: Clock(4),
        },
        commitment_tag: CommitmentTag([5u8; 32]),
        signature: Signature([6u8; 64]),
    };

    let client_msg = WsClientMsg::Update(update.clone());
    let bytes = postcard::to_allocvec(&client_msg).unwrap();
    let decoded: WsClientMsg = postcard::from_bytes(&bytes).unwrap();
    match decoded {
        WsClientMsg::Update(decoded_update) => {
            assert_eq!(update.ciphertext, decoded_update.ciphertext);
        }
        _ => panic!("wrong variant"),
    }

    let server_msg = WsServerMsg::Update {
        seq: Seq(100),
        update,
    };
    let bytes = postcard::to_allocvec(&server_msg).unwrap();
    let decoded: WsServerMsg = postcard::from_bytes(&bytes).unwrap();
    match decoded {
        WsServerMsg::Update {
            seq,
            update: decoded_update,
        } => {
            assert_eq!(seq, Seq(100));
            assert_eq!(decoded_update.ciphertext, Ciphertext(vec![1]));
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn signaling_messages_roundtrip() {
    let client_msg = SignalingClientMsg::Offer {
        target_user_id: UserId::new(),
        sdp: "test sdp".to_string(),
    };
    let bytes = postcard::to_allocvec(&client_msg).unwrap();
    let decoded: SignalingClientMsg = postcard::from_bytes(&bytes).unwrap();
    match decoded {
        SignalingClientMsg::Offer {
            target_user_id: _,
            sdp,
        } => {
            assert_eq!(sdp, "test sdp");
        }
        _ => panic!("wrong variant"),
    }

    let server_msg = SignalingServerMsg::ParticipantJoined {
        user_id: UserId::new(),
    };
    let bytes = postcard::to_allocvec(&server_msg).unwrap();
    let decoded: SignalingServerMsg = postcard::from_bytes(&bytes).unwrap();
    match decoded {
        SignalingServerMsg::ParticipantJoined { user_id: _ } => {}
        _ => panic!("wrong variant"),
    }
}

#[test]
fn version_check() {
    use version::{CURRENT_VERSION, check_version};
    assert!(check_version(CURRENT_VERSION).is_ok());
    let other_version = ambre_core::version::Version::V1;
    assert!(check_version(other_version).is_err());
}

/// `SRV-100`: a wire message that does not decode is a typed error on either side
/// of the socket. The cases are no bytes, a foreign version, and every
/// truncation of a valid wire message.
#[test]
fn a_wire_message_that_does_not_decode_is_a_typed_error() {
    use crate::ws::{
        WireDecodeError, WsClientMsg, decode_client_msg, decode_server_msg, encode_client_msg,
    };
    assert!(matches!(
        decode_client_msg(&[]),
        Err(WireDecodeError::Empty)
    ));
    assert!(matches!(
        decode_server_msg(&[]),
        Err(WireDecodeError::Empty)
    ));
    assert!(matches!(
        decode_client_msg(&[0x01, 0x00]),
        Err(WireDecodeError::UnsupportedVersion(0x01))
    ));
    let wire = encode_client_msg(&WsClientMsg::Subscribe {
        document_id: DocumentId::new(),
    });
    for cut in 1..wire.len() {
        assert!(decode_client_msg(&wire[..cut]).is_err(), "cut at {cut}");
        let _ = decode_server_msg(&wire[..cut]);
    }
}

/// `SYNC-137`: only a frame of a newer wire version calls for an update. An
/// older one is the relay's lag, and the user cannot cure it.
#[test]
fn only_a_newer_wire_version_calls_for_an_update() {
    use crate::ws::{WireDecodeError, decode_server_msg};
    let current = CURRENT_VERSION.as_u8();
    let newer = decode_server_msg(&[current + 1, 0x00]).unwrap_err();
    let older = decode_server_msg(&[current - 1, 0x00]).unwrap_err();
    assert!(newer.is_from_newer_peer());
    assert!(!older.is_from_newer_peer());
    assert!(!WireDecodeError::Empty.is_from_newer_peer());
}

/// The digest of one frame per message variant, frozen under the version
/// byte it was taken at (`ambre-spec-broadcast.md`, `SRV-166`).
const FROZEN_LAYOUT: (u8, &str) = (8, "22b3cf269d91fe8cea311aed8def2bad");

fn sample_update(fill: u8) -> Update {
    Update {
        ciphertext: Ciphertext(vec![fill, fill + 1]),
        nonce: Nonce([fill; 24]),
        public_data: UpdatePublicData {
            document_id: DocumentId::from_bytes([fill; 16]),
            pub_key: PublicKey::from_array([fill + 2; 32]),
            key_id: KeyId::from_array([fill + 3; 16]),
            snapshot_id: SnapshotId::from_bytes([fill + 4; 16]),
            clock: Clock(u64::from(fill) + 5),
        },
        commitment_tag: CommitmentTag([fill + 6; 32]),
        signature: Signature([fill + 7; 64]),
    }
}

fn sample_ephemeral(fill: u8) -> Ephemeral {
    Ephemeral {
        ciphertext: Ciphertext(vec![fill]),
        nonce: Nonce([fill + 1; 24]),
        public_data: EphemeralPublicData {
            document_id: DocumentId::from_bytes([fill + 2; 16]),
            pub_key: PublicKey::from_array([fill + 3; 32]),
            key_id: KeyId::from_array([fill + 4; 16]),
            counter: Counter(u64::from(fill) + 5),
        },
        commitment_tag: CommitmentTag([fill + 6; 32]),
        signature: Signature([fill + 7; 64]),
    }
}

/// One client message per variant, in declaration order.
fn client_samples() -> Vec<crate::ws::WsClientMsg> {
    use crate::ws::WsClientMsg as C;
    let doc = DocumentId::from_bytes([1; 16]);
    vec![
        C::Subscribe { document_id: doc },
        C::Unsubscribe { document_id: doc },
        C::Update(sample_update(10)),
        C::Ephemeral(sample_ephemeral(20)),
        C::RequestMissing {
            document_id: doc,
            after_seq: Seq(3),
        },
        C::Authenticate {
            token: crate::ws::AccessToken::new("t".into()),
        },
        C::ClockResync {
            document_id: doc,
            signature: Signature([41; 64]),
        },
    ]
}

/// One server message per variant, in declaration order.
fn server_samples() -> Vec<crate::ws::WsServerMsg> {
    use crate::ws::WsServerMsg as S;
    let doc = DocumentId::from_bytes([1; 16]);
    vec![
        S::Subscribed { document_id: doc },
        S::Update {
            seq: Seq(4),
            update: sample_update(50),
        },
        S::Ephemeral(sample_ephemeral(60)),
        S::PeerLeft {
            document_id: doc,
            pub_key: PublicKey::from_array([5; 32]),
        },
        S::MissingUpdates {
            document_id: doc,
            updates: vec![SeqUpdate {
                seq: Seq(6),
                update: sample_update(70),
            }],
        },
        S::SnapshotRequired { document_id: doc },
        S::SnapshotAvailable {
            document_id: doc,
            snapshot_id: SnapshotId::from_bytes([7; 16]),
            covered_seq: Seq(12),
        },
        S::KeyRotationRequired { document_id: doc },
        S::DocumentRevoked { document_id: doc },
        S::DocumentShared {
            document_id: doc,
            doc_type: ambre_core::DocType::Toile,
        },
        S::Error {
            document_id: Some(doc),
            code: ambre_core::RefusalCode::Forbidden,
        },
        S::KeyRefused {
            document_id: doc,
            key_id: KeyId::from_array([10; 16]),
            clock: Clock(11),
        },
        S::ClockRefused {
            document_id: doc,
            clock: Clock(12),
            expected: Clock(13),
            signature: Signature([14; 64]),
        },
        S::MembersChanged { document_id: doc },
        S::ReplayStopped {
            document_id: doc,
            after_seq: Seq(15),
        },
    ]
}

/// The variant index each frame opens on, after the version byte. A sample set
/// that skips a variant, or misses one declared after the last, fails here.
fn assert_one_per_variant<T: serde::de::DeserializeOwned>(frames: &[Vec<u8>]) {
    let indices: Vec<u8> = frames.iter().map(|f| f[1]).collect();
    let expected: Vec<u8> = (0..frames.len()).map(|i| i as u8).collect();
    assert_eq!(indices, expected, "one sample per variant, in order");
    // A declared variant reads its index and then runs out of bytes; an
    // undeclared one is refused on the index itself.
    let last = [frames.len() as u8 - 1];
    let past_the_last = [frames.len() as u8];
    assert!(matches!(
        postcard::from_bytes::<T>(&last),
        Err(postcard::Error::DeserializeUnexpectedEnd)
    ));
    assert!(
        matches!(
            postcard::from_bytes::<T>(&past_the_last),
            Err(postcard::Error::SerdeDeCustom)
        ),
        "a variant has no sample"
    );
}

/// `SRV-166`: the wire layout cannot move under an unchanged version byte. The
/// digest covers one frame of every message variant. When it moves, the
/// version moves with it, and the digest is frozen again under the new byte.
///
/// It covers the word of every refusal code too (`SRV-183`). An older peer
/// cannot read a code it does not hold, so a new code moves the version. That
/// peer is then sent to an update rather than left with a frame it drops.
#[test]
fn the_wire_layout_is_frozen_under_its_version() {
    use crate::ws::{WsClientMsg, WsServerMsg, encode_client_msg, encode_server_msg};
    use blake2::{Blake2b512, Digest};
    let client: Vec<Vec<u8>> = client_samples().iter().map(encode_client_msg).collect();
    let server: Vec<Vec<u8>> = server_samples().iter().map(encode_server_msg).collect();
    assert_one_per_variant::<WsClientMsg>(&client);
    assert_one_per_variant::<WsServerMsg>(&server);
    let mut hasher = Blake2b512::new();
    for frame in client.iter().chain(&server) {
        hasher.update((frame.len() as u64).to_le_bytes());
        hasher.update(frame);
    }
    for code in ambre_core::RefusalCode::ALL {
        let word = code.as_str();
        hasher.update((word.len() as u64).to_le_bytes());
        hasher.update(word);
    }
    let digest = hex::encode(&hasher.finalize()[..16]);
    let (version, frozen) = FROZEN_LAYOUT;
    assert_eq!(
        version,
        CURRENT_VERSION.as_u8(),
        "the version moved: freeze the digest again under it"
    );
    assert_eq!(
        digest, frozen,
        "the wire layout moved under an unchanged version byte: bump CURRENT_VERSION"
    );
}

/// `ZK-007`, `ZK-013`: the fields a sealed envelope shows the relay. Each
/// destructuring names every field and uses no `..`. A field added to an
/// envelope fails to compile here until a writer weighs it against `ZK-007`.
/// Beyond the payload and its nonce, the envelope carries the commitment tag
/// and the signature, which bind it and open nothing.
#[test]
fn the_relay_reads_only_the_listed_fields() {
    let doc = DocumentId::new();
    let pk = PublicKey::from_array([5u8; 32]);
    let kid = KeyId::from_array([8u8; 16]);
    let update = Update {
        ciphertext: Ciphertext(vec![1]),
        nonce: Nonce([0u8; 24]),
        public_data: UpdatePublicData {
            document_id: doc,
            pub_key: pk.clone(),
            key_id: kid,
            snapshot_id: SnapshotId::new(),
            clock: Clock(1),
        },
        commitment_tag: CommitmentTag([6u8; 32]),
        signature: Signature([7u8; 64]),
    };
    let snapshot = Snapshot {
        ciphertext: Ciphertext(vec![1]),
        nonce: Nonce([0u8; 24]),
        public_data: SnapshotPublicData {
            document_id: doc,
            pub_key: pk.clone(),
            key_id: kid,
            snapshot_id: SnapshotId::new(),
            parent_snapshot_proof: vec![],
            parent_snapshot_update_clocks: vec![],
        },
        commitment_tag: CommitmentTag([6u8; 32]),
        signature: Signature([7u8; 64]),
    };
    let ephemeral = Ephemeral {
        ciphertext: Ciphertext(vec![1]),
        nonce: Nonce([0u8; 24]),
        public_data: EphemeralPublicData {
            document_id: doc,
            pub_key: pk.clone(),
            key_id: kid,
            counter: Counter(1),
        },
        commitment_tag: CommitmentTag([6u8; 32]),
        signature: Signature([7u8; 64]),
    };

    let Update {
        ciphertext: _,
        nonce: _,
        public_data,
        commitment_tag: _,
        signature: _,
    } = update;
    let UpdatePublicData {
        document_id: _,
        pub_key: _,
        key_id: _,
        snapshot_id: _,
        clock: _,
    } = public_data;
    let Snapshot {
        ciphertext: _,
        nonce: _,
        public_data,
        commitment_tag: _,
        signature: _,
    } = snapshot;
    let SnapshotPublicData {
        document_id: _,
        pub_key: _,
        key_id: _,
        snapshot_id: _,
        parent_snapshot_proof: _,
        parent_snapshot_update_clocks: _,
    } = public_data;
    let Ephemeral {
        ciphertext: _,
        nonce: _,
        public_data,
        commitment_tag: _,
        signature: _,
    } = ephemeral;
    // An ephemeral shows its document, its sender, its key identifier
    // and its counter, and nothing else.
    let EphemeralPublicData {
        document_id,
        pub_key,
        key_id,
        counter,
    } = public_data;
    assert_eq!(
        (document_id, pub_key, key_id, counter),
        (doc, pk, kid, Counter(1))
    );
}
