//! Murmure's command registry: one `Command` per shortcut the shell routes
//! (the Murmure keyboard spec, `UI-THREAD-019`).
//!
//! The chords that act inside a conversation, such as the up arrow of an empty
//! composer, land with the message list. The find chord opens the search of
//! Murmure, as it opens the find of the other applications.

use ambre_commands::{
    Command, CommandCategory, CommandId, Key, KeyBinding, Keymap, Locale, Modifiers,
};

/// Commands that belong to the application shell rather than a channel.
const GENERAL: CommandCategory = CommandCategory("general");
/// Moving from one channel to another.
const NAVIGATE: CommandCategory = CommandCategory("navigate");
/// The read state of the channels.
const UNREAD: CommandCategory = CommandCategory("unread");
/// Finding a message by its words.
const SEARCH_CATEGORY: CommandCategory = CommandCategory("search");

/// Open the command palette, channels first: a UI-only intent the shell
/// intercepts (`CHROME-008`).
pub const PALETTE: CommandId = CommandId("murmure.palette.open");
/// Open the next channel with activity, in sidebar order.
pub const NEXT_ACTIVITY: CommandId = CommandId("murmure.navigate.next_activity");
/// Open the previous channel with activity, in sidebar order.
pub const PREVIOUS_ACTIVITY: CommandId = CommandId("murmure.navigate.previous_activity");
/// Mark every channel of the user read (the Murmure unread spec).
pub const MARK_ALL_READ: CommandId = CommandId("murmure.unread.mark_all_read");
/// Open the search over the loaded segments, on the find chord of every
/// application (the Murmure search spec).
pub const SEARCH: CommandId = CommandId("murmure.search.open");

/// Alt and Shift together, the modifiers of the jumps.
const ALT_SHIFT: Modifiers = Modifiers {
    primary: false,
    alt: true,
    shift: true,
};

fn palette_title(_: &Locale) -> String {
    "Command palette".into()
}

fn next_activity_title(_: &Locale) -> String {
    "Next channel with activity".into()
}

fn previous_activity_title(_: &Locale) -> String {
    "Previous channel with activity".into()
}

fn mark_all_read_title(_: &Locale) -> String {
    "Mark everything read".into()
}

fn search_title(_: &Locale) -> String {
    "Search messages".into()
}

/// Every command Murmure contributes, described once.
pub const COMMANDS: &[Command] = &[
    Command {
        id: PALETTE,
        title: palette_title,
        icon: None,
        keybindings: &[KeyBinding::primary('k')],
        category: GENERAL,
    },
    Command {
        id: NEXT_ACTIVITY,
        title: next_activity_title,
        icon: None,
        keybindings: &[KeyBinding::new(ALT_SHIFT, Key::ArrowDown)],
        category: NAVIGATE,
    },
    Command {
        id: PREVIOUS_ACTIVITY,
        title: previous_activity_title,
        icon: None,
        keybindings: &[KeyBinding::new(ALT_SHIFT, Key::ArrowUp)],
        category: NAVIGATE,
    },
    Command {
        id: MARK_ALL_READ,
        title: mark_all_read_title,
        icon: None,
        // Not Shift+Escape: Chrome opens its task manager on it, and the
        // page never sees the key.
        keybindings: &[KeyBinding::new(ALT_SHIFT, Key::Char('a'))],
        category: UNREAD,
    },
    Command {
        id: SEARCH,
        title: search_title,
        icon: None,
        keybindings: &[KeyBinding::primary('f')],
        category: SEARCH_CATEGORY,
    },
];

/// Bind every chord of the table: the shell routes each one.
pub fn bind_keys(keymap: &mut Keymap) {
    keymap.bind_table(COMMANDS);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_commands::shared;

    /// Each command is described once, with a title in every locale, a
    /// group, no blank icon and no chord another command holds.
    /// Every shortcut is therefore one command of the table (`MURMURE-170`).
    #[test]
    fn the_table_has_no_defect() {
        assert_eq!(ambre_commands::table_defects(COMMANDS), []);
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        let bound = COMMANDS.iter().flat_map(|c| c.keybindings).count();
        assert_eq!(keymap.len(), bound);
    }

    /// No chord of the table is one the browser services. Those are the
    /// clipboard chords, modern and legacy, and the undo and redo chords.
    /// Nor is it one the browser keeps from the page, such as the tab chords
    /// or the task manager of Chrome (`MURMURE-171`).
    #[test]
    fn no_chord_is_one_the_browser_services() {
        let reserved = [
            KeyBinding::shift(Key::Escape),
            KeyBinding::primary('w'),
            KeyBinding::primary('t'),
            KeyBinding::primary('n'),
            KeyBinding::primary_shift('w'),
            KeyBinding::primary_shift('t'),
            KeyBinding::primary_shift('n'),
            KeyBinding::new(Modifiers::PRIMARY, Key::Tab),
        ];
        let serviced: Vec<KeyBinding> = [
            shared::CUT_CHORDS,
            shared::COPY_CHORDS,
            shared::PASTE_CHORDS,
            shared::PASTE_PLAIN_CHORDS,
            shared::UNDO_CHORDS,
            shared::REDO_CHORDS,
            &reserved,
        ]
        .concat();
        for command in COMMANDS {
            for chord in command.keybindings {
                assert!(!serviced.contains(chord), "{} takes {chord}", command.id.0);
            }
        }
    }
}
