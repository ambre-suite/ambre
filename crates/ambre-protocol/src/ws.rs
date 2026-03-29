//! WebSocket frames for real‑time document synchronization.
//!
//! These messages travel over the persistent WebSocket connection between the
//! Sync Worker and the Blind Relay. Snapshots are **not** sent via WebSocket;
//! they are transferred through S3 presigned URLs.

use crate::messages::{Ephemeral, Seq, SeqUpdate, Update};
use ambre_core::Uuid;
use serde::{Deserialize, Serialize};

/// WebSocket error codes (custom protocol, not HTTP).
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WsErrorCode {
    /// Generic invalid message format.
    InvalidMessage = 4000,
    /// Signature verification failed.
    InvalidSignature,
    /// Clock mismatch (non‑sequential update).
    ClockMismatch,
    /// Too many pending updates; snapshot required.
    SnapshotRequired,
    /// Insufficient permissions.
    Forbidden,
    /// Internal server error.
    Internal,
}

/// Client‑to‑server WebSocket messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WsClientMsg {
    /// Send a new update (encrypted CRDT delta).
    Update(Update),
    /// Send an ephemeral message (cursor, presence, etc.).
    Ephemeral(Ephemeral),
    /// Request missing updates after a given sequence number.
    RequestMissing {
        /// Request updates with sequence numbers greater than this value.
        after_seq: Seq,
    },
}

/// Server‑to‑client WebSocket messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WsServerMsg {
    /// Broadcast an update to all connected clients.
    ///
    /// The `seq` field is the server‑assigned monotonic sequence number.
    Update { seq: Seq, update: Update },
    /// Broadcast an ephemeral message to all connected clients.
    Ephemeral(Ephemeral),
    /// Response to a `RequestMissing` message.
    MissingUpdates { updates: Vec<SeqUpdate> },
    /// Notify the client that it has accumulated too many pending updates
    /// and must create a snapshot.
    SnapshotRequired,
    /// Notify the client that a new snapshot is available for download.
    ///
    /// The snapshot itself is stored in S3; the client must fetch it via a
    /// presigned URL obtained through the REST API.
    SnapshotAvailable { snapshot_id: Uuid },
    /// Notify all clients that a user has been revoked and that a key rotation
    /// is required before any further updates can be accepted.
    KeyRotationRequired { revoked_user_id: Uuid },
    /// Generic WebSocket‑level error.
    Error { code: WsErrorCode, message: String },
}
