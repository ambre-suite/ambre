// ambre-toile/src/chrome/mod.rs
// ============================================================================
// Toile-SPECIFIC chrome (ambre-spec-architecture.md §1: per-app chrome lives in
// the app crate, NOT the launcher). Built on the app-agnostic `ambre-chrome`
// toolkit, like `ambre-trame::chrome`. Phase 1 ships the Find panel (§5quater);
// the tool palette, context menu and status bar land alongside their roadmap
// items.
//
// Wasm-only (leptos CSR + DOM), so it is scoped to the wasm32 target and never
// compiled into host logic / render tests. The pure find helpers it relies on
// (`ui::find`) stay in the always-compiled `ui` module so they are host-tested.
// ============================================================================

mod find;
mod menu;
mod menu_bar;
mod palette;

pub use find::{FindPanel, FindQuery};
pub use menu::context_entries_for;
pub use menu_bar::TopMenuBar;
pub use palette::ToolPalette;

// The tool palette's command surface is pure and host-tested in `ui::palette`
// (like `ui::find` / `ui::menu`); the launcher consumes it through these
// re-exports, so it only ever imports from `toile_chrome`.
pub use crate::ui::palette::{
    TOOL_COMMANDS, TOOL_CONNECTOR, TOOL_DRAW, TOOL_HAND, TOOL_PEN, TOOL_SELECT, TOOL_SHAPE,
    TOOL_STICKY, TOOL_TEXT, tool_for_command,
};

use ambre_bus::DocType;
use ambre_chrome::commands::{CommandId, KeyBinding, Keymap};

/// Open the Find panel (§5quater). A UI-only intent: the launcher intercepts it
/// to toggle the panel open rather than dispatching to Logic (the panel emits
/// `ToileCommand::Find` itself).
pub const FIND: CommandId = CommandId("toile.find.open");

/// The document type Toile's landing "new document" button creates.
pub const DOC_TYPE: DocType = DocType::Toile;

/// The label of the landing "new document" button for Toile.
pub fn new_doc_label() -> &'static str {
    "New Toile"
}

/// Bind the Toile command chords into the launcher keymap. Ctrl+F opens Find and
/// the bare-key tool chords (V/P/R/... -> `SetTool`, §5quinquies.A) switch tools;
/// the remaining board shortcuts (undo/redo, clipboard, nudge, ...) are decoded
/// canvas-side in `ToileView::handle_key` (toile-ui §5), not here.
pub fn bind_keys(keymap: &mut Keymap) {
    keymap.bind(KeyBinding::ctrl('f'), FIND);
    crate::ui::palette::bind_tool_keys(keymap);
}
