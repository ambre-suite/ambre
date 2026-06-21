//! Unit tests for the wire protocol types.

use super::*;
use crate::messages::{Ciphertext, Clock, Counter, Seq};
use ambre_core::{DocumentId, PublicKey, SnapshotId, UserId};

#[test]
fn update_roundtrip() {
    let update = Update {
        ciphertext: Ciphertext(vec![1, 2, 3, 4]),
        nonce: Nonce([0u8; 24]),
        public_data: UpdatePublicData {
            document_id: DocumentId::new(),
            pub_key: PublicKey::from_array([5u8; 32]),
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
    let other_version = ambre_core::version::Version::new(2).unwrap();
    assert!(check_version(other_version).is_err());
}
