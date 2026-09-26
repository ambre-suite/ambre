// The page's context menu: the clipboard entries, titled from the command
// table (`PROSE-UI-227`). Built here so the shell only renders it (`ARCH-018`).

use ambre_chrome::MenuEntry;
use ambre_commands::Locale;

use crate::commands::{COMMANDS, CONTEXT_MENU};

/// The entries of the page's context menu, in menu order.
#[must_use]
pub fn context_entries() -> Vec<MenuEntry> {
    CONTEXT_MENU
        .into_iter()
        .filter_map(|id| ambre_commands::find(COMMANDS, id))
        .map(|c| MenuEntry::new(c.id, (c.title)(&Locale::En)))
        .collect()
}
