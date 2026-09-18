// ambre-sync/src/mock.rs
// ============================================================================
// In-memory WebSocket transport for testing the Sync pipeline without WASM or
// network. Two flume pairs form a bidirectional client<->server loop.

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
    pub client_tx: flume::Sender<Vec<u8>>,
    /// Client -> Server: drained by the test harness.
    pub server_rx: flume::Receiver<Vec<u8>>,
    /// Server -> Client: the test harness pushes `WsServerMsg` here.
    pub server_tx: flume::Sender<WsServerMsg>,
    /// Server -> Client: drained by the Sync state (via `receive_server_msg`).
    pub client_rx: flume::Receiver<WsServerMsg>,
}

impl MockWsTransport {
    pub fn new() -> Self {
        let (client_tx, server_rx) = flume::unbounded();
        let (server_tx, client_rx) = flume::unbounded();
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
    /// When the client channel is closed. This is test scaffolding, and a
    /// closed channel there is the test itself going wrong.
    pub fn recv_client_msg(&self) -> Result<WsClientMsg, WireDecodeError> {
        let wire = self.server_rx.recv().expect("client channel disconnected");
        decode_client_msg(&wire)
    }

    /// Convenience: encode and push a server message.
    ///
    /// # Panics
    ///
    /// When the server channel is closed, for the reason
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
    pub fn send_subscribed(&self, document_id: ambre_core::DocumentId) {
        self.send_server_msg(WsServerMsg::Subscribed { document_id });
    }
}
