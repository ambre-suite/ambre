//! Wire protocol types for Ambre client-server communication.
//!
//! Defines all network messages: updates, snapshots, ephemerals, WebSocket frames,
//! and the signalling of a call.
//!
//! This crate is the single source of truth for wire types; neither `ambre-sync`
//! nor `ambre-server` redefine these structures.

// No hand-written `unsafe` in this crate (constitution pillar): any future
// `unsafe` block breaks the build.
#![forbid(unsafe_code)]
// A length, count, position or version here comes from the wire or a peer, so
// an overflow, a wrap or an arithmetic panic is refused. A test asserts by
// panicking, so the lint stops at its boundary, as the panic lints of
// `clippy.toml` do (`ARCH-028`).
#![cfg_attr(not(test), deny(clippy::arithmetic_side_effects))]

pub mod error;
pub mod key_link;
pub mod key_registration;
pub mod master_rotation;
pub mod membership;
pub mod messages;
pub mod public_data;
pub mod signaling;
pub mod snapshot_confirm;
pub mod snapshot_report;
pub mod thread_erasure;
pub mod transcript;
pub mod version;
pub mod ws;

// Re-exports for convenient consumption.
pub use error::Error;
pub use key_link::KeyLink;
pub use membership::{
    DOCUMENT_CAPS_MAX, DocumentCap, DocumentCaps, Elsewhere, EntryDigest, LogHead, LogPlaces,
    MembershipAct, MembershipEntry, ownerless_log, place_after,
};
pub use messages::{CommitmentTag, Ephemeral, Nonce, SeqUpdate, Signature, Snapshot, Update};
pub use public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
pub use signaling::{
    Admission, CallClientMsg, CallParticipants, CallRefusal, CallServerMsg, PARTICIPANT_CAP,
    Seated, SfuMsg,
};
pub use transcript::SignedTranscript;
pub use version::{CURRENT_VERSION, check_version};
pub use ws::{WireFrame, WsClientMsg, WsServerMsg};

#[cfg(test)]
mod tests;
