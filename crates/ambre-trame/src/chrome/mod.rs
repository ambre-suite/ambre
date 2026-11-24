// ambre-trame/src/chrome/mod.rs
// ============================================================================
// Trame-SPECIFIC chrome (ambre-spec-architecture.md: per-app chrome lives in the
// app crate, NOT the launcher). Built on the app-agnostic `ambre-chrome` toolkit,
// this module owns everything the launcher used to hard-code about Trame:
//
//   - the format `StaticKey<Commands>`s (Bold/Italic) and their keybindings;
//   - the StaticKey<Commands> -> `AppCommand` mapping (UI affordance -> IPC payload);
//   - the inbound `UiEvent` -> command-store mirror (authoritative toggle state);
//   - the toolbar markup and the status label.
//
// The launcher (`ambre-app`) stays app-agnostic. It owns the generic AppShell,
// the `CommandStates` store, and the switchboard. It reaches into this module
// for the Trame pieces (`COMMANDS`, `bind_keys`, `TrameHandler`,
// `mirror_event`, the context panel sections, ...).
//
// Wasm-only, like the rest of the chrome stack (leptos CSR + DOM).
//
// Split across submodules (all part of the same flat `chrome::` API via the
// re-exports below, so call sites name `chrome::trame_panel`, `chrome::BOLD`, ...):
//   - `commands`    : command ids, keymap, dispatch, inbound-event mirroring;
//   - `handler`     : what each command means, for the shell (`ARCH-018`);
//   - `style`       : `Toggles` / `StyleState` and the style-event mirror;
//   - `panel`       : the sheet's sections of the context panel;
//   - `popovers`    : the borders / colour / filter popovers;
//   - `formula_bar` : the editable formula bar;
//   - `color`       : shared hex/theme colour helpers (module-private).
// ============================================================================

mod color;
mod commands;
mod find_replace;
mod formula_bar;
mod handler;
mod icons;
mod panel;
mod popovers;
mod relay;
mod sheet_tabs;
mod sort_dialog;
mod status;
mod style;

pub use commands::{
    BOLD, CLEAR_FILTER, COMMANDS, COPY, CUT, CtxScope, DELETE_COLS, DELETE_ROWS, DOC_TYPE,
    EXPORT_CSV, FILTER_COLUMN, FIND, IMPORT_CSV, INSERT_COL_LEFT, INSERT_COL_RIGHT,
    INSERT_FUNCTION, INSERT_ROW_ABOVE, INSERT_ROW_BELOW, ITALIC, MERGE, MenuScope, PALETTE, PASTE,
    REDO, REPLACE, SAVE_FILE, SAVE_SEALED_FILE, SORT_RANGE_ASC, SORT_RANGE_DESC, SORT_RANGE_MULTI,
    SORT_SHEET_ASC, SORT_SHEET_DESC, UNDO, UNMERGE, ZOOM_TO_SELECTION, bind_keys, command_to_app,
    context_entries, context_entries_for, init_command_states, mirror_event, new_doc_label,
    notify_event, status_label,
};
/// The glyph a command of the sheet declares, resolved to its asset.
#[must_use]
pub fn icon_of(command: &ambre_chrome::commands::Command) -> Option<icondata::Icon> {
    command.icon.and_then(icons::resolve)
}

pub use find_replace::{FindMode, FindPosition, FindReplace, FindReplaceState, ReplaceRequest};
pub use formula_bar::FormulaBar;
pub use handler::TrameHandler;
pub use panel::{TramePanelSignals, trame_panel};
pub use popovers::{FilterPopover, FilterPopoverState, SheetFilter};
pub use relay::{TrameIntent, TrameRelay};
pub use sheet_tabs::{ReorderSheet, SheetTabs};
pub use sort_dialog::{SortApply, SortDialog, SortDialogState};
pub use status::{SelStats, StatusBar};
pub use style::{StyleState, Toggles, apply_style_event};
