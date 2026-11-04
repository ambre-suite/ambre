// ambre-toile/src/ui/menu_bar.rs
// ============================================================================
// Command surface for the Toile top menu bar and zoom control
// (`toile-menus`). The host-visible half: the `StaticKey<Commands>`s that only the menu
// bar contributes, that is Undo / Redo / Present and the five zoom actions.
// Also the `MenuId` grouping and the pure per-menu layout the DOM builder
// renders.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so it is
// host-unit-tested, exactly like `ui::menu`, `ui::palette` and `ui::find`.
// The wasm-only Leptos component that renders these (`chrome::TopMenuBar`)
// consumes them. It routes every selection through the same `on_command` path
// as the context menu, the shared command path of the chrome surfaces.
//
// The Edit / Object rows REUSE the ids already declared for the context menu
// (`ui::menu`). A menu id is minted once and shared by every trigger, so a
// menu-bar "Group" and a right-click "Group" are the very same command. Only
// the entries with no other home (Undo/Redo, the zoom actions, Present) are new
// here.
// ============================================================================

use ambre_bus::app::toile::{PresentFocus, ToileCommand};
use ambre_core::{Commands, StaticKey};

use super::menu;

// -- Command identities (new to the menu bar) ---------------------------------
// The clipboard / grouping / z-order / lock / select-all ids live in `ui::menu`
// and are re-used below; these are the ids the menu bar introduces.

/// Undo the last board mutation (-> `ToileCommand::Undo`).
pub const UNDO: StaticKey<Commands> = StaticKey::<Commands>::new("toile.edit.undo");
/// Redo the last undone mutation (-> `ToileCommand::Redo`).
pub const REDO: StaticKey<Commands> = StaticKey::<Commands>::new("toile.edit.redo");
/// Zoom in one notch about the viewport center (local transform, `toile-menus`).
pub const ZOOM_IN: StaticKey<Commands> = StaticKey::<Commands>::new("toile.view.zoom_in");
/// Zoom out one notch about the viewport center.
pub const ZOOM_OUT: StaticKey<Commands> = StaticKey::<Commands>::new("toile.view.zoom_out");
/// Reset the zoom to 100% (the `%` readout click).
pub const ZOOM_RESET: StaticKey<Commands> = StaticKey::<Commands>::new("toile.view.zoom_reset");
/// Fit the whole board into the viewport (Fit).
pub const ZOOM_FIT: StaticKey<Commands> = StaticKey::<Commands>::new("toile.view.zoom_fit");
/// Fit the current selection into the viewport (Zoom-to-selection).
pub const ZOOM_TO_SELECTION: StaticKey<Commands> =
    StaticKey::<Commands>::new("toile.view.zoom_to_selection");
/// Wheel navigation preference, all view-local: `Auto` detects the device
/// per event, `Mouse` makes the wheel zoom, `Trackpad` makes it pan (Ctrl/pinch
/// still zooms).
pub const POINTER_AUTO: StaticKey<Commands> = StaticKey::<Commands>::new("toile.view.pointer_auto");
pub const POINTER_MOUSE: StaticKey<Commands> =
    StaticKey::<Commands>::new("toile.view.pointer_mouse");
pub const POINTER_TRACKPAD: StaticKey<Commands> =
    StaticKey::<Commands>::new("toile.view.pointer_trackpad");
