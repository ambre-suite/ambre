// ============================================================================
// Process-wide PubSub hub.
//
// Redis PubSub subscriptions are GLOBAL to a connection, so we cannot give each
// WebSocket connection its own subscriber over the shared multiplexed client.
// Two connections watching the same document would collide on "already
// subscribed", and juggling N independent subscribers on one connection
// desynchronises its command/reply stream. Instead a SINGLE task owns ONE
// `PubSubStream` on the shared RESP3 connection. It fans every received message
// out IN-PROCESS to the WS connections that registered interest. Redis topic
// (un)subscriptions are ref-counted: SUBSCRIBE on the first interested
// connection, UNSUBSCRIBE when the last one leaves.
//
// The task outlives its stream (`SRV-101`). A panic, a lost stream or a failed
// first subscribe ends one run of the hub. The supervisor keeps the routing
// table, opens a new stream and subscribes it to every topic still routed. The
// sockets keep their registrations, and a broadcast missed in between is
// recovered the way a full outbound queue is: by `RequestMissing`.
// ============================================================================

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::time::Duration;

use ambre_core::id::{Call, Channel, Document, Id, User};
use futures_util::StreamExt;
use rustis::commands::PubSubCommands;
use tokio::sync::{mpsc, oneshot};

use crate::payload::StoredPayload;

/// A Redis PubSub topic: the name a broadcast is published on and a socket
/// subscribes to. One constructor per kind, so the publisher and the
/// subscriber cannot spell a topic two ways.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Topic(String);

impl Topic {
    /// A document's live broadcasts.
    pub fn document(document_id: &Id<Document>) -> Self {
        Self(format!("doc:{document_id}"))
    }

    /// A user's personal events, such as a document shared with them. Every
    /// connection subscribes to its own for the whole session.
    pub fn user(user_id: &Id<User>) -> Self {
        Self(format!("user:{user_id}"))
    }

    /// The activity of a channel's threads. Every socket watching a document
    /// of the channel subscribes to it (`SRV-161`).
    pub fn channel(channel_id: &Id<Channel>) -> Self {
        Self(format!("channel:{channel_id}"))
    }

    /// The signals of a call: its joins, its leaves and its wraps. Every call socket of the
    /// call subscribes to it, and the document plane never does (`SRV-049`).
    pub fn call(call_id: &Id<Call>) -> Self {
        Self(format!("call:{call_id}"))
    }

    /// A topic no kind names, for a test.
    #[cfg(test)]
    pub(crate) fn named(name: &str) -> Self {
        Self(name.to_owned())
    }

    /// The name Redis knows.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The routing table looks a topic up by the name a message carries.
impl std::borrow::Borrow<str> for Topic {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Topic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One socket's key in the hub's routing table.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionId(String);

impl ConnectionId {
    /// A key no other socket holds.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// A fixed key, for a test that registers by name.
    #[cfg(test)]
    pub(crate) fn named(name: &str) -> Self {
        Self(name.to_owned())
    }
}

/// A registration/forwarding request sent from a WS connection to the hub task.
enum HubCmd {
    Subscribe {
        channel: Topic,
        conn_id: ConnectionId,
        /// Where to forward messages on `channel` for this connection (its WS
        /// outbound queue).
        sink: mpsc::Sender<StoredPayload>,
        /// Answered once the topic is routed. Dropped unanswered when the hub
        /// cannot route it, a panic included.
        routed: oneshot::Sender<()>,
    },
    Unsubscribe {
        channel: Topic,
        conn_id: ConnectionId,
    },
}

/// The hub could not route a topic for this connection: its Redis subscribe
/// failed, or the hub's run ended while handling the request. The connection
/// would otherwise stay open and miss every broadcast, so it ends instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotRouted;

/// Handle to the process-wide PubSub hub. Cheap to clone (wraps an `mpsc`
/// sender); stored in `AppState` and used by every WS connection.
#[derive(Clone)]
pub struct PubSubHub {
    cmd_tx: mpsc::Sender<HubCmd>,
}

impl PubSubHub {
    /// Spawn the supervised hub task on the shared command connection and return
    /// its handle.
    ///
    /// RESP3 lets the one multiplexed connection carry both ordinary commands and
    /// this single PubSub subscriber. The stream is seeded on a private topic
    /// (never published to) so it always exists and topics can be added/removed
    /// dynamically.
    pub fn spawn(redis: rustis::client::Client) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<HubCmd>(256);
        tokio::spawn(supervise_hub(redis, cmd_rx));
        Self { cmd_tx }
    }

    /// Register `sink` to receive messages broadcast on `channel` for `conn_id`.
    ///
    /// # Errors
    ///
    /// `NotRouted` when the hub could not route the topic. The caller ends its
    /// connection, which the client then reopens.
    pub async fn subscribe(
        &self,
        channel: Topic,
        conn_id: ConnectionId,
        sink: mpsc::Sender<StoredPayload>,
    ) -> Result<(), NotRouted> {
        let (routed, answer) = oneshot::channel();
        self.cmd_tx
            .send(HubCmd::Subscribe {
                channel,
                conn_id,
                sink,
                routed,
            })
            .await
            .map_err(|_| NotRouted)?;
        answer.await.map_err(|_| NotRouted)
    }

