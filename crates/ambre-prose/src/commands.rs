//! Prose's command registry: one `Command` per affordance the shell offers
//! (`ambre-spec-commands.md`, `UI-THREAD-019`).
//!
//! Only the commands the shell routes live here. The text chords (typing,
//! moving the caret, undo) are decoded by the view from the input pipe, and no
//! table names them.

use ambre_commands::{Command, CommandCategory, CommandId, IconId, KeyBinding, Keymap, Locale};

/// Taking the document out as a file.
const FILE: CommandCategory = CommandCategory("file");
/// Commands that belong to the application shell rather than the document.
const GENERAL: CommandCategory = CommandCategory("general");

/// Open the command palette: a UI-only intent the shell intercepts
/// (`CHROME-008`).
pub const PALETTE: CommandId = CommandId("prose.palette.open");

/// Save the document to a native `.ambre` file (`PROSE-IO-028`).
pub const SAVE_FILE: CommandId = CommandId("prose.file.save");

fn palette_title(_: &Locale) -> String {
    "Command palette".into()
}

fn save_file_title(_: &Locale) -> String {
    // The file is not sealed, and the label says so (`DOCFILE-009`).
    "Save to a file (.ambre, not encrypted)".into()
}

/// Every command Prose contributes, described once.
pub const COMMANDS: &[Command] = &[
    Command {
        id: SAVE_FILE,
        title: save_file_title,
        icon: Some(IconId("content-save")),
        keybindings: &[],
        category: FILE,
    },
    Command {
        id: PALETTE,
        title: palette_title,
        icon: None,
        keybindings: &[KeyBinding::primary('k')],
        category: GENERAL,
    },
];

/// Bind the chords the shell routes for Prose.
pub fn bind_keys(keymap: &mut Keymap) {
    if let Some(chord) = ambre_commands::find(COMMANDS, PALETTE).and_then(Command::chord) {
        keymap.bind(chord, PALETTE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PROSE-IO-028`: the save entry is declared, so the palette lists it.
    #[test]
    fn the_save_entry_is_declared_once() {
        let saves = COMMANDS.iter().filter(|c| c.id == SAVE_FILE).count();
        assert_eq!(saves, 1);
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        assert_eq!(keymap.resolve(KeyBinding::primary('k')), Some(PALETTE));
    }
}
