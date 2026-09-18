// ============================================================================
// Process-wide PubSub hub.
//
// Redis PubSub subscriptions are GLOBAL to a connection, so we cannot give each
// WebSocket connection its own subscriber over the shared multiplexed client.
// Two connections watching the same document would collide on "already
// subscribed", and juggling N independent subscribers on one connection
// desynchronises its command/reply stream. Instead a SINGLE task owns ONE
// `PubSubStream` on the shared RESP3 connection. It fans every received message
// out IN-PROCESS to the WS connections that registered interest. Redis channel
// (un)subscriptions are ref-counted: SUBSCRIBE on the first interested
// connection, UNSUBSCRIBE when the last one leaves.
// ============================================================================

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use futures_util::StreamExt;
use rustis::commands::PubSubCommands;
use tokio::sync::mpsc;

/// A registration/forwarding request sent from a WS connection to the hub task.
enum HubCmd {
    Subscribe {
        channel: String,
        conn_id: String,
        /// Where to forward messages on `channel` for this connection (its WS
        /// outbound queue).
        sink: mpsc::Sender<Vec<u8>>,
    },
    Unsubscribe {
        channel: String,
        conn_id: String,
    },
}

/// Handle to the process-wide PubSub hub. Cheap to clone (wraps an `mpsc`
/// sender); stored in `AppState` and used by every WS connection.
#[derive(Clone)]
pub struct PubSubHub {
    cmd_tx: mpsc::Sender<HubCmd>,
}

impl PubSubHub {
    /// Spawn the hub task on the shared command connection and return its handle.
    ///
    /// RESP3 lets the one multiplexed connection carry both ordinary commands and
    /// this single PubSub subscriber. The stream is seeded on a private channel
    /// (never published to) so it always exists and channels can be added/removed
    /// dynamically.
    pub fn spawn(redis: rustis::client::Client) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<HubCmd>(256);
        tokio::spawn(hub_loop(redis, cmd_rx));
        Self { cmd_tx }
    }

    /// Register `sink` to receive messages broadcast on `channel` for `conn_id`.
    pub async fn subscribe(&self, channel: String, conn_id: String, sink: mpsc::Sender<Vec<u8>>) {
        let _ = self
            .cmd_tx
            .send(HubCmd::Subscribe {
                channel,
                conn_id,
                sink,
            })
            .await;
    }

    /// Stop forwarding `channel` to `conn_id`.
    pub async fn unsubscribe(&self, channel: String, conn_id: String) {
        let _ = self
            .cmd_tx
            .send(HubCmd::Unsubscribe { channel, conn_id })
            .await;
    }
}

/// Private channel the hub stream is seeded on so it always exists. Never
/// published to; its messages (none) are simply ignored.
const HUB_PRIVATE_CHANNEL: &str = "__ambre_pubsub_hub__";

async fn hub_loop(redis: rustis::client::Client, mut cmd_rx: mpsc::Receiver<HubCmd>) {
    let mut pubsub = match redis.subscribe(HUB_PRIVATE_CHANNEL).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("pubsub hub: initial subscribe failed: {e}");
            return;
        }
    };

    // channel -> (conn_id -> sink). The presence of an entry for a channel means
    // we hold a live Redis SUBSCRIBE on it; emptying it triggers UNSUBSCRIBE.
    let mut routes: HashMap<String, HashMap<String, mpsc::Sender<Vec<u8>>>> = HashMap::new();

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Some(HubCmd::Subscribe { channel, conn_id, sink }) => {
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
                None => break,
            },
            msg = pubsub.next() => match msg {
                Some(Ok(m)) => {
                    // Every channel this hub subscribes to is a name it built
                    // itself. A name that is not text is therefore a defect
                    // here, rather than a peer's doing. Say so, instead of
                    // routing a lossy reading of it.
                    let Ok(channel) = m.channel_str() else {
                        tracing::warn!("pubsub hub: non-UTF-8 channel name, message dropped");
                        continue;
                    };
                    if let Some(sinks) = routes.get(channel) {
                        // Fan out to every connection watching this channel. A
                        // full outbound queue drops the message for that one
                        // connection (it will re-sync via RequestMissing); it
                        // never blocks the hub or the other subscribers.
                        for sink in sinks.values() {
                            let _ = sink.try_send(m.payload().to_vec());
                        }
                    }
                }
                Some(Err(e)) => tracing::warn!("pubsub hub: stream error: {e}"),
                None => break,
            }
        }
    }
}
