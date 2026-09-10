// ambre-toile/src/ui/commands.rs
// ============================================================================
// Toile's command registry: one `Command` per affordance (`ambre-spec-commands.md`,
// UI-THREAD-019).
//
// The identities stay in the module that resolves them -- `ui::menu` beside
// `resolve_menu_command`, `ui::menu_bar` beside `menu_bar_edit`, `ui::palette`
// beside `tool_for_command` -- and this file describes them: the label, the
// icon, the chord the affordance announces, the category. Every view reads a
// word from here: the context menu, the menu bar, the tool palette.
//
// The registry carries `IconId`, a string. `chrome::icons` resolves it to the
// concrete `icondata::Icon`, which keeps `ambre-commands` a leaf with an empty
// dependency table (UI-THREAD-027) and keeps Toile's icon set out of the shared
// crate.
//
// A declared chord is what the affordance announces, not a claim about who
// routes it. Toile binds Ctrl+F and the bare tool letters in the launcher
// keymap; the board chords (undo, redo, clipboard, nudge) are decoded
// canvas-side in `ToileView::handle_key`, and the menu bar displays them.
// Undo, Redo and the three clipboard intents take the chord lists the shared
// crate declares, so the two applications announce the same chords for the same
// operation and Ctrl+Y redoes on both.
// ============================================================================

use ambre_commands::{
    Command, CommandCategory, CommandId, IconId, Key, KeyBinding, Locale, Modifiers, shared,
};

use crate::ui::find::FIND;
use crate::ui::menu;
use crate::ui::menu_bar;
use crate::ui::palette;

/// Describe one command per line: the name of its title function, the identity
/// it describes, its label, its icon, the chord it announces, its category.
///
/// A macro because `Command::title` is a plain `fn` pointer: it captures
/// nothing, so every command needs a function item of its own. Generating them
/// from one line each is what makes the single declaration site real -- a second
/// label for a command cannot be written without a second line here.
macro_rules! describe {
    ($($title_fn:ident: $id:expr, $label:literal, $icon:expr, $chords:expr, $category:expr;)*) => {
        // The generated title functions. Named after the identities, so the
        // module carries the case exemption rather than every call site.
        #[allow(non_snake_case)]
        mod titles {
            use super::Locale;
            // One language ships today and nothing supplies an active locale,
            // so the label is the same for both variants
            // (`ambre-spec-commands.md` section 5).
            $(pub fn $title_fn(_: &Locale) -> String { $label.into() })*
        }

        /// Every command Toile contributes, described once. Every view of a
        /// command reads its label and its icon here (`UI-THREAD-019`).
        pub const COMMANDS: &[Command] = &[
            $(Command {
                id: $id,
                title: titles::$title_fn,
                icon: $icon,
                keybindings: $chords,
                category: $category,
            },)*
        ];
    };
}

const CLIPBOARD: CommandCategory = CommandCategory("clipboard");
const HISTORY: CommandCategory = CommandCategory("history");
const OBJECT: CommandCategory = CommandCategory("object");
const REACTION: CommandCategory = CommandCategory("reaction");
const SELECTION: CommandCategory = CommandCategory("selection");
const VIEW: CommandCategory = CommandCategory("view");
const TOOL: CommandCategory = CommandCategory("tool");
const PRESENT: CommandCategory = CommandCategory("present");
const FIND_REPLACE: CommandCategory = CommandCategory("find");
/// Bringing a file onto the board, and taking the board out as one.
const FILE: CommandCategory = CommandCategory("file");
/// Commands that belong to the application shell rather than the board.
const GENERAL: CommandCategory = CommandCategory("general");

/// The Delete accelerator: the bare `Del` key, matching the view's shortcut.
const DEL: KeyBinding = KeyBinding::new(Modifiers::NONE, Key::Delete);

/// A bare letter, the de-facto whiteboard tool chord (`toile-tools`).
const fn letter(c: char) -> KeyBinding {
    KeyBinding::new(Modifiers::NONE, Key::char(c))
}

/// Open the command palette. A UI-only intent the launcher intercepts, and the
/// one command that belongs to no surface: it is the surface
/// (`ambre-spec-chrome.md`, `CHROME-008`).
pub const PALETTE: CommandId = CommandId("toile.palette.open");

