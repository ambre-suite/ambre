//! The UI-thread half of Murmure: the canvas views of the message regions,
//! their painters, and the host-side models the chrome reads
//! (`UI-THREAD-062`).
//!
//! No module here holds an engine state or a document. A view rebuilds what it
//! draws from the events of [`crate::message`], and sends a command back.

pub mod call_controls;
pub mod call_devices;
pub mod call_float;
pub mod call_ring;
pub mod call_videos;
pub mod call_volumes;
pub mod commands;
pub mod composer;
pub mod drafts;
pub mod emoji;
pub(crate) mod layout;
pub mod list;
pub(crate) mod look;
pub mod markers;
pub mod mention_list;
pub mod notices;
pub mod paint;
pub mod pictures;
pub(crate) mod read_aloud;
pub mod select;
pub(crate) mod sidebar;
#[cfg(feature = "testing")]
pub mod testing;
pub mod uploads;
pub mod view;