    /// Stop forwarding `channel` to `conn_id`.
    pub async fn unsubscribe(&self, channel: Topic, conn_id: ConnectionId) {
        let _ = self
            .cmd_tx
            .send(HubCmd::Unsubscribe { channel, conn_id })
            .await;
    }
}

/// Private topic the hub stream is seeded on so it always exists. Never
/// published to; its messages (none) are simply ignored.
const HUB_PRIVATE_CHANNEL: &str = "__ambre_pubsub_hub__";

/// How long the supervisor waits before a new run, so a Redis outage is retried
/// at a bounded rate rather than in a hot loop.
const HUB_RESTART_DELAY: Duration = Duration::from_secs(1);

/// Why one run of the hub ended.
enum HubExit {
    /// Every handle is dropped: the server is shutting down.
    Shutdown,
    /// The stream is gone, or could not be opened or re-subscribed.
    StreamLost,
}

/// Runs the hub until every handle is dropped, starting a new run after any run
/// that panics or loses its stream (`SRV-101`). The routing table lives here, so
/// a new run serves every connection the last one served.
async fn supervise_hub(redis: rustis::client::Client, mut cmd_rx: mpsc::Receiver<HubCmd>) {
    // topic -> (conn_id -> sink). The presence of an entry for a topic means
    // the current stream holds a live Redis SUBSCRIBE on it; emptying it
    // triggers UNSUBSCRIBE.
    let mut routes = HashMap::new();
    loop {
        match crate::supervise::contained("pubsub hub", hub_run(&redis, &mut cmd_rx, &mut routes))
            .await
        {
            Some(HubExit::Shutdown) => return,
            Some(HubExit::StreamLost) | None => tokio::time::sleep(HUB_RESTART_DELAY).await,
        }
    }
}

/// One run of the hub: a fresh stream, subscribed to every topic `routes` still
/// holds, then the command and message loop.
async fn hub_run(
    redis: &rustis::client::Client,
    cmd_rx: &mut mpsc::Receiver<HubCmd>,
    routes: &mut HashMap<Topic, HashMap<ConnectionId, mpsc::Sender<StoredPayload>>>,
) -> HubExit {
    let mut pubsub = match redis.subscribe(HUB_PRIVATE_CHANNEL).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("pubsub hub: subscribe failed, retrying: {e}");
            return HubExit::StreamLost;
        }
    };

    // A connection that ended while no run was serving left its sink behind.
    // Routing to it again would only hold a Redis topic open for nobody.
    routes.retain(|_, sinks| {
        sinks.retain(|_, sink| !sink.is_closed());
        !sinks.is_empty()
    });
    for channel in routes.keys() {
        if let Err(e) = pubsub.subscribe(channel.as_str()).await {
            tracing::error!(channel = %channel, "pubsub hub: re-subscribe failed, retrying: {e}");
            return HubExit::StreamLost;
        }
    }

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Some(HubCmd::Subscribe { channel, conn_id, sink, routed }) => {
                    #[cfg(test)]
                    if channel.as_str() == tests::PANIC_TOPIC {
                        panic!("forced by a test");
                    }
                    match routes.entry(channel.clone()) {
                        Entry::Vacant(slot) => {
                            // First interested connection: subscribe on Redis.
                            if let Err(e) = pubsub.subscribe(channel.as_str()).await {
                                tracing::warn!(channel = %channel, "pubsub hub: subscribe failed: {e}");
                                continue;
                            }
                            slot.insert(HashMap::new()).insert(conn_id, sink);
                        }
                        Entry::Occupied(mut slot) => {
                            slot.get_mut().insert(conn_id, sink);
                        }
                    }
                    // A connection that gave up waiting has nothing to hear.
                    let _ = routed.send(());
                }
                Some(HubCmd::Unsubscribe { channel, conn_id }) => {
                    if let Entry::Occupied(mut slot) = routes.entry(channel.clone()) {
                        slot.get_mut().remove(&conn_id);
                        if slot.get().is_empty() {
                            // Last interested connection left: drop the Redis sub.
                            slot.remove();
                            if let Err(e) = pubsub.unsubscribe(channel.as_str()).await {
                                tracing::warn!(channel = %channel, "pubsub hub: unsubscribe failed: {e}");
                            }
                        }
                    }
                }
                // All handles dropped (server shutting down): stop.
                None => return HubExit::Shutdown,
            },
            msg = pubsub.next() => match msg {
                Some(Ok(m)) => {
                    // Every topic this hub subscribes to is a name it built
                    // itself. A name that is not text is therefore a defect
                    // here, rather than a peer's doing. Say so, instead of
                    // routing a lossy reading of it.
                    let Ok(channel) = m.channel_str() else {
                        tracing::warn!("pubsub hub: non-UTF-8 channel name, message dropped");
                        continue;
                    };
                    if let Some(sinks) = routes.get(channel) {
                        // Fan out to every connection watching this topic. A
                        // full outbound queue drops the message for that one
                        // connection (it will re-sync via RequestMissing); it
                        // never blocks the hub or the other subscribers.
                        let payload = StoredPayload::broadcast(&m);
                        for sink in sinks.values() {
                            let _ = sink.try_send(payload.clone());
                        }
                    }
                }
                Some(Err(e)) => tracing::warn!("pubsub hub: stream error: {e}"),
                None => {
                    tracing::error!("pubsub hub: stream ended, restarting");
                    return HubExit::StreamLost;
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rustis::resp::RefBulkString;

    /// A topic whose subscription makes the hub panic, so a test can force the
    /// failure `SRV-101` is about.
    pub(crate) const PANIC_TOPIC: &str = "__ambre_test_panic__";

    /// The next message `sink` receives, or `None` after a few seconds.
    async fn next_message(sink: &mut mpsc::Receiver<StoredPayload>) -> Option<Vec<u8>> {
        tokio::time::timeout(Duration::from_secs(5), sink.recv())
            .await
            .ok()
            .flatten()
            .map(|frame| frame.expose_bytes().to_vec())
    }

    /// A document frame published by one instance reaches a connection that
    /// subscribed on the other, and one on its own. Each instance routes
    /// through its own hub over its own Redis connection.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_frame_from_one_instance_reaches_a_subscriber_of_the_other(pool: sqlx::PgPool) {
        let one = crate::test_helpers::test_app_state(pool).await;
        let other = crate::test_helpers::second_instance(&one).await;
        let document_id = Id::<Document>::mint();
        let topic = Topic::document(&document_id);

        let (there_tx, mut there) = mpsc::channel(8);
        other
            .pubsub
            .subscribe(topic.clone(), ConnectionId::named("there"), there_tx)
            .await
            .expect("the other instance routes the topic");
        let (here_tx, mut here) = mpsc::channel(8);
        one.pubsub
            .subscribe(topic, ConnectionId::named("here"), here_tx)
            .await
            .expect("this instance routes the topic");

        crate::redis_ops::doc_broadcast(
            &one.redis,
            &document_id,
            &StoredPayload::new(b"frame".to_vec()),
        )
        .await
        .expect("publish");
        assert_eq!(
            next_message(&mut there).await.as_deref(),
            Some(&b"frame"[..])
        );
        assert_eq!(
            next_message(&mut here).await.as_deref(),
            Some(&b"frame"[..])
        );
    }

    /// `SRV-101`: a panic while the hub handles one connection's request
    /// refuses that request alone. Every other connection keeps its route,
    /// and a broadcast published after the panic still reaches it.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn a_panic_in_the_hub_refuses_one_request_and_keeps_fanning_out() {
        let redis = rustis::client::Client::connect("redis://localhost:6379")
            .await
            .expect("a running redis");
        let hub = PubSubHub::spawn(redis.clone());
        let topic = Topic::named(&format!("test:hub-panic:{}", uuid::Uuid::new_v4()));

        let (watcher_tx, mut watcher_rx) = mpsc::channel(8);
        hub.subscribe(topic.clone(), ConnectionId::named("watcher"), watcher_tx)
            .await
            .expect("an ordinary topic is routed");

        let (victim_tx, _victim_rx) = mpsc::channel(8);
        let refused = hub
            .subscribe(
                Topic::named(PANIC_TOPIC),
                ConnectionId::named("victim"),
                victim_tx,
            )
            .await;
        assert_eq!(refused, Err(NotRouted), "the request the hub panicked on");

        // The supervisor restarts the hub after its delay, re-subscribed to the
        // watcher's topic. Publish until the watcher hears it, within a bound.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let heard = loop {
            let _: usize = redis
                .publish(topic.as_str(), RefBulkString::new(b"after the panic"))
                .await
                .expect("publish");
            if let Ok(Some(bytes)) =
                tokio::time::timeout(Duration::from_millis(200), watcher_rx.recv()).await
            {
                break Some(bytes.expose_bytes().to_vec());
            }
            if tokio::time::Instant::now() > deadline {
                break None;
            }
        };
        assert_eq!(heard.as_deref(), Some(&b"after the panic"[..]));

        // A new connection is routed by the restarted hub, too.
        let (late_tx, mut late_rx) = mpsc::channel(8);
        hub.subscribe(topic.clone(), ConnectionId::named("late"), late_tx)
            .await
            .expect("the restarted hub routes a new connection");
        let _: usize = redis
            .publish(topic.as_str(), RefBulkString::new(b"to both"))
            .await
            .expect("publish");
        assert_eq!(
            next_message(&mut late_rx).await.as_deref(),
            Some(&b"to both"[..])
        );
    }
}
