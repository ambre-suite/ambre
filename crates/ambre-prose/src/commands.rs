//! Prose's command registry: one `Command` per affordance the shell offers
//! (`ambre-spec-commands.md`, `UI-THREAD-019`).
//!
//! Only the commands the shell routes live here. The text chords (typing,
//! moving the caret, undo) are decoded by the view from the input pipe, and no
//! table names them.

use ambre_commands::{Command, CommandCategory, CommandId, IconId, KeyBinding, Keymap, Locale};

/// Taking the document out as a file.
const FILE: CommandCategory = CommandCategory("file");
/// Inline formatting: the format bar and its chords.
const FORMAT: CommandCategory = CommandCategory("format");
/// Commands that belong to the application shell rather than the document.
const GENERAL: CommandCategory = CommandCategory("general");

/// Open the command palette: a UI-only intent the shell intercepts
/// (`CHROME-008`).
pub const PALETTE: CommandId = CommandId("prose.palette.open");

/// Save the document to a native `.ambre` file (`PROSE-IO-028`).
pub const SAVE_FILE: CommandId = CommandId("prose.file.save");

/// The five controls of the format bar. A control and its chord invoke the
/// same id, and the id resolves to one mark key (`PROSE-UI-056`).
pub const BOLD: CommandId = CommandId("prose.format.bold");
pub const ITALIC: CommandId = CommandId("prose.format.italic");
pub const UNDERLINE: CommandId = CommandId("prose.format.underline");
pub const STRIKETHROUGH: CommandId = CommandId("prose.format.strikethrough");
pub const CODE: CommandId = CommandId("prose.format.code");

/// Each control of the format bar, in the order the bar shows them, with the
/// mark key it applies or removes.
pub const FORMAT_BAR: [(CommandId, &str); 5] = [
    (BOLD, "b"),
    (ITALIC, "i"),
    (UNDERLINE, "u"),
    (STRIKETHROUGH, "s"),
    (CODE, "c"),
];

/// The mark key a formatting command applies, or `None` for any other id.
#[must_use]
pub fn mark_key(id: CommandId) -> Option<&'static str> {
    FORMAT_BAR
        .iter()
        .find(|(bar, _)| *bar == id)
        .map(|(_, key)| *key)
}

fn bold_title(_: &Locale) -> String {
    "Bold".into()
}

fn italic_title(_: &Locale) -> String {
    "Italic".into()
}

fn underline_title(_: &Locale) -> String {
    "Underline".into()
}

fn strikethrough_title(_: &Locale) -> String {
    "Strikethrough".into()
}

fn code_title(_: &Locale) -> String {
    "Inline code".into()
}

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
        id: BOLD,
        title: bold_title,
        icon: Some(IconId("format-bold")),
        keybindings: &[KeyBinding::primary('b')],
        category: FORMAT,
    },
    Command {
        id: ITALIC,
        title: italic_title,
        icon: Some(IconId("format-italic")),
        keybindings: &[KeyBinding::primary('i')],
        category: FORMAT,
    },
    Command {
        id: UNDERLINE,
        title: underline_title,
        icon: Some(IconId("format-underline")),
        keybindings: &[KeyBinding::primary('u')],
        category: FORMAT,
    },
    Command {
        id: STRIKETHROUGH,
        title: strikethrough_title,
        icon: Some(IconId("format-strikethrough")),
        keybindings: &[KeyBinding::primary_shift('x')],
        category: FORMAT,
    },
    Command {
        id: CODE,
        title: code_title,
        icon: Some(IconId("code-tags")),
        keybindings: &[KeyBinding::primary('e')],
        category: FORMAT,
    },
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
    for id in FORMAT_BAR.map(|(id, _)| id).into_iter().chain([PALETTE]) {
        if let Some(chord) = ambre_commands::find(COMMANDS, id).and_then(Command::chord) {
            keymap.bind(chord, id);
        }
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

    /// `PROSE-UI-056`: each control's chord resolves to the control's own
    /// id, and the id to one mark key.
    #[test]
    fn a_control_and_its_chord_resolve_to_one_command_id() {
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        for (id, key) in FORMAT_BAR {
            let command = ambre_commands::find(COMMANDS, id).expect("the control is declared");
            let chord = command.chord().expect("the control has a chord");
            assert_eq!(keymap.resolve(chord), Some(id));
            assert_eq!(mark_key(id), Some(key));
        }
        assert_eq!(mark_key(PALETTE), None);
    }
}
