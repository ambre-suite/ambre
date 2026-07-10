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
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
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
///
/// Operational note (M15): this is a bearer credential in a URL. The server's
/// `TraceLayer` span logs only the URI path (never the query), and the reverse
/// proxy MUST strip `?token=` from its access logs; the short access-token TTL
/// bounds the residual exposure. See `ambre-spec-server.md` sec.3.
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
    // Bound the inbound frame so the server never buffers an oversized message
    // (axum defaults to 64 MiB). The delta ciphertext cap is enforced per-message
    // in `handle_update`; this is the outer envelope guard, sized with headroom
    // for the wire framing (version byte, postcard tag, public_data) around a
    // `delta_max_bytes` ciphertext.
    let max_frame = state.config.delta_max_bytes.saturating_add(64 * 1024);
    let ws = ws.max_message_size(max_frame).max_frame_size(max_frame);
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

/// How long a `Subscribe`-time membership check is trusted before `handle_update`
/// re-consults `document_members` (H8). Membership is authorized once at
/// `Subscribe`; without this bound, a member revoked mid-session keeps writing
/// accepted updates until they disconnect. A short TTL caps that window while
/// keeping the hot write path off the database on every message.
const MEMBERSHIP_RECHECK_TTL: Duration = Duration::from_secs(30);

/// Per-connection state tracking which documents and channels this client
/// is currently subscribed to.
struct WsConnectionState {
    user_id: UserId,
    /// Ed25519 signing key bound to this session (from the access token's `spk`
    /// claim). Every authored `Update` MUST carry this exact key: it bounds the
    /// per-document clock hash to one entry per member and stops a member from
    /// authoring under an arbitrary self-generated key (impersonation).
    signing_pubkey: ambre_core::PublicKey,
    /// Identifies this socket in Redis presence/subscription sets. Generated
    /// fresh per connection (the access token is stateless and carries no id).
    connection_id: String,
    doc_subscriptions: HashSet<DocumentId>,
    /// Last instant membership was verified for each subscribed document. Drives
    /// the [`MEMBERSHIP_RECHECK_TTL`] re-check in `handle_update` (H8).
    doc_access_verified_at: HashMap<DocumentId, Instant>,
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
        signing_pubkey: session.signing_pubkey.clone(),
        connection_id: connection_id.clone(),
        doc_subscriptions: HashSet::new(),
        doc_access_verified_at: HashMap::new(),
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
                tracing::debug!(
                    "[Server<-Sync] WS Update doc={} author={} clock={}",
                    update.public_data.document_id,
                    update.public_data.pub_key.to_hex(),
                    update.public_data.clock.0,
                );
                handle_update(&state, &mut conn, update, &outbound_tx).await;
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

// Per-message handlers + their tests live in the sibling `message` module,
// reached (and their pub(crate) fns brought into scope) via `use self::message::*`.
mod message;
use message::*;
