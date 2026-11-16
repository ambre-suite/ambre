// ambre-toile/src/ui/commands.rs
// ============================================================================
// Toile's command registry: one `Command` per affordance (`ambre-spec-commands.md`,
// UI-THREAD-019).
//
// The identities stay in the module that resolves them: `ui::menu` beside
// `resolve_menu_command`, `ui::menu_bar` beside `menu_bar_edit`, `ui::palette`
// beside `tool_for_command`. This file describes them, that is the label, the icon,
// the chord the affordance announces, the category. Every view reads a word from
// here: the context menu, the menu bar, the tool palette.
//
// The registry carries `StaticKey<Icons>`, a string. `chrome::icons` resolves it to the
// concrete `icondata::Icon`. That keeps `ambre-commands` a leaf with an empty
// dependency table (UI-THREAD-027), and keeps Toile's icon set out of the shared
// crate.
//
// A declared chord is what the affordance announces, not a claim about who routes
// it. Toile binds Ctrl+F and the bare tool letters in the launcher keymap. The
// board chords (undo, redo, clipboard, nudge) are decoded canvas-side in
// `ToileView::handle_key`, and the menu bar displays them. Undo, Redo and the three
// clipboard intents take the chord lists the shared crate declares. The two
// applications then announce the same chords for the same operation, and Ctrl+Y
// redoes on both.
// ============================================================================

use ambre_commands::{Command, Key, KeyBinding, KeyChar, Locale, ModifierKeys, shared};
use ambre_core::{Categories, Commands, Icons, StaticKey};

use crate::ui::find::FIND;
use crate::ui::menu;
use crate::ui::menu_bar;
use crate::ui::palette;

