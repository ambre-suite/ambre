//! WebSocket signaling messages for WebRTC meetings (Rendez‑Vous).
//!
//! These messages are used to establish peer‑to‑peer connections between
//! participants in a meeting. The server relays signaling messages but does
//! not have access to the encrypted media streams (E2EE via SFrame).

use ambre_core::UserId;
use serde::{Deserialize, Serialize};

/// Client‑to‑server signaling messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalingClientMsg {
    /// Send an SDP offer to a specific user.
    Offer { target_user_id: UserId, sdp: String },
    /// Send an SDP answer to a specific user.
    Answer { target_user_id: UserId, sdp: String },
    /// Send an ICE candidate to a specific user.
    Ice {
        target_user_id: UserId,
        candidate: String,
    },
}

/// Server‑to‑client signaling messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalingServerMsg {
    /// Forward an SDP offer from another participant.
    Offer { from_user_id: UserId, sdp: String },
    /// Forward an SDP answer from another participant.
    Answer { from_user_id: UserId, sdp: String },
    /// Forward an ICE candidate from another participant.
    Ice {
        from_user_id: UserId,
        candidate: String,
    },
    /// Notify that a new participant has joined the meeting.
    ParticipantJoined { user_id: UserId },
    /// Notify that a participant has left the meeting.
    ParticipantLeft { user_id: UserId },
}
