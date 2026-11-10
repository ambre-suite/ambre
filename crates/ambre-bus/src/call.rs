//! The envelopes of a call between the UI thread and the Sync worker
//! (`ambre-spec-call-media.md section 3`, the client of a call).
//!
//! The UI thread holds the peer connection, and the Sync worker holds the socket of the SFU
//! (`ARCH-005`). So the session descriptions and the TURN servers cross the worker hop. The
//! UI thread posts these commands itself, and reads these events on arrival: the Logic thread
//! holds no call. No key crosses here: the sender keys take a port of their own, between the
//! Sync worker and the call worker (`CALL-064`).

use ambre_core::Validated;
use ambre_protocol::signaling::{IceServer, SessionDescription};
use serde::{Deserialize, Serialize};

/// A session description of the peer connection of the UI thread, for the SFU.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PageDescription {
    /// An offer of the UI thread, which the SFU answers.
    Offer(Validated<SessionDescription>),
    /// The answer of the UI thread to an offer of the SFU.
    Answer(Validated<SessionDescription>),
}

/// What the Sync worker tells the call client of the UI thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallSignal {
    /// The TURN servers the SFU minted a credential for: the peer connection starts with them.
    Ice(Vec<IceServer>),
    /// An offer of the SFU, which the UI thread answers.
    Offer(Validated<SessionDescription>),
    /// The answer of the SFU to the offer of the UI thread.
    Answer(Validated<SessionDescription>),
    /// The call ended for this session: the peer connection closes.
    Ended,
}

/// Who is in a call, as the Sync worker read it from the relay for the Logic thread
/// (`MURMURE-409`). Each member passed the check of its signed join against the membership log
/// of the channel (`MURMURE-596`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallMembers {
    /// The relay holds no state of the call: the call ended (`MURMURE-592`).
    Gone,
    /// The members seated in the call, each once, in identifier order.
    Live(Vec<ambre_core::id::Id<ambre_core::id::User>>),
}
