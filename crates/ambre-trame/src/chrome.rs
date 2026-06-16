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
use ambre_chrome::{CommandStates, Kbd, MenuEntry, Toasts};
use leptos::html;
use leptos::prelude::*;

use crate::render::{CommitDirection, FormulaAction};

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
            // Loro `UndoManager` stack availability -> the Undo/Redo affordances'
            // enabled state. The Logic thread is the single source of truth.
            TrameEvent::UndoState { can_undo, can_redo } => {
                store.set_enabled(UNDO, *can_undo);
                store.set_enabled(REDO, *can_redo);
            }
            _ => {}
        }
    }
}

/// Seed the command store for a freshly mounted Trame document: Undo and Redo
/// start disabled (an untouched document has an empty history) until the first
/// [`TrameEvent::UndoState`] arrives. Every other command stays enabled by
/// default. Called by the launcher right after it builds the store.
pub fn init_command_states(store: &CommandStates) {
    store.set_enabled(UNDO, false);
    store.set_enabled(REDO, false);
}

/// Surface a Trame `UiEvent` to the user as a transient toast, when it carries a
/// message the user should see: a rejected merge/edit, a sheet-lifecycle error,
/// or the completion of a shared sort/filter. A no-op for every other event; the
/// launcher calls this for each event it relays, naming no Trame variant itself.
pub fn notify_event(toasts: &Toasts, ev: &UiEvent) {
    let UiEvent::Viewport { event: AppEvent::Trame(te), .. } = ev else { return };
    match te {
        // Rejections / validation failures: no CRDT mutation occurred, the user
        // needs to know why their action did nothing.
        TrameEvent::MergeRejected { reason } => {
            toasts.warning(format!("Merge rejected: {reason}"));
        }
        TrameEvent::ValidationError { message, .. } => {
            toasts.warning(format!("Invalid value: {message}"));
        }
        TrameEvent::Error { message } => toasts.warning(message.clone()),
        // A converged concurrent edit masked a covered cell on an active merge.
        // Purely informational: the value reappears on unmerge.
        TrameEvent::MergeConflictDetected { covered_row, covered_col, .. } => {
            toasts.info(format!(
                "A merge now covers a non-empty cell ({covered_row}, {covered_col}); its value is hidden until unmerge"
            ));
        }
        // Shared sort/filter completion: confirm the action took effect (the data
        // refresh rides on the accompanying `DataReset`).
        TrameEvent::FilterApplied { hidden_row_count } => {
            toasts.info(format!("Filter applied ({hidden_row_count} rows hidden)"));
        }
        TrameEvent::FilterCleared => toasts.info("Filter cleared"),
        _ => {}
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
    /// Undo/Redo availability (Loro `UndoManager` stacks), driving the matching
    /// buttons' `disabled` state. Fed by [`TrameEvent::UndoState`] via
    /// [`mirror_event`].
    pub undo_enabled: RwSignal<bool>,
    pub redo_enabled: RwSignal<bool>,
}

impl Toggles {
    /// Resolve the format toggles and history-enabled signals from a store built
    /// over [`COMMANDS`].
    pub fn from_store(store: &CommandStates) -> Self {
        Self {
            bold: store.toggled_signal(BOLD).expect("bold registered"),
            italic: store.toggled_signal(ITALIC).expect("italic registered"),
            undo_enabled: store.enabled_signal(UNDO).expect("undo registered"),
            redo_enabled: store.enabled_signal(REDO).expect("redo registered"),
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
        <button
            title="Undo"
            disabled=move || !toggles.undo_enabled.get()
            on:click=move |_| on_command.run(UNDO)
        >
            "Undo"
        </button>
        <button
            title="Redo"
            disabled=move || !toggles.redo_enabled.get()
            on:click=move |_| on_command.run(REDO)
        >
            "Redo"
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

// -----------------------------------------------------------------------------
// Formula bar (HTML-based, editable input)
// -----------------------------------------------------------------------------

/// The formula bar component: a name box showing the active cell reference
/// (e.g. `"B2"`) and an HTML `<input>` bound to the cell's raw value or the
/// live edit buffer. The input is always editable; focus or typed characters
/// open an edit session.
///
/// `on_action` fires on Enter (commit Down), Tab (commit Right), or Escape
/// (cancel). The caret, selection, IME, and clipboard are handled natively by
/// the HTML input element.
#[component]
pub fn FormulaBar(
    cell_ref: RwSignal<String>,
    content: RwSignal<String>,
    on_action: Callback<FormulaAction>,
) -> impl IntoView {
    let input_ref = NodeRef::<html::Input>::new();

    // When the signal changes from outside (e.g. a new cell was selected or an
    // edit was seeded), push the new value into the DOM input without resetting
    // the caret. The comparison avoids fighting the user's typing.
    create_effect(move |_| {
        let val = content.get();
        if let Some(input) = input_ref.get() {
            if val != input.value() {
                input.set_value(&val);
            }
        }
    });

    let on_input = move |ev: web_sys::Event| {
        content.set(event_target_value(&ev));
    };

    let on_keydown = {
        let content = content;
        move |ev: web_sys::KeyboardEvent| {
            match ev.key().as_str() {
                "Enter" => {
                    ev.prevent_default();
                    on_action.run(FormulaAction::Commit(CommitDirection::Down));
                }
                "Tab" => {
                    ev.prevent_default();
                    on_action.run(FormulaAction::Commit(CommitDirection::Right));
                }
                "Escape" => {
                    ev.prevent_default();
                    on_action.run(FormulaAction::Cancel);
                }
                _ => {}
            }
        }
    };

    view! {
        <span class="name-box">{move || cell_ref.get()}</span>
        <input
            type="text"
            id="ambre-formula-input"
            class="formula-input"
            node_ref=input_ref
            on:input=on_input
            on:keydown=on_keydown
        />
    }
}
