//! Prose's command registry: one `Command` per affordance the shell offers
//! (`ambre-spec-commands.md`, `UI-THREAD-019`).
//!
//! Only the commands the shell routes live here. The text chords (typing,
//! moving the caret) are decoded by the view from the input pipe, and no
//! table names them. Undo and redo are described here for their menu rows,
//! and the view decodes their chords too: the shell binds none of them.

use ambre_commands::{Command, KeyBinding, KeyChar, Keymap, Locale};
use ambre_core::{Categories, Commands, Icons, StaticKey};

use crate::mark::MarkRoot;

/// Taking the document out as a file.
const FILE: StaticKey<Categories> = StaticKey::<Categories>::new("file");
/// Inline formatting: the format bar and its chords.
const FORMAT: StaticKey<Categories> = StaticKey::<Categories>::new("format");
/// Moving text through the system clipboard.
const CLIPBOARD: StaticKey<Categories> = StaticKey::<Categories>::new("clipboard");
/// Commands that belong to the application shell rather than the document.
const GENERAL: StaticKey<Categories> = StaticKey::<Categories>::new("general");
/// Stepping through the local history.
const HISTORY: StaticKey<Categories> = StaticKey::<Categories>::new("history");

/// Undo this peer's last change (`UNDO-001`).
pub const UNDO: StaticKey<Commands> = StaticKey::<Commands>::new("prose.history.undo");
/// Redo the last change undone.
pub const REDO: StaticKey<Commands> = StaticKey::<Commands>::new("prose.history.redo");

/// Open the command palette: a UI-only intent the shell intercepts
/// (`CHROME-008`).
pub const PALETTE: StaticKey<Commands> = StaticKey::<Commands>::new("prose.palette.open");

/// Save the document to a native `.ambre` file (`PROSE-IO-028`).
pub const SAVE_FILE: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.save");

/// Bring a Markdown file in after the last block (`PROSE-IO-027`).
pub const IMPORT_MARKDOWN: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.file.import_markdown");

/// Take the document out as GFM Markdown.
pub const EXPORT_MARKDOWN: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.file.export_markdown");

/// Bring a Word file in after the last block, through the Office module
/// (`PROSE-IO-060`).
pub const IMPORT_WORD: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.import_word");

/// Take the document out as a Word package, through the Office module, with
/// no comment and no suggestion (`PROSE-IO-014`, `PROSE-IO-081`).
pub const EXPORT_WORD: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.export_word");

/// Take the document out as a Word package with its comments and its
/// suggestions: publishing the review is an act of its own (`PROSE-IO-081`).
pub const EXPORT_WORD_REVIEWED: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.file.export_word_reviewed");

/// Bring an OpenDocument text in after the last block, through the Office
/// module, as the same text from Word would land (`PROSE-IO-088`).
pub const IMPORT_ODT: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.import_odt");

/// Take the document out as an OpenDocument text, with no comment and no
/// suggestion (`PROSE-IO-089`, `PROSE-IO-081`).
pub const EXPORT_ODT: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.export_odt");

/// Take the document out as an OpenDocument text with its comments and its
/// suggestions (`PROSE-IO-081`).
pub const EXPORT_ODT_REVIEWED: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.file.export_odt_reviewed");

/// The clipboard intents carry the shared identities: a cut is one command in
/// every application (`UI-THREAD-011`).
pub const CUT: StaticKey<Commands> = ambre_commands::shared::CUT;
pub const COPY: StaticKey<Commands> = ambre_commands::shared::COPY;
pub const PASTE: StaticKey<Commands> = ambre_commands::shared::PASTE;
/// Paste the text alone, every richer format dropped (`PROSE-UI-227`).
pub const PASTE_PLAIN: StaticKey<Commands> = ambre_commands::shared::PASTE_PLAIN;

/// The entries of the page's context menu, in the order it shows them. The
/// history group comes first, then the clipboard group.
pub const CONTEXT_MENU: [StaticKey<Commands>; 6] = [UNDO, REDO, CUT, COPY, PASTE, PASTE_PLAIN];

/// The five controls of the format bar. A control and its chord invoke the
/// same id, and the id resolves to one mark key (`PROSE-UI-056`).
pub const BOLD: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.bold");
pub const ITALIC: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.italic");
pub const UNDERLINE: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.underline");
pub const STRIKETHROUGH: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.format.strikethrough");
pub const CODE: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.code");

/// Each control of the format bar, in the order the bar shows them, with the
/// mark key it applies or removes.
pub const FORMAT_BAR: [(StaticKey<Commands>, MarkRoot); 5] = [
    (BOLD, MarkRoot::Bold),
    (ITALIC, MarkRoot::Italic),
    (UNDERLINE, MarkRoot::Underline),
    (STRIKETHROUGH, MarkRoot::Strike),
    (CODE, MarkRoot::Code),
];

