// ambre-trame/src/chrome.rs
// ============================================================================
// Trame-SPECIFIC chrome (ambre-spec-architecture.md, 1: per-app chrome lives in
// the app crate, NOT the launcher). Built on the app-agnostic `ambre-chrome`
// toolkit, this module owns everything the launcher used to hard-code about
// Trame:
//
//   - the format `CommandId`s (Bold/Italic) and their keybindings;
//   - the CommandId -> `AppCommand` mapping (UI affordance -> IPC payload);
//   - the inbound `UiEvent` -> command-store mirror (authoritative toggle state);
//   - the toolbar / floating-selection-toolbar markup and the status label.
//
// The launcher (`ambre-app`) stays app-agnostic: it owns the generic AppShell,
// the `CommandStates` store, and the per-frame relay, and reaches into this
// module for the Trame pieces (`COMMANDS`, `bind_keys`, `command_to_app`,
// `mirror_event`, the `<FormatBar>` / `<SelectionBar>` components, ...).
//
// Wasm-only, like the rest of the chrome stack (leptos CSR + DOM).
// ============================================================================

use ambre_bus::{AppCommand, AppEvent, DocType, TrameCommand, TrameEvent, UiEvent};
use ambre_chrome::commands::{CommandId, KeyBinding, Keymap};
use ambre_chrome::{CommandStates, Kbd, MenuEntry};
use leptos::prelude::*;

/// The Trame "Bold" command. A UI affordance id (ambre-commands), distinct from
/// the `TrameCommand::ToggleBold` IPC payload it dispatches to.
pub const BOLD: CommandId = CommandId("trame.format.bold");
/// The Trame "Italic" command (see [`BOLD`]).
pub const ITALIC: CommandId = CommandId("trame.format.italic");
/// The Trame "Undo" command (Loro `UndoManager`, document-session scope).
pub const UNDO: CommandId = CommandId("trame.history.undo");
/// The Trame "Redo" command (see [`UNDO`]).
pub const REDO: CommandId = CommandId("trame.history.redo");

/// Every command Trame contributes to the registry. The launcher builds the
/// `CommandStates` store and the keymap from this list, naming no command itself.
pub const COMMANDS: &[CommandId] = &[BOLD, ITALIC, UNDO, REDO];

/// Bind Trame's command chords into the launcher's keymap.
pub fn bind_keys(keymap: &mut Keymap) {
    keymap.bind(KeyBinding::ctrl('b'), BOLD);
    keymap.bind(KeyBinding::ctrl('i'), ITALIC);
    keymap.bind(KeyBinding::ctrl('z'), UNDO);
    // Ctrl+Shift+Z and Ctrl+Y both redo (Excel/Sheets convention).
    keymap.bind(KeyBinding::ctrl_shift('z'), REDO);
    keymap.bind(KeyBinding::ctrl('y'), REDO);
}

/// Translate a Trame `CommandId` into the `AppCommand` IPC payload the Logic
/// thread expects. `None` for a command Trame does not own (the launcher logs it).
pub fn command_to_app(id: CommandId) -> Option<AppCommand> {
    let cmd = if id == BOLD {
        TrameCommand::ToggleBold
    } else if id == ITALIC {
        TrameCommand::ToggleItalic
    } else if id == UNDO {
        TrameCommand::Undo
    } else if id == REDO {
        TrameCommand::Redo
    } else {
        return None;
    };
    Some(AppCommand::Trame(cmd))
}

/// Mirror authoritative toggle state from an inbound `UiEvent` into the command
/// store, so the chrome's `aria-pressed` buttons reflect the Logic thread. A
/// no-op for any event that is not a Trame format-state notification; the
/// launcher calls this for every event it relays, naming no Trame variant.
pub fn mirror_event(store: &CommandStates, ev: &UiEvent) {
    if let UiEvent::Viewport { event: AppEvent::Trame(te), .. } = ev {
        match te {
            TrameEvent::BoldState { enabled } => store.set_toggled(BOLD, *enabled),
            TrameEvent::ItalicState { enabled } => store.set_toggled(ITALIC, *enabled),
            _ => {}
        }
    }
}

/// The right-click context-menu entries Trame contributes.
pub fn context_entries() -> Vec<MenuEntry> {
    vec![
        MenuEntry::new(BOLD, "Bold"),
        MenuEntry::new(ITALIC, "Italic"),
        MenuEntry::new(UNDO, "Undo"),
        MenuEntry::new(REDO, "Redo"),
    ]
}

/// The status-bar label for a Trame document.
pub fn status_label() -> &'static str {
    "Trame"
}

/// The document type this app creates from the landing "new document" button.
pub const DOC_TYPE: DocType = DocType::Trame;

/// The label of the landing "new document" button for this app.
pub fn new_doc_label() -> &'static str {
    "New Trame"
}

/// The Send toggle signals the Trame chrome components subscribe to, resolved
/// once from the launcher's `CommandStates` store. Plain `RwSignal`s, so this is
/// `Copy` and `Send` and may live inside the `<Router>` subtree (unlike the
/// `!Send` store itself, which the launcher keeps outside the reactive tree).
#[derive(Clone, Copy)]
pub struct Toggles {
    pub bold: RwSignal<bool>,
    pub italic: RwSignal<bool>,
}

impl Toggles {
    /// Resolve the format toggles from a store built over [`COMMANDS`].
    pub fn from_store(store: &CommandStates) -> Self {
        Self {
            bold: store.toggled_signal(BOLD).expect("bold registered"),
            italic: store.toggled_signal(ITALIC).expect("italic registered"),
        }
    }
}

/// The Trame format toolbar (the launcher mounts it inside the chrome `Toolbar`
/// slot). Buttons reflect `toggles` and invoke their `CommandId` through
/// `on_command` -- the same command path as the keyboard and the context menu.
#[component]
pub fn FormatBar(toggles: Toggles, on_command: Callback<CommandId>) -> impl IntoView {
    view! {
        <button
            title="Bold"
            aria-pressed=move || toggles.bold.get().then_some("true")
            on:click=move |_| on_command.run(BOLD)
        >
            "B"
        </button>
        <button
            title="Italic"
            aria-pressed=move || toggles.italic.get().then_some("true")
            on:click=move |_| on_command.run(ITALIC)
        >
            <em>"I"</em>
        </button>
        <span class="sep"></span>
        <Kbd binding=KeyBinding::ctrl('b') />
    }
}

/// The floating selection toolbar's inner content, transform-synced onto the
/// active cell by the launcher's generic `.ambre-selbar-anchor` wrapper.
#[component]
pub fn SelectionBar(toggles: Toggles, on_command: Callback<CommandId>) -> impl IntoView {
    view! {
        <div class="ambre-selbar">
            <button
                title="Bold"
                aria-pressed=move || toggles.bold.get().then_some("true")
                on:click=move |_| on_command.run(BOLD)
            >
                "B"
            </button>
            <button
                title="Italic"
                aria-pressed=move || toggles.italic.get().then_some("true")
                on:click=move |_| on_command.run(ITALIC)
            >
                <em>"I"</em>
            </button>
        </div>
    }
}
