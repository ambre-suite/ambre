// ambre-commands/src/mode.rs
// ============================================================================
// The two-way choices a command or a chord makes for its view.
//
// Each was a `bool` at one time, and a `bool` at a call site reads as `true`
// or `false`, never as the mode it picks. The shell and every view share
// these, so they sit in the leaf that both already depend on.
// ============================================================================

/// What a clipboard write does to the selection it copies.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ClipboardWrite {
    /// Copy the selection and leave it in place.
    #[default]
    Copy,
    /// Copy the selection, then delete it.
    Cut,
}

/// Which halves of the clipboard a paste reads.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PasteForm {
    /// Every half the view can read, formatting included.
    #[default]
    Rich,
    /// The plain text alone: the formatting is dropped.
    Plain,
}
