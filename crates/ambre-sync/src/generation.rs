//! The families of counters that tell a late answer from the current one.
//!
//! A timer and a request both carry the number they were armed or asked
//! under. The answer is live only while that number is the latest one. Each
//! kind of timer counts on its own axis, so each family is its own marker. A
//! replay ticket compared to a resend ticket does not compile.

use ambre_core::GenerationKind;

/// The ticket of a replay timer, armed when the relay stopped a replay
/// (`SYNC-153`).
pub(crate) enum ReplayTicket {}

impl GenerationKind for ReplayTicket {
    type IssueRight = ();
    const NAME: &'static str = "ReplayTicket";
}

/// The ticket of a resend timer, armed when the relay failed to sequence a
/// delta (`SYNC-159`).
pub(crate) enum ResendTicket {}

impl GenerationKind for ResendTicket {
    type IssueRight = ();
    const NAME: &'static str = "ResendTicket";
}

/// A listing asked of the relay. Two listings can cross on the way, and only
/// the answer to the latest one counts (`SYNC-037`).
pub(crate) enum Listing {}

impl GenerationKind for Listing {
    type IssueRight = ();
    const NAME: &'static str = "Listing";
}
