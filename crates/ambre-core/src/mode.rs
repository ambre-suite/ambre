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
