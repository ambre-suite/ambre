// ambre-toile/src/chrome/mod.rs
// ============================================================================
// Toile-SPECIFIC chrome (ambre-spec-architecture.md: per-app chrome lives in
// the app crate, NOT the launcher). Built on the app-agnostic `ambre-chrome`
// toolkit, like `ambre-trame::chrome`. Phase 1 ships the Find panel;
// the tool palette, context menu and status bar land alongside their roadmap
// items.
//
// Wasm-only (leptos CSR + DOM), so it is scoped to the wasm32 target and never
// compiled into host logic / render tests. The pure find helpers it relies on
// (`ui::find`) stay in the always-compiled `ui` module so they are host-tested.
// ============================================================================

mod commands;
mod comments;
mod deck;
mod find;
mod floatbar;
mod handler;
pub use handler::{ToileDialogs, ToileHandler, install_file_drop};
mod menu;
mod menu_bar;
mod notify;
pub use notify::notify_event;
mod palette;
mod presenter;
mod status;
mod style;

pub use commands::{COMMANDS, Toggles, init_command_states, mirror_event};

pub use comments::{CommentComposer, CommentsPanel};
pub use deck::DeckPanel;

// The deck panel's pure model (the label and the three order edits) is
// host-tested in `ui::deck`; the launcher names it through `toile_chrome`.
pub use crate::ui::deck::{DeckIntent, DeckSignals, Move as DeckMove};

// What the board reads from the file system is pure and host-tested in
// `ui::interop`. The shell asks the board rather than holding a format list of
// its own (`toile-import-export`).
pub use crate::ui::embed::{AddressError, EmbedTarget, PlacedEmbed, normalize_web_address};
pub use crate::ui::interop::{DroppedFile, IMPORT_ACCEPT, accepts as accepts_file};
pub use crate::ui::link::{LinkEdit, ShellNav};
pub use find::{FindPanel, FindQuery};

// The pins, the pick, the badge and the panel's filter are pure and host-tested
// in `ui::comments` (like `ui::find`); the launcher names them through
// `toile_chrome`.
pub use crate::ui::comments::{CommentDraft, CommentIntent, CommentSignals, ThreadFilter};
pub use floatbar::SelectionToolbar;
pub use menu::context_entries_for;
pub use menu_bar::TopMenuBar;
mod icons;
pub use palette::ToolPalette;
pub use presenter::{PresenterIntent, PresenterPanel};
pub use status::StatusBar;

// The presenter panel's pure model (counter + notes diff) is host-tested in
// `ui::presenter`, like `ui::find`. Re-export it so the launcher and tests
// name it through `toile_chrome`.
pub use crate::ui::presenter::{notes_delta, slide_counter_label};
pub use style::StyleInspector;

// The inspector's pure model is host-tested in `ui::style` (like `ui::status` /
// `ui::palette`); re-export it so the launcher names it through `toile_chrome`.
pub use crate::ui::style::StyleModel;

// The status-bar aggregates model is pure and host-tested in `ui::status`
// (like `ui::find`); re-export it so the launcher names it through `toile_chrome`.
pub use crate::ui::status::SelStats;

// The floating selection toolbar's model + single-field style payload are pure
// and host-tested in `ui::floatbar` (like `ui::status` / `ui::style`); the
// launcher names them through `toile_chrome`.
pub use crate::ui::floatbar::{FloatBar, StyleEdit};

// The tool palette's command surface is pure and host-tested in
// `ui::palette`, like `ui::find` / `ui::menu`. The launcher consumes it
// through these re-exports, so it only ever imports from `toile_chrome`.
pub use crate::ui::palette::{
    TOOL_COMMANDS, TOOL_CONNECTOR, TOOL_DRAW, TOOL_HAND, TOOL_PEN, TOOL_SELECT, TOOL_SHAPE,
    TOOL_STICKY, TOOL_TEXT, tool_for_command,
};

use ambre_bus::DocType;
use ambre_chrome::commands::{CommandId, KeyBinding, Keymap};

pub use crate::ui::commands::PALETTE;
pub use crate::ui::find::FIND;
pub use crate::ui::menu_bar::{
    EMBED_DOC, EMBED_WEB, IMPORT_FILE, LINK_CLEAR, LINK_SET, REDO, SAVE_SEALED_FILE, UNDO,
};

/// The document type Toile's landing "new document" button creates.
pub const DOC_TYPE: DocType = DocType::Toile;

/// The label of the landing "new document" button for Toile.
pub fn new_doc_label() -> &'static str {
    "New Toile"
}

/// Bind the Toile command chords into the launcher keymap. Ctrl+F opens Find and
/// the bare-key tool chords (V/P/R/... -> `SetTool`,) switch tools; the
/// remaining board shortcuts (undo/redo, clipboard, nudge, ...) are decoded
/// canvas-side in `ToileView::handle_key` (toile-tools), not here.
///
/// Every chord comes from the registry, which declares what each affordance
/// announces. Only these two families are routed through the keymap. Binding the
/// board chords here too would decode them twice, once here and once on the
/// canvas (`ambre-spec-commands.md` section 5).
pub fn bind_keys(keymap: &mut Keymap) {
    for id in [FIND, crate::ui::commands::PALETTE] {
        if let Some(chord) = crate::ui::commands::chord(id) {
            keymap.bind(chord, id);
        }
    }
    crate::ui::palette::bind_tool_keys(keymap);
}
