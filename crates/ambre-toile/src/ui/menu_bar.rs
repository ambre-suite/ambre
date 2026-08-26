// ambre-toile/src/ui/menu_bar.rs
// ============================================================================
// Command surface for the Toile top menu bar and zoom control
// (`toile-menus`). The host-visible half: the `CommandId`s that only the menu
// bar contributes (Undo / Redo / Present and the five zoom actions), the
// `MenuId` grouping, and the pure per-menu layout the DOM builder renders.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so it is
// host-unit-tested, exactly like `ui::menu`, `ui::palette` and `ui::find`.
// The wasm-only Leptos component that renders these (`chrome::TopMenuBar`)
// consumes them and routes every selection through the same `on_command` path as
// the context menu (the shared command channel of the chrome surfaces).
//
// The Edit / Object rows REUSE the ids already declared for the context menu
// (`ui::menu`): a menu id is minted once and shared by every trigger, so a menu-
// bar "Group" and a right-click "Group" are the very same command. Only the
// entries with no other home (Undo/Redo, the zoom actions, Present) are new here.
// ============================================================================

use ambre_bus::app::toile::ToileCommand;
use ambre_commands::{CommandId, Key, KeyBinding, Modifiers};

use super::menu;

// -- Command identities (new to the menu bar) ---------------------------------
// The clipboard / grouping / z-order / lock / select-all ids live in `ui::menu`
// and are re-used below; these are the ids the menu bar introduces.

/// Undo the last board mutation (-> `ToileCommand::Undo`).
pub const UNDO: CommandId = CommandId("toile.edit.undo");
/// Redo the last undone mutation (-> `ToileCommand::Redo`).
pub const REDO: CommandId = CommandId("toile.edit.redo");
/// Zoom in one notch about the viewport center (local transform, `toile-menus`).
pub const ZOOM_IN: CommandId = CommandId("toile.view.zoom_in");
/// Zoom out one notch about the viewport center.
pub const ZOOM_OUT: CommandId = CommandId("toile.view.zoom_out");
/// Reset the zoom to 100% (the `%` readout click).
pub const ZOOM_RESET: CommandId = CommandId("toile.view.zoom_reset");
/// Fit the whole board into the viewport (Fit).
pub const ZOOM_FIT: CommandId = CommandId("toile.view.zoom_fit");
/// Fit the current selection into the viewport (Zoom-to-selection).
pub const ZOOM_TO_SELECTION: CommandId = CommandId("toile.view.zoom_to_selection");
/// Wheel navigation preference, all view-local: `Auto` detects the device
/// per event, `Mouse` makes the wheel zoom, `Trackpad` makes it pan (Ctrl/pinch
/// still zooms).
pub const POINTER_AUTO: CommandId = CommandId("toile.view.pointer_auto");
pub const POINTER_MOUSE: CommandId = CommandId("toile.view.pointer_mouse");
pub const POINTER_TRACKPAD: CommandId = CommandId("toile.view.pointer_trackpad");
/// Enter presentation mode (the deck panel of `toile-presentation`). A stub
/// here: the panel is owned by that feature, not this surface.
pub const PRESENT: CommandId = CommandId("toile.present.start");

/// Every id the menu bar introduces (not shared with `ui::menu`). Used by the
/// view to recognise the ids it must resolve locally, and by tests.
pub const MENU_BAR_COMMANDS: &[CommandId] = &[
    UNDO,
    REDO,
    ZOOM_IN,
    ZOOM_OUT,
    ZOOM_RESET,
    ZOOM_FIT,
    ZOOM_TO_SELECTION,
    POINTER_AUTO,
    POINTER_MOUSE,
    POINTER_TRACKPAD,
    PRESENT,
];

// -- Menu grouping -------------------------------------------------------------

/// The five top-level menus (`toile-menus`), in bar order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuId {
    File,
    Edit,
    View,
    Object,
    Present,
}

/// The menus in bar order, each with its title.
pub const TOP_MENUS: &[(MenuId, &str)] = &[
    (MenuId::File, "File"),
    (MenuId::Edit, "Edit"),
    (MenuId::View, "View"),
    (MenuId::Object, "Object"),
    (MenuId::Present, "Present"),
];

