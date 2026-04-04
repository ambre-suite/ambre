// ============================================================================
// WebSocket protocol — single multiplexed connection per client session.
//
// Endpoint: GET /api/ws -> upgrade WebSocket
// One connection carries messages for ALL open documents + channels.
// Demux by document_id / channel_id in each message.
// ============================================================================

use crate::{Ephemeral, SeqUpdate, Update};
use crate::messages::Seq;
use ambre_core::Uuid;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Client -> Server
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub enum WsClientMsg {
    /// Start receiving real-time events for a document.
    /// Server adds this doc to the connection's subscription set.
    /// Server responds with Subscribed { document_id }.
    Subscribe { document_id: Uuid },

    /// Stop receiving events for a document.
    /// Server removes this doc from the subscription set.
    Unsubscribe { document_id: Uuid },

    /// Encrypted CRDT delta. document_id is inside update.public_data.
    Update(Update),

    /// Encrypted ephemeral (cursor/presence). document_id is inside public_data.
    Ephemeral(Ephemeral),

    /// Request missing updates for a specific document after a known seq.
    RequestMissing { document_id: Uuid, after_seq: Seq },

    /// Subscribe to a channel (Murmure). Same WS, different subscription set.
    SubscribeChannel { channel_id: Uuid },

    /// Unsubscribe from a channel.
    UnsubscribeChannel { channel_id: Uuid },

    /// Encrypted channel update. Routed to the active segment by the server.
    ChannelUpdate { channel_id: Uuid, update: Update },

    /// Encrypted channel ephemeral (typing indicator).
    ChannelEphemeral {
        channel_id: Uuid,
        ephemeral: Ephemeral,
    },
}

// ---------------------------------------------------------------------------
// Server -> Client
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub enum WsServerMsg {
    /// Subscription confirmed.
    Subscribed { document_id: Uuid },

    /// Encrypted CRDT delta with server-assigned seq.
    Update { seq: Seq, update: Update },

    /// Encrypted ephemeral.
    Ephemeral(Ephemeral),

    /// Response to RequestMissing.
    MissingUpdates {
        document_id: Uuid,
        updates: Vec<SeqUpdate>,
    },

    /// Too many deltas accumulated — client must compact.
    SnapshotRequired { document_id: Uuid },

    /// New snapshot available for download via S3.
    SnapshotAvailable {
        document_id: Uuid,
        snapshot_id: Uuid,
    },

    /// A member was revoked — fetch new doc_key_enc.
    KeyRotationRequired {
        document_id: Uuid,
        revoked_user_id: Uuid,
    },

    /// Channel subscription confirmed.
    ChannelSubscribed { channel_id: Uuid },

    /// Channel update with server-assigned seq.
    ChannelUpdate {
        channel_id: Uuid,
        seq: Seq,
        update: Update,
    },

    /// Channel ephemeral.
    ChannelEphemeral {
        channel_id: Uuid,
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
