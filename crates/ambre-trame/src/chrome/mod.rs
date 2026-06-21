// ambre-trame/src/chrome/mod.rs
// ============================================================================
// Trame-SPECIFIC chrome (ambre-spec-architecture.md, 1: per-app chrome lives in
// the app crate, NOT the launcher). Built on the app-agnostic `ambre-chrome`
// toolkit, this module owns everything the launcher used to hard-code about
// Trame:
//
//   - the format `CommandId`s (Bold/Italic) and their keybindings;
//   - the CommandId -> `AppCommand` mapping (UI affordance -> IPC payload);
//   - the inbound `UiEvent` -> command-store mirror (authoritative toggle state);
//   - the toolbar markup and the status label.
//
// The launcher (`ambre-app`) stays app-agnostic: it owns the generic AppShell,
// the `CommandStates` store, and the per-frame relay, and reaches into this
// module for the Trame pieces (`COMMANDS`, `bind_keys`, `command_to_app`,
// `mirror_event`, the `<FormatBar>` / `<SelectionBar>` components, ...).
//
// Wasm-only, like the rest of the chrome stack (leptos CSR + DOM).
//
// Split across submodules (all part of the same flat `chrome::` API via the
// re-exports below, so call sites name `chrome::FormatBar`, `chrome::BOLD`, ...):
//   - `commands`    : command ids, keymap, dispatch, inbound-event mirroring;
//   - `style`       : `Toggles` / `StyleState` and the style-event mirror;
//   - `bars`        : the `FormatBar` toolbar;
//   - `popovers`    : the borders / colour / filter popovers;
//   - `formula_bar` : the editable formula bar;
//   - `color`       : shared hex/theme colour helpers (module-private).
// ============================================================================

mod bars;
mod color;
mod commands;
mod find_replace;
mod formula_bar;
mod popovers;
mod status;
mod style;

pub use bars::FormatBar;
pub use commands::{
    bind_keys, command_to_app, context_entries, init_command_states, mirror_event, new_doc_label,
    notify_event, status_label, BOLD, CLEAR_FILTER, COMMANDS, COPY, CUT, DELETE_COLS, DELETE_ROWS,
    DOC_TYPE, EXPORT_CSV, FILTER_COLUMN, FIND, IMPORT_CSV, INSERT_COL_LEFT, INSERT_COL_RIGHT,
    INSERT_ROW_ABOVE,
    INSERT_ROW_BELOW, ITALIC, MERGE, PASTE, REDO, REPLACE, SORT_RANGE_ASC, SORT_RANGE_DESC,
    SORT_SHEET_ASC, SORT_SHEET_DESC, UNDO, UNMERGE,
};
pub use find_replace::{FindMode, FindQuery, FindReplace, FindReplaceState, ReplaceRequest};
pub use formula_bar::FormulaBar;
pub use popovers::{FilterPopover, FilterPopoverState};
pub use status::{SelStats, StatusBar};
pub use style::{apply_style_event, StyleState, Toggles};
