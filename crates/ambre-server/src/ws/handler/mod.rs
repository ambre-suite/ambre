// ============================================================================
// Multiplexed WebSocket handler -- ONE connection per client session.
//
// Endpoint: GET /api/ws -> upgrade WebSocket.
// Each connection maintains a set of subscribed document IDs. A Murmure
// channel travels as its documents, so it has no subscription of its own.
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

use ambre_core::id::{Channel, Document, Id, User};
use ambre_core::{Bounded, FixedBytes, PublicKey, RefusalCode, Sec, TimeSpan};
use ambre_protocol::ws::{CloseCode, RelayClose, WsClientMsg, WsServerMsg};

use crate::payload::{self, StoredPayload};

use crate::auth;
use crate::db;
use crate::error::AppError;
use crate::redis_ops;
use crate::state::AppState;
use crate::ws::pubsub_hub::{ConnectionId, Topic};

/// HTTP upgrade handler. The socket address carries no credential. The access
/// token arrives as the socket's first frame, so no proxy record, log line or
/// URL holds it (`ambre-spec-session.md`, `SRV-031`). A query on the address
/// is never read.
pub async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // Bound the inbound message so the server never buffers an oversized one
    // (axum defaults to 64 MiB). The delta ciphertext cap is enforced per-message
    // in `handle_update`. This is the outer envelope guard, sized with headroom
    // for the wire framing (version byte, postcard tag, public_data) around a
    // `delta_max_size` ciphertext.
    let max_frame = state
        .config
        .delta_max_size
        .saturating_len()
        .to_usize()
        .saturating_add(64 * 1024);
    let ws = ws.max_message_size(max_frame).max_frame_size(max_frame);
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// How long an open socket may wait for the frame that authenticates it.
const AUTH_DEADLINE: Duration = Duration::from_secs(10);

/// Why the first frame of a socket opens no session.
enum FirstFrameRefusal {
    /// The frame opens on an older wire version than the relay's (`SRV-269`).
    OlderWire,
    /// Any other refusal: no frame, another message, or a session refused.
    Session(AppError),
}

/// The session the socket's first frame opens. A first frame other than
/// `Authenticate`, one that does not decode, or none before the deadline is
/// refused as unauthorized. A frame of an older wire version is refused apart,
/// so the client is told to update rather than to renew (`SRV-269`).
async fn authenticate_first_frame(
    state: &AppState,
    socket: &mut WebSocket,
) -> Result<(auth::AuthSession, TimeSpan<Sec>), FirstFrameRefusal> {
    let first = tokio::time::timeout(AUTH_DEADLINE, payload::first_frame(socket))
        .await
        .ok()
        .flatten()
        .ok_or(FirstFrameRefusal::Session(AppError::Unauthorized))?;
    match first.client_msg() {
        Ok(WsClientMsg::Authenticate { token }) => {
            let session =
                auth::authenticate_token(state, &token).map_err(FirstFrameRefusal::Session)?;
            let left = auth::token_left(state, &token, crate::s3::unix_now())
                .map_err(FirstFrameRefusal::Session)?;
            Ok((session, left))
        }
        Err(ambre_protocol::ws::WireDecodeError::UnsupportedVersion(version))
            if version < ambre_protocol::version::CURRENT_VERSION.get() =>
        {
            Err(FirstFrameRefusal::OlderWire)
        }
        _ => Err(FirstFrameRefusal::Session(AppError::Unauthorized)),
    }
}

/// How long a `Subscribe`-time membership check is trusted before `handle_update`
/// re-consults `document_members` (H8). Membership is authorized once at
/// `Subscribe`; without this bound, a member revoked mid-session keeps writing
/// accepted updates until they disconnect. A short TTL caps that window while
/// keeping the hot write path off the database on every message.
const MEMBERSHIP_RECHECK_TTL: Duration = Duration::from_secs(30);