/// Open the comments panel on the whole board (`toile-comments` section 4). A
/// UI-only intent: the panel is chrome, and the board keeps no open flag.
pub const COMMENTS: StaticKey<Commands> = StaticKey::<Commands>::new("toile.view.comments");
/// Enter presentation mode (the deck panel of `toile-presentation`). A stub
/// here: the panel is owned by that feature, not this surface.
pub const PRESENT: StaticKey<Commands> = StaticKey::<Commands>::new("toile.present.start");
/// Open the deck panel: the surface that adds a frame to the presentation,
/// reorders it and removes it (`TOILE-FEAT-026`). A UI-only intent, like
/// [`COMMENTS`]: the panel is chrome, and the board keeps no open flag.
pub const DECK: StaticKey<Commands> = StaticKey::<Commands>::new("toile.present.deck");
/// Bring a file onto the board through a picker (`TOILE-IO-020`). A UI-only
/// intent: the launcher owns the file dialog, and the board learns of the file
/// only once its bytes are read.
pub const IMPORT_FILE: StaticKey<Commands> = StaticKey::<Commands>::new("toile.file.import");
/// Put a web page on the board (`TOILE-087`). A UI-only intent: the shell owns
/// the dialog that takes the address. The board learns of it only once the
/// address has passed its rule.
pub const EMBED_WEB: StaticKey<Commands> = StaticKey::<Commands>::new("toile.file.embed_web");
/// Put another Ambre document on the board (`TOILE-086`). A UI-only intent for
/// the same reason: only the launcher knows which documents this user may open,
/// so only it can offer the list.
pub const EMBED_DOC: StaticKey<Commands> = StaticKey::<Commands>::new("toile.file.embed_doc");
/// Put a link on the selected element (`TOILE-066`). A UI-only intent: the
/// shell owns the chooser, because two of the three targets are things only it
/// can offer. Those are an address it validates like an embed's, and the
/// documents this user may open.
pub const LINK_SET: StaticKey<Commands> = StaticKey::<Commands>::new("toile.object.link_set");
/// Take the link off the selected element (`TOILE-066`). Resolves through the
/// same intent as the chooser, carrying no target.
pub const LINK_CLEAR: StaticKey<Commands> = StaticKey::<Commands>::new("toile.object.link_clear");
/// Take the board out as an SVG document (`TOILE-IO-021`). Resolves to a board
/// command: the Logic thread holds the board and serializes it.
pub const EXPORT_SVG: StaticKey<Commands> = StaticKey::<Commands>::new("toile.file.export_svg");
/// Take the deck out as a PDF, one page per slide (`TOILE-IO-029`). Resolves to
/// a board command, like [`EXPORT_SVG`], but the render walk runs on the Logic
/// thread rather than in the board's own state. The pages are drawn, not
/// serialized, and the base font faces live there.
pub const EXPORT_PDF: StaticKey<Commands> = StaticKey::<Commands>::new("toile.file.export_pdf");
/// Write the whole board to a native `.ambre` file (`ambre-spec-docfile.md`).
/// Unlike the two exports above it drops nothing: the payload **is** the
/// document (`DOCFILE-001`). Unlike them it is also plaintext, which is why its
/// own label says so (`DOCFILE-009`). Resolves to a board command the Logic
/// thread serves, because the container's header carries the document's identity
/// and the board does not know it.
pub const SAVE_FILE: StaticKey<Commands> = StaticKey::<Commands>::new("toile.file.save_file");
/// Write the whole board to a native `.ambre` file **sealed under a
/// passphrase** (`ambre-spec-docfile.md section 5`). The same file as
/// [`SAVE_FILE`], encrypted. A UI-only intent, because the user types the
/// passphrase into a dialog the launcher owns and no `StaticKey<Commands>` carries it.
pub const SAVE_SEALED_FILE: StaticKey<Commands> =
    StaticKey::<Commands>::new("toile.file.save_sealed_file");

/// Every id the menu bar introduces (not shared with `ui::menu`). Used by the
/// view to recognise the ids it must resolve locally, and by tests.
pub const MENU_BAR_COMMANDS: &[StaticKey<Commands>] = &[
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
    SAVE_FILE,
    SAVE_SEALED_FILE,
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
/// the label and the accelerator from `ui::commands` (`UI-THREAD-019`). A
/// layout declares an order and a grouping and never a word the user reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuRow {
    /// A selectable command, shown with the label and chord it declares.
    Command(StaticKey<Commands>),
    /// A divider between two groups.
    Separator,
}

/// The rows of one menu (`toile-menus`), grouped with separators.
/// Inapplicable-row hiding by selection state is NOT done here, unlike the
/// context menu. A menu bar shows a stable layout and relies on the command's
/// registry state to disable an entry. The layout is a pure function of the
/// `MenuId`.
pub fn menu_layout(id: MenuId) -> Vec<MenuRow> {
    let row = MenuRow::Command;
    match id {
        // The share dialog is app-agnostic chrome (E2), so File carries the
        // local file, the two lossy exports and the presentation entry point.
        // The save sits above the separator with the exports below it. One of
        // the three keeps everything, and the grouping is the only place that
        // difference shows before the user clicks.
        MenuId::File => vec![
            row(IMPORT_FILE),
            row(EMBED_WEB),
            row(EMBED_DOC),
            MenuRow::Separator,
            row(SAVE_FILE),
            row(SAVE_SEALED_FILE),
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
            row(menu::ALIGN_LEFT),
            row(menu::ALIGN_HCENTER),
            row(menu::ALIGN_RIGHT),
            row(menu::ALIGN_TOP),
            row(menu::ALIGN_VCENTER),
            row(menu::ALIGN_BOTTOM),
            MenuRow::Separator,
            row(menu::DISTRIBUTE_H),
            row(menu::DISTRIBUTE_V),
            MenuRow::Separator,
            row(LINK_SET),
            row(LINK_CLEAR),
        ],
        MenuId::Present => vec![row(PRESENT), row(DECK)],
    }
}

