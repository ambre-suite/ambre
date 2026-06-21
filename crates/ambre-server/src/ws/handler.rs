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

/// Redis PubSub channel name for a document's live broadcasts.
fn doc_pubsub_channel(document_id: &DocumentId) -> String {
    format!("doc:{document_id}")
}

/// Redis PubSub channel name for a Murmure channel's live broadcasts.
fn chan_pubsub_channel(channel_id: &ChannelId) -> String {
    format!("chan:{channel_id}")
}

/// Redis PubSub channel name for a user's personal events (e.g. a document was
/// shared with them). Every connection subscribes to its own user channel for
/// the whole session so server-initiated, non-document-scoped pushes reach it.
pub(crate) fn user_pubsub_channel(user_id: &UserId) -> String {
    format!("user:{user_id}")
}

/// Query parameters on the WS upgrade. The browser WebSocket API forbids custom
/// headers, so the access JWT travels in the `token` query parameter; it is
/// verified at the handshake exactly like a `Authorization: Bearer` on REST.
#[derive(serde::Deserialize)]
pub struct WsQuery {
    token: Option<String>,
}

/// HTTP upgrade handler. Authenticates via the `?token=<jwt>` access token, then
/// hands off to `handle_ws` for the full lifecycle.
pub async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    axum::extract::Query(query): axum::extract::Query<WsQuery>,
) -> impl IntoResponse {
    let session = match query.token {
        Some(token) => auth::authenticate_token(&state, &token),
        None => Err(AppError::Unauthorized),
    };
    // The WS upgrade always returns 101, then `handle_ws` drops the connection
    // if auth failed -- so the error never reaches `AppError::into_response` and
    // would otherwise be invisible. Log it here, by severity: a rejected session
    // is routine (client), a server fault (Redis/DB) must be loud.
    if let Err(e) = &session {
        match e {
            AppError::Unauthorized | AppError::SessionExpired => {
                tracing::debug!("WS auth rejected: {e}")
            }
            _ => tracing::error!("WS auth failed (server fault): {e}"),
        }
    }
    ws.on_upgrade(move |socket| handle_ws(socket, state, session))
}

