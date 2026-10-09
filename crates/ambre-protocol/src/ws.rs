// ============================================================================
// WebSocket protocol -- single multiplexed connection per client session.
//
// Endpoint: GET /api/ws -> upgrade WebSocket
// One connection carries messages for ALL open documents + channels.
// Demux by document_id / channel_id in each message.
// ============================================================================

use crate::messages::{Clock, Seq};
use crate::version::CURRENT_VERSION;
use crate::{Ephemeral, SeqUpdate, Update};
use ambre_core::{ChannelId, DocType, DocumentId, KeyId, PublicKey, SnapshotId};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Client -> Server
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug)]
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
    RequestMissing {
        document_id: DocumentId,
        after_seq: Seq,
    },

    /// Subscribe to a channel (Murmure). Same WS, different subscription set.
    SubscribeChannel { channel_id: ChannelId },

    /// Unsubscribe from a channel.
    UnsubscribeChannel { channel_id: ChannelId },

    /// Encrypted channel update. Routed to the active segment by the server.
    ChannelUpdate {
        channel_id: ChannelId,
        update: Update,
    },

    /// Encrypted channel ephemeral (typing indicator).
    ChannelEphemeral {
        channel_id: ChannelId,
        ephemeral: Ephemeral,
    },

    /// The access token, as the first frame of the socket. The socket address
    /// carries none, so no proxy record and no log line holds it
    /// (`ambre-spec-session.md`, `SRV-031`). The relay reads nothing else
    /// until this frame is accepted.
    Authenticate { token: AccessToken },
}

/// An access token on the wire. `Debug` prints no byte of it, so a logged
/// frame logs no token (`SRV-031`).
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct AccessToken(String);

impl AccessToken {
    /// Wraps the token text.
    pub fn new(token: String) -> Self {
        Self(token)
    }

    /// The token text, for the one call that verifies it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccessToken(<redacted>)")
    }
}

// ---------------------------------------------------------------------------
// Server -> Client
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug)]
pub enum WsServerMsg {
    /// Subscription confirmed.
    Subscribed { document_id: DocumentId },

    /// Encrypted CRDT delta with server-assigned seq.
    Update { seq: Seq, update: Update },

    /// Encrypted ephemeral.
    Ephemeral(Ephemeral),

    /// A peer stopped watching a document: they unsubscribed, or their
    /// connection closed. Broadcast on the document topic to everyone still
    /// there, so a departure is a message and not a timeout
    /// (`1-Foundations/ambre-spec-presence.md`, `PRESENCE-014`).
    ///
    /// It carries what the relay already routes on -- a document id and an
    /// author key -- and nothing else. No content, and no claim about why the
    /// peer left (`ambre-spec-zero-knowledge.md`). The recipient resolves the
    /// key to a `UserId` exactly as it does for an ephemeral. A departure is
    /// attributed by the same spoof-proof path as a cursor.
    ///
    /// Best-effort, like presence itself: a lost `PeerLeft` leaves the peer to
    /// age out of the live set.
    PeerLeft {
        document_id: DocumentId,
        pub_key: PublicKey,
    },

    /// Response to RequestMissing.
    MissingUpdates {
        document_id: DocumentId,
        updates: Vec<SeqUpdate>,
    },

    /// Too many deltas accumulated -- client must compact.
    SnapshotRequired { document_id: DocumentId },

    /// New snapshot available for download via S3. It is pending: a member who
    /// holds the deltas it covers checks it and confirms it (`SRV-129`).
    SnapshotAvailable {
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        /// The highest delta its author says it folds in.
        covered_seq: Seq,
    },

    /// The document key was rotated: every REMAINING member must re-fetch its
    /// freshly wrapped `doc_key_enc` from `/members` and swap it in. Broadcast on
    /// the document topic. Carries no user identity: it is not a revocation
    /// signal. No recipient can mistake it for "you were revoked" (that is the
    /// separate, targeted `DocumentRevoked`). Rotation typically follows a revoke,
    /// but the two are distinct events.
    KeyRotationRequired { document_id: DocumentId },

