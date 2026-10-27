// ambre-commands/src/mode.rs
// ============================================================================
// The two-way choices a command or a chord makes for its view.
//
// Each was a `bool` at one time, and a `bool` at a call site reads as `true`
// or `false`, never as the mode it picks. The shell and every view share
// these, so they sit in the leaf that both already depend on.
// ============================================================================

use crate::Modifiers;

/// What a clipboard write does to the selection it copies.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ClipboardWrite {
    /// Copy the selection and leave it in place.
    #[default]
    Copy,
    /// Copy the selection, then delete it.
    Cut,
}

/// Which halves of the clipboard a paste reads. The bus carries it on the
/// blob a paste hands its view, so it lives in `ambre-core`.
pub use ambre_core::PasteForm;

/// What a caret move does to the selection anchor.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SelectionMode {
    /// The anchor follows the caret, so the selection collapses.
    #[default]
    Move,
    /// The anchor stays, so the selection grows or shrinks.
    Extend,
}

impl SelectionMode {
    /// The mode a chord picks: Shift extends the selection.
    #[must_use]
    pub const fn from_mods(mods: Modifiers) -> Self {
        if mods.shift() {
            Self::Extend
        } else {
            Self::Move
        }
    }
}
