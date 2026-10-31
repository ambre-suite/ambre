//! WebSocket signaling messages for WebRTC meetings (Rendez-Vous).
//!
//! These messages are used to establish peer-to-peer connections between
//! participants in a meeting. The server relays signaling messages but does
//! not have access to the encrypted media streams (E2EE via SFrame).

use ambre_core::id::{Id, User};
use serde::{Deserialize, Serialize};

/// One WebRTC session description, an offer or an answer, as the browser
/// writes it. The relay forwards it whole and reads none of it. It encodes as
/// the bare string it replaced. `Debug` prints the length: the text names the
/// peer's addresses.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionDescription(String);

impl SessionDescription {
    /// The description the browser wrote.
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The text the browser reads back.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SessionDescription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SessionDescription({} bytes)", self.0.len())
    }
}

/// One ICE candidate, as the browser writes it. The relay forwards it whole
/// and reads none of it. It encodes as the bare string it replaced. `Debug`
/// prints the length: the text names an address of the peer.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IceCandidate(String);

impl IceCandidate {
    /// The candidate the browser wrote.
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The text the browser reads back.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for IceCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IceCandidate({} bytes)", self.0.len())
    }
}

/// Client-to-server signaling messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalingClientMsg {
    /// Send an SDP offer to a specific user.
    Offer {
        target_user_id: Id<User>,
        sdp: SessionDescription,
    },
    /// Send an SDP answer to a specific user.
    Answer {
        target_user_id: Id<User>,
        sdp: SessionDescription,
    },
    /// Send an ICE candidate to a specific user.
    Ice {
        target_user_id: Id<User>,
        candidate: IceCandidate,
    },
}

/// Server-to-client signaling messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalingServerMsg {
    /// Forward an SDP offer from another participant.
    Offer {
        from_user_id: Id<User>,
        sdp: SessionDescription,
    },
    /// Forward an SDP answer from another participant.
    Answer {
        from_user_id: Id<User>,
        sdp: SessionDescription,
    },
    /// Forward an ICE candidate from another participant.
    Ice {
        from_user_id: Id<User>,
        candidate: IceCandidate,
    },
    /// Notify that a new participant has joined the meeting.
    ParticipantJoined { user_id: Id<User> },
    /// Notify that a participant has left the meeting.
    ParticipantLeft { user_id: Id<User> },
}
