//! Wire protocol types for Ambre client-server communication.
//!
//! Defines all network messages: updates, snapshots, ephemerals, WebSocket frames,
//! and signaling messages for WebRTC meetings.
//!
//! This crate is the single source of truth for wire types; neither `ambre-sync`
//! nor `ambre-server` redefine these structures.

// No hand-written `unsafe` in this crate (constitution pillar): any future
// `unsafe` block breaks the build.
#![forbid(unsafe_code)]

pub mod error;
pub mod key_link;
pub mod membership;
pub mod messages;
pub mod public_data;
pub mod signaling;
pub mod snapshot_report;
pub mod version;
pub mod ws;

// Re-exports for convenient consumption.
pub use error::Error;
pub use key_link::KeyLink;
pub use membership::{EntryDigest, MembershipAct, MembershipEntry, ownerless_log};
pub use messages::{CommitmentTag, Ephemeral, Nonce, SeqUpdate, Signature, Snapshot, Update};
pub use public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
pub use signaling::{SignalingClientMsg, SignalingServerMsg};
pub use version::{CURRENT_VERSION, check_version};
pub use ws::{WsClientMsg, WsServerMsg};

#[cfg(test)]
mod tests;
