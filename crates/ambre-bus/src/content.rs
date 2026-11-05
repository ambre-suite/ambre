//! The two traits that let an application's own command and event cross the
//! envelopes while the closed sums over them live downstream (`BUS-032`).
//!
//! An application declares its command and event in its crate. The Logic
//! runtime declares `AppCommand` and `AppEvent`, the closed sums over them,
//! because it sits downstream of every application. [`Embed`] is the seam
//! between the two: the launcher embeds a view's command into the sum, and
//! projects the view's event out of it. [`Mutation`] holds the two questions
//! the role gate and the lossy class ask of a command they do not know
//! (`BUS-007`, `BUS-023`).

use std::convert::Infallible;

/// A closed sum that holds `A` as one of its variants.
///
/// `AppCommand` embeds each application's command, and `AppEvent` each
/// application's event. The projection gives the value back unchanged when it
/// belongs to another variant.
pub trait Embed<A>: Sized {
    /// The sum's variant for `part`.
    fn embed(part: A) -> Self;
    /// The `A` this value holds.
    ///
    /// # Errors
    ///
    /// The value itself, unchanged, when it holds another variant.
    fn project(self) -> Result<A, Self>;
    /// The `A` this value holds, borrowed. An application's chrome reads its
    /// own event out of the sum the launcher relays, without a copy.
    fn peek(&self) -> Option<&A>;
}

// A value is the one variant of itself. A test rig uses the application's own
// type where the launcher uses the sum, and so does a chrome under test.
impl<A> Embed<A> for A {
    fn embed(part: A) -> Self {
        part
    }

    fn project(self) -> Result<A, Self> {
        Ok(self)
    }

    fn peek(&self) -> Option<&A> {
        Some(self)
    }
}

/// The traffic class of a content command: what a full queue may do to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Traffic {
    /// Intent the user gave once. A full queue holds it in the outbox
    /// (`BUS-011`).
    Lossless,
    /// A request over a state the screen already shows. A full queue drops
    /// it, and the next frame offers it again (`BUS-024`).
    Lossy,
}

/// The two questions the bus asks of a content command it does not know.
///
/// The Logic dispatcher gates a read-only caller on [`is_mutating`]
/// (`BUS-007`). The lossy class carries a command only when [`traffic`]
/// answers [`Traffic::Lossy`] and [`is_mutating`] answers no (`BUS-023`).
/// Ephemeral presence and local view state are not mutating: a viewer still
/// scrolls, selects, copies and broadcasts a cursor. Not mutating is not
/// lossy: a selection changes no document and still carries intent.
///
/// [`is_mutating`]: Mutation::is_mutating
/// [`traffic`]: Mutation::traffic
pub trait Mutation {
    /// Whether the command can change the document.
    fn is_mutating(&self) -> bool;

    /// What a full queue may do to the command. An application names its
    /// lossy commands one by one, and every other command is lossless.
    fn traffic(&self) -> Traffic;
}

impl Mutation for Infallible {
    fn is_mutating(&self) -> bool {
        match *self {}
    }

    fn traffic(&self) -> Traffic {
        match *self {}
    }
}
