// ambre-chrome/src/handler.rs
// ============================================================================
// What an application hands the shell for its commands (`ARCH-018`).
//
// The shell resolves a click, a menu entry or a chord to a `CommandId`. What
// that id means belongs to the application that declared it. So the shell asks
// the application's handler, and performs only what the answer names. That is
// a command for the routed document, a shell overlay, the clipboard or a file
// picker. The handler opens its own panels itself, through its own
// signals.
//
// The command type stays generic: this crate depends on no bus crate
// (`SCHEMA-002`), so the launcher fixes it to the bus's application command.
// ============================================================================

use ambre_commands::CommandId;

/// One thing a command asks of the shell, once its application has decided.
pub enum ShellAction<C> {
    /// Send this command to the routed document. Dropped when no document is
    /// open, since nothing could receive it.
    Send(C),
    /// Open the command palette, the shell's overlay.
    OpenPalette,
    /// Open the passphrase prompt of a sealed `.ambre` file. The prompt is the
    /// shell's, and the handler's `sealed_save` builds the command it ends in
    /// (`DOCFILE-015`).
    OpenSealedSave,
    /// Copy, or cut, the current selection through the browser.
    Copy {
        /// Cut rather than copy.
        cut: bool,
    },
    /// Paste the system clipboard into the routed document.
    Paste {
        /// Drop the formatting, keep the text.
        plain: bool,
    },
    /// Ask the server for the documents this user may open again. A chooser
    /// that offers them must not trust an older list.
    RefreshDocuments,
    /// Let the user pick a file, and send the command its bytes become.
    PickFile(FilePick<C>),
}

/// A file picker the shell opens on an application's behalf.
pub struct FilePick<C> {
    /// The picker's `accept` attribute: what the application reads.
    pub accept: &'static str,
    /// What the user reads when the browser could not read the file.
    pub unreadable: &'static str,
    /// The command the file's bytes become.
    pub read: Box<dyn Fn(Vec<u8>) -> C>,
}

/// What a chord resolved in the keymap does.
pub enum KeyRoute<C> {
    /// Leave the key to the focused field: its default stays.
    Yield,
    /// Prevent the key's default, then perform these, possibly none. An empty
    /// list leaves the key to the canvas listener alone.
    Run(Vec<ShellAction<C>>),
}

/// An application's half of the command path (`ARCH-018`). The shell routes
/// every id the application declared here, and never matches on what one
/// means.
pub trait CommandHandler {
    /// The command an application sends its Logic half.
    type Command;

    /// What the command `id` asks of the shell. The handler opens its own
    /// panels before it returns.
    fn on_command(&self, id: CommandId) -> Vec<ShellAction<Self::Command>>;

    /// What a chord resolving to `id` does. `in_text_field` is whether the key
    /// went to a text field, which keeps a chord the field needs.
    fn on_key(&self, id: CommandId, in_text_field: bool) -> KeyRoute<Self::Command>;

    /// The command a sealed save ends in, once the prompt holds a passphrase.
    /// `None` for an application with no sealed file.
    fn sealed_save(&self, passphrase: String) -> Option<Self::Command>;
}
