// ambre-chrome/src/lib.rs
// ============================================================================
// Shared, app-agnostic Leptos chrome toolkit (ambre-spec-chrome.md).
//
// This crate owns the chrome components reused across every Ambre app:
// AppShell, command palette, menus/context menus, dialogs, toolbars, toasts,
// sidebar, status bar. Each is a *view* over the command registry from
// `ambre-commands`. App-SPECIFIC chrome (Trame's formula bar, a cell-format
// dialog) lives in the app crate and is built on top of this toolkit.
//
// The toolkit never reaches across the canvas/DOM boundary: it invokes
// `StaticKey<Commands>`s and reflects state via Leptos signals (ambre-spec-ui.md). It
// depends on no app crate (dependency inversion, symmetric to `AppView`).
//
// Module map. What is missing is `ambre-spec-chrome.md`'s own gap list, and the
// schedule for it is ambre-roadmap.md; neither belongs here.
//   - shortcut     -- keybinding presentation (the <Kbd> component)
//   - app_shell    -- CSS-grid shell: header, command bar, status bar, canvas
//   - avatar       -- a user's initials on their identity colour
//   - command      -- command-state store + keybinding manager
//   - composed     -- a name composed for display, which converts to no `String`
//   - dialog       -- native <dialog> modal (rAF-suspending)
//   - context_menu -- canvas right-click DOM menu (flip, keyboard nav)
//   - context_panel -- the one frame of the context panel: sections, rail, dock
//   - controls     -- the control primitives of the visual system, none generic
//   - creation     -- the creation bar, one bar of what a page can add
//   - gallery      -- the test page of every primitive in each state (feature `gallery`)
//   - kit          -- the typed widgets a panel section is written from
//   - menu         -- the key map every menu shares, and two-state ARIA values
//   - handler      -- what an application hands the shell for its commands
//   - palette      -- the command palette: every declared command, filtered
//   - passphrase   -- the passphrase prompt of a sealed `.ambre` file
//   - toast/status/format/font/color/debounce -- the shared controls
// ============================================================================

// Re-export the registry so app/chrome code has a single `ambre_chrome`-rooted
// path to command types when convenient.
pub use ambre_commands as commands;

mod app_shell;
mod avatar;
mod color;
mod command;
mod composed;
mod context_menu;
mod context_panel;
mod controls;
/// Each floating surface measures what it covers of the canvas (`CHROME-171`).
pub mod cover;
mod creation;
mod debounce;
mod dialog;
mod font;
mod format;
#[cfg(feature = "gallery")]
mod gallery;
mod handler;
mod header;
#[cfg(test)]
mod hover_scan;
mod kit;
mod menu;
mod palette;
mod passphrase;
mod shortcut;
mod status;
mod tip;
mod toast;

pub use app_shell::{
    AppShell, CanvasCell, CanvasOverlay, ContextRegion, FormulaBar, Header, Panel, RowShown,
    ShellRegion, ShellRows, Sidebar, StatusBar, Tabs, Toolbar,
};
pub use avatar::{Avatar, avatar_initials};
pub use color::{AlphaEdit, ColorPopover, PickedColour};
pub use command::{Availability, CommandStates, Toggle, keybinding_from_event};
pub use composed::ComposedName;
pub use context_menu::{ContextMenu, MenuEntry, MenuItem};
pub use context_panel::{
    ContextPanel, DockSide, Fold, PanelContent, PanelPrefs, PanelSection, PanelWidth,
};
pub use controls::{
    ChoiceRow, ChoiceView, ColorSwatch, ControlSize, IconButton, IconLook, Readout, SegmentBar,
    Select, Slider, Stepper,
};
pub use creation::{CreationBar, CreationSlot, creation_entries};
pub use debounce::{DRAG_DEBOUNCE, Debouncer};
pub use dialog::{Dialog, Dismissal, ModalHold};
pub use font::{FONT_FAMILIES, FamilyChoice, FontPicker};
pub use format::{
    FONT_SIZES, FORMAT_BAR_LIFT, FontSizeSelect, FormatBar, FormatControl, FormatToggle, Pressed,
};
#[cfg(feature = "gallery")]
pub use gallery::Gallery;
pub use handler::{CommandHandler, FileAccept, FilePick, KeyRoute, KeyTarget, ShellAction};
pub use header::{CommandSearch, HeaderMenu, PrimaryButton, SEARCH_LABEL};
pub use kit::{
    Choice, ColourField, ColourLayout, CommandButton, Field, FieldLength, FontField, Labelled,
    LengthField, LengthUnit, PropertyToggle, Row, Segmented, SelectField, Step, native_controls,
    reach_markers, repeated_title, unreached_controls,
};
pub use menu::{
    Checked, MenuKey, MenuRow, Openness, Selected, focus_first_item, initial, menu_key,
    on_menu_keydown,
};
pub use palette::{
    CommandPalette, PaletteEntry, PaletteKey, PaletteRow, filter as filter_commands,
    matches as command_matches, rows as palette_rows,
};
pub use passphrase::{
    PassphraseDialog, PassphraseIntent, PassphraseRefusal, vet as vet_passphrase,
};
pub use shortcut::Kbd;
pub use status::{Build, PerfChip, PerfDetail, StatusLabel, ZoomChip};
pub use tip::{CommandTip, untipped_icons};
pub use toast::{ToastHost, ToastKind, ToastVoice, Toasts};
