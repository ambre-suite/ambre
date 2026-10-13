// ambre-prose/src/chrome/menu.rs
// The page's context menu: the history entries, then the clipboard entries,
// titled from the command table (`PROSE-UI-227`). Built here so the shell only
// renders it (`ARCH-018`).

use ambre_chrome::MenuEntry;
use ambre_commands::Locale;

use super::History;
use crate::commands::{COMMANDS, CONTEXT_MENU, REDO, UNDO};

/// The entries of the page's context menu, in menu order. An undo or redo
/// row is inert when the history holds no step for it. The menu reads
/// `history`, which the Logic thread reports, and counts nothing (`UNDO-006`).
#[must_use]
pub fn context_entries(history: History) -> Vec<MenuEntry> {
    let mut entries = Vec::with_capacity(CONTEXT_MENU.len() + 1);
    for command in CONTEXT_MENU
        .into_iter()
        .filter_map(|id| ambre_commands::find(COMMANDS, id))
    {
        let entry = MenuEntry::command(command, &Locale::En);
        let inert =
            (command.id == UNDO && !history.can_undo) || (command.id == REDO && !history.can_redo);
        entries.push(if inert { entry.disabled() } else { entry });
        if command.id == REDO {
            entries.push(MenuEntry::separator());
        }
    }
    entries
}
