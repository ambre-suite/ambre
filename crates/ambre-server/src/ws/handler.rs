// ============================================================================
// Multiplexed WebSocket handler -- ONE connection per client session.
//
// Endpoint: GET /api/ws -> upgrade WebSocket.
// Each connection maintains a set of subscribed document and channel IDs.
// Subscribe/Unsubscribe messages dynamically add or remove subscriptions.
// Updates are validated (Ed25519 signature + clock monotonicity via Lua),
// assigned a global seq, stored in Postgres, and broadcast via Redis PubSub.
// ============================================================================

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;
#[allow(clippy::single_component_path_imports)]
use tracing;

use ambre_core::{ChannelId, DocumentId, UserId};
use ambre_protocol::ws::{WsClientMsg, WsServerMsg, decode_client_msg, encode_server_msg};

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::redis_ops;
use crate::state::AppState;

/// HTTP upgrade handler. Authenticates via the session cookie, then hands off
/// to `handle_ws` for the full lifecycle.
pub async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> impl IntoResponse {
    let session = auth::authenticate(&state, &jar).await;
    ws.on_upgrade(move |socket| handle_ws(socket, state, session))
}

/// Per-connection state tracking which documents and channels this client
/// is currently subscribed to.
struct WsConnectionState {
    user_id: UserId,
    session_id: String,
    doc_subscriptions: HashSet<DocumentId>,
    channel_subscriptions: HashSet<ChannelId>,
}

