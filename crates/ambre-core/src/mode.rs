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