/// Describe one command per line: the name of its title function, the identity
/// it describes, its label, its icon, the chord it announces, its category.
///
/// A macro because `Command::title` is a plain `fn` pointer: it captures
/// nothing, so every command needs a function item of its own. Generating them
/// from one line each is what makes the single declaration site real. A second
/// label for a command cannot be written without a second line here.
macro_rules! describe {
    ($($title_fn:ident: $id:expr, $label:literal, $icon:expr, $chords:expr, $category:expr;)*) => {
        // The generated title functions. Named after the identities, so the
        // module carries the case exemption rather than every call site.
        #[expect(
            non_snake_case,
            reason = "each title function takes the name of its command constant"
        )]
        mod titles {
            use super::Locale;
            // One language ships today and nothing supplies an active locale,
            // so the label is the same for both variants
            // (`ambre-spec-commands.md` section 5).
            $(pub fn $title_fn(_: &Locale) -> ambre_core::Validated<ambre_commands::CommandTitle> { ambre_core::Validated::new($label) })*
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

const CLIPBOARD: StaticKey<Categories> = StaticKey::<Categories>::new("clipboard");
const HISTORY: StaticKey<Categories> = StaticKey::<Categories>::new("history");
const OBJECT: StaticKey<Categories> = StaticKey::<Categories>::new("object");
const REACTION: StaticKey<Categories> = StaticKey::<Categories>::new("reaction");
const SELECTION: StaticKey<Categories> = StaticKey::<Categories>::new("selection");
const VIEW: StaticKey<Categories> = StaticKey::<Categories>::new("view");
const TOOL: StaticKey<Categories> = StaticKey::<Categories>::new("tool");
const PRESENT: StaticKey<Categories> = StaticKey::<Categories>::new("present");
const FIND_REPLACE: StaticKey<Categories> = StaticKey::<Categories>::new("find");
/// Bringing a file onto the board, and taking the board out as one.
const FILE: StaticKey<Categories> = StaticKey::<Categories>::new("file");
/// Commands that belong to the application shell rather than the board.
const GENERAL: StaticKey<Categories> = StaticKey::<Categories>::new("general");

/// The Delete accelerator: the bare `Del` key, matching the view's shortcut.
const DEL: KeyBinding = KeyBinding::new(ModifierKeys::NONE, Key::Delete);

/// A bare letter, the de-facto whiteboard tool chord (`toile-tools`).
const fn letter(c: KeyChar) -> KeyBinding {
    KeyBinding::new(ModifierKeys::NONE, Key::Char(c))
}

/// Open the command palette. A UI-only intent the launcher intercepts, and the
/// one command that belongs to no surface: it is the surface
/// (`ambre-spec-chrome.md`, `CHROME-008`).
pub const PALETTE: StaticKey<Commands> = StaticKey::<Commands>::new("toile.palette.open");

// The labels are the ones the chrome shows today. They are the context-menu
// entries of `chrome::menu`, the dropdown rows of `ui::menu_bar`, the zoom
// cluster and the tool buttons of `chrome::palette`. Nine of them were written
// twice, once for the menu bar and once for the right-click menu.
describe! {
    CUT: menu::CUT, "Cut", Some(StaticKey::<Icons>::new("content-cut")), shared::CUT_CHORDS, CLIPBOARD;
    COPY: menu::COPY, "Copy", Some(StaticKey::<Icons>::new("content-copy")), shared::COPY_CHORDS, CLIPBOARD;
    PASTE: menu::PASTE, "Paste", Some(StaticKey::<Icons>::new("content-paste")), shared::PASTE_CHORDS, CLIPBOARD;
    DUPLICATE: menu::DUPLICATE, "Duplicate", Some(StaticKey::<Icons>::new("content-duplicate")), &[KeyBinding::primary(KeyChar::D)], CLIPBOARD;
    DELETE: menu::DELETE, "Delete", Some(StaticKey::<Icons>::new("delete")), &[DEL], CLIPBOARD;
    GROUP: menu::GROUP, "Group", Some(StaticKey::<Icons>::new("group")), &[KeyBinding::primary(KeyChar::G)], OBJECT;
    UNGROUP: menu::UNGROUP, "Ungroup", Some(StaticKey::<Icons>::new("ungroup")), &[KeyBinding::primary_shift(KeyChar::G)], OBJECT;
    BRING_FRONT: menu::BRING_FRONT, "Bring to front", Some(StaticKey::<Icons>::new("arrange-bring-to-front")), &[], OBJECT;
    FORWARD: menu::FORWARD, "Bring forward", Some(StaticKey::<Icons>::new("arrange-bring-forward")), &[], OBJECT;
    BACKWARD: menu::BACKWARD, "Send backward", Some(StaticKey::<Icons>::new("arrange-send-backward")), &[], OBJECT;
    SEND_BACK: menu::SEND_BACK, "Send to back", Some(StaticKey::<Icons>::new("arrange-send-to-back")), &[], OBJECT;
    LOCK: menu::LOCK, "Lock", Some(StaticKey::<Icons>::new("lock")), &[], OBJECT;
    UNLOCK: menu::UNLOCK, "Unlock", Some(StaticKey::<Icons>::new("lock-open-variant")), &[], OBJECT;
    ALIGN_LEFT: menu::ALIGN_LEFT, "Align left", Some(StaticKey::<Icons>::new("align-horizontal-left")), &[], OBJECT;
    ALIGN_HCENTER: menu::ALIGN_HCENTER, "Align horizontal centres", Some(StaticKey::<Icons>::new("align-horizontal-center")), &[], OBJECT;
    ALIGN_RIGHT: menu::ALIGN_RIGHT, "Align right", Some(StaticKey::<Icons>::new("align-horizontal-right")), &[], OBJECT;
    ALIGN_TOP: menu::ALIGN_TOP, "Align top", Some(StaticKey::<Icons>::new("align-vertical-top")), &[], OBJECT;
    ALIGN_VCENTER: menu::ALIGN_VCENTER, "Align vertical centres", Some(StaticKey::<Icons>::new("align-vertical-center")), &[], OBJECT;
    ALIGN_BOTTOM: menu::ALIGN_BOTTOM, "Align bottom", Some(StaticKey::<Icons>::new("align-vertical-bottom")), &[], OBJECT;
    DISTRIBUTE_H: menu::DISTRIBUTE_H, "Distribute horizontally", Some(StaticKey::<Icons>::new("align-horizontal-distribute")), &[], OBJECT;
    DISTRIBUTE_V: menu::DISTRIBUTE_V, "Distribute vertically", Some(StaticKey::<Icons>::new("align-vertical-distribute")), &[], OBJECT;
    ADD_TO_FRAME: menu::ADD_TO_FRAME, "Add to frame", Some(StaticKey::<Icons>::new("image-frame")), &[], OBJECT;
    REMOVE_FROM_FRAME: menu::REMOVE_FROM_FRAME, "Remove from frame", Some(StaticKey::<Icons>::new("selection-drag")), &[], OBJECT;
    EXPORT_FRAME_PNG: menu::EXPORT_FRAME_PNG, "Export frame as PNG", Some(StaticKey::<Icons>::new("file-image-outline")), &[], FILE;
    SELECT_ALL: menu::SELECT_ALL, "Select all", Some(StaticKey::<Icons>::new("select-all")), &[KeyBinding::primary(KeyChar::A)], SELECTION;
    REACT_THUMBSUP: menu::REACT_THUMBSUP, "React: thumbs up", Some(StaticKey::<Icons>::new("thumb-up")), &[], REACTION;
    REACT_HEART: menu::REACT_HEART, "React: heart", Some(StaticKey::<Icons>::new("heart")), &[], REACTION;
    REACT_CHECK: menu::REACT_CHECK, "React: check", Some(StaticKey::<Icons>::new("check")), &[], REACTION;
    REACT_PARTY: menu::REACT_PARTY, "React: celebrate", Some(StaticKey::<Icons>::new("party-popper")), &[], REACTION;
    REACT_CLEAR: menu::REACT_CLEAR, "Clear my reaction", Some(StaticKey::<Icons>::new("close")), &[], REACTION;
    UNDO: menu_bar::UNDO, "Undo", None, shared::UNDO_CHORDS, HISTORY;
    REDO: menu_bar::REDO, "Redo", None, shared::REDO_CHORDS, HISTORY;
    ZOOM_IN: menu_bar::ZOOM_IN, "Zoom in", Some(StaticKey::<Icons>::new("plus")), &[], VIEW;
    ZOOM_OUT: menu_bar::ZOOM_OUT, "Zoom out", Some(StaticKey::<Icons>::new("minus")), &[], VIEW;
    ZOOM_RESET: menu_bar::ZOOM_RESET, "Zoom to 100%", None, &[], VIEW;
    ZOOM_FIT: menu_bar::ZOOM_FIT, "Fit to content", Some(StaticKey::<Icons>::new("fit-to-screen-outline")), &[], VIEW;
    ZOOM_TO_SELECTION: menu_bar::ZOOM_TO_SELECTION, "Zoom to selection", Some(StaticKey::<Icons>::new("selection-search")), &[], VIEW;
    POINTER_AUTO: menu_bar::POINTER_AUTO, "Pointer: automatic", None, &[], VIEW;
    POINTER_MOUSE: menu_bar::POINTER_MOUSE, "Pointer: mouse (wheel zooms)", None, &[], VIEW;
    POINTER_TRACKPAD: menu_bar::POINTER_TRACKPAD, "Pointer: trackpad (wheel pans)", None, &[], VIEW;
    PRESENT_START: menu_bar::PRESENT, "Start presentation", None, &[], PRESENT;
    IMPORT_FILE: menu_bar::IMPORT_FILE, "Import a file...", Some(StaticKey::<Icons>::new("file-import-outline")), &[], FILE;
    EMBED_WEB: menu_bar::EMBED_WEB, "Embed a web page...", Some(StaticKey::<Icons>::new("web")), &[], FILE;
    EMBED_DOC: menu_bar::EMBED_DOC, "Embed a document...", Some(StaticKey::<Icons>::new("file-document-outline")), &[], FILE;
    LINK_SET: menu_bar::LINK_SET, "Link...", Some(StaticKey::<Icons>::new("link-variant")), &[], OBJECT;
    LINK_CLEAR: menu_bar::LINK_CLEAR, "Remove link", Some(StaticKey::<Icons>::new("link-variant-off")), &[], OBJECT;
    SAVE_FILE: menu_bar::SAVE_FILE, "Save to a file (.ambre, not encrypted)", Some(StaticKey::<Icons>::new("content-save")), &[], FILE;
    SAVE_SEALED_FILE: menu_bar::SAVE_SEALED_FILE, "Save to an encrypted file (.ambre)...", Some(StaticKey::<Icons>::new("lock-outline")), &[], FILE;
    EXPORT_SVG: menu_bar::EXPORT_SVG, "Export as SVG", Some(StaticKey::<Icons>::new("file-export-outline")), &[], FILE;
    EXPORT_PNG: menu_bar::EXPORT_PNG, "Export board as PNG", Some(StaticKey::<Icons>::new("file-image-outline")), &[], FILE;
    EXPORT_PDF: menu_bar::EXPORT_PDF, "Export deck as PDF", Some(StaticKey::<Icons>::new("file-pdf-box")), &[], FILE;
    EXPORT_PPTX: menu_bar::EXPORT_PPTX, "Export deck as PowerPoint", Some(StaticKey::<Icons>::new("file-export-outline")), &[], FILE;
    EXPORT_SHOWCASE: menu_bar::EXPORT_SHOWCASE, "Export a showcase page (.html, not encrypted, readable by anyone)", Some(StaticKey::<Icons>::new("file-export-outline")), &[], FILE;
    PRESENT_DECK: menu_bar::DECK, "Order the deck", Some(StaticKey::<Icons>::new("view-carousel-outline")), &[], PRESENT;
    TOOL_SELECT: palette::TOOL_SELECT, "Select", Some(StaticKey::<Icons>::new("cursor-default")), &[letter(KeyChar::V)], TOOL;
    TOOL_PEN: palette::TOOL_PEN, "Pen", Some(StaticKey::<Icons>::new("vector-polyline")), &[letter(KeyChar::P)], TOOL;
    TOOL_SHAPE: palette::TOOL_SHAPE, "Shape", Some(StaticKey::<Icons>::new("shape-outline")), &[letter(KeyChar::R)], TOOL;
    TOOL_CONNECTOR: palette::TOOL_CONNECTOR, "Connector", Some(StaticKey::<Icons>::new("vector-line")), &[letter(KeyChar::C)], TOOL;
    TOOL_DRAW: palette::TOOL_DRAW, "Draw", Some(StaticKey::<Icons>::new("draw")), &[letter(KeyChar::D)], TOOL;
    TOOL_STICKY: palette::TOOL_STICKY, "Sticky note", Some(StaticKey::<Icons>::new("note-outline")), &[letter(KeyChar::N)], TOOL;
    TOOL_TEXT: palette::TOOL_TEXT, "Text", Some(StaticKey::<Icons>::new("format-text")), &[letter(KeyChar::T)], TOOL;
    TOOL_COMMENT: palette::TOOL_COMMENT, "Comment", Some(StaticKey::<Icons>::new("comment-outline")), &[letter(KeyChar::M)], TOOL;
    TOOL_LASER: palette::TOOL_LASER, "Highlighter", Some(StaticKey::<Icons>::new("cursor-pointer")), &[letter(KeyChar::L)], TOOL;
    TOOL_HAND: palette::TOOL_HAND, "Hand", Some(StaticKey::<Icons>::new("hand-back-right")), &[letter(KeyChar::H)], TOOL;
    FIND_OPEN: FIND, "Find", None, &[KeyBinding::primary(KeyChar::F)], FIND_REPLACE;
    COMMENTS_OPEN: menu_bar::COMMENTS, "Comments", Some(StaticKey::<Icons>::new("comment-multiple-outline")), &[], VIEW;
    PALETTE_OPEN: PALETTE, "Command palette", None, &[KeyBinding::primary(KeyChar::K)], GENERAL;
}

/// The label a view shows for `id`, in `locale`. `None` for a command Toile does
/// not declare.
///
/// The one reader of a command's title. A view that wants a label calls this and
/// never writes one of its own (`UI-THREAD-019`).
pub fn title(
    id: StaticKey<Commands>,
    locale: &Locale,
) -> Option<ambre_core::Validated<ambre_commands::CommandTitle>> {
    ambre_commands::find(COMMANDS, id).map(|c| (c.title)(locale))
}

/// The icon handle `id` declares, if it has one.
pub fn icon(id: StaticKey<Commands>) -> Option<StaticKey<Icons>> {
    ambre_commands::find(COMMANDS, id).and_then(|c| c.icon)
}

/// The chord `id` announces, if it has one. What a menu displays beside a row;
/// which chords the launcher keymap actually routes is `chrome::bind_keys`.
pub fn chord(id: StaticKey<Commands>) -> Option<KeyBinding> {
    ambre_commands::find(COMMANDS, id).and_then(|c| c.chord())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each command is described once, with a title in every locale, a
    /// group, no blank icon and no chord another command holds
    /// (`UI-THREAD-019`, `UI-THREAD-037`).
    #[test]
    fn the_table_has_no_defect() {
        assert_eq!(ambre_commands::table_defects(COMMANDS), []);
    }

    /// `DOCFILE-009`: the surface that offers the export states that the file
    /// carries no encryption of its own. The label is where a user reads it
    /// before clicking. Trame asserts the same on its own label -- the rule is
    /// per surface, and the suite has two.
    #[test]
    fn the_save_label_says_the_file_is_not_encrypted() {
        let label = title(menu_bar::SAVE_FILE, &Locale::En).expect("SAVE_FILE is declared");
        assert!(
            label.as_str().contains("not encrypted"),
            "the export surface must state that the file is plaintext: {label}"
        );
    }

    /// `TOILE-IO-096`: before the click, the showcase entry says that the page
    /// leaves the encryption. It is not encrypted, and anyone with the file
    /// reads it. The label is the one string every surface shows.
    #[test]
    fn the_showcase_label_says_the_page_leaves_the_encryption() {
        let label = title(menu_bar::EXPORT_SHOWCASE, &Locale::En).expect("declared");
        let text = label.as_str();
        assert!(text.contains("not encrypted"), "{label}");
        assert!(text.contains("readable by anyone"), "{label}");
    }

    /// A command carries its own label, and a view reads it from here. An empty
    /// title is a command no menu and no palette can show.
    #[test]
    fn every_command_has_a_label() {
        for cmd in COMMANDS {
            let label = (cmd.title)(&Locale::En);
            assert!(!label.as_str().is_empty(), "{:?} has no label", cmd.id);
            assert_eq!(title(cmd.id, &Locale::En), Some(label));
        }
    }

    /// The three clipboard intents are described under the shared identities,
    /// and announce the shared chords, the modern one and the legacy one. A
    /// copy is then one command in every application (`UI-THREAD-011`).
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

    /// `UNDO-011`: redo announces Ctrl+Shift+Z and answers Ctrl+Y as well, the
    /// pair the shared crate decides for every application. The board decodes
    /// it canvas-side, so the table is where Toile carries the decision.
    #[test]
    fn redo_announces_both_chords() {
        let redo = ambre_commands::find(COMMANDS, menu_bar::REDO).expect("described");
        assert_eq!(redo.keybindings, shared::REDO_CHORDS);
        assert_eq!(redo.chord(), Some(KeyBinding::primary_shift(KeyChar::Z)));
        // The chip beside the Undo entry prints the chord the user presses.
        let undo = chord(menu_bar::UNDO).map(|k| k.to_string());
        assert_eq!(undo.as_deref(), Some("Ctrl+Z"));
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
