// ============================================================================
// Multiplexed WebSocket handler — ONE connection per client session.
//
// Endpoint: GET /api/ws -> upgrade WebSocket
// The handler maintains typed subscription sets (HashSet<DocumentId> /
// HashSet<ChannelId>) per connection.
// Subscribe/Unsubscribe messages add/remove documents from the set.
// Redis PubSub channels are subscribed/unsubscribed dynamically.
// ============================================================================

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;
use ambre_core::{ChannelId, DocumentId, UserId};

use crate::state::AppState;
use ambre_protocol::ws::{WsClientMsg, WsServerMsg, decode_client_msg, encode_server_msg};

/// HTTP upgrade handler.
pub async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    // Session extracted from cookie via middleware (tower layer).
    session: Session,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state, session))
}

/// Per-connection state.
struct WsConnectionState {
    user_id: UserId,
    session_id: String,
    /// Documents this connection is subscribed to.
    doc_subscriptions: HashSet<DocumentId>,
    /// Channels this connection is subscribed to.
    channel_subscriptions: HashSet<ChannelId>,
}

async fn handle_ws(socket: WebSocket, _state: Arc<AppState>, session: Session) {
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<Vec<u8>>(256);

    let mut conn = WsConnectionState {
        user_id: session.user_id,
        session_id: session.session_id.clone(),
        doc_subscriptions: HashSet::new(),
        channel_subscriptions: HashSet::new(),
    };

    // -- Redis PubSub subscriber (one per connection, N channels) --
    // Using rustis PubSub: subscribe/unsubscribe dynamically as docs are opened/closed.
    // let mut pubsub = state.redis.subscribe().await;

    // -- Spawn outbound task: forward messages from mpsc to WebSocket --
    let (mut ws_sink, mut ws_stream) = socket.split();
    let outbound_handle = tokio::spawn(async move {
        while let Some(bytes) = outbound_rx.recv().await {
            if ws_sink.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
    });

    // -- Inbound loop: read from WebSocket, dispatch by message type --
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
                // 1. ACL check: is this user a member of this document?
                // let has_access = db::check_document_access(&state.db, document_id, conn.user_id).await;
                // if !has_access {
                //     send_error(&outbound_tx, 403, "access denied").await;
                //     continue;
                // }

                // 2. Add to subscription set.
                conn.doc_subscriptions.insert(document_id);

                // 3. Register session in Redis (for hasBroadcastAccess).
                // state.redis.hset(format!("doc:{document_id}:sessions"), &conn.session_id, "1").await;

                // 4. Subscribe to Redis PubSub channel for this doc.
                // pubsub.subscribe(format!("doc:{document_id}")).await;

                // 5. Confirm to client.
                let ack = encode_server_msg(&WsServerMsg::Subscribed { document_id });
                let _ = outbound_tx.send(ack).await;
            }

            WsClientMsg::Unsubscribe { document_id } => {
                conn.doc_subscriptions.remove(&document_id);
                // state.redis.hdel(format!("doc:{document_id}:sessions"), &conn.session_id).await;
                // pubsub.unsubscribe(format!("doc:{document_id}")).await;
            }

            WsClientMsg::Update(update) => {
                let doc_id = update.public_data.document_id;

                // Verify this connection is subscribed to this document.
                if !conn.doc_subscriptions.contains(&doc_id) {
                    let err = encode_server_msg(&WsServerMsg::Error {
                        code: 403,
                        message: "not subscribed to this document".into(),
                    });
                    let _ = outbound_tx.send(err).await;
                    continue;
                }

                // 1. Verify Ed25519 signature (server-side, using ed25519-dalek directly).
                //    The server does NOT decrypt — it only verifies the signature is valid.
                // let author_pubkey = update.public_data.pub_key;
                // if !ed25519_verify(&author_pubkey, &update) {
                //     send_error(&outbound_tx, 401, "invalid signature").await;
                //     continue;
                // }

                // 2. Validate clock monotonicity + assign global seq (atomic Lua script).
                // let seq = state.redis.eval_lua(
                //     VERIFY_AND_INCREMENT_CLOCK,
                //     &[format!("doc:{doc_id}:clocks"), format!("doc:{doc_id}:seq")],
                //     &[hex::encode(update.public_data.pub_key), update.public_data.clock.to_string()],
                // ).await;

                // 3. Append to pending list (batched to S3 every 10-30s).
                // state.redis.rpush(format!("doc:{doc_id}:pending"), &data).await;

                // 4. Broadcast to all other subscribed sessions via Redis PubSub.
                // let server_msg = encode_server_msg(&WsServerMsg::Update { seq, update });
                // state.redis.publish(format!("doc:{doc_id}"), &server_msg).await;
            }

            WsClientMsg::Ephemeral(eph) => {
                let doc_id = eph.public_data.document_id;
                if !conn.doc_subscriptions.contains(&doc_id) {
                    continue; // silent drop for ephemeral
                }

                // Broadcast via PubSub (never persisted).
                // let server_msg = encode_server_msg(&WsServerMsg::Ephemeral(eph));
                // state.redis.publish(format!("doc:{doc_id}"), &server_msg).await;
            }

            WsClientMsg::RequestMissing {
                document_id,
                after_seq: _,
            } => {
                if !conn.doc_subscriptions.contains(&document_id) {
                    continue;
                }
                // Query Postgres for updates with seq > after_seq for this document.
                // let updates = db::get_updates_after(&state.db, document_id, after_seq).await;
                // let msg = encode_server_msg(&WsServerMsg::MissingUpdates { document_id, updates });
                // let _ = outbound_tx.send(msg).await;
            }

            WsClientMsg::SubscribeChannel { channel_id } => {
                // Same pattern as Subscribe but for channels.
                conn.channel_subscriptions.insert(channel_id);
                let ack = encode_server_msg(&WsServerMsg::ChannelSubscribed { channel_id });
                let _ = outbound_tx.send(ack).await;
            }

            WsClientMsg::UnsubscribeChannel { channel_id } => {
                conn.channel_subscriptions.remove(&channel_id);
            }

            WsClientMsg::ChannelUpdate { channel_id, update: _ } => {
                if !conn.channel_subscriptions.contains(&channel_id) {
                    continue;
                }
                // Same as document Update but routed to the active segment.
            }

            WsClientMsg::ChannelEphemeral {
                channel_id,
                ephemeral: _,
            } => {
                if !conn.channel_subscriptions.contains(&channel_id) {
                    continue;
                }
                // Broadcast via PubSub (typing indicators).
            }
        }
    }

    // -- Cleanup on disconnect --
    // Remove session from all subscribed documents in Redis.
    for _doc_id in &conn.doc_subscriptions {
        // state.redis.hdel(format!("doc:{doc_id}:sessions"), &conn.session_id).await;
        // state.redis.hdel(format!("doc:{doc_id}:presence"), &hex_pubkey).await;
    }
    for _ch_id in &conn.channel_subscriptions {
        // same cleanup for channels
    }
    outbound_handle.abort();
}

// -- Redis PubSub forwarding --
// In the real implementation, a second tokio task listens on the PubSub subscriber
// and forwards matching messages to outbound_tx. When a new doc is subscribed, the
// task dynamically subscribes to the Redis channel. When unsubscribed, it unsubscribes.
//
// async fn pubsub_forwarder(
//     mut pubsub: PubSubStream,
//     outbound_tx: mpsc::Sender<Vec<u8>>,
//     conn_subscriptions: Arc<RwLock<HashSet<Uuid>>>,
// ) {
//     while let Some(msg) = pubsub.next().await {
//         // msg.channel = "doc:{doc_id}" -> extract doc_id
//         // if conn_subscriptions.read().contains(&doc_id) {
//         //     outbound_tx.send(msg.payload).await;
//         // }
//     }
// }

// Placeholder types for compilation sketch:
struct Session {
    user_id: UserId,
    session_id: String,
}
