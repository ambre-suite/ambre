//! The Logic-thread half of Murmure: the engine states, the documents they
//! hold, and the checks each delta passes (`UI-THREAD-062`).
//!
//! No module here runs on the UI thread. A view reads the engine through the
//! events of [`crate::message`] alone, and may name a plain value of a module
//! here, never a state.

pub(crate) mod attachment;
pub mod call;
pub(crate) mod channel;
pub(crate) mod delta;
pub(crate) mod error;
pub(crate) mod members;
pub(crate) mod mention;
pub(crate) mod opening;
pub(crate) mod reaction;
pub mod schema;
pub(crate) mod search;
pub(crate) mod sequence;
pub(crate) mod session;
pub(crate) mod space;
pub(crate) mod style;
pub(crate) mod watchers;

#[cfg(test)]
mod amending_tests;
#[cfg(test)]
mod attachment_tests;
#[cfg(test)]
mod authorship_tests;
#[cfg(test)]
mod call_tests;
#[cfg(test)]
mod channel_tests;
#[cfg(test)]
mod closing_tests;
// Host only: proptest does not build on wasm32.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod convergence_tests;
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
#[cfg(test)]
mod space_tests;
#[cfg(test)]
mod style_tests;
#[cfg(test)]
mod thread_tests;
#[cfg(test)]
mod unread_tests;

/// The name `text` is, for a test that names a channel, a space or a
/// category within the cap.
#[cfg(test)]
pub(crate) fn fit_name(text: &str) -> ambre_core::Validated<ambre_core::Name> {
    ambre_core::Validated::<ambre_core::Name>::parse(text).expect("a name within the cap")
}