/// One row of a dropdown: a selectable command (with an optional accelerator to
/// display) or a group separator. Pure data; the wasm builder turns it into an
/// `ambre_chrome::MenuEntry`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MenuRow {
    Item {
        command: CommandId,
        label: &'static str,
        shortcut: Option<KeyBinding>,
    },
    Separator,
}

impl MenuRow {
    const fn item(command: CommandId, label: &'static str, shortcut: KeyBinding) -> Self {
        Self::Item {
            command,
            label,
            shortcut: Some(shortcut),
        }
    }
    const fn bare(command: CommandId, label: &'static str) -> Self {
        Self::Item {
            command,
            label,
            shortcut: None,
        }
    }
}

/// The Delete accelerator (the bare `Del` key, matching the view's shortcut).
const DEL: KeyBinding = KeyBinding::new(Modifiers::NONE, Key::Delete);

/// The rows of one menu (`toile-menus`), grouped with separators. Inapplicable-
/// row hiding by selection state is NOT done here (unlike the context menu): a
/// menu bar shows a stable layout and relies on the command's registry state to
/// disable an entry, so the layout is a pure function of the `MenuId`.
pub fn menu_layout(id: MenuId) -> Vec<MenuRow> {
    match id {
        // File is thin in the MVP: the share dialog is app-agnostic chrome (E2),
        // and there is no local "save" (the board lives in `LoroDoc`). Only the
        // presentation entry point lands here for now.
        MenuId::File => vec![MenuRow::bare(PRESENT, "Start presentation")],
        MenuId::Edit => vec![
            MenuRow::item(UNDO, "Undo", KeyBinding::ctrl('z')),
            MenuRow::item(REDO, "Redo", KeyBinding::ctrl_shift('z')),
            MenuRow::Separator,
            MenuRow::item(menu::CUT, "Cut", KeyBinding::ctrl('x')),
            MenuRow::item(menu::COPY, "Copy", KeyBinding::ctrl('c')),
            MenuRow::item(menu::PASTE, "Paste", KeyBinding::ctrl('v')),
            MenuRow::item(menu::DUPLICATE, "Duplicate", KeyBinding::ctrl('d')),
            MenuRow::item(menu::DELETE, "Delete", DEL),
            MenuRow::Separator,
            MenuRow::item(menu::SELECT_ALL, "Select all", KeyBinding::ctrl('a')),
        ],
        MenuId::View => vec![
            MenuRow::bare(ZOOM_IN, "Zoom in"),
            MenuRow::bare(ZOOM_OUT, "Zoom out"),
            MenuRow::bare(ZOOM_RESET, "Zoom to 100%"),
            MenuRow::Separator,
            MenuRow::bare(ZOOM_FIT, "Fit to content"),
            MenuRow::bare(ZOOM_TO_SELECTION, "Zoom to selection"),
            MenuRow::Separator,
            MenuRow::bare(POINTER_AUTO, "Pointer: automatic"),
            MenuRow::bare(POINTER_MOUSE, "Pointer: mouse (wheel zooms)"),
            MenuRow::bare(POINTER_TRACKPAD, "Pointer: trackpad (wheel pans)"),
        ],
        MenuId::Object => vec![
            MenuRow::item(menu::GROUP, "Group", KeyBinding::ctrl('g')),
            MenuRow::item(menu::UNGROUP, "Ungroup", KeyBinding::ctrl_shift('g')),
            MenuRow::Separator,
            MenuRow::bare(menu::BRING_FRONT, "Bring to front"),
            MenuRow::bare(menu::FORWARD, "Bring forward"),
            MenuRow::bare(menu::BACKWARD, "Send backward"),
            MenuRow::bare(menu::SEND_BACK, "Send to back"),
            MenuRow::Separator,
            MenuRow::bare(menu::LOCK, "Lock"),
            MenuRow::bare(menu::UNLOCK, "Unlock"),
        ],
        MenuId::Present => vec![MenuRow::bare(PRESENT, "Start presentation")],
    }
}

