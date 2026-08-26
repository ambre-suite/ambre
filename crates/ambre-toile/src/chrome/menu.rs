// ambre-toile/src/chrome/menu.rs
// ============================================================================
// Right-click context-menu entry builder (`toile-menus`), wasm-only. Renders
// through the shared `ambre-chrome` `ContextMenu`, so it inherits the launcher's
// `.ambre-ctx-*` CSS with zero style additions -- pixel-identical to the Trame
// menu. Every entry uses `MenuEntry::with_icon` with an `icondata` glyph rendered
// by `leptos-icons` inside `ContextMenu`.
//
// The pure command surface it consumes (the `CommandId`s, the `CtxScope`
// descriptor, the id -> command resolver) lives in the always-compiled
// `ui::menu`, so it is host-tested; this file only maps a `CtxScope` to icon rows.
// ============================================================================

use crate::ui::menu::{
    ADD_TO_FRAME, BACKWARD, BRING_FRONT, COPY, CUT, CtxScope, DELETE, DUPLICATE, FORWARD, GROUP,
    LOCK, PASTE, REACT_CHECK, REACT_CLEAR, REACT_HEART, REACT_PARTY, REACT_THUMBSUP,
    REMOVE_FROM_FRAME, SELECT_ALL, SEND_BACK, UNGROUP, UNLOCK,
};
use ambre_chrome::MenuEntry;

/// The Toile right-click context-menu entries (`toile-menus`), iconed and
/// separated. Inapplicable rows are omitted per `scope` (the Trame pattern):
/// on empty canvas only Paste + Select-All; over a selection the full command set
/// filtered to what applies (Group only for a multi-selection, Ungroup only when
/// a group is selected, Lock/Unlock by lock state, frame entries only inside a
/// frame). Every entry carries an `icondata` glyph.
pub fn context_entries_for(scope: &CtxScope) -> Vec<MenuEntry> {
    let mut entries = Vec::new();

    if !scope.over_selection {
        // Empty canvas: Paste + Select-All only.
        entries.push(MenuEntry::with_icon(
            PASTE,
            "Paste",
            icondata::MdiContentPaste,
        ));
        entries.push(MenuEntry::with_icon(
            SELECT_ALL,
            "Select all",
            icondata::MdiSelectAll,
        ));
        return entries;
    }

    // Clipboard group.
    entries.push(MenuEntry::with_icon(CUT, "Cut", icondata::MdiContentCut));
    entries.push(MenuEntry::with_icon(COPY, "Copy", icondata::MdiContentCopy));
    entries.push(MenuEntry::with_icon(
        PASTE,
        "Paste",
        icondata::MdiContentPaste,
    ));
    entries.push(MenuEntry::with_icon(
        DUPLICATE,
        "Duplicate",
        icondata::MdiContentDuplicate,
    ));
    entries.push(MenuEntry::with_icon(DELETE, "Delete", icondata::MdiDelete));

    // Reactions: a minimal fixed emoji set plus a clear entry. The badge
    // renders the color emoji glyph itself; these menu icons are just cues.
    entries.push(MenuEntry::separator());
    entries.push(MenuEntry::with_icon(
        REACT_THUMBSUP,
        "React: thumbs up",
        icondata::MdiThumbUp,
    ));
    entries.push(MenuEntry::with_icon(
        REACT_HEART,
        "React: heart",
        icondata::MdiHeart,
    ));
    entries.push(MenuEntry::with_icon(
        REACT_CHECK,
        "React: check",
        icondata::MdiCheck,
    ));
    entries.push(MenuEntry::with_icon(
        REACT_PARTY,
        "React: celebrate",
        icondata::MdiPartyPopper,
    ));
    entries.push(MenuEntry::with_icon(
        REACT_CLEAR,
        "Clear my reaction",
        icondata::MdiClose,
    ));

    // Grouping (only the applicable side).
    if scope.multi || scope.has_group {
        entries.push(MenuEntry::separator());
        if scope.multi {
            entries.push(MenuEntry::with_icon(GROUP, "Group", icondata::MdiGroup));
        }
        if scope.has_group {
            entries.push(MenuEntry::with_icon(
                UNGROUP,
                "Ungroup",
                icondata::MdiUngroup,
            ));
        }
    }

    // Z-order.
    entries.push(MenuEntry::separator());
    entries.push(MenuEntry::with_icon(
        BRING_FRONT,
        "Bring to front",
        icondata::MdiArrangeBringToFront,
    ));
    entries.push(MenuEntry::with_icon(
        FORWARD,
        "Bring forward",
        icondata::MdiArrangeBringForward,
    ));
    entries.push(MenuEntry::with_icon(
        BACKWARD,
        "Send backward",
        icondata::MdiArrangeSendBackward,
    ));
    entries.push(MenuEntry::with_icon(
        SEND_BACK,
        "Send to back",
        icondata::MdiArrangeSendToBack,
    ));

    // Lock / unlock (only the applicable side).
    if scope.any_unlocked || scope.any_locked {
        entries.push(MenuEntry::separator());
        if scope.any_unlocked {
            entries.push(MenuEntry::with_icon(LOCK, "Lock", icondata::MdiLock));
        }
        if scope.any_locked {
            entries.push(MenuEntry::with_icon(
                UNLOCK,
                "Unlock",
                icondata::MdiLockOpenVariant,
            ));
        }
    }

    // Frame membership (only when a containing frame exists).
    if scope.containing_frame.is_some() {
        entries.push(MenuEntry::separator());
        entries.push(MenuEntry::with_icon(
            ADD_TO_FRAME,
            "Add to frame",
            icondata::MdiImageFrame,
        ));
        entries.push(MenuEntry::with_icon(
            REMOVE_FROM_FRAME,
            "Remove from frame",
            icondata::MdiSelectionDrag,
        ));
    }

    entries
}
