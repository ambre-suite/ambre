// ambre-sync/src/mock.rs
// ============================================================================
// In-memory WebSocket transport for testing the Sync pipeline without WASM or
// network. Two `ambre_bus::conduit` pairs form a bidirectional client<->server loop.

#![allow(
    clippy::expect_used,
    reason = "a test double fails the test that misuses it"
)]

use ambre_core::id::{Document, Id};
use ambre_protocol::ws::{WireDecodeError, WsClientMsg, WsServerMsg, decode_client_msg};

/// Bidirectional in-memory WebSocket transport for host tests.
///
/// ```
/// // ---- client side (SyncState) ----
/// //   sends via `transport.client_tx`
/// //   reads  via `transport.client_rx`
///
/// // ---- server side (test harness) ----
/// //   reads  via `transport.server_rx`  (what the client sent)
/// //   writes via `transport.server_tx`  (messages to inject)
/// ```
pub struct MockWsTransport {
    /// Client -> Server: the Sync state pushes encoded `WsClientMsg` bytes here.
    pub client_tx: ambre_bus::conduit::UnboundedSender<ambre_protocol::WireFrame>,
    /// Client -> Server: drained by the test harness.
    pub server_rx: ambre_bus::conduit::Receiver<ambre_protocol::WireFrame>,
    /// Server -> Client: the test harness pushes `WsServerMsg` here.
    pub server_tx: ambre_bus::conduit::UnboundedSender<WsServerMsg>,
    /// Server -> Client: drained by the Sync state (via `receive_server_msg`).
    pub client_rx: ambre_bus::conduit::Receiver<WsServerMsg>,
}

impl MockWsTransport {
    pub fn new() -> Self {
        let (client_tx, server_rx) = ambre_bus::conduit::unbounded();
        let (server_tx, client_rx) = ambre_bus::conduit::unbounded();
        Self {
            client_tx,
            server_rx,
            server_tx,
            client_rx,
        }
    }
}

impl Default for MockWsTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl MockWsTransport {
    /// Convenience: decode the next pending client message.
    ///
    /// # Errors
    ///
    /// [`WireDecodeError`] when the pending bytes do not decode.
    ///
    /// # Panics
    ///
    /// When no client message is pending. This is test scaffolding, and the
    /// client runs on the test's own thread. A message not queued yet never
    /// will be, so the test itself went wrong.
    pub fn recv_client_msg(&self) -> Result<WsClientMsg, WireDecodeError> {
        let wire = self
            .server_rx
            .try_recv()
            .expect("no pending client message");
        decode_client_msg(&wire)
    }

    /// Convenience: encode and push a server message.
    ///
    /// # Panics
    ///
    /// When the server queue is closed, for the reason
    /// [`Self::recv_client_msg`] states.
    pub fn send_server_msg(&self, msg: WsServerMsg) {
        self.server_tx
            .send(msg)
            .expect("server channel disconnected");
    }

    /// Convenience: drain all pending client messages.
    pub fn drain_client_msgs(&self) -> Vec<WsClientMsg> {
        self.server_rx
            .try_iter()
            .filter_map(|wire| decode_client_msg(&wire).ok())
            .collect()
    }

    /// Encode and send a `Subscribed` server message for the given document.
    pub fn send_subscribed(&self, document_id: Id<Document>) {
        self.send_server_msg(WsServerMsg::Subscribed { document_id });
    }
}