/// Minimum interval between two `RequestMissing` replays for the same document on
/// one connection. A single `RequestMissing{after_seq:0}` triggers a full paged
/// replay of the document history from S3, a concurrent GET fan-out per page. An
/// unthrottled client could spam it and turn a trivial message into an O(n)
/// storage-read amplification. One replay per reconnect is the legitimate need;
/// this cooldown bounds the abuse without touching that path. The value is the
/// protocol's, so the Sync worker's spacing cannot fall under it (`SYNC-191`).
const REPLAY_MIN_INTERVAL: Duration = ambre_protocol::ws::REPLAY_MIN_INTERVAL.to_duration();

/// What the caller's role on a subscribed document permits: writing, or
/// reading alone. The write gate of `handle_update` reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocWrite {
    /// An owner or an editor: a delta is sequenced and broadcast.
    Allowed,
    /// A viewer, or a role no document holds: a delta is refused.
    ReadOnly,
}

impl DocWrite {
    /// The permission a role carries on a document.
    const fn of(role: ambre_core::Role) -> Self {
        if role.can_write_document() {
            Self::Allowed
        } else {
            Self::ReadOnly
        }
    }
}

/// Per-connection state tracking which documents this client is currently
/// subscribed to.
struct WsConnectionState {
    user_id: Id<User>,
    /// Ed25519 signing key bound to this session (from the access token's `spk`
    /// claim). Every authored `Update` MUST carry this exact key. It bounds the
    /// per-document clock hash to one entry per member, and stops a member from
    /// authoring under an arbitrary self-generated key (impersonation).
    signing_pubkey: FixedBytes<PublicKey>,
    doc_subscriptions: HashSet<Id<Document>>,
    /// Last instant membership was verified for each subscribed document. Drives
    /// the [`MEMBERSHIP_RECHECK_TTL`] re-check in `handle_update` (H8).
    doc_access_verified_at: HashMap<Id<Document>, Instant>,
    /// Whether the caller's role on each subscribed document permits writing
    /// (`owner`/`editor`, not `viewer`). Captured at `Subscribe` and refreshed on
    /// the H8 membership recheck, so `handle_update` can reject a read-only
    /// member's delta without a per-message DB hit. The REST write paths gate on
    /// the same role set. Without this the hot WS write path would let a `viewer`
    /// author sequenced, persisted, broadcast updates.
    doc_write: HashMap<Id<Document>, DocWrite>,
    /// Last instant a `RequestMissing` replay was served per document, enforcing
    /// [`REPLAY_MIN_INTERVAL`] against replay amplification.
    last_replay_at: HashMap<Id<Document>, Instant>,
    /// Per-connection inbound-message rate limiter. The WS upgrade is rate-limited
    /// once, but a long-lived socket is otherwise unbounded. Each `Update` drives
    /// a Postgres + Redis + S3 write and a fan-out, so an authorized flooder is a
    /// storage/cost amplification. One token is spent per inbound message; when
    /// the bucket is empty the message is rejected before any handler runs.
    msg_bucket: TokenBucket,
    /// The documents whose deltas this socket drops until the author answers
    /// a refused clock (`SRV-169`).
    clock_fences: ClockFences,
    /// The channel of each subscribed document that belongs to one, with its
    /// kind there. Read once at `Subscribe`. A socket watching a document of
    /// a channel hears the activity of its threads, and a delta on a thread
    /// signals it (`SRV-161`).
    doc_channels: HashMap<Id<Document>, (Id<Channel>, ambre_core::ChannelDocumentKind)>,
}

impl WsConnectionState {
    /// The channel `document_id` belongs to, while this socket watches it.
    fn channel_of(&self, document_id: &Id<Document>) -> Option<Id<Channel>> {
        self.doc_channels
            .get(document_id)
            .map(|(channel, _)| *channel)
    }

