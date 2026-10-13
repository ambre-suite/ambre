// ambre-toile/src/chrome/menu.rs
// ============================================================================
// Right-click context-menu entry builder (`toile-menus`), wasm-only. Renders
// through the shared `ambre-chrome` `ContextMenu`, so it inherits the launcher's
// `.ambre-ctx-*` CSS with zero style additions -- pixel-identical to the Trame
// menu.
//
// The pure command surface it consumes (the `CommandId`s, the `CtxScope`
// descriptor, the id -> command resolver) lives in the always-compiled
// `ui::menu`. The label and icon of each row come from `ui::commands`
// (`UI-THREAD-019`). This file decides which rows apply, never what they say.
// ============================================================================

use crate::ui::commands as registry;
use crate::ui::menu::{CtxScope, context_menu_ids};
use ambre_chrome::MenuEntry;
use ambre_chrome::commands::{CommandId, Locale};

use super::icons;

/// The menu entry for one command, labelled and iconed from the registry.
/// `None` for an identity the registry does not describe, which no layout below
/// produces.
fn entry(id: CommandId) -> Option<MenuEntry> {
    let cmd = ambre_chrome::commands::find(registry::COMMANDS, id)?;
    let entry = MenuEntry::command(cmd, &Locale::En);
    Some(match cmd.icon.and_then(icons::resolve) {
        Some(icon) => entry.icon(icon),
        None => entry,
    })
}

/// The Toile right-click context-menu entries (`toile-menus`), iconed and
/// separated. Inapplicable rows are omitted per `scope`, the Trame pattern: on
/// empty canvas only Paste + Select-All. Over a selection it is the full command
/// set filtered to what applies. The omission decision itself is the host-tested
/// [`context_menu_ids`](crate::ui::menu::context_menu_ids); this file only maps
/// each id to its iconed entry.
pub fn context_entries_for(scope: &CtxScope) -> Vec<MenuEntry> {
    context_menu_ids(scope)
        .into_iter()
        .filter_map(|id| match id {
            Some(id) => entry(id),
            None => Some(MenuEntry::separator()),
        })
        .collect()
}
