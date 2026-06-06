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
    let ciphertext_len = update.ciphertext.0.len();

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

    let _ = redis_ops::append_pending_delta(&state.redis, &doc_id, seq, &s3_key, ciphertext_len).await;

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

    let public_data_bytes = postcard::to_allocvec(&update.public_data)
        .map_err(|e| AppError::Decode(e))?;

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
