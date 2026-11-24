// ambre-toile/src/ui/menu_bar.rs
// ============================================================================
// The board commands that no context-menu entry carries (`toile-menus`).
// They are undo, redo, the zoom actions, the pointer mode, the panels, the
// presentation entry point, the file and the links. A menu bar once laid them out, and it
// left the board (`CHROME-160`). Each command now lives in its home region
// (`ambre-spec-shell.md` section 2), and the command palette reaches every
// one of them.
//
// The Ambre menu holds the file and the history, and the status bar holds the
// zoom. The avatar menu holds the pointer mode, and the context panel the
// rest.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so it is
// host-unit-tested, exactly like `ui::menu`, `ui::palette` and `ui::find`.
//
// The context ids live in `ui::menu`. An id is minted once and shared by every
// trigger, so the palette's "Group" and a right-click "Group" are the very
// same command.
// ============================================================================

use crate::message::{PngScope, PresentFocus, ToileCommand, ToileExport};
use ambre_core::{Commands, StaticKey};

// -- Command identities (beyond the context menu) -----------------------------
// The clipboard / grouping / z-order / lock / select-all ids live in `ui::menu`;
// these are the ids no context-menu entry carries.

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
/// Take the whole board out as a PNG image (`toile-import-export`). Resolves
/// to a board command: the Logic thread records the page, and the converter
/// thread rasterizes it (`TOILE-IO-017`).
pub const EXPORT_PNG: StaticKey<Commands> = StaticKey::<Commands>::new("toile.file.export_png");
/// Take the deck out as a PDF, one page per slide (`TOILE-IO-029`). Resolves to
/// a board command, like [`EXPORT_SVG`], but the render walk runs on the Logic
/// thread rather than in the board's own state. The pages are drawn, not
/// serialized, and the base font faces live there.
pub const EXPORT_PDF: StaticKey<Commands> = StaticKey::<Commands>::new("toile.file.export_pdf");
/// Take the deck out as a PowerPoint file, one slide per slide
/// (`TOILE-IO-126`). Resolves to a board command: the board maps itself onto
/// the deck, and the Office converter module writes the package.
pub const EXPORT_PPTX: StaticKey<Commands> = StaticKey::<Commands>::new("toile.file.export_pptx");
/// Take the board out as a showcase page, readable by anyone who holds the
/// file (`TOILE-IO-138`). Resolves to a board command, like [`EXPORT_SVG`].
/// The page leaves the encryption boundary, so its label says so before the
/// click (`TOILE-IO-096`).
pub const EXPORT_SHOWCASE: StaticKey<Commands> =
    StaticKey::<Commands>::new("toile.file.export_showcase");
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

/// Every id this module introduces (not shared with `ui::menu`). Used by the
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
    EXPORT_PNG,
    EXPORT_PDF,
    EXPORT_PPTX,
    EXPORT_SHOWCASE,
    SAVE_FILE,
    SAVE_SEALED_FILE,
];