/// The mark key a formatting command applies, or `None` for any other id.
#[must_use]
pub fn mark_key(id: StaticKey<Commands>) -> Option<MarkRoot> {
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

fn undo_title(_: &Locale) -> String {
    "Undo".into()
}

fn redo_title(_: &Locale) -> String {
    "Redo".into()
}

fn palette_title(_: &Locale) -> String {
    "Command palette".into()
}

fn import_markdown_title(_: &Locale) -> String {
    "Import a Markdown file".into()
}

fn export_markdown_title(_: &Locale) -> String {
    "Export to Markdown (.md)".into()
}

fn import_word_title(_: &Locale) -> String {
    "Import a Word file".into()
}

fn export_word_title(_: &Locale) -> String {
    "Export to Word (.docx)".into()
}

fn export_word_reviewed_title(_: &Locale) -> String {
    "Export to Word with comments and suggestions (.docx)".into()
}

fn import_odt_title(_: &Locale) -> String {
    "Import an OpenDocument text".into()
}

fn export_odt_title(_: &Locale) -> String {
    "Export to OpenDocument (.odt)".into()
}

fn export_odt_reviewed_title(_: &Locale) -> String {
    "Export to OpenDocument with comments and suggestions (.odt)".into()
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
        icon: Some(StaticKey::<Icons>::new("format-bold")),
        keybindings: &[KeyBinding::primary(KeyChar::B)],
        category: FORMAT,
    },
    Command {
        id: ITALIC,
        title: italic_title,
        icon: Some(StaticKey::<Icons>::new("format-italic")),
        keybindings: &[KeyBinding::primary(KeyChar::I)],
        category: FORMAT,
    },
    Command {
        id: UNDERLINE,
        title: underline_title,
        icon: Some(StaticKey::<Icons>::new("format-underline")),
        keybindings: &[KeyBinding::primary(KeyChar::U)],
        category: FORMAT,
    },
    Command {
        id: STRIKETHROUGH,
        title: strikethrough_title,
        icon: Some(StaticKey::<Icons>::new("format-strikethrough")),
        keybindings: &[KeyBinding::primary_shift(KeyChar::X)],
        category: FORMAT,
    },
    Command {
        id: CODE,
        title: code_title,
        icon: Some(StaticKey::<Icons>::new("code-tags")),
        keybindings: &[KeyBinding::primary(KeyChar::E)],
        category: FORMAT,
    },
    Command {
        id: SAVE_FILE,
        title: save_file_title,
        icon: Some(StaticKey::<Icons>::new("content-save")),
        keybindings: &[],
        category: FILE,
    },
    Command {
        id: IMPORT_MARKDOWN,
        title: import_markdown_title,
        icon: Some(StaticKey::<Icons>::new("file-import-outline")),
        keybindings: &[],
        category: FILE,
    },
    Command {
        id: EXPORT_MARKDOWN,
        title: export_markdown_title,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
    },
    Command {
        id: IMPORT_WORD,
        title: import_word_title,
        icon: Some(StaticKey::<Icons>::new("file-import-outline")),
        keybindings: &[],
        category: FILE,
    },
    Command {
        id: EXPORT_WORD,
        title: export_word_title,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
    },
    Command {
        id: EXPORT_WORD_REVIEWED,
        title: export_word_reviewed_title,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
    },
    Command {
        id: IMPORT_ODT,
        title: import_odt_title,
        icon: Some(StaticKey::<Icons>::new("file-import-outline")),
        keybindings: &[],
        category: FILE,
    },
    Command {
        id: EXPORT_ODT,
        title: export_odt_title,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
    },
    Command {
        id: EXPORT_ODT_REVIEWED,
        title: export_odt_reviewed_title,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
    },
    Command {
        id: CUT,
        title: cut_title,
        icon: Some(StaticKey::<Icons>::new("content-cut")),
        keybindings: ambre_commands::shared::CUT_CHORDS,
        category: CLIPBOARD,
    },
    Command {
        id: COPY,
        title: copy_title,
        icon: Some(StaticKey::<Icons>::new("content-copy")),
        keybindings: ambre_commands::shared::COPY_CHORDS,
        category: CLIPBOARD,
    },
    Command {
        id: PASTE,
        title: paste_title,
        icon: Some(StaticKey::<Icons>::new("content-paste")),
        keybindings: ambre_commands::shared::PASTE_CHORDS,
        category: CLIPBOARD,
    },
    Command {
        id: PASTE_PLAIN,
        title: paste_plain_title,
        icon: Some(StaticKey::<Icons>::new("content-paste")),
        keybindings: ambre_commands::shared::PASTE_PLAIN_CHORDS,
        category: CLIPBOARD,
    },
    Command {
        id: UNDO,
        title: undo_title,
        icon: Some(StaticKey::<Icons>::new("undo")),
        keybindings: ambre_commands::shared::UNDO_CHORDS,
        category: HISTORY,
    },
    Command {
        id: REDO,
        title: redo_title,
        icon: Some(StaticKey::<Icons>::new("redo")),
        keybindings: ambre_commands::shared::REDO_CHORDS,
        category: HISTORY,
    },
    Command {
        id: PALETTE,
        title: palette_title,
        icon: None,
        keybindings: &[KeyBinding::primary(KeyChar::K)],
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

    /// Each command is described once, with a title in every locale, a
    /// group, no blank icon and no chord another command holds
    /// (`UI-THREAD-019`, `UI-THREAD-037`).
    #[test]
    fn the_table_has_no_defect() {
        assert_eq!(ambre_commands::table_defects(COMMANDS), []);
    }

    /// `PROSE-IO-028`, `PROSE-IO-027`: the save, import and export entries
    /// are declared once each, so the palette lists them.
    #[test]
    fn the_save_entry_is_declared_once() {
        for id in [
            SAVE_FILE,
            IMPORT_MARKDOWN,
            EXPORT_MARKDOWN,
            IMPORT_WORD,
            EXPORT_WORD,
            EXPORT_WORD_REVIEWED,
            IMPORT_ODT,
            EXPORT_ODT,
            EXPORT_ODT_REVIEWED,
        ] {
            let declared = COMMANDS.iter().filter(|c| c.id == id).count();
            assert_eq!(declared, 1, "{id:?}");
        }
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        assert_eq!(
            keymap.resolve(KeyBinding::primary(KeyChar::K)),
            Some(PALETTE)
        );
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
    }
}