    /// The authenticated user was removed from a document. Pushed ONLY to the
    /// revoked user, on their per-user topic (`user:{id}`), by the server's
    /// revoke handler. The client closes the document locally. Distinct from
    /// `KeyRotationRequired` so a member managing keys never revokes themselves.
    DocumentRevoked { document_id: DocumentId },

    /// A document was just shared with this user. Pushed on the per-user topic
    /// (`user:{id}`), which every connection subscribes to, so the invitee learns
    /// about the new document without polling. The client refreshes its document
    /// list and notifies the user.
    DocumentShared {
        document_id: DocumentId,
        doc_type: DocType,
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

    /// A delta refused for its key: the document owes a rotation, and every key
    /// it held is one a departed member holds too (`ambre-spec-server.md`,
    /// `SRV-123`). Sent to the author alone. The frame took no sequence number
    /// and consumed no clock (`SRV-125`). So the author seals it again under
    /// the next key, at the same clock, and sends it again (`SYNC-129`).
    KeyRefused {
        document_id: DocumentId,
        key_id: KeyId,
        clock: Clock,
    },
}

// ---------------------------------------------------------------------------
// Wire encoding (postcard + version byte)
// ---------------------------------------------------------------------------

/// Encode a client message for the wire.
///
/// # Panics
///
/// When postcard cannot serialize `msg`. Every field of [`WsClientMsg`] is a
/// postcard type, so the encoder fails only when the allocator does.
#[expect(
    clippy::expect_used,
    reason = "every field is a postcard type and a Vec takes any length, so the encode cannot fail"
)]
pub fn encode_client_msg(msg: &WsClientMsg) -> Vec<u8> {
    let buf = vec![CURRENT_VERSION.as_u8()];
    postcard::to_extend(msg, buf).expect("postcard encode WsClientMsg")
}

/// Decode a client message from the wire.
///
/// # Errors
///
/// * [`WireDecodeError::Empty`] on no bytes at all.
/// * [`WireDecodeError::UnsupportedVersion`] when the first byte is not the
///   wire version.
/// * [`WireDecodeError::Postcard`] when the body does not decode.
pub fn decode_client_msg(data: &[u8]) -> Result<WsClientMsg, WireDecodeError> {
    let Some((&version, body)) = data.split_first() else {
        return Err(WireDecodeError::Empty);
    };
    match version {
        v if v == CURRENT_VERSION.as_u8() => Ok(postcard::from_bytes(body)?),
        v => Err(WireDecodeError::UnsupportedVersion(v)),
    }
}

/// Encode a server message for the wire.
///
/// # Panics
///
/// When postcard cannot serialize `msg`, for the reason
/// [`encode_client_msg`] states.
#[expect(
    clippy::expect_used,
    reason = "every field is a postcard type and a Vec takes any length, so the encode cannot fail"
)]
pub fn encode_server_msg(msg: &WsServerMsg) -> Vec<u8> {
    let buf = vec![CURRENT_VERSION.as_u8()];
    postcard::to_extend(msg, buf).expect("postcard encode WsServerMsg")
}

/// Decode a server message from the wire.
///
/// # Errors
///
/// The three errors [`decode_client_msg`] raises, on the same three inputs.
pub fn decode_server_msg(data: &[u8]) -> Result<WsServerMsg, WireDecodeError> {
    let Some((&version, body)) = data.split_first() else {
        return Err(WireDecodeError::Empty);
    };
    match version {
        v if v == CURRENT_VERSION.as_u8() => Ok(postcard::from_bytes(body)?),
        v => Err(WireDecodeError::UnsupportedVersion(v)),
    }
}

#[derive(Debug)]
pub enum WireDecodeError {
    Empty,
    UnsupportedVersion(u8),
    Postcard(postcard::Error),
}

impl WireDecodeError {
    /// Whether the frame came from a peer on a newer wire version than this
    /// build's. Only an update cures it (`ambre-spec-sync.md`, `SYNC-137`).
    /// An older peer is the relay's lag, never the user's to fix.
    pub fn is_from_newer_peer(&self) -> bool {
        matches!(self, WireDecodeError::UnsupportedVersion(v) if *v > CURRENT_VERSION.as_u8())
    }
}

impl From<postcard::Error> for WireDecodeError {
    fn from(e: postcard::Error) -> Self {
        WireDecodeError::Postcard(e)
    }
}