async fn handle_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    session_result: Result<auth::AuthSession, AppError>,
) {
    let session = match session_result {
        Ok(s) => s,
        Err(_) => return,
    };

    let (outbound_tx, mut outbound_rx) = mpsc::channel::<Vec<u8>>(256);

    let user_id = session.user_id;
    let session_id = session.session_id.clone();

    let mut conn = WsConnectionState {
        user_id,
        session_id: session_id.clone(),
        doc_subscriptions: HashSet::new(),
        channel_subscriptions: HashSet::new(),
    };

    let (mut ws_sink, mut ws_stream) = socket.split();

    let outbound_handle = tokio::spawn(async move {
        while let Some(bytes) = outbound_rx.recv().await {
            if ws_sink.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = ws_stream.next().await {
        let Message::Binary(data) = msg else { continue };

        let client_msg = match decode_client_msg(&data) {
            Ok(m) => m,
            Err(e) => {
                let err = encode_server_msg(&WsServerMsg::Error {
                    code: 400,
                    message: format!("decode error: {e:?}"),
                });
                let _ = outbound_tx.send(err).await;
                continue;
            }
        };

        match client_msg {
            WsClientMsg::Subscribe { document_id } => {
                handle_subscribe(&state, &mut conn, document_id, &outbound_tx).await;
            }
            WsClientMsg::Unsubscribe { document_id } => {
                handle_unsubscribe(&state, &mut conn, document_id).await;
            }
            WsClientMsg::Update(update) => {
                handle_update(&state, &conn, update, &outbound_tx).await;
            }
            WsClientMsg::Ephemeral(eph) => {
                handle_ephemeral(&state, &conn, eph).await;
            }
            WsClientMsg::RequestMissing { document_id, after_seq } => {
                handle_request_missing(&state, &conn, document_id, after_seq, &outbound_tx).await;
            }
            WsClientMsg::SubscribeChannel { channel_id } => {
                handle_subscribe_channel(&state, &mut conn, channel_id, &outbound_tx).await;
            }
            WsClientMsg::UnsubscribeChannel { channel_id } => {
                conn.channel_subscriptions.remove(&channel_id);
            }
            WsClientMsg::ChannelUpdate { channel_id, update } => {
                if !conn.channel_subscriptions.contains(&channel_id) {
                    let err = encode_server_msg(&WsServerMsg::Error {
                        code: 403,
                        message: "not subscribed to this channel".into(),
                    });
                    let _ = outbound_tx.send(err).await;
                    continue;
                }
                handle_channel_update(&state, channel_id, update).await;
            }
            WsClientMsg::ChannelEphemeral { channel_id, ephemeral } => {
                if !conn.channel_subscriptions.contains(&channel_id) { continue; }
                handle_channel_ephemeral(&state, channel_id, ephemeral).await;
            }
        }
    }

    for doc_id in &conn.doc_subscriptions {
        let _ = redis_ops::doc_unsubscribe_session(&state.redis, doc_id, &session_id).await;
    }
    outbound_handle.abort();
}

async fn handle_subscribe(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    document_id: DocumentId,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    if let Err(e) = verify_document_access(&state.db, document_id, conn.user_id).await {
        let err = encode_server_msg(&WsServerMsg::Error { code: 403, message: e.to_string() });
        let _ = outbound_tx.send(err).await;
        return;
    }
    conn.doc_subscriptions.insert(document_id);
    let _ = redis_ops::doc_subscribe_session(&state.redis, &document_id, &conn.session_id).await;
    let ack = encode_server_msg(&WsServerMsg::Subscribed { document_id });
    let _ = outbound_tx.send(ack).await;
}

async fn handle_unsubscribe(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    document_id: DocumentId,
) {
    conn.doc_subscriptions.remove(&document_id);
    let _ = redis_ops::doc_unsubscribe_session(&state.redis, &document_id, &conn.session_id).await;
}

async fn handle_update(
    state: &Arc<AppState>,
    conn: &WsConnectionState,
    update: ambre_protocol::Update,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    let doc_id = update.public_data.document_id;
    if !conn.doc_subscriptions.contains(&doc_id) {
        let err = encode_server_msg(&WsServerMsg::Error { code: 403, message: "not subscribed".into() });
        let _ = outbound_tx.send(err).await;
        return;
    }

    let author_pubkey = update.public_data.pub_key.clone();
    if let Err(e) = verify_update_signature(&author_pubkey, &update) {
        let err = encode_server_msg(&WsServerMsg::Error { code: 401, message: format!("invalid signature: {e}") });
        let _ = outbound_tx.send(err).await;
        return;
    }

    let pub_key_hex = author_pubkey.to_hex();
    let clock = update.public_data.clock.0;
    let snapshot_id = update.public_data.snapshot_id;

    let seq = match redis_ops::verify_clock_and_assign_seq(&state.redis, &doc_id, &pub_key_hex, clock).await {
        Ok(s) => s,
        Err(e) => {
            let err = encode_server_msg(&WsServerMsg::Error { code: 409, message: e.to_string() });
            let _ = outbound_tx.send(err).await;
            return;
        }
    };

    let s3_key = crate::s3::format_s3_delta_key(&state.config.s3_env_prefix, &doc_id, seq);
    let _ = db::insert_update(
        &state.db, &doc_id, author_pubkey.as_array(),
        &snapshot_id, clock as i64, seq as i64, &s3_key,
    ).await;

    let _ = redis_ops::append_pending_delta(&state.redis, &doc_id, seq, &s3_key, &update.ciphertext.0).await;

    let broadcast = encode_server_msg(&WsServerMsg::Update {
        seq: ambre_protocol::messages::Seq(seq),
        update,
    });
    let _ = redis_ops::doc_broadcast(&state.redis, &doc_id, &broadcast).await;
}

async fn handle_ephemeral(
    state: &Arc<AppState>,
    conn: &WsConnectionState,
    eph: ambre_protocol::Ephemeral,
) {
    use base64::Engine;
    let doc_id = eph.public_data.document_id;
    if !conn.doc_subscriptions.contains(&doc_id) { return; }

    let pub_key_hex = eph.public_data.pub_key.to_hex();
    let presence_enc_b64 = base64::engine::general_purpose::URL_SAFE.encode(&eph.ciphertext.0);
    let _ = redis_ops::doc_presence_update(&state.redis, &doc_id, &pub_key_hex, &presence_enc_b64, 15).await;

    let broadcast = encode_server_msg(&WsServerMsg::Ephemeral(eph));
    let _ = redis_ops::doc_broadcast(&state.redis, &doc_id, &broadcast).await;
}

async fn handle_request_missing(
    state: &Arc<AppState>,
    conn: &WsConnectionState,
    document_id: DocumentId,
    after_seq: ambre_protocol::messages::Seq,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    if !conn.doc_subscriptions.contains(&document_id) { return; }

    let updates = match db::get_updates_after(&state.db, &document_id, after_seq.0 as i64, 1000).await {
        Ok(u) => u,
        Err(e) => { tracing::error!(error = %e, "failed to get updates after seq"); return; }
    };

    let seq_updates: Vec<ambre_protocol::SeqUpdate> = updates
        .into_iter()
        .map(|u| ambre_protocol::SeqUpdate {
            seq: ambre_protocol::messages::Seq(u.seq as u64),
            update: placeholder_update(document_id, u.clock as u64),
        })
        .collect();

    let msg = encode_server_msg(&WsServerMsg::MissingUpdates { document_id, updates: seq_updates });
    let _ = outbound_tx.send(msg).await;
}

fn placeholder_update(document_id: DocumentId, clock: u64) -> ambre_protocol::Update {
    ambre_protocol::Update {
        ciphertext: ambre_protocol::messages::Ciphertext(vec![]),
        nonce: ambre_protocol::messages::Nonce([0u8; 24]),
        public_data: ambre_protocol::public_data::UpdatePublicData {
            document_id,
            pub_key: ambre_core::PublicKey::from_array([0u8; 32]),
            snapshot_id: ambre_core::SnapshotId::from_bytes([0u8; 16]),
            clock: ambre_protocol::messages::Clock(clock),
        },
        commitment_tag: ambre_protocol::messages::CommitmentTag([0u8; 32]),
        signature: ambre_protocol::messages::Signature([0u8; 64]),
    }
}

async fn handle_subscribe_channel(
    _state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    channel_id: ChannelId,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    conn.channel_subscriptions.insert(channel_id);
    let ack = encode_server_msg(&WsServerMsg::ChannelSubscribed { channel_id });
    let _ = outbound_tx.send(ack).await;
}

async fn handle_channel_update(
    state: &Arc<AppState>,
    channel_id: ChannelId,
    update: ambre_protocol::Update,
) {
    let broadcast = encode_server_msg(&WsServerMsg::ChannelUpdate {
        channel_id,
        seq: ambre_protocol::messages::Seq(0),
        update,
    });
    let _ = redis_ops::chan_broadcast(&state.redis, &channel_id, &broadcast).await;
}

async fn handle_channel_ephemeral(
    state: &Arc<AppState>,
    channel_id: ChannelId,
    ephemeral: ambre_protocol::Ephemeral,
) {
    let broadcast = encode_server_msg(&WsServerMsg::ChannelEphemeral { channel_id, ephemeral });
    let _ = redis_ops::chan_broadcast(&state.redis, &channel_id, &broadcast).await;
}

// -- ACL ----------------------------------------------------------

/// Verifies the caller has a membership row for the document.
async fn verify_document_access(pool: &sqlx::PgPool, document_id: DocumentId, user_id: UserId) -> Result<(), AppError> {
    db::get_document_member(pool, &document_id, &user_id)
        .await?
        .ok_or(AppError::Forbidden)?;
    Ok(())
}

/// Verifies an Ed25519 signature over `nonce || ciphertext || public_data_bytes || commitment_tag`.
///
/// This is the ONLY cryptographic operation performed by the Blind Relay.
/// The server never decrypts content -- it only checks that the sender
/// owns the private key matching the public key in the message.
fn verify_update_signature(
    pubkey: &ambre_core::PublicKey,
    update: &ambre_protocol::Update,
) -> Result<(), AppError> {
    use ed25519_dalek::{Signature as DalekSig, Verifier, VerifyingKey};

    let public_data_bytes =
        postcard::to_allocvec(&update.public_data).map_err(AppError::Decode)?;

    let mut signed_data = Vec::new();
    signed_data.extend_from_slice(&update.nonce.0);
    signed_data.extend_from_slice(&update.ciphertext.0);
    signed_data.extend_from_slice(&public_data_bytes);
    signed_data.extend_from_slice(&update.commitment_tag.0);

    let vk = VerifyingKey::from_bytes(pubkey.as_array())
        .map_err(|_| AppError::InvalidSignature)?;
    let sig = DalekSig::from_slice(&update.signature.0)
        .map_err(|_| AppError::InvalidSignature)?;

    vk.verify(&signed_data, &sig)
        .map_err(|_| AppError::InvalidSignature)?;

    Ok(())
}

#[cfg(test)]
mod postcard_tests {
    use ambre_core::{DocumentId, PublicKey, SnapshotId, UserId};
    use ambre_protocol::messages::{
        Ciphertext, Clock, CommitmentTag, Counter, Ephemeral, Nonce,
        Seq, SeqUpdate, Signature, Update,
    };
    use ambre_protocol::public_data::{EphemeralPublicData, UpdatePublicData};
    use ambre_protocol::ws::{encode_client_msg, decode_client_msg, encode_server_msg, decode_server_msg, WsClientMsg, WsServerMsg};

    fn make_update() -> Update {
        Update {
            ciphertext: Ciphertext(vec![1, 2, 3]),
            nonce: Nonce([4u8; 24]),
            public_data: UpdatePublicData {
                document_id: DocumentId::from_bytes([5u8; 16]),
                pub_key: PublicKey::from_array([6u8; 32]),
                snapshot_id: SnapshotId::from_bytes([7u8; 16]),
                clock: Clock(42),
            },
            commitment_tag: CommitmentTag([8u8; 32]),
            signature: Signature([9u8; 64]),
        }
    }

    fn make_ephemeral() -> Ephemeral {
        Ephemeral {
            ciphertext: Ciphertext(vec![10, 11]),
            nonce: Nonce([12u8; 24]),
            public_data: EphemeralPublicData {
                document_id: DocumentId::from_bytes([13u8; 16]),
                pub_key: PublicKey::from_array([14u8; 32]),
                counter: Counter(15),
            },
            commitment_tag: CommitmentTag([16u8; 32]),
            signature: Signature([17u8; 64]),
        }
    }

    #[test]
    fn roundtrip_client_subscribe() {
        let msg = WsClientMsg::Subscribe { document_id: DocumentId::from_bytes([0u8; 16]) };
        let bytes = encode_client_msg(&msg);
        assert_eq!(bytes[0], 0x01);
        let decoded = decode_client_msg(&bytes).unwrap();
        assert!(matches!(decoded, WsClientMsg::Subscribe { .. }));
    }

    #[test]
    fn roundtrip_client_unsubscribe() {
        let msg = WsClientMsg::Unsubscribe { document_id: DocumentId::from_bytes([1u8; 16]) };
        let decoded = decode_client_msg(&encode_client_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsClientMsg::Unsubscribe { .. }));
    }

    #[test]
    fn roundtrip_client_update() {
        let update = make_update();
        let msg = WsClientMsg::Update(update);
        let decoded = decode_client_msg(&encode_client_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsClientMsg::Update(_)));
    }

    #[test]
    fn roundtrip_client_ephemeral() {
        let eph = make_ephemeral();
        let msg = WsClientMsg::Ephemeral(eph);
        let decoded = decode_client_msg(&encode_client_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsClientMsg::Ephemeral(_)));
    }

    #[test]
    fn roundtrip_client_request_missing() {
        let msg = WsClientMsg::RequestMissing {
            document_id: DocumentId::from_bytes([1u8; 16]),
            after_seq: Seq(5),
        };
        let decoded = decode_client_msg(&encode_client_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsClientMsg::RequestMissing { .. }));
    }

    #[test]
    fn roundtrip_server_subscribed() {
        let msg = WsServerMsg::Subscribed { document_id: DocumentId::from_bytes([2u8; 16]) };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::Subscribed { .. }));
    }

    #[test]
    fn roundtrip_server_update() {
        let msg = WsServerMsg::Update { seq: Seq(100), update: make_update() };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::Update { seq: _, update: _ }));
    }

    #[test]
    fn roundtrip_server_ephemeral() {
        let msg = WsServerMsg::Ephemeral(make_ephemeral());
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::Ephemeral(_)));
    }

    #[test]
    fn roundtrip_server_missing_updates() {
        let msg = WsServerMsg::MissingUpdates {
            document_id: DocumentId::from_bytes([3u8; 16]),
            updates: vec![SeqUpdate {
                seq: Seq(1),
                update: make_update(),
            }],
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::MissingUpdates { .. }));
    }

    #[test]
    fn roundtrip_server_snapshot_required() {
        let msg = WsServerMsg::SnapshotRequired { document_id: DocumentId::from_bytes([4u8; 16]) };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::SnapshotRequired { .. }));
    }

    #[test]
    fn roundtrip_server_snapshot_available() {
        let msg = WsServerMsg::SnapshotAvailable {
            document_id: DocumentId::from_bytes([5u8; 16]),
            snapshot_id: SnapshotId::from_bytes([6u8; 16]),
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::SnapshotAvailable { .. }));
    }

    #[test]
    fn roundtrip_server_key_rotation_required() {
        let msg = WsServerMsg::KeyRotationRequired {
            document_id: DocumentId::from_bytes([7u8; 16]),
            revoked_user_id: UserId::from_bytes([8u8; 16]),
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::KeyRotationRequired { .. }));
    }

    #[test]
    fn roundtrip_server_error() {
        let msg = WsServerMsg::Error { code: 409, message: "conflict".into() };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        match decoded {
            WsServerMsg::Error { code, message } => {
                assert_eq!(code, 409);
                assert_eq!(message, "conflict");
            }
            _ => panic!("expected Error variant"),
        }
    }

    #[test]
    fn version_byte_is_01() {
        let msg = WsClientMsg::Subscribe { document_id: DocumentId::from_bytes([0u8; 16]) };
        let bytes = encode_client_msg(&msg);
        assert_eq!(bytes[0], 0x01);
    }

    #[test]
    fn empty_frame_is_error() {
        assert!(decode_client_msg(&[]).is_err());
    }

    #[test]
    fn wrong_version_byte_is_error() {
        let msg = vec![0x02, 0x00];
        assert!(decode_client_msg(&msg).is_err());
    }

    #[test]
    fn garbage_data_returns_error_not_panics() {
        let result = decode_client_msg(&[0x01, 0xFF, 0xFE, 0xFD]);
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod ed25519_tests {
    use super::*;
    use ambre_core::{DocumentId, PublicKey, SnapshotId};
    use ambre_protocol::messages::{Ciphertext, Clock, CommitmentTag, Nonce, Signature, Update};
    use ambre_protocol::public_data::UpdatePublicData;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn valid_signature_is_accepted() {
        let secret_bytes: [u8; 32] = [
            0xcd, 0x06, 0x85, 0x43, 0x6b, 0xa1, 0xf7, 0x1e,
            0x32, 0xce, 0x9a, 0x84, 0xbd, 0xc6, 0xfe, 0xa0,
            0xb6, 0x49, 0x12, 0x1d, 0x8b, 0xf8, 0x43, 0xa2,
            0xdd, 0xc6, 0x9f, 0x14, 0xf3, 0x5a, 0x3c, 0xb9,
        ];
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let pubkey_bytes: [u8; 32] = signing_key.verifying_key().to_bytes();
        let pubkey = PublicKey::from_array(pubkey_bytes);

        let public_data = UpdatePublicData {
            document_id: DocumentId::from_bytes([0u8; 16]),
            pub_key: pubkey.clone(),
            snapshot_id: SnapshotId::from_bytes([1u8; 16]),
            clock: Clock(1),
        };
        let nonce = Nonce([3u8; 24]);
        let ciphertext = Ciphertext(vec![4, 5, 6]);
        let commitment_tag = CommitmentTag([7u8; 32]);

        let public_data_bytes = postcard::to_allocvec(&public_data).unwrap();
        let mut signed_data = Vec::new();
        signed_data.extend_from_slice(&nonce.0);
        signed_data.extend_from_slice(&ciphertext.0);
        signed_data.extend_from_slice(&public_data_bytes);
        signed_data.extend_from_slice(&commitment_tag.0);

        let sig = signing_key.sign(&signed_data);
        let signature = Signature(sig.to_bytes());

        let update = Update {
            ciphertext,
            nonce,
            public_data,
            commitment_tag,
            signature,
        };

        verify_update_signature(&pubkey, &update).expect("valid signature must be accepted");
    }

    #[test]
    fn invalid_signature_is_rejected() {
        let pubkey = PublicKey::from_array([42u8; 32]);

        let update = Update {
            ciphertext: Ciphertext(vec![1u8]),
            nonce: Nonce([0u8; 24]),
            public_data: UpdatePublicData {
                document_id: DocumentId::from_bytes([0u8; 16]),
                pub_key: pubkey,
                snapshot_id: SnapshotId::from_bytes([0u8; 16]),
                clock: Clock(0),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        };

        assert!(verify_update_signature(&update.public_data.pub_key, &update).is_err());
    }

    #[test]
    fn tampered_ciphertext_rejects_signature() {
        let secret_bytes: [u8; 32] = [
            0xab, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde,
            0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
            0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
        ];
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let pubkey = PublicKey::from_array(signing_key.verifying_key().to_bytes());

        let public_data = UpdatePublicData {
            document_id: DocumentId::from_bytes([0u8; 16]),
            pub_key: pubkey.clone(),
            snapshot_id: SnapshotId::from_bytes([0u8; 16]),
            clock: Clock(0),
        };
        let nonce = Nonce([0u8; 24]);
        let ciphertext = Ciphertext(vec![1, 2, 3]);
        let commitment_tag = CommitmentTag([0u8; 32]);

        let public_data_bytes = postcard::to_allocvec(&public_data).unwrap();
        let mut signed_data = vec![];
        signed_data.extend_from_slice(&nonce.0);
        signed_data.extend_from_slice(&ciphertext.0);
        signed_data.extend_from_slice(&public_data_bytes);
        signed_data.extend_from_slice(&commitment_tag.0);

        let sig = signing_key.sign(&signed_data);

        let tampered = Update {
            ciphertext: Ciphertext(vec![9, 9, 9]),
            nonce,
            public_data,
            commitment_tag,
            signature: Signature(sig.to_bytes()),
        };

        assert!(verify_update_signature(&pubkey, &tampered).is_err());
    }
}
