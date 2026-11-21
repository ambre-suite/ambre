//! Two-way choices that more than one crate makes, each once a `bool`.
//!
//! A `bool` at a call site reads as `true` or `false`, never as the mode it
//! picks (`ai/ambre-rust-style.md`). A choice that crosses the bus sits here,
//! because the bus and every application reach this crate.
//!
//! **Wire form.** postcard writes a fieldless enum as its variant index, a
//! varint, as it writes a `bool`. So variant 0 of a type the bus carries is the
//! value that was `false`. A frame from before the change reads the same.

use serde::{Deserialize, Serialize};

/// Which way one step walks an ordered set. The set is the matches of a find,
/// the cells of a table, the text of a flow, or the entries of a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StepDirection {
    /// To the next item, in reading order, or down.
    #[default]
    Forward,
    /// To the previous item, in reading order, or up.
    Backward,
}

/// Which stack of the edit history one step walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum HistoryStep {
    /// Take back the last change.
    #[default]
    Undo,
    /// Apply again the last change an undo took back.
    Redo,
}

/// Whether a space lists a channel in its catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Visibility {
    /// Listed nowhere: only its members know of it.
    #[default]
    Private,
    /// Listed in the catalogue of its space, which any member may join.
    Public,
}

impl Visibility {
    /// [`Visibility::Public`] when a store row says `public`.
    #[must_use]
    pub fn public_if(public: bool) -> Self {
        if public {
            Visibility::Public
        } else {
            Visibility::Private
        }
    }
}

/// Whether a created document is a Murmure space, which its members list,
/// or a document of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DocumentScope {
    /// A document of its own, or a channel document.
    #[default]
    Personal,
    /// A Murmure space, which its members list.
    Space,
}

/// How a failed attempt to finalize a segment leaves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FinalizeEnd {
    /// The attempt failed, and this copy asks again later.
    #[default]
    Failed,
    /// This copy holds an edit the relay refused, so it never finalizes the
    /// segment: another member does.
    Withdrawn,
}

/// What a member does with a segment once the relay dropped one of its
/// snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AfterDrop {
    /// Nothing: the snapshot was not the final one, or this copy sealed it.
    #[default]
    Keep,
    /// Finalize the segment again: the final snapshot was dropped.
    Refinalize,
}

/// Whether a search tells upper case from lower case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CaseMatch {
    /// Both sides are lowercased before they are compared.
    #[default]
    Insensitive,
    Sensitive,
}

/// Whether a text match may sit inside a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WordMatch {
    /// The query may sit anywhere in the text.
    #[default]
    Anywhere,
    /// A match must not touch a letter, a digit or an underscore on either
    /// side.
    WholeWord,
}

/// Whether a document's link to the relay is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LinkState {
    #[default]
    Disconnected,
    Connected,
}

impl LinkState {
    /// [`LinkState::Connected`] when the socket answers that it is open.
    #[must_use]
    pub fn connected_if(open: bool) -> Self {
        if open {
            LinkState::Connected
        } else {
            LinkState::Disconnected
        }
    }

    #[must_use]
    pub fn is_connected(self) -> bool {
        self == LinkState::Connected
    }
}

/// Which halves of the clipboard a paste reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PasteForm {
    /// Every half the view can read, formatting included.
    #[default]
    Rich,
    /// The plain text alone: the formatting is dropped.
    Plain,
}

/// Where a comment thread stands: open, or resolved. Resolving destroys
/// neither the thread nor its messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ThreadState {
    #[default]
    Open,
    Resolved,
}

impl ThreadState {
    /// [`ThreadState::Resolved`] when the stored flag holds, `Open` otherwise.
    #[must_use]
    pub fn resolved_if(resolved: bool) -> Self {
        if resolved {
            ThreadState::Resolved
        } else {
            ThreadState::Open
        }
    }

    /// The flag as a document stores it.
    #[must_use]
    pub fn is_resolved(self) -> bool {
        self == ThreadState::Resolved
    }
}

crate::define_flag! {
    /// Whether a toggle command is on, as the Logic thread reports it.
    pub enum Toggle {
        /// The command's effect is off.
        Off,
        /// The command's effect is on.
        On,
    }

    /// Whether a command can run now, as the Logic thread reports it. The
    /// default is `Disabled`, the value that was `false`: a store that must
    /// start a command as actionable writes `Enabled`.
    pub enum Availability {
        /// The command cannot run now.
        Disabled,
        /// The command can run now.
        Enabled,
    }

    /// Whether a provider proved an account's address, or the account only
    /// asserted it at setup. An asserted address may be a squat. Whoever
    /// claimed it first receives what is shared with it. So a share shows it
    /// unproven before it wraps a key (`CHROME-138`). Its serde form is the
    /// `email_verified` bool of the relay's answers.
    pub enum EmailProof {
        /// Self-asserted: a provider sign-in may reclaim it.
        Asserted,
        /// Proven by a provider: it wins any collision.
        Verified,
    }

    /// Whether an account still holds the address it was set up under, or a
    /// provider sign-in reclaimed it (`SRV-029`). The relay's account answer
    /// carries it, so the launcher names the reclaim (`CHROME-146`). Its serde
    /// form is the `address_reclaimed` bool of that answer.
    pub enum AddressStanding {
        /// The account holds its address.
        Held,
        /// A provider-verified account took the address. Shares to it reach
        /// that account now.
        Reclaimed,
    }

    /// Whether a request made the record it answers, or found one an earlier
    /// request made. A race that lost answers `Existing`, with the winner's
    /// record.
    pub enum Creation {
        /// An earlier request made the record.
        Existing,
        /// This request made the record.
        Created,
    }

    /// Whether a document owes a key rotation: a member left and nobody
    /// re-keyed since (`SRV-122`, `SYNC-128`).
    pub enum RotationDebt {
        /// No rotation is owed.
        Settled,
        /// A writer pays a rotation before its next frame.
        Owed,
    }
}
