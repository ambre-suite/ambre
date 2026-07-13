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

pub use find::{FindPanel, FindQuery};

use ambre_chrome::commands::{CommandId, KeyBinding, Keymap};

/// Open the Find panel (§5quater). A UI-only intent: the launcher intercepts it
/// to toggle the panel open rather than dispatching to Logic (the panel emits
/// `ToileCommand::Find` itself).
pub const FIND: CommandId = CommandId("toile.find.open");

/// Bind the Toile command chords into the launcher keymap. Ctrl+F opens Find;
/// the remaining board shortcuts (undo/redo, clipboard, nudge, ...) are decoded
/// canvas-side in `ToileView::handle_key` (toile-ui §5), not here.
pub fn bind_keys(keymap: &mut Keymap) {
    keymap.bind(KeyBinding::ctrl('f'), FIND);
}
