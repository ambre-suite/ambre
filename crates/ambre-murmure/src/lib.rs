//! Murmure, the messaging application of Ambre: the engine core
//! (`specs/5-Murmure/features/ambre-spec-murmure-engine.md`).
//!
//! The Logic thread holds one [`ChannelState`] per open channel, and one
//! [`SpaceState`] per open space. A channel is a
//! set of Loro documents, never one (`MURMURE-021`). The engine reads no clock
//! and no key: the view stamps the time, and the Sync worker seals and routes
//! the deltas this crate exports.
//!
//! The commands and events the engine answers live in `ambre_bus::app::murmure`,
//! as pure data (`BUS-027`).

#![forbid(unsafe_code)]

mod channel;
mod delta;
mod error;
mod members;
mod opening;
pub mod schema;
mod sequence;
mod session;
mod space;

#[cfg(test)]
mod authorship_tests;
#[cfg(test)]
mod channel_tests;
#[cfg(test)]
mod schema_reading;
#[cfg(test)]
mod schema_shape;
#[cfg(test)]
mod space_tests;
#[cfg(test)]
mod unread_tests;

pub use channel::{ChannelState, Named, Sent};
pub use delta::{JSON_SCHEMA_VERSION, Refusal, Signer};
pub use error::MurmureError;
pub use members::{
    PRESENCE_EXPIRY_MS, REFRESH_PERIOD_MS, Roster, find_members, is_presence_sample,
    presence_sample,
};
pub use opening::{Opening, OpeningChannel};
pub use schema::{ChannelType, DocumentKind, SCHEMA_VERSION, SegmentState};
pub use sequence::{Order, Position};
pub use space::{Listing, SpaceState};
