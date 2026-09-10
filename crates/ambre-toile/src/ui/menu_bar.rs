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
use ambre_commands::CommandId;

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
/// Open the comments panel on the whole board (`toile-comments` section 4). A
/// UI-only intent: the panel is chrome, and the board keeps no open flag.
pub const COMMENTS: CommandId = CommandId("toile.view.comments");
/// Enter presentation mode (the deck panel of `toile-presentation`). A stub
/// here: the panel is owned by that feature, not this surface.
pub const PRESENT: CommandId = CommandId("toile.present.start");
/// Open the deck panel: the surface that adds a frame to the presentation,
/// reorders it and removes it (`TOILE-FEAT-026`). A UI-only intent, like
/// [`COMMENTS`]: the panel is chrome, and the board keeps no open flag.
pub const DECK: CommandId = CommandId("toile.present.deck");
/// Bring a file onto the board through a picker (`TOILE-IO-020`). A UI-only
/// intent: the launcher owns the file dialog, and the board learns of the file
/// only once its bytes are read.
pub const IMPORT_FILE: CommandId = CommandId("toile.file.import");
/// Put a web page on the board (`TOILE-087`). A UI-only intent: the shell owns
/// the dialog that takes the address, and the board learns of it only once the
/// address has passed `ui::embed::normalize_web_address`.
pub const EMBED_WEB: CommandId = CommandId("toile.file.embed_web");
/// Put another Ambre document on the board (`TOILE-086`). A UI-only intent for
/// the same reason: only the launcher knows which documents this user may open,
/// so only it can offer the list.
pub const EMBED_DOC: CommandId = CommandId("toile.file.embed_doc");
/// Put a link on the selected element (`TOILE-066`). A UI-only intent: the
/// shell owns the chooser, because two of the three targets are things only it
/// can offer -- an address it validates like an embed's, and the documents this
/// user may open.
pub const LINK_SET: CommandId = CommandId("toile.object.link_set");
/// Take the link off the selected element (`TOILE-066`). Resolves through the
/// same intent as the chooser, carrying no target.
pub const LINK_CLEAR: CommandId = CommandId("toile.object.link_clear");
/// Take the board out as an SVG document (`TOILE-IO-021`). Resolves to a board
/// command: the Logic thread holds the board and serializes it.
pub const EXPORT_SVG: CommandId = CommandId("toile.file.export_svg");
/// Take the deck out as a PDF, one page per slide (`TOILE-IO-029`). Resolves to
/// a board command, like [`EXPORT_SVG`], but the render walk runs on the Logic
/// thread rather than in the board's own state: the pages are drawn, not
/// serialized, and the base font faces live there.
pub const EXPORT_PDF: CommandId = CommandId("toile.file.export_pdf");

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
    COMMENTS,
    PRESENT,
    DECK,
    IMPORT_FILE,
    EMBED_WEB,
    EMBED_DOC,
    LINK_SET,
    LINK_CLEAR,
    EXPORT_SVG,
    EXPORT_PDF,
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

/// One row of a dropdown: a selectable command, or a group separator. Pure
/// data; the wasm builder turns it into an `ambre_chrome::MenuEntry`, reading
/// the label and the accelerator from `ui::commands` (`UI-THREAD-019`), so a
/// layout declares an order and a grouping and never a word the user reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuRow {
    /// A selectable command, shown with the label and chord it declares.
    Command(CommandId),
    /// A divider between two groups.
    Separator,
}

