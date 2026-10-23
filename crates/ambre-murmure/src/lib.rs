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

mod attachment;
mod channel;
/// Murmure-specific chrome, built on the `ambre-chrome` toolkit. Wasm-only
/// (leptos CSR + DOM).
#[cfg(target_arch = "wasm32")]
pub mod chrome;
pub mod commands;
pub mod composer;
mod delta;
pub mod drafts;
pub mod emoji;
mod error;
mod layout;
pub mod list;
mod look;
pub mod markers;
mod members;
mod mention;
pub mod mention_list;
mod opening;
pub mod paint;
pub mod pictures;
mod reaction;
mod read_aloud;
pub mod schema;
mod search;
pub mod select;
mod sequence;
mod session;
mod sidebar;
mod space;
mod style;
#[cfg(feature = "testing")]
pub mod testing;
pub mod uploads;
pub mod view;
mod watchers;

#[cfg(test)]
mod amending_tests;
#[cfg(test)]
mod attachment_tests;
#[cfg(test)]
mod authorship_tests;
#[cfg(test)]
mod channel_tests;
#[cfg(test)]
mod closing_tests;
#[cfg(test)]
mod edit_tests;
#[cfg(test)]
mod finalizing_tests;
#[cfg(test)]
mod folding_tests;
#[cfg(test)]
mod mention_tests;
#[cfg(test)]
mod reaction_tests;
#[cfg(test)]
mod schema_reading;
#[cfg(test)]
mod schema_shape;

/// The name `text` is, for a test that names a channel, a space or a
/// category within the cap.
#[cfg(test)]
pub(crate) fn fit_name(text: &str) -> ambre_core::Name {
    ambre_core::Name::new(text).expect("a name within the cap")
}
#[cfg(test)]
mod space_tests;
#[cfg(test)]
mod style_tests;
#[cfg(test)]
mod thread_tests;
#[cfg(test)]
mod unread_tests;

pub use channel::{
    ChangeTarget, ChannelState, Closed, FINALIZE_RETRY_MS, FOLD_DELAY_MS, FinalSnapshot,
    FoldSnapshot, Linked, Named, Relocated, Replay, Replied, SEGMENT_BYTES, SEGMENT_MESSAGES,
    SUCCESSOR_RETRY_MS, SeekStep, Sent, Settled, Successor, ThreadLinked, ThreadTarget,
};
pub use delta::{JSON_SCHEMA_VERSION, Refusal, Signer};
pub use error::MurmureError;
pub use layout::{MessageRef, MurmurePane, Region};
pub use members::{
    PRESENCE_EXPIRY_MS, REFRESH_PERIOD_MS, Roster, find_members, is_presence_sample,
    presence_sample,
};
pub use mention::{CONFIRM_CHANNEL_PAST, Screened, members_only, screen};
pub use opening::{Opening, OpeningChannel};
pub use schema::{ChannelType, DocumentKind, SCHEMA_VERSION, SegmentState};
pub use search::Query;
pub use sequence::{Order, Position};
pub use sidebar::{Activity, Sidebar, SidebarEntry, SidebarGroup, SidebarSection};
pub use space::{Listing, SpaceState};
pub use watchers::{Sample, TICK_MS, TYPING_EXPIRY_MS, TYPING_REFRESH_MS, Watchers};
