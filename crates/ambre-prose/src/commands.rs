//! Prose's command registry: one `Command` per affordance the shell offers
//! (`ambre-spec-commands.md`, `UI-THREAD-019`).
//!
//! Only the commands the shell routes live here. The text chords (typing,
//! moving the caret, undo) are decoded by the view from the input pipe, and no
//! table names them.

pub use ambre_commands::Locale;
use ambre_commands::{Command, CommandCategory, CommandId, IconId, KeyBinding, Keymap};

/// Taking the document out as a file.
const FILE: CommandCategory = CommandCategory("file");
/// Inline formatting: the format bar and its chords.
const FORMAT: CommandCategory = CommandCategory("format");
/// Moving text through the system clipboard.
const CLIPBOARD: CommandCategory = CommandCategory("clipboard");
/// Commands that belong to the application shell rather than the document.
const GENERAL: CommandCategory = CommandCategory("general");

/// Open the command palette: a UI-only intent the shell intercepts
/// (`CHROME-008`).
pub const PALETTE: CommandId = CommandId("prose.palette.open");

/// Save the document to a native `.ambre` file (`PROSE-IO-028`).
pub const SAVE_FILE: CommandId = CommandId("prose.file.save");

/// The clipboard intents carry the shared identities: a cut is one command in
/// every application (`UI-THREAD-011`).
pub const CUT: CommandId = ambre_commands::shared::CUT;
pub const COPY: CommandId = ambre_commands::shared::COPY;
pub const PASTE: CommandId = ambre_commands::shared::PASTE;
/// Paste the text alone, every richer format dropped (`PROSE-UI-227`).
pub const PASTE_PLAIN: CommandId = ambre_commands::shared::PASTE_PLAIN;

/// The entries of the page's context menu, in the order it shows them.
pub const CONTEXT_MENU: [CommandId; 4] = [CUT, COPY, PASTE, PASTE_PLAIN];

/// Each entry of the context menu with its title, in menu order.
pub fn context_menu(locale: &Locale) -> Vec<(CommandId, String)> {
    CONTEXT_MENU
        .into_iter()
        .filter_map(|id| ambre_commands::find(COMMANDS, id))
        .map(|c| (c.id, (c.title)(locale)))
        .collect()
}

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

fn cut_title(_: &Locale) -> String {
    "Cut".into()
}

fn copy_title(_: &Locale) -> String {
    "Copy".into()
}

fn paste_title(_: &Locale) -> String {
    "Paste".into()
}

fn paste_plain_title(_: &Locale) -> String {
    "Paste as plain text".into()
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
        id: CUT,
        title: cut_title,
        icon: Some(IconId("content-cut")),
        keybindings: ambre_commands::shared::CUT_CHORDS,
        category: CLIPBOARD,
    },
    Command {
        id: COPY,
        title: copy_title,
        icon: Some(IconId("content-copy")),
        keybindings: ambre_commands::shared::COPY_CHORDS,
        category: CLIPBOARD,
    },
    Command {
        id: PASTE,
        title: paste_title,
        icon: Some(IconId("content-paste")),
        keybindings: ambre_commands::shared::PASTE_CHORDS,
        category: CLIPBOARD,
    },
    Command {
        id: PASTE_PLAIN,
        title: paste_plain_title,
        icon: Some(IconId("content-paste")),
        keybindings: ambre_commands::shared::PASTE_PLAIN_CHORDS,
        category: CLIPBOARD,
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

    /// `PROSE-UI-227`: each entry of the context menu is declared, and the
    /// keymap leaves the clipboard chords to the browser's own events.
    #[test]
    fn the_context_menu_entries_are_declared_and_left_unbound() {
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        for id in CONTEXT_MENU {
            let command = ambre_commands::find(COMMANDS, id).expect("the entry is declared");
            for chord in command.keybindings {
                assert_eq!(keymap.resolve(*chord), None);
            }
        }
        assert_eq!(context_menu(&Locale::En).len(), CONTEXT_MENU.len());
    }
}