    /// Whether this socket still watches a document of `channel`.
    fn watches_channel(&self, channel: &Id<Channel>) -> bool {
        self.doc_channels.values().any(|(held, _)| held == channel)
    }
}

/// A document whose unsubscribe makes the socket loop panic, so a test can
/// force the failure `SRV-101` is about.
#[cfg(test)]
pub(crate) const PANIC_DOCUMENT: Id<Document> = Id::<Document>::from_bytes([0xff; 16]);

/// The close code a refused upgrade sends for `error`. The code carries its
/// reason text.
fn refusal_close(error: &AppError) -> Bounded<CloseCode> {
    match error {
        AppError::SetupRequired => Bounded::<CloseCode>::SETUP_REQUIRED,
        _ => Bounded::<CloseCode>::UNAUTHORIZED,
    }
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>) {
    let (session, token_left) = match authenticate_first_frame(&state, &mut socket).await {
        Ok(s) => s,
        Err(FirstFrameRefusal::OlderWire) => {
            // An older client cannot read the close code below, which it does
            // not know. It reads the version byte of any frame, and a newer one
            // tells it to install an update and stop reconnecting (`SYNC-137`).
            tracing::debug!("WS refused a client of an older wire version");
            let refusal = WsServerMsg::Error {
                document_id: None,
                code: RefusalCode::BadRequest,
            };
            let _ = socket
                .send(StoredPayload::server_msg(&refusal).into_message())
                .await;
            let code = Bounded::<CloseCode>::UPDATE_REQUIRED;
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: code.get(),
                    reason: code.reason().unwrap_or_default().into(),
                })))
                .await;
            return;
        }
        Err(FirstFrameRefusal::Session(e)) => {
            // The refusal never reaches `AppError::into_response`, so log it here
            // by severity: a rejected session is routine, a server fault is loud.
            match e {
                AppError::Unauthorized | AppError::SessionExpired | AppError::SetupRequired => {
                    tracing::debug!("WS auth rejected: {e}")
                }
                _ => tracing::error!("WS auth failed (server fault): {e}"),
            }
            // Tell the client explicitly why the socket was refused, with an
            // application close code, instead of a generic network drop.
            let code = refusal_close(&e);
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: code.get(),
                    reason: code.reason().unwrap_or_default().into(),
                })))
                .await;
            return;
        }
    };

    // Counted until the session ends, whichever path ends it (`SRV-257`).
    let _open = crate::metrics::OpenSocket::of(crate::metrics::Socket::Document);
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<StoredPayload>(256);

    let user_id = session.user_id;
    let connection_id = ConnectionId::new();

    let mut conn = WsConnectionState {
        user_id,
        signing_pubkey: session.signing_pubkey,
        doc_subscriptions: HashSet::new(),
        doc_access_verified_at: HashMap::new(),
        doc_write: HashMap::new(),
        last_replay_at: HashMap::new(),
        msg_bucket: TokenBucket::new(state.config.ws_msg_burst, state.config.ws_msg_rate_per_sec),
        clock_fences: ClockFences::default(),
        doc_channels: HashMap::new(),
    };

    let (mut ws_sink, mut ws_stream) = socket.split();

    let outbound_handle = tokio::spawn(async move {
        while let Some(bytes) = outbound_rx.recv().await {
            if ws_sink.send(bytes.into_message()).await.is_err() {
                break;
            }
        }
    });

    // Live broadcasts (a peer's update / ephemeral / membership change) are
    // published to Redis PubSub by the writer's connection. THIS connection
    // registers its outbound queue with the process-wide hub for every topic
    // it watches. The hub holds the single Redis subscriber per topic, and
    // fans each message in-process to all registered connections. (The writer's
    // own broadcast is echoed back to it too; a client's CRDT import of its own
    // ops is an idempotent no-op.)
    let pubsub = state.pubsub.clone();

    // Personal topic: subscribe for the whole session so server-initiated
    // pushes reach this connection even when no document is open. A
    // `DocumentShared` on a fresh invite is one.
    // A topic the hub cannot route ends the socket, which the client reopens:
    // left open, it would miss every broadcast on that topic (`SRV-101`).
    let routed = pubsub
        .subscribe(
            Topic::user(&user_id),
            connection_id.clone(),
            outbound_tx.clone(),
        )
        .await;

    // A panic while serving one frame ends this socket alone (`SRV-101`). It
    // still reaches the cleanup below, so the hub stops routing to it and its
    // peers hear it leave.
    let serve = async {
        if routed.is_err() {
            return;
        }
        // The membership of each watched document is read again on this period, so a
        // removal stops the broadcasts of a silent socket too (`SRV-043`).
        let mut sweep = tokio::time::interval(MEMBERSHIP_RECHECK_TTL);
        sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick fires at once, and every membership was just read.
        sweep.reset();
        // The socket ends when the token that opened it expires. The client
        // renews, and its next socket opens under the fresh token. A stolen
        // token thus opens no socket that outlives it, a sign-out or a
        // revoked refresh token (`SRV-303`).
        let expiry = tokio::time::sleep(token_left.to_duration());
        tokio::pin!(expiry);
        loop {
            let frame = tokio::select! {
                frame = payload::next_frame(&mut ws_stream) => frame,
                _ = sweep.tick() => {
                    message::sweep_revoked(&state, &mut conn, &connection_id).await;
                    continue;
                }
                () = &mut expiry => {
                    tracing::debug!("WS session ended at the expiry of its token");
                    break;
                }
            };
            let Some(frame) = frame else {
                break;
            };
            let client_msg = match frame.client_msg() {
                Ok(m) => m,
                Err(e) => {
                    let detail = format!("decode error: {e:?}");
                    refuse(&outbound_tx, None, RefusalCode::BadRequest, &detail).await;
                    continue;
                }
            };

            // Per-connection message rate limit (#1). Spend one token per inbound
            // message before dispatching. An open socket is otherwise unbounded, and an
            // `Update`/`Ephemeral` flood turns into a Postgres + Redis + S3 + fan-out
            // amplification. When the bucket is empty we reject with 429 and drop the
            // message before any handler (and any durable write) runs. The token costs
            // no Redis round-trip; it lives entirely on `conn`.
            if !conn.msg_bucket.try_consume(Instant::now()) {
                let detail = "message rate limit exceeded";
                refuse(&outbound_tx, None, RefusalCode::RateLimited, detail).await;
                continue;
            }

            #[cfg(test)]
            if matches!(
                client_msg,
                WsClientMsg::Unsubscribe { document_id } if document_id == PANIC_DOCUMENT
            ) {
                panic!("forced by a test");
            }

            // The document a frame names, when this connection watches it. A
            // frame whose membership check fails drops that subscription, and
            // its hub routes go with it below (`SRV-301`).
            let watched = match &client_msg {
                WsClientMsg::Update(update) => Some(update.public_data.document_id),
                WsClientMsg::Ephemeral(eph) => Some(eph.public_data.document_id),
                WsClientMsg::RequestMissing { document_id, .. } => Some(*document_id),
                _ => None,
            }
            .filter(|doc| conn.doc_subscriptions.contains(doc));

            match client_msg {
                WsClientMsg::Subscribe { document_id } => {
                    handle_subscribe(&state, &mut conn, document_id, &outbound_tx).await;
                    // Start forwarding this doc's live broadcasts only if access was
                    // granted (handle_subscribe records the subscription on success).
                    if conn.doc_subscriptions.contains(&document_id)
                        && pubsub
                            .subscribe(
                                Topic::document(&document_id),
                                connection_id.clone(),
                                outbound_tx.clone(),
                            )
                            .await
                            .is_err()
                    {
                        break;
                    }
                    // A document of a channel also hears the activity of its
                    // threads. The hub keeps one route per socket and topic,
                    // so a second document of the channel adds none.
                    if let Some(channel) = conn.channel_of(&document_id)
                        && pubsub
                            .subscribe(
                                Topic::channel(&channel),
                                connection_id.clone(),
                                outbound_tx.clone(),
                            )
                            .await
                            .is_err()
                    {
                        break;
                    }
                }
                WsClientMsg::Unsubscribe { document_id } => {
                    // Announce the departure to the peers still on the document
                    // before the subscription goes (`PRESENCE-014`). Their live
                    // set then drops this peer at once, rather than on a timeout.
                    if conn.doc_subscriptions.contains(&document_id) {
                        announce_departure(&state, &document_id, &conn.signing_pubkey).await;
                    }
                    let channel = conn.channel_of(&document_id);
                    handle_unsubscribe(&mut conn, document_id);
                    pubsub
                        .unsubscribe(Topic::document(&document_id), connection_id.clone())
                        .await;
                    if let Some(channel) = channel
                        && !conn.watches_channel(&channel)
                    {
                        pubsub
                            .unsubscribe(Topic::channel(&channel), connection_id.clone())
                            .await;
                    }
                }
                WsClientMsg::Update(update) => {
                    tracing::debug!(
                        "[Server<-Sync] WS Update doc={} author={} clock={}",
                        update.public_data.document_id,
                        update.public_data.pub_key.to_hex(),
                        update.public_data.clock.get(),
                    );
                    handle_update(&state, &mut conn, update, &outbound_tx).await;
                }
                WsClientMsg::ClockResync {
                    document_id,
                    signature,
                } => conn.clock_fences.answer(document_id, &signature),
                WsClientMsg::Ephemeral(eph) => {
                    handle_ephemeral(&state, &mut conn, eph).await;
                }
                WsClientMsg::RequestMissing {
                    document_id,
                    after_seq,
                } => {
                    handle_request_missing(&state, &mut conn, document_id, after_seq, &outbound_tx)
                        .await;
                }
                // The session is bound once, by the first frame. A later token
                // changes nothing on this connection.
                WsClientMsg::Authenticate { .. } => {}
            }
            if let Some(doc) = watched {
                message::drop_lost_routes(&state, &mut conn, doc, &connection_id).await;
            }
        }
    };
    crate::supervise::contained("socket", serve).await;

    // Deregister every topic this connection watched from the hub (the hub
    // drops its Redis subscription when the last interested connection leaves).
    for doc_id in &conn.doc_subscriptions {
        // A closed connection is a departure from every document it watched
        // (`PRESENCE-014`). The peers still there hear it now, instead of
        // waiting out the presence field's TTL.
        announce_departure(&state, doc_id, &conn.signing_pubkey).await;
        pubsub
            .unsubscribe(Topic::document(doc_id), connection_id.clone())
            .await;
    }
    let channels: HashSet<Id<Channel>> = conn.doc_channels.values().map(|(c, _)| *c).collect();
    for channel in &channels {
        pubsub
            .unsubscribe(Topic::channel(channel), connection_id.clone())
            .await;
    }
    pubsub
        .unsubscribe(Topic::user(&user_id), connection_id.clone())
        .await;
    outbound_handle.abort();
}

// The call socket, an endpoint of its own (`SRV-049`).
mod call;
pub use call::call_upgrade;

// Per-message handlers + their tests live in the sibling `message` module,
// reached (and their pub(crate) fns brought into scope) via `use self::message::*`.
mod message;
use message::*;

// The envelope signature check, the one reading of a ciphertext's bytes on the
// socket path, in a file that holds no log line (`SRV-001`).
mod signature;
use signature::*;

// Per-connection inbound-message rate limiter, kept in its own module so the
// pure token-bucket logic is unit-testable without a socket.
mod token_bucket;
use token_bucket::TokenBucket;

// The load run of the write path, by hand (`typist_load.rs`).
#[cfg(test)]
mod typist_load;
