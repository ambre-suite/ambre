// ============================================================================
// WebSocket protocol — single multiplexed connection per client session.
//
// Endpoint: GET /api/ws -> upgrade WebSocket
// One connection carries messages for ALL open documents + channels.
// Demux by document_id / channel_id in each message.
// ============================================================================

use crate::{Ephemeral, SeqUpdate, Update};
use crate::messages::Seq;
use ambre_core::{ChannelId, DocumentId, SnapshotId, UserId};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Client -> Server
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub enum WsClientMsg {
    /// Start receiving real-time events for a document.
    /// Server adds this doc to the connection's subscription set.
    /// Server responds with Subscribed { document_id }.
    Subscribe { document_id: DocumentId },

    /// Stop receiving events for a document.
    /// Server removes this doc from the subscription set.
    Unsubscribe { document_id: DocumentId },

    /// Encrypted CRDT delta. document_id is inside update.public_data.
    Update(Update),

    /// Encrypted ephemeral (cursor/presence). document_id is inside public_data.
    Ephemeral(Ephemeral),

    /// Request missing updates for a specific document after a known seq.
    RequestMissing { document_id: DocumentId, after_seq: Seq },

    /// Subscribe to a channel (Murmure). Same WS, different subscription set.
    SubscribeChannel { channel_id: ChannelId },

    /// Unsubscribe from a channel.
    UnsubscribeChannel { channel_id: ChannelId },

    /// Encrypted channel update. Routed to the active segment by the server.
    ChannelUpdate { channel_id: ChannelId, update: Update },

    /// Encrypted channel ephemeral (typing indicator).
    ChannelEphemeral {
        channel_id: ChannelId,
        ephemeral: Ephemeral,
    },
}

// ---------------------------------------------------------------------------
// Server -> Client
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub enum WsServerMsg {
    /// Subscription confirmed.
    Subscribed { document_id: DocumentId },

    /// Encrypted CRDT delta with server-assigned seq.
    Update { seq: Seq, update: Update },

    /// Encrypted ephemeral.
    Ephemeral(Ephemeral),

    /// Response to RequestMissing.
    MissingUpdates {
        document_id: DocumentId,
        updates: Vec<SeqUpdate>,
    },

    /// Too many deltas accumulated — client must compact.
    SnapshotRequired { document_id: DocumentId },

    /// New snapshot available for download via S3.
    SnapshotAvailable {
        document_id: DocumentId,
        snapshot_id: SnapshotId,
    },

    /// A member was revoked — fetch new doc_key_enc.
    KeyRotationRequired {
        document_id: DocumentId,
        revoked_user_id: UserId,
    },

    /// Channel subscription confirmed.
    ChannelSubscribed { channel_id: ChannelId },

    /// Channel update with server-assigned seq.
    ChannelUpdate {
        channel_id: ChannelId,
        seq: Seq,
        update: Update,
    },

    /// Channel ephemeral.
    ChannelEphemeral {
        channel_id: ChannelId,
        ephemeral: Ephemeral,
    },

    /// Error (may be global or scoped to a document/channel).
    Error { code: u16, message: String },
}

// ---------------------------------------------------------------------------
// Wire encoding (postcard + version byte)
// ---------------------------------------------------------------------------

/// Encode a client message for the wire.
pub fn encode_client_msg(msg: &WsClientMsg) -> Vec<u8> {
    let buf = vec![0x01]; // wire version
    postcard::to_extend(msg, buf).expect("postcard encode WsClientMsg")
}

/// Decode a client message from the wire.
pub fn decode_client_msg(data: &[u8]) -> Result<WsClientMsg, WireDecodeError> {
    if data.is_empty() {
        return Err(WireDecodeError::Empty);
    }
    match data[0] {
        0x01 => Ok(postcard::from_bytes(&data[1..])?),
        v => Err(WireDecodeError::UnsupportedVersion(v)),
    }
}

/// Encode a server message for the wire.
pub fn encode_server_msg(msg: &WsServerMsg) -> Vec<u8> {
    let buf = vec![0x01];
    postcard::to_extend(msg, buf).expect("postcard encode WsServerMsg")
}

/// Decode a server message from the wire.
pub fn decode_server_msg(data: &[u8]) -> Result<WsServerMsg, WireDecodeError> {
    if data.is_empty() {
        return Err(WireDecodeError::Empty);
    }
    match data[0] {
        0x01 => Ok(postcard::from_bytes(&data[1..])?),
        v => Err(WireDecodeError::UnsupportedVersion(v)),
    }
}

#[derive(Debug)]
pub enum WireDecodeError {
    Empty,
    UnsupportedVersion(u8),
    Postcard(postcard::Error),
}

impl From<postcard::Error> for WireDecodeError {
    fn from(e: postcard::Error) -> Self {
        WireDecodeError::Postcard(e)
    }
}