/// Per-connection state tracking which documents and channels this client
/// is currently subscribed to.
struct WsConnectionState {
    user_id: UserId,
    /// Identifies this socket in Redis presence/subscription sets. Generated
    /// fresh per connection (the access token is stateless and carries no id).
    connection_id: String,
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
        Err(_) => {
            // Tell the client explicitly that the upgrade was refused for auth
            // reasons (expired/rotated JWT) with an application close code, so it
            // can refresh its token and reconnect silently instead of treating it
            // as a generic network drop. 4401 = our "unauthorized" (4000-4999 is
            // the reserved application range).
            let mut socket = socket;
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 4401,
                    reason: "unauthorized".into(),
                })))
                .await;
            return;
        }
    };

    let (outbound_tx, mut outbound_rx) = mpsc::channel::<Vec<u8>>(256);

    let user_id = session.user_id;
    let connection_id = uuid::Uuid::new_v4().to_string();

    let mut conn = WsConnectionState {
        user_id,
        connection_id: connection_id.clone(),
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

    // Live broadcasts (a peer's update / ephemeral / membership change) are
    // published to Redis PubSub by the writer's connection. THIS connection
    // registers its outbound queue with the process-wide hub for every channel
    // it watches; the hub holds the single Redis subscriber per channel and fans
    // each message in-process to all registered connections. (The writer's own
    // broadcast is echoed back to it too; a client's CRDT import of its own ops
    // is an idempotent no-op.)
    let pubsub = state.pubsub.clone();

    // Personal channel: subscribe for the whole session so server-initiated
    // pushes not tied to an already-open document (e.g. `DocumentShared` on a
    // fresh invite) reach this connection.
    pubsub
        .subscribe(
            user_pubsub_channel(&user_id),
            connection_id.clone(),
            outbound_tx.clone(),
        )
        .await;

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
                // Start forwarding this doc's live broadcasts only if access was
                // granted (handle_subscribe records the subscription on success).
                if conn.doc_subscriptions.contains(&document_id) {
                    pubsub
                        .subscribe(
                            doc_pubsub_channel(&document_id),
                            connection_id.clone(),
                            outbound_tx.clone(),
                        )
                        .await;
                }
            }
            WsClientMsg::Unsubscribe { document_id } => {
                handle_unsubscribe(&state, &mut conn, document_id).await;
                pubsub
                    .unsubscribe(doc_pubsub_channel(&document_id), connection_id.clone())
                    .await;
            }
            WsClientMsg::Update(update) => {
                tracing::info!(
                    "[Server←Sync] WS Update doc={} author={} clock={}",
                    update.public_data.document_id,
                    update.public_data.pub_key.to_hex(),
                    update.public_data.clock.0,
                );
                handle_update(&state, &conn, update, &outbound_tx).await;
            }
            WsClientMsg::Ephemeral(eph) => {
                handle_ephemeral(&state, &conn, eph).await;
            }
            WsClientMsg::RequestMissing {
                document_id,
                after_seq,
            } => {
                handle_request_missing(&state, &conn, document_id, after_seq, &outbound_tx).await;
            }
            WsClientMsg::SubscribeChannel { channel_id } => {
                handle_subscribe_channel(&state, &mut conn, channel_id, &outbound_tx).await;
                if conn.channel_subscriptions.contains(&channel_id) {
                    pubsub
                        .subscribe(
                            chan_pubsub_channel(&channel_id),
                            connection_id.clone(),
                            outbound_tx.clone(),
                        )
                        .await;
                }
            }
            WsClientMsg::UnsubscribeChannel { channel_id } => {
                conn.channel_subscriptions.remove(&channel_id);
                pubsub
                    .unsubscribe(chan_pubsub_channel(&channel_id), connection_id.clone())
                    .await;
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
            WsClientMsg::ChannelEphemeral {
                channel_id,
                ephemeral,
            } => {
                if !conn.channel_subscriptions.contains(&channel_id) {
                    continue;
                }
                handle_channel_ephemeral(&state, channel_id, ephemeral).await;
            }
        }
    }

    // Deregister every channel this connection watched from the hub (the hub
    // drops its Redis subscription when the last interested connection leaves).
    for doc_id in &conn.doc_subscriptions {
        let _ = redis_ops::doc_unsubscribe_session(&state.redis, doc_id, &connection_id).await;
        pubsub
            .unsubscribe(doc_pubsub_channel(doc_id), connection_id.clone())
            .await;
    }
    for channel_id in &conn.channel_subscriptions {
        pubsub
            .unsubscribe(chan_pubsub_channel(channel_id), connection_id.clone())
            .await;
    }
    pubsub
        .unsubscribe(user_pubsub_channel(&user_id), connection_id.clone())
        .await;
    outbound_handle.abort();
}

async fn handle_subscribe(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    document_id: DocumentId,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    if let Err(e) = verify_document_access(&state.db, document_id, conn.user_id).await {
        let err = encode_server_msg(&WsServerMsg::Error {
            code: 403,
            message: e.to_string(),
        });
        let _ = outbound_tx.send(err).await;
        return;
    }
    conn.doc_subscriptions.insert(document_id);
    let _ = redis_ops::doc_subscribe_session(&state.redis, &document_id, &conn.connection_id).await;
    let ack = encode_server_msg(&WsServerMsg::Subscribed { document_id });
    let _ = outbound_tx.send(ack).await;
}

async fn handle_unsubscribe(
    state: &Arc<AppState>,
    conn: &mut WsConnectionState,
    document_id: DocumentId,
) {
    conn.doc_subscriptions.remove(&document_id);
    let _ =
        redis_ops::doc_unsubscribe_session(&state.redis, &document_id, &conn.connection_id).await;
}