/// The direct board command an id of this module emits, if any. Undo / Redo and the
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
        EXPORT_SVG => Some(ToileCommand::export(ToileExport::Svg)),
        EXPORT_PNG => Some(ToileCommand::export(ToileExport::Png(PngScope::Board))),
        EXPORT_PDF => Some(ToileCommand::export(ToileExport::Pdf)),
        EXPORT_PPTX => Some(ToileCommand::export(ToileExport::Pptx)),
        EXPORT_SHOWCASE => Some(ToileCommand::export(ToileExport::Showcase)),
        SAVE_FILE => Some(ToileCommand::export(ToileExport::Ambre)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::menu;
    use ambre_bus::interop::InteropRequest;
    use ambre_commands::Home;

    /// The home region the board's table gives `id`, or `None` for an id it
    /// does not describe.
    fn home_of(id: StaticKey<Commands>) -> Option<Home> {
        ambre_commands::find(crate::ui::commands::COMMANDS, id).map(|c| c.home)
    }

    /// `TOILE-UI-122`: the board homes commands in the Ambre menu and in the
    /// avatar menu, so neither header menu of a board opens empty.
    #[test]
    fn the_board_homes_commands_in_the_ambre_and_avatar_menus() {
        for region in [Home::AmbreMenu, Home::AvatarMenu] {
            assert!(
                crate::ui::commands::COMMANDS
                    .iter()
                    .any(|c| c.home == region),
                "the board homes nothing in {region:?}"
            );
        }
    }

    /// `TOILE-UI-418` (`ambre-spec-docfile.md section 4`): the save gesture
    /// exists, it lives in the Ambre menu, and it resolves to the board command
    /// the Logic thread serves.
    #[test]
    fn the_ambre_menu_offers_the_native_file_and_resolves_it() {
        assert_eq!(home_of(SAVE_FILE), Some(Home::AmbreMenu));
        assert_eq!(home_of(SAVE_SEALED_FILE), Some(Home::AmbreMenu));
        assert!(matches!(
            menu_bar_edit(SAVE_FILE),
            Some(ToileCommand::Interop(InteropRequest::Export(
                ToileExport::Ambre
            )))
        ));
    }

    /// `TOILE-UI-018`: no two ids this module introduces are equal.
    #[test]
    fn menu_bar_ids_are_distinct() {
        let mut ids: Vec<&str> = MENU_BAR_COMMANDS.iter().map(|c| c.as_str()).collect();
        ids.sort_unstable();
        let len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len, "duplicate command id");
    }

    /// `TOILE-UI-121`: no id this module introduces is also a context-menu
    /// id. A command both surfaces reach reuses the context constant. It is
    /// one command reached two ways, never a second id with the same name.
    #[test]
    fn no_bar_id_is_also_a_context_id() {
        for id in MENU_BAR_COMMANDS {
            assert!(
                !menu::COMMANDS.iter().any(|c| c.as_str() == id.as_str()),
                "{id:?} is declared by both surfaces"
            );
        }
    }

    /// `TOILE-UI-123`: the five zoom actions live in the status bar, beside
    /// the zoom chip that shows the percentage.
    #[test]
    fn the_zoom_actions_home_in_the_status_bar() {
        for id in [ZOOM_IN, ZOOM_OUT, ZOOM_RESET, ZOOM_FIT, ZOOM_TO_SELECTION] {
            assert_eq!(home_of(id), Some(Home::StatusBar), "{id:?}");
        }
    }

    /// `TOILE-UI-124`, `TOILE-UI-364`: undo and redo live in the Ambre menu,
    /// and cut, copy and paste in the context menu under the context menu's
    /// own ids. Select all is a command of the context panel.
    #[test]
    fn undo_and_redo_home_in_the_ambre_menu_and_the_clipboard_in_the_context_menu() {
        assert_eq!(home_of(UNDO), Some(Home::AmbreMenu));
        assert_eq!(home_of(REDO), Some(Home::AmbreMenu));
        for id in [menu::CUT, menu::COPY, menu::PASTE] {
            assert_eq!(home_of(id), Some(Home::ContextMenu), "{id:?}");
        }
        assert_eq!(home_of(menu::SELECT_ALL), Some(Home::ContextPanel));
    }

    /// `TOILE-UI-155`: the presentation entry asks for presentation mode with
    /// focus.
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

    /// The exports are board commands, not view-local intents: the document
    /// lives on the Logic thread, and so does everything that serializes it.
    /// Each lives in the Ambre menu. The PowerPoint entry is the gesture of
    /// `TOILE-IO-126`.
    #[test]
    fn the_exports_home_in_the_ambre_menu_and_resolve_to_board_commands() {
        for id in [EXPORT_SVG, EXPORT_PNG, EXPORT_PDF, EXPORT_PPTX] {
            assert_eq!(home_of(id), Some(Home::AmbreMenu), "{id:?}");
        }
        assert!(matches!(
            menu_bar_edit(EXPORT_PPTX),
            Some(ToileCommand::Interop(InteropRequest::Export(
                ToileExport::Pptx
            )))
        ));
        assert!(matches!(
            menu_bar_edit(EXPORT_SVG),
            Some(ToileCommand::Interop(InteropRequest::Export(
                ToileExport::Svg
            )))
        ));
        assert!(matches!(
            menu_bar_edit(EXPORT_PDF),
            Some(ToileCommand::Interop(InteropRequest::Export(
                ToileExport::Pdf
            )))
        ));
    }

    /// `TOILE-IO-104`, `TOILE-IO-191`: an export reads the board and writes
    /// nothing, so the role gate must let a read-only viewer take a copy out.
    #[test]
    fn exports_are_not_mutating() {
        assert!(!ToileCommand::export(ToileExport::Svg).is_mutating());
        assert!(!ToileCommand::export(ToileExport::Pdf).is_mutating());
        assert!(!ToileCommand::export(ToileExport::Pptx).is_mutating());
        assert!(!ToileCommand::export(ToileExport::Showcase).is_mutating());
        assert!(!ToileCommand::export(ToileExport::Png(PngScope::Board)).is_mutating());
    }

    /// `TOILE-IO-096`: the Ambre menu offers the showcase page, and the entry
    /// resolves to its own board command, never to another export's.
    #[test]
    fn the_ambre_menu_offers_the_showcase_page_as_its_own_gesture() {
        assert_eq!(home_of(EXPORT_SHOWCASE), Some(Home::AmbreMenu));
        assert!(matches!(
            menu_bar_edit(EXPORT_SHOWCASE),
            Some(ToileCommand::Interop(InteropRequest::Export(
                ToileExport::Showcase
            )))
        ));
    }

    /// `TOILE-IO-192`: the Ambre menu offers the board's image, and the entry
    /// resolves to the board command the Logic thread serves.
    #[test]
    fn the_ambre_menu_offers_the_board_as_png() {
        assert_eq!(home_of(EXPORT_PNG), Some(Home::AmbreMenu));
        assert!(matches!(
            menu_bar_edit(EXPORT_PNG),
            Some(ToileCommand::Interop(InteropRequest::Export(
                ToileExport::Png(PngScope::Board)
            )))
        ));
    }

    /// The two placement entries are UI-only intents (`TOILE-086`,
    /// `TOILE-087`). The shell owns the dialog, and the board is handed the
    /// answer, an address or a document, never the gesture. Each inserts an
    /// object, so it lives in the creation bar.
    #[test]
    fn the_placement_entries_are_shell_intents() {
        for id in [EMBED_WEB, EMBED_DOC] {
            assert_eq!(home_of(id), Some(Home::CreationBar), "{id:?}");
            assert!(
                menu_bar_edit(id).is_none(),
                "{id:?} must resolve in the shell, not to a board command"
            );
        }
    }

    /// `TOILE-UI-417`: the link chooser is a shell intent for the same reason
    /// the embed entries are: the shell owns the dialog. Clearing is not -- it
    /// names no target, so the view sends it straight to the board. Both act
    /// on the selection, so both live in the context panel.
    #[test]
    fn the_link_entries_home_in_the_context_panel() {
        assert_eq!(home_of(LINK_SET), Some(Home::ContextPanel));
        assert_eq!(home_of(LINK_CLEAR), Some(Home::ContextPanel));
        assert!(
            menu_bar_edit(LINK_SET).is_none(),
            "the chooser resolves in the shell"
        );
        assert!(
            menu_bar_edit(LINK_CLEAR).is_none(),
            "clearing needs the selection, which an id does not carry"
        );
    }

    /// `TOILE-UI-125`: the group, z-order and lock commands live in the
    /// context panel, under the ids the context menu emits.
    #[test]
    fn the_group_order_and_lock_commands_home_in_the_context_panel() {
        for id in [
            menu::GROUP,
            menu::UNGROUP,
            menu::BRING_FRONT,
            menu::FORWARD,
            menu::BACKWARD,
            menu::SEND_BACK,
            menu::LOCK,
            menu::UNLOCK,
        ] {
            assert_eq!(home_of(id), Some(Home::ContextPanel), "{id:?}");
        }
    }

    /// `TOILE-UI-355`, `TOILE-UI-375`: the six alignments and the two
    /// distributions live in the context panel. Each resolves against the
    /// selection rather than to a direct board command.
    #[test]
    fn the_align_entries_home_in_the_context_panel() {
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
            assert_eq!(home_of(id), Some(Home::ContextPanel), "{id:?}");
            assert!(
                menu_bar_edit(id).is_none(),
                "{id:?} needs the selection an id does not carry"
            );
        }
    }
}
