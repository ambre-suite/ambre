// ambre-toile/src/chrome/menu.rs
// ============================================================================
// Right-click context-menu entry builder (`toile-menus`), wasm-only. Renders
// through the shared `ambre-chrome` `ContextMenu`, so it inherits the launcher's
// `.ambre-ctx-*` CSS with zero style additions -- pixel-identical to the Trame
// menu.
//
// The pure command surface it consumes (the `CommandId`s, the `CtxScope`
// descriptor, the id -> command resolver) lives in the always-compiled
// `ui::menu`, and the label and icon of each row come from `ui::commands`
// (`UI-THREAD-019`). This file decides which rows apply, never what they say.
// ============================================================================

use crate::ui::commands as registry;
use crate::ui::menu::{
    ADD_TO_FRAME, ALIGN_BOTTOM, ALIGN_HCENTER, ALIGN_LEFT, ALIGN_RIGHT, ALIGN_TOP, ALIGN_VCENTER,
    BACKWARD, BRING_FRONT, COPY, CUT, CtxScope, DELETE, DISTRIBUTE_H, DISTRIBUTE_V, DUPLICATE,
    FORWARD, GROUP, LOCK, PASTE, REACT_CHECK, REACT_CLEAR, REACT_HEART, REACT_PARTY,
    REACT_THUMBSUP, REMOVE_FROM_FRAME, SELECT_ALL, SEND_BACK, UNGROUP, UNLOCK,
};
use ambre_chrome::MenuEntry;
use ambre_chrome::commands::{CommandId, Locale};

use super::icons;

/// The menu entry for one command, labelled and iconed from the registry.
/// `None` for an identity the registry does not describe, which no layout below
/// produces.
fn entry(id: CommandId) -> Option<MenuEntry> {
    let label = registry::title(id, &Locale::En)?;
    Some(match registry::icon(id).and_then(icons::resolve) {
        Some(icon) => MenuEntry::with_icon(id, label, icon),
        None => MenuEntry::new(id, label),
    })
}

/// Append the entries for `ids` that the registry describes.
fn push_all(entries: &mut Vec<MenuEntry>, ids: &[CommandId]) {
    entries.extend(ids.iter().filter_map(|&id| entry(id)));
}

/// The Toile right-click context-menu entries (`toile-menus`), iconed and
/// separated. Inapplicable rows are omitted per `scope` (the Trame pattern):
/// on empty canvas only Paste + Select-All; over a selection the full command set
/// filtered to what applies (Group only for a multi-selection, Ungroup only when
/// a group is selected, Lock/Unlock by lock state, frame entries only inside a
/// frame).
pub fn context_entries_for(scope: &CtxScope) -> Vec<MenuEntry> {
    let mut entries = Vec::new();

    if !scope.over_selection {
        // Empty canvas: Paste + Select-All only.
        push_all(&mut entries, &[PASTE, SELECT_ALL]);
        return entries;
    }

    // Clipboard group.
    push_all(&mut entries, &[CUT, COPY, PASTE, DUPLICATE, DELETE]);

    // Reactions: a minimal fixed emoji set plus a clear entry. The badge
    // renders the color emoji glyph itself; these menu icons are just cues.
    entries.push(MenuEntry::separator());
    push_all(
        &mut entries,
        &[
            REACT_THUMBSUP,
            REACT_HEART,
            REACT_CHECK,
            REACT_PARTY,
            REACT_CLEAR,
        ],
    );

    // Grouping (only the applicable side).
    if scope.multi || scope.has_group {
        entries.push(MenuEntry::separator());
        if scope.multi {
            push_all(&mut entries, &[GROUP]);
        }
        if scope.has_group {
            push_all(&mut entries, &[UNGROUP]);
        }
    }

    // Z-order.
    entries.push(MenuEntry::separator());
    push_all(&mut entries, &[BRING_FRONT, FORWARD, BACKWARD, SEND_BACK]);

    // Lock / unlock (only the applicable side).
    if scope.any_unlocked || scope.any_locked {
        entries.push(MenuEntry::separator());
        if scope.any_unlocked {
            push_all(&mut entries, &[LOCK]);
        }
        if scope.any_locked {
            push_all(&mut entries, &[UNLOCK]);
        }
    }

    // Align and distribute: an alignment needs two boxes to have a reference
    // edge, a distribution needs three to have an interior gap. Below those
    // counts the rows are omitted rather than shown inert (the Trame pattern).
    if scope.count >= 2 {
        entries.push(MenuEntry::separator());
        push_all(
            &mut entries,
            &[
                ALIGN_LEFT,
                ALIGN_HCENTER,
                ALIGN_RIGHT,
                ALIGN_TOP,
                ALIGN_VCENTER,
                ALIGN_BOTTOM,
            ],
        );
        if scope.count >= 3 {
            push_all(&mut entries, &[DISTRIBUTE_H, DISTRIBUTE_V]);
        }
    }

    // Frame membership (only when a containing frame exists).
    if scope.containing_frame.is_some() {
        entries.push(MenuEntry::separator());
        push_all(&mut entries, &[ADD_TO_FRAME, REMOVE_FROM_FRAME]);
    }

    entries
}