/// The direct board command a menu-bar id emits, if any. Undo / Redo and the
/// presentation entry point resolve to a `ToileCommand` independent of the
/// selection or local transform; the zoom ids are local-transform intents and
/// return `None` (serviced by the view). Pure, so the wiring is host-tested.
pub fn menu_bar_edit(id: CommandId) -> Option<ToileCommand> {
    match id {
        UNDO => Some(ToileCommand::Undo),
        REDO => Some(ToileCommand::Redo),
        // Enter presentation (focus) mode; the presenter overlay / playback
        // controls of toile-presentation will layer on the resulting
        // PresentModeChanged signal later.
        PRESENT => Some(ToileCommand::EnterPresent { focus: true }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_commands(rows: &[MenuRow]) -> Vec<CommandId> {
        rows.iter()
            .filter_map(|r| match r {
                MenuRow::Item { command, .. } => Some(*command),
                MenuRow::Separator => None,
            })
            .collect()
    }

    #[test]
    fn every_menu_has_items() {
        for (id, _) in TOP_MENUS {
            assert!(
                !item_commands(&menu_layout(*id)).is_empty(),
                "menu {id:?} is empty"
            );
        }
    }

    #[test]
    fn menu_bar_ids_are_distinct() {
        let mut ids: Vec<&str> = MENU_BAR_COMMANDS.iter().map(|c| c.0).collect();
        ids.sort_unstable();
        let len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len, "duplicate menu-bar command id");
    }

    #[test]
    fn view_menu_exposes_all_zoom_actions() {
        let cmds = item_commands(&menu_layout(MenuId::View));
        for expected in [ZOOM_IN, ZOOM_OUT, ZOOM_RESET, ZOOM_FIT, ZOOM_TO_SELECTION] {
            assert!(cmds.contains(&expected), "View missing {expected:?}");
        }
    }

    #[test]
    fn edit_menu_carries_undo_redo_and_reuses_clipboard_ids() {
        let cmds = item_commands(&menu_layout(MenuId::Edit));
        assert!(cmds.contains(&UNDO));
        assert!(cmds.contains(&REDO));
        // Reused, not redeclared: the same ids the context menu emits.
        assert!(cmds.contains(&menu::COPY));
        assert!(cmds.contains(&menu::SELECT_ALL));
    }

    #[test]
    fn shortcuts_render_canonically() {
        // The Display form the DOM chip shows must match the view's real chord.
        let rows = menu_layout(MenuId::Edit);
        let undo = rows.iter().find_map(|r| match r {
            MenuRow::Item {
                command, shortcut, ..
            } if *command == UNDO => *shortcut,
            _ => None,
        });
        assert_eq!(undo.map(|k| k.to_string()).as_deref(), Some("Ctrl+Z"));
    }

    #[test]
    fn present_entry_emits_enter_present() {
        // `ToileCommand` is not `PartialEq`, so match on the shape instead.
        assert!(matches!(
            menu_bar_edit(PRESENT),
            Some(ToileCommand::EnterPresent { focus: true })
        ));
    }

    #[test]
    fn undo_redo_map_to_edits() {
        assert!(matches!(menu_bar_edit(UNDO), Some(ToileCommand::Undo)));
        assert!(matches!(menu_bar_edit(REDO), Some(ToileCommand::Redo)));
    }

    #[test]
    fn zoom_ids_are_view_local() {
        for id in [ZOOM_IN, ZOOM_OUT, ZOOM_RESET, ZOOM_FIT, ZOOM_TO_SELECTION] {
            assert!(menu_bar_edit(id).is_none(), "{id:?} should be view-local");
        }
    }

    #[test]
    fn object_menu_reuses_z_order_and_lock_ids() {
        let cmds = item_commands(&menu_layout(MenuId::Object));
        for id in [
            menu::GROUP,
            menu::UNGROUP,
            menu::BRING_FRONT,
            menu::SEND_BACK,
            menu::LOCK,
            menu::UNLOCK,
        ] {
            assert!(cmds.contains(&id), "Object missing {id:?}");
        }
    }
}
