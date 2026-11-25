// ambre-prose/src/chrome/menu.rs
// The page's context menu: the clipboard entries first, then the formatting
// the context panel also holds, titled from the command table
// (`PROSE-UI-227`, `CHROME-179`, `CHROME-182`). Built here so the shell only
// renders it (`ARCH-018`). Undo and redo live in the Ambre menu.

use ambre_chrome::MenuEntry;

use crate::commands::{COMMANDS, CONTEXT_MENU, MENU_CLIPBOARD};

/// The entries of the page's context menu, in menu order: the clipboard,
/// a separator, then the formatting.
#[must_use]
pub fn context_entries() -> Vec<MenuEntry> {
    let mut entries = Vec::with_capacity(CONTEXT_MENU.len() + 1);
    for command in CONTEXT_MENU
        .into_iter()
        .filter_map(|id| ambre_commands::find(COMMANDS, id))
    {
        entries.push(MenuEntry::command(command, &ambre_chrome::language()));
        if Some(&command.id) == MENU_CLIPBOARD.last() {
            entries.push(MenuEntry::separator());
        }
    }
    entries
}