// The labels are the ones the chrome shows today: the context-menu entries of
// `chrome::menu`, the dropdown rows of `ui::menu_bar`, the zoom cluster and the
// tool buttons of `chrome::palette`. Nine of them were written twice, once for
// the menu bar and once for the right-click menu.
describe! {
    CUT: menu::CUT, "Cut", Some(IconId("content-cut")), shared::CUT_CHORDS, CLIPBOARD;
    COPY: menu::COPY, "Copy", Some(IconId("content-copy")), shared::COPY_CHORDS, CLIPBOARD;
    PASTE: menu::PASTE, "Paste", Some(IconId("content-paste")), shared::PASTE_CHORDS, CLIPBOARD;
    DUPLICATE: menu::DUPLICATE, "Duplicate", Some(IconId("content-duplicate")), &[KeyBinding::primary('d')], CLIPBOARD;
    DELETE: menu::DELETE, "Delete", Some(IconId("delete")), &[DEL], CLIPBOARD;
    GROUP: menu::GROUP, "Group", Some(IconId("group")), &[KeyBinding::primary('g')], OBJECT;
    UNGROUP: menu::UNGROUP, "Ungroup", Some(IconId("ungroup")), &[KeyBinding::primary_shift('g')], OBJECT;
    BRING_FRONT: menu::BRING_FRONT, "Bring to front", Some(IconId("arrange-bring-to-front")), &[], OBJECT;
    FORWARD: menu::FORWARD, "Bring forward", Some(IconId("arrange-bring-forward")), &[], OBJECT;
    BACKWARD: menu::BACKWARD, "Send backward", Some(IconId("arrange-send-backward")), &[], OBJECT;
    SEND_BACK: menu::SEND_BACK, "Send to back", Some(IconId("arrange-send-to-back")), &[], OBJECT;
    LOCK: menu::LOCK, "Lock", Some(IconId("lock")), &[], OBJECT;
    UNLOCK: menu::UNLOCK, "Unlock", Some(IconId("lock-open-variant")), &[], OBJECT;
    ALIGN_LEFT: menu::ALIGN_LEFT, "Align left", Some(IconId("align-horizontal-left")), &[], OBJECT;
    ALIGN_HCENTER: menu::ALIGN_HCENTER, "Align horizontal centres", Some(IconId("align-horizontal-center")), &[], OBJECT;
    ALIGN_RIGHT: menu::ALIGN_RIGHT, "Align right", Some(IconId("align-horizontal-right")), &[], OBJECT;
    ALIGN_TOP: menu::ALIGN_TOP, "Align top", Some(IconId("align-vertical-top")), &[], OBJECT;
    ALIGN_VCENTER: menu::ALIGN_VCENTER, "Align vertical centres", Some(IconId("align-vertical-center")), &[], OBJECT;
    ALIGN_BOTTOM: menu::ALIGN_BOTTOM, "Align bottom", Some(IconId("align-vertical-bottom")), &[], OBJECT;
    DISTRIBUTE_H: menu::DISTRIBUTE_H, "Distribute horizontally", Some(IconId("align-horizontal-distribute")), &[], OBJECT;
    DISTRIBUTE_V: menu::DISTRIBUTE_V, "Distribute vertically", Some(IconId("align-vertical-distribute")), &[], OBJECT;
    ADD_TO_FRAME: menu::ADD_TO_FRAME, "Add to frame", Some(IconId("image-frame")), &[], OBJECT;
    REMOVE_FROM_FRAME: menu::REMOVE_FROM_FRAME, "Remove from frame", Some(IconId("selection-drag")), &[], OBJECT;
    SELECT_ALL: menu::SELECT_ALL, "Select all", Some(IconId("select-all")), &[KeyBinding::primary('a')], SELECTION;
    REACT_THUMBSUP: menu::REACT_THUMBSUP, "React: thumbs up", Some(IconId("thumb-up")), &[], REACTION;
    REACT_HEART: menu::REACT_HEART, "React: heart", Some(IconId("heart")), &[], REACTION;
    REACT_CHECK: menu::REACT_CHECK, "React: check", Some(IconId("check")), &[], REACTION;
    REACT_PARTY: menu::REACT_PARTY, "React: celebrate", Some(IconId("party-popper")), &[], REACTION;
    REACT_CLEAR: menu::REACT_CLEAR, "Clear my reaction", Some(IconId("close")), &[], REACTION;
    UNDO: menu_bar::UNDO, "Undo", None, shared::UNDO_CHORDS, HISTORY;
    REDO: menu_bar::REDO, "Redo", None, shared::REDO_CHORDS, HISTORY;
    ZOOM_IN: menu_bar::ZOOM_IN, "Zoom in", Some(IconId("plus")), &[], VIEW;
    ZOOM_OUT: menu_bar::ZOOM_OUT, "Zoom out", Some(IconId("minus")), &[], VIEW;
    ZOOM_RESET: menu_bar::ZOOM_RESET, "Zoom to 100%", None, &[], VIEW;
    ZOOM_FIT: menu_bar::ZOOM_FIT, "Fit to content", Some(IconId("fit-to-screen-outline")), &[], VIEW;
    ZOOM_TO_SELECTION: menu_bar::ZOOM_TO_SELECTION, "Zoom to selection", Some(IconId("selection-search")), &[], VIEW;
    POINTER_AUTO: menu_bar::POINTER_AUTO, "Pointer: automatic", None, &[], VIEW;
    POINTER_MOUSE: menu_bar::POINTER_MOUSE, "Pointer: mouse (wheel zooms)", None, &[], VIEW;
    POINTER_TRACKPAD: menu_bar::POINTER_TRACKPAD, "Pointer: trackpad (wheel pans)", None, &[], VIEW;
    PRESENT_START: menu_bar::PRESENT, "Start presentation", None, &[], PRESENT;
    IMPORT_FILE: menu_bar::IMPORT_FILE, "Import a file...", Some(IconId("file-import-outline")), &[], FILE;
    EMBED_WEB: menu_bar::EMBED_WEB, "Embed a web page...", Some(IconId("web")), &[], FILE;
    EMBED_DOC: menu_bar::EMBED_DOC, "Embed a document...", Some(IconId("file-document-outline")), &[], FILE;
    LINK_SET: menu_bar::LINK_SET, "Link...", Some(IconId("link-variant")), &[], OBJECT;
    LINK_CLEAR: menu_bar::LINK_CLEAR, "Remove link", Some(IconId("link-variant-off")), &[], OBJECT;
    EXPORT_SVG: menu_bar::EXPORT_SVG, "Export as SVG", Some(IconId("file-export-outline")), &[], FILE;
    EXPORT_PDF: menu_bar::EXPORT_PDF, "Export deck as PDF", Some(IconId("file-pdf-box")), &[], FILE;
    PRESENT_DECK: menu_bar::DECK, "Order the deck", Some(IconId("view-carousel-outline")), &[], PRESENT;
    TOOL_SELECT: palette::TOOL_SELECT, "Select", Some(IconId("cursor-default")), &[letter('v')], TOOL;
    TOOL_PEN: palette::TOOL_PEN, "Pen", Some(IconId("vector-polyline")), &[letter('p')], TOOL;
    TOOL_SHAPE: palette::TOOL_SHAPE, "Shape", Some(IconId("shape-outline")), &[letter('r')], TOOL;
    TOOL_CONNECTOR: palette::TOOL_CONNECTOR, "Connector", Some(IconId("vector-line")), &[letter('c')], TOOL;
    TOOL_DRAW: palette::TOOL_DRAW, "Draw", Some(IconId("draw")), &[letter('d')], TOOL;
    TOOL_STICKY: palette::TOOL_STICKY, "Sticky note", Some(IconId("note-outline")), &[letter('n')], TOOL;
    TOOL_TEXT: palette::TOOL_TEXT, "Text", Some(IconId("format-text")), &[letter('t')], TOOL;
    TOOL_COMMENT: palette::TOOL_COMMENT, "Comment", Some(IconId("comment-outline")), &[letter('m')], TOOL;
    TOOL_LASER: palette::TOOL_LASER, "Highlighter", Some(IconId("cursor-pointer")), &[letter('l')], TOOL;
    TOOL_HAND: palette::TOOL_HAND, "Hand", Some(IconId("hand-back-right")), &[letter('h')], TOOL;
    FIND_OPEN: FIND, "Find", None, &[KeyBinding::primary('f')], FIND_REPLACE;
    COMMENTS_OPEN: menu_bar::COMMENTS, "Comments", Some(IconId("comment-multiple-outline")), &[], VIEW;
    PALETTE_OPEN: PALETTE, "Command palette", None, &[KeyBinding::primary('k')], GENERAL;
}

