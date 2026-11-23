//! Unit tests for the wire protocol types.

use super::*;
use crate::messages::{Ciphertext, DeltaAuthor, Ephemerals, Relay, Seq};
use ambre_core::id::{self, Document, Id, User};
use ambre_core::{FixedBytes, Payload};

#[test]
fn update_roundtrip() {
    let update = Update {
        ciphertext: Payload::<Ciphertext>::new(vec![1, 2, 3, 4]),
        nonce: FixedBytes::from_array([0u8; 24]),
        public_data: UpdatePublicData {
            document_id: Id::<Document>::mint(),
            pub_key: FixedBytes::from_array([5u8; 32]),
            key_id: FixedBytes::from_array([8u8; 16]),
            snapshot_id: Id::<id::Snapshot>::mint(),
            clock: Seq::<DeltaAuthor>::new(42),
            log_head: crate::LogHead::EMPTY,
            log_entry: None,
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
        ciphertext: Payload::<Ciphertext>::new(vec![8, 9, 10]),
        nonce: FixedBytes::from_array([11u8; 24]),
        public_data: SnapshotPublicData {
            document_id: Id::<Document>::mint(),
            pub_key: FixedBytes::from_array([12u8; 32]),
            key_id: FixedBytes::from_array([8u8; 16]),
            snapshot_id: Id::<id::Snapshot>::mint(),
            covered_seq: Seq::<Relay>::new(14),
            parent_snapshot_proof: Some(FixedBytes::from_array([13u8; 32])),
            parent_snapshot_update_clocks: vec![
                (
                    FixedBytes::from_array([15u8; 32]),
                    Seq::<DeltaAuthor>::new(16),
                ),
                (
                    FixedBytes::from_array([17u8; 32]),
                    Seq::<DeltaAuthor>::new(18),
                ),
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
        ciphertext: Payload::<Ciphertext>::new(vec![21, 22, 23]),
        nonce: FixedBytes::from_array([24u8; 24]),
        public_data: EphemeralPublicData {
            document_id: Id::<Document>::mint(),
            pub_key: FixedBytes::from_array([25u8; 32]),
            key_id: FixedBytes::from_array([8u8; 16]),
            counter: Seq::<Ephemerals>::new(26),
            log_head: crate::LogHead::EMPTY,
            log_entry: None,
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
        ciphertext: Payload::<Ciphertext>::new(vec![1]),
        nonce: FixedBytes::from_array([2u8; 24]),
        public_data: UpdatePublicData {
            document_id: Id::<Document>::mint(),
            pub_key: FixedBytes::from_array([3u8; 32]),
            key_id: FixedBytes::from_array([8u8; 16]),
            snapshot_id: Id::<id::Snapshot>::mint(),
            clock: Seq::<DeltaAuthor>::new(4),
            log_head: crate::LogHead::EMPTY,
            log_entry: None,
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
        seq: Seq::<Relay>::new(100),
        update,
    };
    let bytes = postcard::to_allocvec(&server_msg).unwrap();
    let decoded: WsServerMsg = postcard::from_bytes(&bytes).unwrap();
    match decoded {
        WsServerMsg::Update {
            seq,
            update: decoded_update,
        } => {
            assert_eq!(seq, Seq::<Relay>::new(100));
            assert_eq!(
                decoded_update.ciphertext,
                Payload::<Ciphertext>::new(vec![1])
            );
        }
        _ => panic!("wrong variant"),
    }
}

/// `ZK-023`: an admission names the call, the participant and its member by three pseudonyms.
/// It adds the sender index of the seat and an expiry, and nothing else. Every key identifier shows the SFU that
/// index already. The destructure names every field without `..`, so a new field
/// fails to compile until a writer weighs it against the row. The token reads back whole.
#[test]
fn an_admission_names_the_call_and_the_participant_by_pseudonyms_alone() {
    use ambre_core::{Instant, UnixSec};
    let admission = Admission {
        room: FixedBytes::from_array([1; 16]),
        participant: FixedBytes::from_array([2; 16]),
        member: FixedBytes::from_array([4; 16]),
        sender: Seq::new(3),
        expires: Instant::<UnixSec>::new(1_700_000_060),
    };
    let Admission {
        room,
        participant,
        member,
        sender,
        expires,
    } = admission.clone();
    assert_eq!(
        (
            room.as_slice(),
            participant.as_slice(),
            member.as_slice(),
            sender.get(),
            expires.get()
        ),
        (&[1; 16][..], &[2; 16][..], &[4; 16][..], 3, 1_700_000_060)
    );
    let signed = admission.signed_bytes().expect("an admission encodes");
    assert!(signed.as_bytes().starts_with(b"ambre-call-admission-v3"));
    let signature = FixedBytes::from_array([9; 64]);
    let token = admission.token(&signature).expect("a token encodes");
    let printed = format!("{token:?}");
    assert!(!printed.contains("0101"), "the debug form prints {printed}");
    let wire = postcard::to_allocvec(&token).expect("a token travels");
    let text: String = postcard::from_bytes(&wire).expect("a token travels as its text");
    let bytes = hex::decode(text).expect("a token is hex");
    assert_eq!(
        Admission::read(&Payload::new(bytes)),
        Ok((admission, signature))
    );
    assert_eq!(
        Admission::read(&Payload::new(vec![0; 10])),
        Err(signaling::AdmissionDecodeError::Truncated)
    );
}

/// `ZK-022`: the relay reads four fields of a call frame. They are the channel, the opaque
/// signed join, the sender index a wrap is for and the opaque wrap. Each variant is destructured
/// without `..`, so a new field fails to compile until a writer weighs it against the row.
/// The access token prints no byte.
#[test]
fn the_relay_reads_only_the_listed_fields_of_a_call_frame() {
    use ambre_core::id::Channel;
    let frames = [
        CallClientMsg::Join {
            token: ambre_core::Credential::new("eyJ.secret".to_owned()),
            channel: Id::<Channel>::from_bytes([3; 16]),
        },
        CallClientMsg::Announce(Payload::new(vec![6; 192])),
        CallClientMsg::Wrap {
            to: ambre_core::Seq::new(4),
            wrap: Payload::new(vec![5; 80]),
        },
        CallClientMsg::Start {
            token: ambre_core::Credential::new("eyJ.secret".to_owned()),
            channel: Id::<Channel>::from_bytes([3; 16]),
        },
        CallClientMsg::Leave,
    ];
    for frame in frames {
        let bytes = postcard::to_allocvec(&frame).expect("a frame encodes");
        let printed = format!("{frame:?}");
        assert!(!printed.contains("secret"), "{printed}");
        match postcard::from_bytes::<CallClientMsg>(&bytes).expect("a frame decodes") {
            CallClientMsg::Join { token: _, channel }
            | CallClientMsg::Start { token: _, channel } => {
                assert_eq!(channel, Id::<Channel>::from_bytes([3; 16]));
            }
            CallClientMsg::Leave => {}
            CallClientMsg::Announce(join) => {
                assert_eq!(join.len(), ambre_core::Count::new(192));
            }
            CallClientMsg::Wrap { to, wrap } => {
                assert_eq!(to, ambre_core::Seq::new(4));
                assert_eq!(wrap.len(), ambre_core::Count::new(80));
            }
        }
    }
}

/// The messages of the socket of the SFU keep the JSON of one key that the SFU reads. A
/// session description longer than the SFU reads is refused before it leaves.
#[test]
fn a_message_of_the_sfu_socket_is_json_of_one_key() {
    use ambre_core::Validated;
    use signaling::{SessionDescription, SfuMsg};
    let offer = SfuMsg::Offer(Validated::parse("v=0\r\n").expect("a short description"));
    let text = serde_json::to_string(&offer).expect("a message encodes");
    assert_eq!(text, r#"{"offer":"v=0\r\n"}"#);
    let read: SfuMsg = serde_json::from_str(r#"{"ice":[]}"#).expect("a message decodes");
    assert!(matches!(read, SfuMsg::Ice(servers) if servers.is_empty()));
    let long = "a".repeat(
        signaling::SESSION_DESCRIPTION_MAX
            .saturating_len()
            .to_usize()
            + 1,
    );
    assert!(Validated::<SessionDescription>::parse(&long).is_err());
}

/// `CALL-131`: a choice of videos reads as JSON of one key, with the count of videos its
/// receiver lays out. The SFU reads none of a choice that names too many senders. It reads none
/// whose count is past the desktop count or under its senders, nor a sender twice, nor a stray
/// large sender.
#[test]
fn a_choice_of_too_many_videos_or_a_stray_large_sender_reads_as_nothing() {
    use ambre_core::{Count, Seq};
    use signaling::{ChoiceError, PHONE_VIDEOS, SfuMsg, VIDEOS_PER_RECEIVER, VideoChoice};
    let choice = VideoChoice::new(
        vec![Seq::new(3), Seq::new(7)],
        Some(Seq::new(7)),
        PHONE_VIDEOS,
    )
    .expect("a choice of two");
    let text = serde_json::to_string(&SfuMsg::Videos(choice.clone())).expect("a choice encodes");
    assert_eq!(text, r#"{"videos":{"senders":[3,7],"large":7,"count":9}}"#);
    let read: SfuMsg = serde_json::from_str(&text).expect("a choice decodes");
    assert!(matches!(read, SfuMsg::Videos(c) if c == choice));
    let bare: SfuMsg =
        serde_json::from_str(r#"{"videos":{"senders":[3],"large":null}}"#).expect("no count");
    assert!(
        matches!(bare, SfuMsg::Videos(c) if c.count() == VIDEOS_PER_RECEIVER),
        "a choice with no count asks for the desktop count"
    );
    let nine: Vec<Seq<_>> = (1..=9).map(Seq::new).collect();
    assert!(
        VideoChoice::new(nine, None, VIDEOS_PER_RECEIVER).is_ok(),
        "nine senders fit"
    );
    let ten = r#"{"videos":{"senders":[1,2,3,4,5,6,7,8,9,10],"large":null}}"#;
    let twice = r#"{"videos":{"senders":[4,4],"large":null}}"#;
    let stray = r#"{"videos":{"senders":[4],"large":5}}"#;
    let past = r#"{"videos":{"senders":[4],"large":null,"count":10}}"#;
    let under = r#"{"videos":{"senders":[4,5],"large":null,"count":1}}"#;
    for refused in [ten, twice, stray, past, under] {
        assert!(
            serde_json::from_str::<SfuMsg>(refused).is_err(),
            "{refused} was read"
        );
    }
    let ten: Vec<Seq<_>> = (1..=10).map(Seq::new).collect();
    assert_eq!(
        VideoChoice::new(ten, None, VIDEOS_PER_RECEIVER),
        Err(ChoiceError::TooMany)
    );
    assert_eq!(
        VideoChoice::new(vec![Seq::new(4)], None, Count::new(10)),
        Err(ChoiceError::Count)
    );
    assert_eq!(
        VideoChoice::new(vec![Seq::new(4), Seq::new(4)], None, PHONE_VIDEOS),
        Err(ChoiceError::Repeated)
    );
    assert_eq!(
        VideoChoice::new(vec![Seq::new(4)], Some(Seq::new(5)), PHONE_VIDEOS),
        Err(ChoiceError::LargeNotChosen)
    );
}

#[test]
fn version_check() {
    use version::{CURRENT_VERSION, check_version};
    assert!(check_version(CURRENT_VERSION).is_ok());
    let other_version = ambre_core::version::Version::<ambre_core::version::Protocol>::V0;
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
    let frame = |bytes: &[u8]| Payload::<WireFrame>::new(bytes.to_vec());
    assert!(matches!(
        decode_client_msg(&frame(&[])),
        Err(WireDecodeError::Empty)
    ));
    assert!(matches!(
        decode_server_msg(&frame(&[])),
        Err(WireDecodeError::Empty)
    ));
    let newer = CURRENT_VERSION.get() + 1;
    assert!(matches!(
        decode_client_msg(&frame(&[newer, 0x00])),
        Err(WireDecodeError::UnsupportedVersion(v)) if v == newer
    ));
    let wire = encode_client_msg(&WsClientMsg::Subscribe {
        document_id: Id::<Document>::mint(),
    });
    for cut in 1..wire.as_bytes().len() {
        let cut_frame = frame(&wire.as_bytes()[..cut]);
        assert!(decode_client_msg(&cut_frame).is_err(), "cut at {cut}");
        let _ = decode_server_msg(&cut_frame);
    }
}

/// `SYNC-137`: only a frame of a newer wire version calls for an update. An
/// older one is the relay's lag, and the user cannot cure it.
#[test]
fn only_a_newer_wire_version_calls_for_an_update() {
    use crate::ws::{WireDecodeError, decode_server_msg};
    let current = CURRENT_VERSION.get();
    let newer = decode_server_msg(&Payload::new(vec![current + 1, 0x00])).unwrap_err();
    let older = decode_server_msg(&Payload::new(vec![current - 1, 0x00])).unwrap_err();
    assert!(newer.is_from_newer_peer());
    assert!(!older.is_from_newer_peer());
    assert!(!WireDecodeError::Empty.is_from_newer_peer());
}

/// The digest of one frame per message variant, frozen under the version
/// byte it was taken at (`ambre-spec-broadcast.md`, `SRV-166`).
const FROZEN_LAYOUT: (u8, &str) = (5, "19272405ee1116959338ed0dcf9f28cb");

fn sample_update(fill: u8) -> Update {
    Update {
        ciphertext: Payload::<Ciphertext>::new(vec![fill, fill + 1]),
        nonce: FixedBytes::from_array([fill; 24]),
        public_data: UpdatePublicData {
            document_id: Id::<Document>::from_bytes([fill; 16]),
            pub_key: FixedBytes::from_array([fill + 2; 32]),
            key_id: FixedBytes::from_array([fill + 3; 16]),
            snapshot_id: Id::<id::Snapshot>::from_bytes([fill + 4; 16]),
            clock: Seq::<DeltaAuthor>::new(u64::from(fill) + 5),
            log_head: crate::LogHead::EMPTY.after(FixedBytes::from_array([fill + 8; 32])),
            // The entry the head names, so the layout of an entry is frozen
            // too (`SYNC-218`).
            log_entry: Some(Box::new(crate::MembershipEntry {
                document: Id::<Document>::from_bytes([fill; 16]),
                position: Seq::new(0),
                member: Id::<User>::from_bytes([fill + 9; 16]),
                member_key: FixedBytes::from_array([fill + 10; 32]),
                member_exchange_key: FixedBytes::from_array([fill + 11; 32]),
                act: crate::MembershipAct::Create,
                role: ambre_core::Role::Owner,
                key: Some(FixedBytes::from_array([fill + 12; 16])),
                previous: None,
                author: Id::<User>::from_bytes([fill + 9; 16]),
                cap: None,
                signature: FixedBytes::from_array([fill + 13; 64]),
            })),
        },
        commitment_tag: FixedBytes::from_array([fill + 6; 32]),
        signature: FixedBytes::from_array([fill + 7; 64]),
    }
}

fn sample_ephemeral(fill: u8) -> Ephemeral {
    Ephemeral {
        ciphertext: Payload::<Ciphertext>::new(vec![fill]),
        nonce: FixedBytes::from_array([fill + 1; 24]),
        public_data: EphemeralPublicData {
            document_id: Id::<Document>::from_bytes([fill + 2; 16]),
            pub_key: FixedBytes::from_array([fill + 3; 32]),
            key_id: FixedBytes::from_array([fill + 4; 16]),
            counter: Seq::<Ephemerals>::new(u64::from(fill) + 5),
            // The head and the entry of the update sample, so the layout of
            // both on a presence sample is frozen too (`SYNC-262`).
            log_head: sample_update(fill).public_data.log_head,
            log_entry: sample_update(fill).public_data.log_entry,
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
            after_seq: Seq::<Relay>::new(3),
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
            seq: Seq::<Relay>::new(4),
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
                seq: Seq::<Relay>::new(6),
                update: sample_update(70),
            }],
        },
        S::SnapshotRequired { document_id: doc },
        S::SnapshotAvailable {
            document_id: doc,
            snapshot_id: Id::<id::Snapshot>::from_bytes([7; 16]),
            covered_seq: Seq::<Relay>::new(12),
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
            clock: Seq::<DeltaAuthor>::new(11),
        },
        S::ClockRefused {
            document_id: doc,
            clock: Seq::<DeltaAuthor>::new(12),
            expected: Seq::<DeltaAuthor>::new(13),
            signature: FixedBytes::from_array([14; 64]),
        },
        S::MembersChanged { document_id: doc },
        S::ReplayStopped {
            document_id: doc,
            after_seq: Seq::<Relay>::new(15),
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
        S::AddressReclaimed {
            document_id: doc,
            member: Id::<User>::from_bytes([17; 16]),
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
        CURRENT_VERSION.get(),
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
        ciphertext: Payload::<Ciphertext>::new(vec![1]),
        nonce: FixedBytes::from_array([0u8; 24]),
        public_data: UpdatePublicData {
            document_id: doc,
            pub_key: pk,
            key_id: kid,
            snapshot_id: Id::<id::Snapshot>::mint(),
            clock: Seq::<DeltaAuthor>::new(1),
            log_head: crate::LogHead::EMPTY,
            log_entry: None,
        },
        commitment_tag: FixedBytes::from_array([6u8; 32]),
        signature: FixedBytes::from_array([7u8; 64]),
    };
    let snapshot = Snapshot {
        ciphertext: Payload::<Ciphertext>::new(vec![1]),
        nonce: FixedBytes::from_array([0u8; 24]),
        public_data: SnapshotPublicData {
            document_id: doc,
            pub_key: pk,
            key_id: kid,
            snapshot_id: Id::<id::Snapshot>::mint(),
            covered_seq: Seq::<Relay>::new(9),
            parent_snapshot_proof: None,
            parent_snapshot_update_clocks: vec![],
        },
        commitment_tag: FixedBytes::from_array([6u8; 32]),
        signature: FixedBytes::from_array([7u8; 64]),
    };
    let ephemeral = Ephemeral {
        ciphertext: Payload::<Ciphertext>::new(vec![1]),
        nonce: FixedBytes::from_array([0u8; 24]),
        public_data: EphemeralPublicData {
            document_id: doc,
            pub_key: pk,
            key_id: kid,
            counter: Seq::<Ephemerals>::new(1),
            log_head: crate::LogHead::EMPTY,
            log_entry: None,
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
        log_head: _,
        log_entry: _,
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
        covered_seq: _,
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
    // An ephemeral shows its document, its sender, its key identifier, its
    // counter, and the log head and entry its sender read (`SYNC-262`).
    let EphemeralPublicData {
        document_id,
        pub_key,
        key_id,
        counter,
        log_head,
        log_entry,
    } = public_data;
    assert_eq!(
        (document_id, pub_key, key_id, counter, log_head, log_entry),
        (
            doc,
            pk,
            kid,
            Seq::<Ephemerals>::new(1),
            crate::LogHead::EMPTY,
            None
        )
    );
}

/// A close code is one a close frame can carry, 1000 to 4999 (RFC 6455
/// section 7.4). Zero and the codes past the range build none, so no number
/// stands for a socket that failed without a close.
#[test]
fn a_close_code_is_one_a_close_frame_can_carry() {
    use crate::ws::{CloseCode, RelayClose};
    use ambre_core::Bounded;
    assert_eq!(Bounded::<CloseCode>::new(0), None);
    assert_eq!(Bounded::<CloseCode>::new(999), None);
    assert_eq!(Bounded::<CloseCode>::new(5000), None);
    assert_eq!(
        Bounded::<CloseCode>::new(1000),
        Some(Bounded::<CloseCode>::MIN)
    );
    assert_eq!(
        Bounded::<CloseCode>::new(4999),
        Some(Bounded::<CloseCode>::MAX)
    );
    assert_eq!(
        Bounded::<CloseCode>::new(4401),
        Some(Bounded::<CloseCode>::UNAUTHORIZED)
    );
    assert_eq!(
        Bounded::<CloseCode>::new(4403),
        Some(Bounded::<CloseCode>::SETUP_REQUIRED)
    );
    assert_ne!(
        Bounded::<CloseCode>::UNAUTHORIZED,
        Bounded::<CloseCode>::SETUP_REQUIRED
    );
    assert_eq!(
        Bounded::<CloseCode>::new(1006).map(Bounded::<CloseCode>::get),
        Some(1006)
    );
    assert_eq!(
        Bounded::<CloseCode>::UNAUTHORIZED.reason(),
        Some("unauthorized")
    );
    assert_eq!(
        Bounded::<CloseCode>::SETUP_REQUIRED.reason(),
        Some("setup required")
    );
    assert_eq!(
        Bounded::<CloseCode>::new(1006).and_then(Bounded::<CloseCode>::reason),
        None
    );
    assert_eq!(Bounded::<CloseCode>::UNAUTHORIZED.to_string(), "4401");
    assert_eq!(
        format!("{:?}", Bounded::<CloseCode>::UNAUTHORIZED),
        "CloseCode(4401)"
    );
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
/// the call frames take, frozen. No version digest covers them: a link
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
        head: crate::LogHead {
            position: ambre_core::Seq::<crate::LogPlaces>::new(2),
            previous: Some(FixedBytes::from_array([8; 32])),
        },
        sealed: ambre_core::Payload::new(vec![7; 5]),
    };
    let entry = MembershipEntry {
        document,
        position: ambre_core::Seq::<crate::LogPlaces>::new(1),
        member: Id::<User>::from_bytes([2; 16]),
        member_key: FixedBytes::from_array([3; 32]),
        member_exchange_key: FixedBytes::from_array([13; 32]),
        act: MembershipAct::Invite,
        role: Role::Editor,
        key: Some(FixedBytes::from_array([4; 16])),
        previous: Some(FixedBytes::from_array([5; 32])),
        author: Id::<User>::from_bytes([6; 16]),
        cap: None,
        signature: FixedBytes::from_array([7; ENTRY_SIGNATURE_LEN]),
    };
    let transcript = report_transcript(
        document,
        Id::<id::Snapshot>::from_bytes([8; 16]),
        &FixedBytes::from_array([9; 32]),
    );
    let sender = ambre_core::Seq::new(10);
    let wrap = || Payload::<ambre_core::SenderKeyWraps>::new(vec![11; 8]);
    let seated = || Seated {
        sender,
        join: Payload::new(vec![17; 8]),
    };
    let client = [
        CallClientMsg::Join {
            token: ambre_core::Credential::new("token".to_owned()),
            channel: Id::<ambre_core::id::Channel>::from_bytes([12; 16]),
        },
        CallClientMsg::Announce(Payload::new(vec![17; 8])),
        CallClientMsg::Wrap {
            to: sender,
            wrap: wrap(),
        },
    ];
    let admission = Admission {
        room: FixedBytes::from_array([13; 16]),
        participant: FixedBytes::from_array([14; 16]),
        member: FixedBytes::from_array([18; 16]),
        sender,
        expires: ambre_core::Instant::new(15),
    };
    let server = [
        CallServerMsg::Admitted {
            call: Id::<ambre_core::id::Call>::from_bytes([19; 16]),
            admission: admission
                .token(&FixedBytes::from_array([16; 64]))
                .expect("a token encodes"),
            sfu: Some(ambre_core::Validated::parse("wss://sfu.example/call").expect("an address")),
            sender,
            roster: vec![seated()],
        },
        CallServerMsg::Refused(CallRefusal::Unauthorized),
        CallServerMsg::Refused(CallRefusal::NotMember),
        CallServerMsg::Refused(CallRefusal::Full),
        CallServerMsg::Refused(CallRefusal::Exhausted),
        CallServerMsg::Joined(seated()),
        CallServerMsg::Left(sender),
        CallServerMsg::Wrap {
            from: sender,
            to: sender,
            wrap: wrap(),
        },
        CallServerMsg::Lost,
        CallServerMsg::Renewed(
            admission
                .token(&FixedBytes::from_array([19; 64]))
                .expect("a token encodes"),
        ),
    ];
    let mut signaling = Vec::new();
    for msg in &client {
        signaling.extend(postcard::to_allocvec(msg).expect("a frame encodes"));
    }
    for msg in &server {
        signaling.extend(postcard::to_allocvec(msg).expect("a frame encodes"));
    }
    // The frames of a call message came later: a frozen run of their own, so the frames
    // above keep the digest they had.
    let mut ending = Vec::new();
    for msg in [
        CallClientMsg::Start {
            token: ambre_core::Credential::new("token".to_owned()),
            channel: Id::<ambre_core::id::Channel>::from_bytes([12; 16]),
        },
        CallClientMsg::Leave,
    ] {
        ending.extend(postcard::to_allocvec(&msg).expect("a frame encodes"));
    }
    for msg in [
        CallServerMsg::Refused(CallRefusal::Ended),
        CallServerMsg::Departed(signaling::Departure::Others),
        CallServerMsg::Departed(signaling::Departure::Last),
    ] {
        ending.extend(postcard::to_allocvec(&msg).expect("a frame encodes"));
    }
    signaling.extend(
        admission
            .signed_bytes()
            .expect("an admission encodes")
            .as_bytes(),
    );
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
        ("call message", ending),
    ];
    let digests: Vec<(&str, String)> = frames
        .iter()
        .map(|(name, frame)| (*name, frozen_digest(frame)))
        .collect();
    assert_eq!(
        digests,
        [
            // Version 2 of the link names the log head its rotator read
            // (`SYNC-204`), and no stored link of version 1 counts.
            ("key link", "adb8ebdc49084e76c0772f81c413eaad".to_owned()),
            // Version 3 of the entry signs the cap of a revoke (`SYNC-300`),
            // after version 2 signed the member's exchange key (`SYNC-197`).
            // No stored entry of an older version verifies.
            ("entry", "3dea743cbcbdb6dc370690e050f24d65".to_owned()),
            (
                "entry signed",
                "5e3add2dbf24171c8d70a30e7a0d901c".to_owned()
            ),
            ("report", "d58f6df5ac304352e01488b1981f97be".to_owned()),
            // The admission carries the sender index and the member pseudonym of the
            // seat, under the domain `ambre-call-admission-v3`, and a heartbeat renews it
            // (`CALL-116`, audit `N25`). An admission lives sixty seconds, and no store
            // keeps one.
            ("signaling", "556182ef8014c2b6bef7674e18ed362c".to_owned()),
            (
                "call message",
                "414fb3e2c545f7ab6bd80e4003a3b2af".to_owned()
            ),
        ]
    );
}

/// The bytes a log head adds to the public data of an update
/// (`ambre-spec-budgets.md section 1`). It adds 2 on an empty log, 34 below
/// 128 entries, and 43 at most. The place takes a varint of up to 10 bytes.
#[test]
fn a_log_head_costs_an_update_at_most_43_bytes() {
    let size = |log_head: crate::LogHead| {
        let public_data = UpdatePublicData {
            document_id: Id::<Document>::from_bytes([1; 16]),
            pub_key: FixedBytes::from_array([2; 32]),
            key_id: FixedBytes::from_array([3; 16]),
            snapshot_id: Id::<id::Snapshot>::from_bytes([4; 16]),
            clock: Seq::<DeltaAuthor>::new(1),
            log_head,
            log_entry: None,
        };
        postcard::to_allocvec(&public_data)
            .expect("public data encodes")
            .len()
    };
    let head = |place: u64| crate::LogHead {
        position: Seq::new(place),
        previous: Some(FixedBytes::from_array([5; 32])),
    };
    // Postcard writes a struct as the tuple of its fields, so the five
    // other fields alone are the public data before the head.
    let without = postcard::to_allocvec(&(
        Id::<Document>::from_bytes([1; 16]),
        FixedBytes::<ambre_core::PublicKey>::from_array([2; 32]),
        FixedBytes::<ambre_core::KeyId>::from_array([3; 16]),
        Id::<id::Snapshot>::from_bytes([4; 16]),
        Seq::<DeltaAuthor>::new(1),
        // The log entry an empty log names: none (`SYNC-218`).
        Option::<u8>::None,
    ))
    .expect("the fields encode")
    .len();
    assert_eq!(size(crate::LogHead::EMPTY) - without, 2);
    assert_eq!(size(head(127)) - without, 34);
    assert_eq!(size(head(128)) - without, 35);
    assert_eq!(size(head(u64::MAX)) - without, 43);
}

/// `SYNC-218`: the last log entry an update carries beside its head, by
/// postcard. An empty log costs the one byte of an absent option. A revoke
/// carries its cap. The field costs 232 B below 128 entries and clocks, and
/// 250 B at a place and a cap of `u64::MAX`.
#[test]
fn a_log_entry_costs_an_update_at_most_250_bytes() {
    let size = |log_entry: Option<crate::MembershipEntry>| {
        let public_data = UpdatePublicData {
            document_id: Id::<Document>::from_bytes([1; 16]),
            pub_key: FixedBytes::from_array([2; 32]),
            key_id: FixedBytes::from_array([3; 16]),
            snapshot_id: Id::<id::Snapshot>::from_bytes([4; 16]),
            clock: Seq::<DeltaAuthor>::new(1),
            log_head: crate::LogHead::EMPTY,
            log_entry: log_entry.map(Box::new),
        };
        postcard::to_allocvec(&public_data)
            .expect("public data encodes")
            .len()
    };
    let entry = |place: u64, key: Option<FixedBytes<ambre_core::KeyId>>, cap: Option<u64>| {
        crate::MembershipEntry {
            document: Id::<Document>::from_bytes([1; 16]),
            position: Seq::new(place),
            member: Id::<User>::from_bytes([5; 16]),
            member_key: FixedBytes::from_array([6; 32]),
            member_exchange_key: FixedBytes::from_array([7; 32]),
            act: crate::MembershipAct::Revoke,
            role: ambre_core::Role::Editor,
            key,
            previous: Some(FixedBytes::from_array([8; 32])),
            author: Id::<User>::from_bytes([9; 16]),
            cap: cap.map(Seq::new),
            signature: FixedBytes::from_array([10; 64]),
        }
    };
    // The public data with no field at all: the absent option is its last byte.
    let without = size(None) - 1;
    let key = Some(FixedBytes::from_array([11; 16]));
    assert_eq!(size(None) - without, 1);
    assert_eq!(size(Some(entry(1, None, None))) - without, 215);
    assert_eq!(size(Some(entry(127, key, Some(127)))) - without, 232);
    assert_eq!(size(Some(entry(128, key, Some(128)))) - without, 234);
    assert_eq!(
        size(Some(entry(u64::MAX, key, Some(u64::MAX)))) - without,
        250
    );
}

/// `SYNC-262`: the log head and the entry it names on one presence sample,
/// by postcard. An empty log costs 3 B: the empty head and the absent entry.
/// The pair costs 266 B below 128 entries and clocks, and 293 B at a place
/// and a cap of `u64::MAX`.
#[test]
fn a_log_head_and_entry_cost_an_ephemeral_at_most_293_bytes() {
    let size = |log_head: crate::LogHead, log_entry: Option<crate::MembershipEntry>| {
        let public_data = EphemeralPublicData {
            document_id: Id::<Document>::from_bytes([1; 16]),
            pub_key: FixedBytes::from_array([2; 32]),
            key_id: FixedBytes::from_array([3; 16]),
            counter: Seq::<Ephemerals>::new(1),
            log_head,
            log_entry: log_entry.map(Box::new),
        };
        postcard::to_allocvec(&public_data)
            .expect("public data encodes")
            .len()
    };
    // Postcard writes a struct as the tuple of its fields, so the four
    // fields before the head are the sample without the pair.
    let without = postcard::to_allocvec(&(
        Id::<Document>::from_bytes([1; 16]),
        FixedBytes::<ambre_core::PublicKey>::from_array([2; 32]),
        FixedBytes::<ambre_core::KeyId>::from_array([3; 16]),
        Seq::<Ephemerals>::new(1),
    ))
    .expect("the fields encode")
    .len();
    let head = |place: u64| crate::LogHead {
        position: Seq::new(place),
        previous: Some(FixedBytes::from_array([4; 32])),
    };
    let entry = |place: u64, cap: u64| crate::MembershipEntry {
        document: Id::<Document>::from_bytes([1; 16]),
        position: Seq::new(place),
        member: Id::<User>::from_bytes([5; 16]),
        member_key: FixedBytes::from_array([6; 32]),
        member_exchange_key: FixedBytes::from_array([7; 32]),
        act: crate::MembershipAct::Revoke,
        role: ambre_core::Role::Editor,
        key: Some(FixedBytes::from_array([11; 16])),
        previous: Some(FixedBytes::from_array([8; 32])),
        author: Id::<User>::from_bytes([9; 16]),
        cap: Some(Seq::new(cap)),
        signature: FixedBytes::from_array([10; 64]),
    };
    assert_eq!(size(crate::LogHead::EMPTY, None) - without, 3);
    assert_eq!(size(head(127), Some(entry(126, 127))) - without, 266);
    assert_eq!(
        size(head(u64::MAX), Some(entry(u64::MAX, u64::MAX))) - without,
        293
    );
}
