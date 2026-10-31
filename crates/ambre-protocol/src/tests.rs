//! Unit tests for the wire protocol types.

use super::*;
use crate::messages::{Ciphertext, Clock, Counter, Seq};
use ambre_core::FixedBytes;
use ambre_core::id::{self, Document, Id, User};

#[test]
fn update_roundtrip() {
    let update = Update {
        ciphertext: Ciphertext(vec![1, 2, 3, 4]),
        nonce: FixedBytes::from_array([0u8; 24]),
        public_data: UpdatePublicData {
            document_id: Id::<Document>::mint(),
            pub_key: FixedBytes::from_array([5u8; 32]),
            key_id: FixedBytes::from_array([8u8; 16]),
            snapshot_id: Id::<id::Snapshot>::mint(),
            clock: Clock(42),
        },
        commitment_tag: FixedBytes::from_array([6u8; 32]),
        signature: FixedBytes::from_array([7u8; 64]),
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
        nonce: FixedBytes::from_array([11u8; 24]),
        public_data: SnapshotPublicData {
            document_id: Id::<Document>::mint(),
            pub_key: FixedBytes::from_array([12u8; 32]),
            key_id: FixedBytes::from_array([8u8; 16]),
            snapshot_id: Id::<id::Snapshot>::mint(),
            parent_snapshot_proof: Some(FixedBytes::from_array([13u8; 32])),
            parent_snapshot_update_clocks: vec![
                (FixedBytes::from_array([15u8; 32]), Clock(16)),
                (FixedBytes::from_array([17u8; 32]), Clock(18)),
            ],
        },
        commitment_tag: FixedBytes::from_array([19u8; 32]),
        signature: FixedBytes::from_array([20u8; 64]),
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
        nonce: FixedBytes::from_array([24u8; 24]),
        public_data: EphemeralPublicData {
            document_id: Id::<Document>::mint(),
            pub_key: FixedBytes::from_array([25u8; 32]),
            key_id: FixedBytes::from_array([8u8; 16]),
            counter: Counter(26),
        },
        commitment_tag: FixedBytes::from_array([27u8; 32]),
        signature: FixedBytes::from_array([28u8; 64]),
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
        nonce: FixedBytes::from_array([2u8; 24]),
        public_data: UpdatePublicData {
            document_id: Id::<Document>::mint(),
            pub_key: FixedBytes::from_array([3u8; 32]),
            key_id: FixedBytes::from_array([8u8; 16]),
            snapshot_id: Id::<id::Snapshot>::mint(),
            clock: Clock(4),
        },
        commitment_tag: FixedBytes::from_array([5u8; 32]),
        signature: FixedBytes::from_array([6u8; 64]),
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
        target_user_id: Id::<User>::mint(),
        sdp: SessionDescription::new("test sdp".to_owned()),
    };
    let bytes = postcard::to_allocvec(&client_msg).unwrap();
    let decoded: SignalingClientMsg = postcard::from_bytes(&bytes).unwrap();
    match decoded {
        SignalingClientMsg::Offer {
            target_user_id: _,
            sdp,
        } => {
            assert_eq!(sdp.as_str(), "test sdp");
        }
        _ => panic!("wrong variant"),
    }

    let server_msg = SignalingServerMsg::ParticipantJoined {
        user_id: Id::<User>::mint(),
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
    let other_version = ambre_core::version::Version::V0;
    assert!(check_version(other_version).is_err());
}

/// `SRV-100`: a wire message that does not decode is a typed error on either side
/// of the socket. The cases are no bytes, a foreign version, and every
/// truncation of a valid wire message.
#[test]
fn a_wire_message_that_does_not_decode_is_a_typed_error() {
    use crate::ws::{
        WireDecodeError, WireFrame, WsClientMsg, decode_client_msg, decode_server_msg,
        encode_client_msg,
    };
    let frame = |bytes: &[u8]| WireFrame::new(bytes.to_vec());
    assert!(matches!(
        decode_client_msg(&frame(&[])),
        Err(WireDecodeError::Empty)
    ));
    assert!(matches!(
        decode_server_msg(&frame(&[])),
        Err(WireDecodeError::Empty)
    ));
    assert!(matches!(
        decode_client_msg(&frame(&[0x02, 0x00])),
        Err(WireDecodeError::UnsupportedVersion(0x02))
    ));
    let wire = encode_client_msg(&WsClientMsg::Subscribe {
        document_id: Id::<Document>::mint(),
    });
    for cut in 1..wire.len() {
        let cut_frame = frame(&wire.as_bytes()[..cut]);
        assert!(decode_client_msg(&cut_frame).is_err(), "cut at {cut}");
        let _ = decode_server_msg(&cut_frame);
    }
}

/// `SYNC-137`: only a frame of a newer wire version calls for an update. An
/// older one is the relay's lag, and the user cannot cure it.
#[test]
fn only_a_newer_wire_version_calls_for_an_update() {
    use crate::ws::{WireDecodeError, WireFrame, decode_server_msg};
    let current = CURRENT_VERSION.as_u8();
    let newer = decode_server_msg(&WireFrame::new(vec![current + 1, 0x00])).unwrap_err();
    let older = decode_server_msg(&WireFrame::new(vec![current - 1, 0x00])).unwrap_err();
    assert!(newer.is_from_newer_peer());
    assert!(!older.is_from_newer_peer());
    assert!(!WireDecodeError::Empty.is_from_newer_peer());
}

/// The digest of one frame per message variant, frozen under the version
/// byte it was taken at (`ambre-spec-broadcast.md`, `SRV-166`).
const FROZEN_LAYOUT: (u8, &str) = (1, "c1985c0e3889a14100b99c48d8952d65");

fn sample_update(fill: u8) -> Update {
    Update {
        ciphertext: Ciphertext(vec![fill, fill + 1]),
        nonce: FixedBytes::from_array([fill; 24]),
        public_data: UpdatePublicData {
            document_id: Id::<Document>::from_bytes([fill; 16]),
            pub_key: FixedBytes::from_array([fill + 2; 32]),
            key_id: FixedBytes::from_array([fill + 3; 16]),
            snapshot_id: Id::<id::Snapshot>::from_bytes([fill + 4; 16]),
            clock: Clock(u64::from(fill) + 5),
        },
        commitment_tag: FixedBytes::from_array([fill + 6; 32]),
        signature: FixedBytes::from_array([fill + 7; 64]),
    }
}

fn sample_ephemeral(fill: u8) -> Ephemeral {
    Ephemeral {
        ciphertext: Ciphertext(vec![fill]),
        nonce: FixedBytes::from_array([fill + 1; 24]),
        public_data: EphemeralPublicData {
            document_id: Id::<Document>::from_bytes([fill + 2; 16]),
            pub_key: FixedBytes::from_array([fill + 3; 32]),
            key_id: FixedBytes::from_array([fill + 4; 16]),
            counter: Counter(u64::from(fill) + 5),
        },
        commitment_tag: FixedBytes::from_array([fill + 6; 32]),
        signature: FixedBytes::from_array([fill + 7; 64]),
    }
}

/// One client message per variant, in declaration order.
fn client_samples() -> Vec<crate::ws::WsClientMsg> {
    use crate::ws::WsClientMsg as C;
    let doc = Id::<Document>::from_bytes([1; 16]);
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
            token: crate::ws::Credential::new("t".into()),
        },
        C::ClockResync {
            document_id: doc,
            signature: FixedBytes::from_array([41; 64]),
        },
    ]
}