async fn handle_update(
    state: &Arc<AppState>,
    conn: &WsConnectionState,
    update: ambre_protocol::Update,
    outbound_tx: &mpsc::Sender<Vec<u8>>,
) {
    let doc_id = update.public_data.document_id;
    if !conn.doc_subscriptions.contains(&doc_id) {
        let err = encode_server_msg(&WsServerMsg::Error {
            code: 403,
            message: "not subscribed".into(),
        });
        let _ = outbound_tx.send(err).await;
        return;
    }

    let author_pubkey = update.public_data.pub_key.clone();
    if let Err(e) = verify_update_signature(&author_pubkey, &update) {
        let err = encode_server_msg(&WsServerMsg::Error {
            code: 401,
            message: format!("invalid signature: {e}"),
        });
        let _ = outbound_tx.send(err).await;
        return;
    }

    let pub_key_hex = author_pubkey.to_hex();
    let clock = update.public_data.clock.0;
    let snapshot_id = update.public_data.snapshot_id;

    let seq =
        match redis_ops::verify_clock_and_assign_seq(&state.redis, &doc_id, &pub_key_hex, clock)
            .await
        {
            Ok(s) => {
                tracing::info!(
                    "[Server] seq assigned doc={doc_id} author={pub_key_hex} clock={clock} seq={s}"
                );
                s
            }
            Err(e) => {
                let err = encode_server_msg(&WsServerMsg::Error {
                    code: 409,
                    message: e.to_string(),
                });
                let _ = outbound_tx.send(err).await;
                return;
            }
        };

    let s3_key = crate::s3::format_s3_delta_key(&state.config.s3_env_prefix, &doc_id, seq);
    tracing::info!("[Server→Postgres] insert_update doc={doc_id} seq={seq} clock={clock}");
    if let Err(e) = db::insert_update(
        &state.db,
        &doc_id,
        author_pubkey.as_array(),
        &snapshot_id,
        clock as i64,
        seq as i64,
        &s3_key,
    )
    .await
    {
        // The delta index is how reconnecting clients replay missed updates;
        // a failed insert silently loses durability, so it must be loud.
        tracing::error!("insert_update failed doc={doc_id} seq={seq}: {e}");
    }

    // Persist the full sealed Update so it can be replayed verbatim later
    // (`handle_request_missing`). Serialize before the broadcast moves `update`.
    let update_wire = postcard::to_allocvec(&update).expect("postcard encode Update");
    tracing::info!("[Server→Redis] append_pending_delta doc={doc_id} seq={seq} s3_key={s3_key}");
    let _ =
        redis_ops::append_pending_delta(&state.redis, &doc_id, seq, &s3_key, &update_wire).await;

    let broadcast = encode_server_msg(&WsServerMsg::Update {
        seq: ambre_protocol::messages::Seq(seq),
        update,
    });
    tracing::info!("[Server→RedisPubSub] broadcast Update doc={doc_id} seq={seq}");
    let _ = redis_ops::doc_broadcast(&state.redis, &doc_id, &broadcast).await;
}

async fn handle_ephemeral(
    state: &Arc<AppState>,
    conn: &WsConnectionState,
    eph: ambre_protocol::Ephemeral,
) {
    use base64::Engine;
    let doc_id = eph.public_data.document_id;
    if !conn.doc_subscriptions.contains(&doc_id) {
        return;
    }

    let pub_key_hex = eph.public_data.pub_key.to_hex();
    let presence_enc_b64 = base64::engine::general_purpose::URL_SAFE.encode(&eph.ciphertext.0);
    let _ =
        redis_ops::doc_presence_update(&state.redis, &doc_id, &pub_key_hex, &presence_enc_b64, 15)
            .await;

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
    if !conn.doc_subscriptions.contains(&document_id) {
        return;
    }

    let updates =
        match db::get_updates_after(&state.db, &document_id, after_seq.0 as i64, 1000).await {
            Ok(u) => u,
            Err(e) => {
                tracing::error!(error = %e, "failed to get updates after seq");
                return;
            }
        };

    // Rehydrate each indexed delta into the full sealed Update the client needs
    // to decrypt and verify. The bytes live in S3 once flushed, or in the Redis
    // pending list until then; together they cover every seq. A delta we cannot
    // locate or decode is skipped (logged) rather than dropping the whole batch.
    let mut seq_updates: Vec<ambre_protocol::SeqUpdate> = Vec::with_capacity(updates.len());
    for row in updates {
        let seq = row.seq as u64;
        let wire = match crate::s3::fetch_object(&state.s3, &state.s3_bucket, &row.s3_key).await {
            Ok(Some(bytes)) => Some(bytes),
            Ok(None) => match redis_ops::get_pending_delta(&state.redis, &document_id, seq).await {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(seq, error = %e, "replay: pending lookup failed");
                    None
                }
            },
            Err(e) => {
                tracing::error!(seq, error = %e, "replay: S3 fetch failed");
                None
            }
        };
        let Some(wire) = wire else {
            tracing::error!(seq, s3_key = %row.s3_key, "replay: delta not found in S3 or pending");
            continue;
        };
        match postcard::from_bytes::<ambre_protocol::Update>(&wire) {
            Ok(update) => seq_updates.push(ambre_protocol::SeqUpdate {
                seq: ambre_protocol::messages::Seq(seq),
                update,
            }),
            Err(e) => tracing::error!(seq, error = %e, "replay: Update decode failed"),
        }
    }

    let msg = encode_server_msg(&WsServerMsg::MissingUpdates {
        document_id,
        updates: seq_updates,
    });
    let _ = outbound_tx.send(msg).await;
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
    let broadcast = encode_server_msg(&WsServerMsg::ChannelEphemeral {
        channel_id,
        ephemeral,
    });
    let _ = redis_ops::chan_broadcast(&state.redis, &channel_id, &broadcast).await;
}