/// The direct board command a menu-bar id emits, if any. Undo / Redo and the
/// presentation entry point resolve to a `ToileCommand` independent of the
/// selection or local transform. The zoom ids are local-transform intents and
/// return `None` (serviced by the view). Pure, so the wiring is host-tested.
pub fn menu_bar_edit(id: StaticKey<Commands>) -> Option<ToileCommand> {
    match id {
        UNDO => Some(ToileCommand::Undo),
        REDO => Some(ToileCommand::Redo),
        // Enter presentation (focus) mode; the presenter overlay / playback
        // controls of toile-presentation will layer on the resulting
        // PresentModeChanged signal later.
        PRESENT => Some(ToileCommand::EnterPresent {
            focus: PresentFocus::Focus,
        }),
        // The board is on the Logic thread, so the serialization is too. The
        // bytes come back as an event the view hands to the browser
        // (`TOILE-IO-085`).
        EXPORT_SVG => Some(ToileCommand::ExportSvg),
        EXPORT_PDF => Some(ToileCommand::ExportPdf),
        SAVE_FILE => Some(ToileCommand::ExportAmbreFile),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_commands(rows: &[MenuRow]) -> Vec<StaticKey<Commands>> {
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

    /// `TOILE-UI-418` (`ambre-spec-docfile.md section 4`): the save gesture
    /// exists, it is in the File menu, and it resolves to the board command
    /// the Logic thread serves.
    #[test]
    fn the_file_menu_offers_the_native_file_and_resolves_it() {
        let cmds = item_commands(&menu_layout(MenuId::File));
        assert!(
            cmds.contains(&SAVE_FILE),
            "File must offer the native file: {cmds:?}"
        );
        assert!(matches!(
            menu_bar_edit(SAVE_FILE),
            Some(ToileCommand::ExportAmbreFile)
        ));
    }

    #[test]
    fn menu_bar_ids_are_distinct() {
        let mut ids: Vec<&str> = MENU_BAR_COMMANDS.iter().map(|c| c.as_str()).collect();
        ids.sort_unstable();
        let len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len, "duplicate menu-bar command id");
    }

    /// `TOILE-UI-121`: no id the bar introduces is also a context-menu id. An
    /// entry both surfaces show reuses the context constant. It is one
    /// command reached two ways, never a second id with the same name.
    #[test]
    fn no_bar_id_is_also_a_context_id() {
        for id in MENU_BAR_COMMANDS {
            assert!(
                !menu::COMMANDS.iter().any(|c| c.as_str() == id.as_str()),
                "{id:?} is declared by both surfaces"
            );
        }
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

    /// Every dropdown row resolves in the registry, so a menu never shows an
    /// entry the registry cannot label (`UI-THREAD-019`).
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
            Some(ToileCommand::EnterPresent {
                focus: PresentFocus::Focus
            })
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

    /// `TOILE-IO-104`: an export reads the board and writes nothing, so the
    /// role gate must let a read-only viewer take a copy out.
    #[test]
    fn exports_are_not_mutating() {
        assert!(!ToileCommand::ExportSvg.is_mutating());
        assert!(!ToileCommand::ExportPdf.is_mutating());
    }

    /// The two placement entries are UI-only intents (`TOILE-086`,
    /// `TOILE-087`). The shell owns the dialog, and the board is handed the
    /// answer, an address or a document, never the gesture.
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

    /// `TOILE-UI-417`: the link chooser is a shell intent for the same reason
    /// the embed entries are: the shell owns the dialog. Clearing is not -- it
    /// names no target, so the view sends it straight to the board.
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

    /// The Object menu carries the six alignments and the two distributions,
    /// and each resolves against the selection rather than to a direct board
    /// command.
    #[test]
    fn the_align_entries_sit_under_object() {
        let cmds = item_commands(&menu_layout(MenuId::Object));
        for id in [
            menu::ALIGN_LEFT,
            menu::ALIGN_HCENTER,
            menu::ALIGN_RIGHT,
            menu::ALIGN_TOP,
            menu::ALIGN_VCENTER,
            menu::ALIGN_BOTTOM,
            menu::DISTRIBUTE_H,
            menu::DISTRIBUTE_V,
        ] {
            assert!(cmds.contains(&id), "{id:?} missing from the Object menu");
            assert!(
                menu_bar_edit(id).is_none(),
                "{id:?} needs the selection a menu id does not carry"
            );
        }
    }
}
