//! Murmure, the messaging application of Ambre: the engine core
//! (`specs/5-Murmure/features/ambre-spec-murmure-engine.md`).
//!
//! The Logic thread holds one [`ChannelState`] per open channel, and one
//! [`SpaceState`] per open space. A channel is a
//! set of Loro documents, never one (`MURMURE-021`). The engine reads no clock
//! and no key: the view stamps the time, and the Sync worker seals and routes
//! the deltas this crate exports.
//!
//! The commands and events the engine answers live in [`message`], as pure
//! data, and travel through the [`message::Murmure`] channel plane (`BUS-027`,
//! `BUS-032`).
//!
//! Each module sits under the path of the thread it serves, so a gate reads
//! the thread off the path (`UI-THREAD-062`):
//!
//! | Path | Thread |
//! | :--- | :--- |
//! | [`logic`] | the Logic thread: the engine states and their documents |
//! | [`ui`] | the UI thread: the views, their painters and their host-side models |
//! | `chrome` | the UI thread: the page elements, on wasm alone |
//! | [`message`] | both: the vocabulary the two threads exchange |

#![forbid(unsafe_code)]

pub mod logic;
pub mod message;
pub mod ui;

/// Murmure-specific chrome, built on the `ambre-chrome` toolkit. Wasm-only
/// (leptos CSR + DOM).
#[cfg(target_arch = "wasm32")]
pub mod chrome;

pub use logic::channel::{
    ChangeTarget, ChannelState, Closed, FINALIZE_RETRY, FOLD_DELAY, FinalSnapshot, FoldSnapshot,
    Linked, LinkedOpening, Named, Relocated, Replay, Replied, RootProof, SEGMENT_MESSAGES,
    SEGMENT_SNAPSHOT_SIZE, SUCCESSOR_RETRY, SeekStep, Sent, Settled, Successor, SweepStep, Swept,
    ThreadLinked, ThreadTarget,
};
pub use logic::delta::{JSON_SCHEMA_VERSION, JsonUpdate, Refusal, Signer};
pub use logic::error::MurmureError;
pub use logic::members::{
    PRESENCE_EXPIRY, REFRESH_PERIOD, Roster, find_members, is_presence_sample, presence_sample,
};
pub use logic::mention::{CONFIRM_CHANNEL_PAST, Screened, members_only, screen};
pub use logic::opening::{Opening, OpeningChannel, Receipt};
pub use logic::schema::{ChannelType, DocumentKind, SCHEMA_VERSION, SegmentState};
pub use logic::search::Query;
pub use logic::sequence::{Order, Position};
pub use logic::space::{Listing, SpaceState};
pub use logic::watchers::{Sample, TICK_PERIOD, TYPING_EXPIRY, TYPING_REFRESH, Watchers};
pub use ui::layout::{MessageRef, MurmurePane, Region};
pub use ui::sidebar::{
    Activity, DrawnSection, Folding, Folds, HiddenMark, Kept, LiveCall, LiveCalls, PhoneTab,
    Sidebar, SidebarEntry, SidebarGroup, SidebarSection, SpaceHead, SpaceRanks, fold,
};
