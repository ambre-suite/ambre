// ambre-chrome/src/lib.rs
// ============================================================================
// Shared, app-agnostic Leptos chrome toolkit (ambre-spec-chrome.md).
//
// This crate owns the chrome components reused across every Ambre app:
// AppShell, command palette, menus/context menus, dialogs, toolbars, toasts,
// sidebar, status bar -- each a *view* over the command registry from
// `ambre-commands`. App-SPECIFIC chrome (Trame's formula bar, a cell-format
// dialog) lives in the app crate and is built on top of this toolkit.
//
// The toolkit never reaches across the canvas/DOM boundary: it invokes
// `CommandId`s and reflects state via Leptos signals (ambre-spec-ui.md).
// It depends on no app crate (dependency inversion, symmetric to `AppView`).
//
// Module map. What is missing is `ambre-spec-chrome.md`'s own gap list, and the
// schedule for it is ambre-roadmap.md; neither belongs here.
//   shortcut     -- keybinding presentation (the <Kbd> component)
//   app_shell    -- CSS-grid shell: header, command bar, status bar, canvas
//   command      -- command-state store + keybinding manager
//   dialog       -- native <dialog> modal (rAF-suspending)
//   context_menu -- canvas right-click DOM menu (flip, keyboard nav)
//   palette      -- the command palette: every declared command, filtered
//   passphrase   -- the passphrase prompt of a sealed `.ambre` file
//   toast/status/format/font/color/debounce -- the shared controls
// ============================================================================

// Re-export the registry so app/chrome code has a single `ambre_chrome`-rooted
// path to command types when convenient.
pub use ambre_commands as commands;

mod app_shell;
mod color;
mod command;
mod context_menu;
mod debounce;
mod dialog;
mod font;
mod format;
mod palette;
mod passphrase;
mod shortcut;
mod status;
mod toast;

pub use app_shell::{AppShell, FormulaBar, Header, StatusBar, Tabs, Toolbar};
pub use color::{ColorPopover, color_hex, parse_hex_color};
pub use command::{CommandStates, keybinding_from_event};
pub use context_menu::{ContextMenu, MenuEntry};
pub use debounce::Debouncer;
pub use dialog::Dialog;
pub use font::{FONT_FAMILIES, FontPicker};
pub use format::{FONT_SIZES, FontSizeSelect, FormatToggle};
pub use palette::{CommandPalette, filter as filter_commands, matches as command_matches};
pub use passphrase::{PassphraseDialog, PassphraseIntent};
pub use shortcut::Kbd;
pub use status::{PerfChip, PerfDetail, StatusLabel, ZOOM_PRESETS, ZoomChip};
pub use toast::{ToastHost, ToastKind, Toasts};
