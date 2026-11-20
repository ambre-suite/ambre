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
    /// A later one holds a fresh credential, and the page restarts ICE with it (`CALL-155`).
    Ice(Vec<IceServer>),
    /// An offer of the SFU, which the UI thread answers.
    Offer(Validated<SessionDescription>),
    /// The answer of the SFU to the offer of the UI thread.
    Answer(Validated<SessionDescription>),
    /// The call ended for this session: the peer connection closes.
    Ended,
    /// The Sync worker admitted the sender index `sender`, whose signed join names `member`
    /// and passed the membership log of the channel (`CALL-019`). The page sets the volume of
    /// that member on each slot that carries the index (`CALL-087`).
    Admitted {
        sender: ambre_core::Seq<ambre_core::SenderIndex>,
        member: ambre_core::id::Id<ambre_core::id::User>,
    },
    /// The session of the sender index left the call: its member leaves the volumes the panel
    /// offers.
    Departed(ambre_core::Seq<ambre_core::SenderIndex>),
    /// A later share of another member took the screen of the room: the share of the page
    /// stops (`CALL-107`).
    ShareTakenOver,
}

/// Who is in a call, as the Sync worker read it from the relay for the Logic thread
/// (the Murmure calls spec). Each member passed the check of its signed join against the membership log
/// of the channel (the Murmure calls spec).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallMembers {
    /// The relay holds no state of the call: the call ended (the Murmure calls spec).
    Gone,
    /// The members seated in the call, each once, in identifier order.
    Live(Vec<ambre_core::id::Id<ambre_core::id::User>>),
}
