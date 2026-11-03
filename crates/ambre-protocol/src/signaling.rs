//! WebSocket signaling messages for WebRTC meetings (Rendez-Vous).
//!
//! These messages are used to establish peer-to-peer connections between
//! participants in a meeting. The server relays signaling messages but does
//! not have access to the encrypted media streams (E2EE via SFrame).

use ambre_core::id::{Id, User};
use ambre_core::{Opaque, OpaqueKind, OpaqueLength, WireOpaque};
use serde::{Deserialize, Serialize};

/// The kind of a WebRTC session description, an offer or an answer, as the
/// browser writes it: `Opaque<SessionDescriptions>`. The relay forwards it
/// whole and reads none of it. It encodes as the bare string it replaced.
/// `Debug` prints the length: the text names the peer's addresses.
///
/// An ICE candidate never stands in for a session description:
///
/// ```compile_fail,E0308
/// use ambre_core::Opaque;
/// use ambre_protocol::signaling::{IceCandidates, SessionDescriptions};
/// let _: Opaque<SessionDescriptions> = Opaque::<IceCandidates>::new("candidate:1");
/// ```
pub enum SessionDescriptions {}

impl OpaqueKind for SessionDescriptions {
    const NAME: &'static str = "SessionDescription";
    const LENGTH: OpaqueLength = OpaqueLength::Bytes;
}

impl WireOpaque for SessionDescriptions {}

/// The kind of one ICE candidate, as the browser writes it:
/// `Opaque<IceCandidates>`. The relay forwards it whole and reads none of
/// it. It encodes as the bare string it replaced. `Debug` prints the length:
/// the text names an address of the peer.
pub enum IceCandidates {}

impl OpaqueKind for IceCandidates {
    const NAME: &'static str = "IceCandidate";
    const LENGTH: OpaqueLength = OpaqueLength::Bytes;
}

impl WireOpaque for IceCandidates {}

/// Client-to-server signaling messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalingClientMsg {
    /// Send an SDP offer to a specific user.
    Offer {
        target_user_id: Id<User>,
        sdp: Opaque<SessionDescriptions>,
    },
    /// Send an SDP answer to a specific user.
    Answer {
        target_user_id: Id<User>,
        sdp: Opaque<SessionDescriptions>,
    },
    /// Send an ICE candidate to a specific user.
    Ice {
        target_user_id: Id<User>,
        candidate: Opaque<IceCandidates>,
    },
}

/// Server-to-client signaling messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalingServerMsg {
    /// Forward an SDP offer from another participant.
    Offer {
        from_user_id: Id<User>,
        sdp: Opaque<SessionDescriptions>,
    },
    /// Forward an SDP answer from another participant.
    Answer {
        from_user_id: Id<User>,
        sdp: Opaque<SessionDescriptions>,
    },
    /// Forward an ICE candidate from another participant.
    Ice {
        from_user_id: Id<User>,
        candidate: Opaque<IceCandidates>,
    },
    /// Notify that a new participant has joined the meeting.
    ParticipantJoined { user_id: Id<User> },
    /// Notify that a participant has left the meeting.
    ParticipantLeft { user_id: Id<User> },
}