/// The rows of one menu (`toile-menus`), grouped with separators. Inapplicable-
/// row hiding by selection state is NOT done here (unlike the context menu): a
/// menu bar shows a stable layout and relies on the command's registry state to
/// disable an entry, so the layout is a pure function of the `MenuId`.
pub fn menu_layout(id: MenuId) -> Vec<MenuRow> {
    let row = MenuRow::Command;
    match id {
        // File is thin in the MVP: the share dialog is app-agnostic chrome (E2),
        // and there is no local "save" (the board lives in `LoroDoc`). Only the
        // presentation entry point lands here for now.
        MenuId::File => vec![
            row(IMPORT_FILE),
            row(EMBED_WEB),
            row(EMBED_DOC),
            MenuRow::Separator,
            row(EXPORT_SVG),
            row(EXPORT_PDF),
            MenuRow::Separator,
            row(PRESENT),
        ],
        MenuId::Edit => vec![
            row(UNDO),
            row(REDO),
            MenuRow::Separator,
            row(menu::CUT),
            row(menu::COPY),
            row(menu::PASTE),
            row(menu::DUPLICATE),
            row(menu::DELETE),
            MenuRow::Separator,
            row(menu::SELECT_ALL),
        ],
        MenuId::View => vec![
            row(ZOOM_IN),
            row(ZOOM_OUT),
            row(ZOOM_RESET),
            MenuRow::Separator,
            row(ZOOM_FIT),
            row(ZOOM_TO_SELECTION),
            MenuRow::Separator,
            row(POINTER_AUTO),
            row(POINTER_MOUSE),
            row(POINTER_TRACKPAD),
            MenuRow::Separator,
            row(COMMENTS),
        ],
        MenuId::Object => vec![
            row(menu::GROUP),
            row(menu::UNGROUP),
            MenuRow::Separator,
            row(menu::BRING_FRONT),
            row(menu::FORWARD),
            row(menu::BACKWARD),
            row(menu::SEND_BACK),
            MenuRow::Separator,
            row(menu::LOCK),
            row(menu::UNLOCK),
            MenuRow::Separator,
            row(LINK_SET),
            row(LINK_CLEAR),
        ],
        MenuId::Present => vec![row(PRESENT), row(DECK)],
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
        // The board is on the Logic thread, so the serialization is too; the
        // bytes come back as an event the view hands to the browser
        // (`TOILE-IO-085`).
        EXPORT_SVG => Some(ToileCommand::ExportSvg),
        EXPORT_PDF => Some(ToileCommand::ExportPdf),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_commands(rows: &[MenuRow]) -> Vec<CommandId> {
        rows.iter()
            .filter_map(|r| match r {
                MenuRow::Command(command) => Some(*command),
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
        // The chip reads the registry, so that is what the test reads.
        let undo = crate::ui::commands::chord(UNDO);
        assert_eq!(undo.map(|k| k.to_string()).as_deref(), Some("Ctrl+Z"));
        assert!(item_commands(&menu_layout(MenuId::Edit)).contains(&UNDO));
    }

    /// A dropdown row shows the label the registry declares (`UI-THREAD-019`).
    /// A literal written back into a layout would break this.
    #[test]
    fn every_dropdown_row_is_described() {
        for (id, _) in TOP_MENUS {
            for command in item_commands(&menu_layout(*id)) {
                assert!(
                    crate::ui::commands::title(command, &ambre_commands::Locale::En).is_some(),
                    "{id:?} lists {command:?}, which the registry does not describe"
                );
            }
        }
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

    /// Both exports are board commands, not view-local intents: the document
    /// lives on the Logic thread, and so does everything that serializes it.
    #[test]
    fn file_menu_exports_resolve_to_board_commands() {
        let cmds = item_commands(&menu_layout(MenuId::File));
        assert!(cmds.contains(&EXPORT_SVG));
        assert!(cmds.contains(&EXPORT_PDF));
        assert!(matches!(
            menu_bar_edit(EXPORT_SVG),
            Some(ToileCommand::ExportSvg)
        ));
        assert!(matches!(
            menu_bar_edit(EXPORT_PDF),
            Some(ToileCommand::ExportPdf)
        ));
    }

    /// An export reads the board and writes nothing, so the role gate must let a
    /// read-only viewer take a copy out.
    #[test]
    fn exports_are_not_mutating() {
        assert!(!ToileCommand::ExportSvg.is_mutating());
        assert!(!ToileCommand::ExportPdf.is_mutating());
    }

    /// The two placement entries are UI-only intents (`TOILE-086`,
    /// `TOILE-087`): the shell owns the dialog, and the board is handed the
    /// answer -- an address or a document -- never the gesture.
    #[test]
    fn the_placement_entries_are_shell_intents() {
        let cmds = item_commands(&menu_layout(MenuId::File));
        for id in [EMBED_WEB, EMBED_DOC] {
            assert!(cmds.contains(&id), "File missing {id:?}");
            assert!(
                menu_bar_edit(id).is_none(),
                "{id:?} must resolve in the shell, not to a board command"
            );
        }
    }

    /// The link chooser is a shell intent for the same reason the embed entries
    /// are: the shell owns the dialog. Clearing is not -- it names no target, so
    /// the view sends it straight to the board.
    #[test]
    fn the_link_entries_sit_under_object() {
        let cmds = item_commands(&menu_layout(MenuId::Object));
        assert!(cmds.contains(&LINK_SET));
        assert!(cmds.contains(&LINK_CLEAR));
        assert!(
            menu_bar_edit(LINK_SET).is_none(),
            "the chooser resolves in the shell"
        );
        assert!(
            menu_bar_edit(LINK_CLEAR).is_none(),
            "clearing needs the selection, which a menu id does not carry"
        );
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