// -- ACL ----------------------------------------------------------

/// Verifies the caller has a membership row for the document.
async fn verify_document_access(
    pool: &sqlx::PgPool,
    document_id: DocumentId,
    user_id: UserId,
) -> Result<(), AppError> {
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

    let public_data_bytes = postcard::to_allocvec(&update.public_data).map_err(AppError::Decode)?;

    let mut signed_data = Vec::new();
    signed_data.extend_from_slice(&update.nonce.0);
    signed_data.extend_from_slice(&update.ciphertext.0);
    signed_data.extend_from_slice(&public_data_bytes);
    signed_data.extend_from_slice(&update.commitment_tag.0);

    let vk = VerifyingKey::from_bytes(pubkey.as_array()).map_err(|_| AppError::InvalidSignature)?;
    let sig = DalekSig::from_slice(&update.signature.0).map_err(|_| AppError::InvalidSignature)?;

    vk.verify(&signed_data, &sig)
        .map_err(|_| AppError::InvalidSignature)?;

    Ok(())
}

#[cfg(test)]
mod postcard_tests {
    use ambre_core::{DocumentId, PublicKey, SnapshotId, UserId};
    use ambre_protocol::messages::{
        Ciphertext, Clock, CommitmentTag, Counter, Ephemeral, Nonce, Seq, SeqUpdate, Signature,
        Update,
    };
    use ambre_protocol::public_data::{EphemeralPublicData, UpdatePublicData};
    use ambre_protocol::ws::{
        WsClientMsg, WsServerMsg, decode_client_msg, decode_server_msg, encode_client_msg,
        encode_server_msg,
    };

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
        let msg = WsClientMsg::Subscribe {
            document_id: DocumentId::from_bytes([0u8; 16]),
        };
        let bytes = encode_client_msg(&msg);
        assert_eq!(bytes[0], 0x01);
        let decoded = decode_client_msg(&bytes).unwrap();
        assert!(matches!(decoded, WsClientMsg::Subscribe { .. }));
    }

    #[test]
    fn roundtrip_client_unsubscribe() {
        let msg = WsClientMsg::Unsubscribe {
            document_id: DocumentId::from_bytes([1u8; 16]),
        };
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
        let msg = WsServerMsg::Subscribed {
            document_id: DocumentId::from_bytes([2u8; 16]),
        };
        let decoded = decode_server_msg(&encode_server_msg(&msg)).unwrap();
        assert!(matches!(decoded, WsServerMsg::Subscribed { .. }));
    }

    #[test]
    fn roundtrip_server_update() {
        let msg = WsServerMsg::Update {
            seq: Seq(100),
            update: make_update(),
        };
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
        let msg = WsServerMsg::SnapshotRequired {
            document_id: DocumentId::from_bytes([4u8; 16]),
        };
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
        let msg = WsServerMsg::Error {
            code: 409,
            message: "conflict".into(),
        };
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
        let msg = WsClientMsg::Subscribe {
            document_id: DocumentId::from_bytes([0u8; 16]),
        };
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
            0xcd, 0x06, 0x85, 0x43, 0x6b, 0xa1, 0xf7, 0x1e, 0x32, 0xce, 0x9a, 0x84, 0xbd, 0xc6,
            0xfe, 0xa0, 0xb6, 0x49, 0x12, 0x1d, 0x8b, 0xf8, 0x43, 0xa2, 0xdd, 0xc6, 0x9f, 0x14,
            0xf3, 0x5a, 0x3c, 0xb9,
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
            0xab, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55,
            0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x01, 0x02, 0x03,
            0x04, 0x05, 0x06, 0x07,
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