/// One server message per variant, in declaration order.
fn server_samples() -> Vec<crate::ws::WsServerMsg> {
    use crate::ws::WsServerMsg as S;
    let doc = Id::<Document>::from_bytes([1; 16]);
    vec![
        S::Subscribed { document_id: doc },
        S::Update {
            seq: Seq(4),
            update: sample_update(50),
        },
        S::Ephemeral(sample_ephemeral(60)),
        S::PeerLeft {
            document_id: doc,
            pub_key: FixedBytes::from_array([5; 32]),
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
            snapshot_id: Id::<id::Snapshot>::from_bytes([7; 16]),
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
            key_id: FixedBytes::from_array([10; 16]),
            clock: Clock(11),
        },
        S::ClockRefused {
            document_id: doc,
            clock: Clock(12),
            expected: Clock(13),
            signature: FixedBytes::from_array([14; 64]),
        },
        S::MembersChanged { document_id: doc },
        S::ReplayStopped {
            document_id: doc,
            after_seq: Seq(15),
        },
        S::DeltaRefused {
            document_id: doc,
            signature: FixedBytes::from_array([16; 64]),
            code: ambre_core::RefusalCode::PayloadTooLarge,
        },
        S::SnapshotDropped {
            document_id: doc,
            snapshot_id: Id::<id::Snapshot>::from_bytes([17; 16]),
            author: FixedBytes::from_array([18; 32]),
            reporter: FixedBytes::from_array([19; 32]),
            after: ambre_core::AfterDrop::Refinalize,
        },
        S::ThreadActivity { document_id: doc },
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
    let client: Vec<Vec<u8>> = client_samples()
        .iter()
        .map(|msg| encode_client_msg(msg).into_bytes())
        .collect();
    let server: Vec<Vec<u8>> = server_samples()
        .iter()
        .map(|msg| encode_server_msg(msg).into_bytes())
        .collect();
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
    let doc = Id::<Document>::mint();
    let pk = FixedBytes::from_array([5u8; 32]);
    let kid = FixedBytes::from_array([8u8; 16]);
    let update = Update {
        ciphertext: Ciphertext(vec![1]),
        nonce: FixedBytes::from_array([0u8; 24]),
        public_data: UpdatePublicData {
            document_id: doc,
            pub_key: pk,
            key_id: kid,
            snapshot_id: Id::<id::Snapshot>::mint(),
            clock: Clock(1),
        },
        commitment_tag: FixedBytes::from_array([6u8; 32]),
        signature: FixedBytes::from_array([7u8; 64]),
    };
    let snapshot = Snapshot {
        ciphertext: Ciphertext(vec![1]),
        nonce: FixedBytes::from_array([0u8; 24]),
        public_data: SnapshotPublicData {
            document_id: doc,
            pub_key: pk,
            key_id: kid,
            snapshot_id: Id::<id::Snapshot>::mint(),
            parent_snapshot_proof: None,
            parent_snapshot_update_clocks: vec![],
        },
        commitment_tag: FixedBytes::from_array([6u8; 32]),
        signature: FixedBytes::from_array([7u8; 64]),
    };
    let ephemeral = Ephemeral {
        ciphertext: Ciphertext(vec![1]),
        nonce: FixedBytes::from_array([0u8; 24]),
        public_data: EphemeralPublicData {
            document_id: doc,
            pub_key: pk,
            key_id: kid,
            counter: Counter(1),
        },
        commitment_tag: FixedBytes::from_array([6u8; 32]),
        signature: FixedBytes::from_array([7u8; 64]),
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

/// A close code is one a close frame can carry, 1000 to 4999 (RFC 6455
/// section 7.4). Zero and the codes past the range build none, so no number
/// stands for a socket that failed without a close.
#[test]
fn a_close_code_is_one_a_close_frame_can_carry() {
    use crate::ws::CloseCode;
    assert_eq!(CloseCode::new(0), None);
    assert_eq!(CloseCode::new(999), None);
    assert_eq!(CloseCode::new(5000), None);
    assert_eq!(CloseCode::new(4401), Some(CloseCode::UNAUTHORIZED));
    assert_eq!(CloseCode::new(4403), Some(CloseCode::SETUP_REQUIRED));
    assert_ne!(CloseCode::UNAUTHORIZED, CloseCode::SETUP_REQUIRED);
    assert_eq!(CloseCode::new(1006).map(CloseCode::get), Some(1006));
    assert_eq!(CloseCode::UNAUTHORIZED.reason(), Some("unauthorized"));
    assert_eq!(CloseCode::SETUP_REQUIRED.reason(), Some("setup required"));
    assert_eq!(CloseCode::new(1006).and_then(CloseCode::reason), None);
}

/// The Blake2b digest of `frame`, first 16 bytes, in hex: the form the frames
/// below are frozen in.
fn frozen_digest(frame: &[u8]) -> String {
    use blake2::{Blake2b512, Digest};
    let mut hasher = Blake2b512::new();
    hasher.update((frame.len() as u64).to_le_bytes());
    hasher.update(frame);
    hex::encode(&hasher.finalize()[..16])
}

/// The bytes a key link, a membership entry, the two signed transcripts and
/// the signaling frames take, frozen. No version digest covers them: a link
/// and an entry are stored, and a signature over a transcript must keep
/// verifying. A type that wraps one of them leaves these digests as they are.
#[test]
fn the_frames_outside_the_versioned_layout_keep_their_bytes() {
    use crate::membership::ENTRY_SIGNATURE_LEN;
    use crate::snapshot_report::report_transcript;
    use ambre_core::Role;
    let document = Id::<Document>::from_bytes([1; 16]);
    let link = KeyLink {
        document_id: document,
        key: FixedBytes::from_array([2; 16]),
        previous: FixedBytes::from_array([3; 16]),
        sealed: ambre_core::Payload::new(vec![7; 5]),
    };
    let entry = MembershipEntry {
        document,
        position: LogPosition::new(1),
        member: Id::<User>::from_bytes([2; 16]),
        member_key: FixedBytes::from_array([3; 32]),
        act: MembershipAct::Invite,
        role: Role::Editor,
        key: Some(FixedBytes::from_array([4; 16])),
        previous: Some(FixedBytes::from_array([5; 32])),
        author: Id::<User>::from_bytes([6; 16]),
        signature: FixedBytes::from_array([7; ENTRY_SIGNATURE_LEN]),
    };
    let transcript = report_transcript(
        document,
        Id::<id::Snapshot>::from_bytes([8; 16]),
        &FixedBytes::from_array([9; 32]),
    );
    let peer = Id::<User>::from_bytes([10; 16]);
    let client = [
        SignalingClientMsg::Offer {
            target_user_id: peer,
            sdp: SessionDescription::new("v=0 offer".to_owned()),
        },
        SignalingClientMsg::Answer {
            target_user_id: peer,
            sdp: SessionDescription::new("v=0 answer".to_owned()),
        },
        SignalingClientMsg::Ice {
            target_user_id: peer,
            candidate: IceCandidate::new("candidate:1 1 udp".to_owned()),
        },
    ];
    let server = [
        SignalingServerMsg::Offer {
            from_user_id: peer,
            sdp: SessionDescription::new("v=0 offer".to_owned()),
        },
        SignalingServerMsg::Answer {
            from_user_id: peer,
            sdp: SessionDescription::new("v=0 answer".to_owned()),
        },
        SignalingServerMsg::Ice {
            from_user_id: peer,
            candidate: IceCandidate::new("candidate:1 1 udp".to_owned()),
        },
        SignalingServerMsg::ParticipantJoined { user_id: peer },
        SignalingServerMsg::ParticipantLeft { user_id: peer },
    ];
    let mut signaling = Vec::new();
    for msg in &client {
        signaling.extend(postcard::to_allocvec(msg).expect("a frame encodes"));
    }
    for msg in &server {
        signaling.extend(postcard::to_allocvec(msg).expect("a frame encodes"));
    }
    let frames = [
        (
            "key link",
            link.encode().expect("a link encodes").into_bytes(),
        ),
        (
            "entry",
            entry.encode().expect("an entry encodes").into_bytes(),
        ),
        (
            "entry signed",
            entry
                .signed_bytes()
                .expect("an entry encodes")
                .as_bytes()
                .to_vec(),
        ),
        ("report", transcript.as_bytes().to_vec()),
        ("signaling", signaling),
    ];
    let digests: Vec<(&str, String)> = frames
        .iter()
        .map(|(name, frame)| (*name, frozen_digest(frame)))
        .collect();
    assert_eq!(
        digests,
        [
            ("key link", "cbf2375a41882f5485f3e8bc4221acf7".to_owned()),
            ("entry", "9ae9a8b1e141a84e0dfc37395bb2947d".to_owned()),
            (
                "entry signed",
                "5794caa1a99cb3f42d232280d662a90b".to_owned()
            ),
            ("report", "d58f6df5ac304352e01488b1981f97be".to_owned()),
            ("signaling", "6a9b7d2c9be10fb8fc203094c53eff55".to_owned()),
        ]
    );
}