/// The label a view shows for `id`, in `locale`. `None` for a command Toile does
/// not declare.
///
/// The one reader of a command's title. A view that wants a label calls this and
/// never writes one of its own (`UI-THREAD-019`).
pub fn title(id: CommandId, locale: &Locale) -> Option<String> {
    ambre_commands::find(COMMANDS, id).map(|c| (c.title)(locale))
}

/// The icon handle `id` declares, if it has one.
pub fn icon(id: CommandId) -> Option<IconId> {
    ambre_commands::find(COMMANDS, id).and_then(|c| c.icon)
}

/// The chord `id` announces, if it has one. What a menu displays beside a row;
/// which chords the launcher keymap actually routes is `chrome::bind_keys`.
pub fn chord(id: CommandId) -> Option<KeyBinding> {
    ambre_commands::find(COMMANDS, id).and_then(|c| c.chord())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is the single declaration site, so two commands that share
    /// an identity would make one of them unreachable through `find`.
    #[test]
    fn every_identity_is_declared_once() {
        for (i, cmd) in COMMANDS.iter().enumerate() {
            let twin = COMMANDS.iter().skip(i + 1).find(|c| c.id == cmd.id);
            assert!(twin.is_none(), "{:?} is declared twice", cmd.id);
        }
    }

    /// A command carries its own label, and a view reads it from here. An empty
    /// title is a command no menu and no palette can show.
    #[test]
    fn every_command_has_a_label() {
        for cmd in COMMANDS {
            let label = (cmd.title)(&Locale::En);
            assert!(!label.is_empty(), "{:?} has no label", cmd.id);
            assert_eq!(title(cmd.id, &Locale::En), Some(label));
        }
    }

    /// The three clipboard intents are described under the shared identities
    /// and announce the shared chords, the modern one and the legacy one, so a
    /// copy is one command in every application (`UI-THREAD-011`).
    #[test]
    fn the_clipboard_intents_carry_the_shared_identity() {
        let intents = [
            (shared::CUT, shared::CUT_CHORDS),
            (shared::COPY, shared::COPY_CHORDS),
            (shared::PASTE, shared::PASTE_CHORDS),
        ];
        for (id, chords) in intents {
            let cmd = ambre_commands::find(COMMANDS, id).expect("described");
            assert_eq!(cmd.keybindings, chords, "{id:?} announces both chords");
        }
        // And no keymap seeded from this table resolves one of them: the
        // browser raises the event itself.
        let keymap = ambre_commands::Keymap::from_commands([COMMANDS]);
        for (_, chords) in intents {
            for chord in chords {
                assert_eq!(
                    keymap.resolve(*chord),
                    None,
                    "{chord} must reach no command"
                );
            }
        }
    }

    /// Redo announces Ctrl+Shift+Z and answers Ctrl+Y as well, the pair the
    /// shared crate decides for every application. The board decodes it
    /// canvas-side, so the table is where Toile carries the decision.
    #[test]
    fn redo_announces_both_chords() {
        let redo = ambre_commands::find(COMMANDS, menu_bar::REDO).expect("described");
        assert_eq!(redo.keybindings, shared::REDO_CHORDS);
        assert_eq!(redo.chord(), Some(KeyBinding::primary_shift('z')));
    }

    /// Every identity the three surfaces expose is described. An identity with
    /// no description is a row no view can label.
    #[test]
    fn every_exposed_identity_is_described() {
        let exposed = menu::COMMANDS
            .iter()
            .chain(menu_bar::MENU_BAR_COMMANDS)
            .chain(palette::TOOL_COMMANDS)
            .chain(std::iter::once(&FIND));
        for id in exposed {
            assert!(
                title(*id, &Locale::En).is_some(),
                "{id:?} is exposed and not described"
            );
        }
    }

    /// The chords the table declares must not collide: a chord resolves to at
    /// most one command (`UI-THREAD-026`).
    #[test]
    fn declared_chords_are_distinct() {
        let chords: Vec<KeyBinding> = COMMANDS
            .iter()
            .flat_map(|c| c.keybindings.iter().copied())
            .collect();
        for (i, chord) in chords.iter().enumerate() {
            assert!(
                !chords[i + 1..].contains(chord),
                "{chord:?} is declared twice"
            );
        }
    }
}
