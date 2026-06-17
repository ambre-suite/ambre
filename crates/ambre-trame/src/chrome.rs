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

use ambre_bus::{AppCommand, AppEvent, DocType, FilterKind, FilterOp, TrameCommand, TrameEvent, UiEvent};
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

// Structural / merge commands (grid context menu, `-ui.md` sec.5.C). Each is a
// parameterless intent: the Logic thread resolves its geometry against the
// active cell / selection it tracks (via `SetActiveCell`).
/// Insert a row above the active cell.
pub const INSERT_ROW_ABOVE: CommandId = CommandId("trame.row.insert_above");
/// Insert a row below the active cell.
pub const INSERT_ROW_BELOW: CommandId = CommandId("trame.row.insert_below");
/// Insert a column left of the active cell.
pub const INSERT_COL_LEFT: CommandId = CommandId("trame.col.insert_left");
/// Insert a column right of the active cell.
pub const INSERT_COL_RIGHT: CommandId = CommandId("trame.col.insert_right");
/// Delete the rows spanned by the selection.
pub const DELETE_ROWS: CommandId = CommandId("trame.row.delete");
/// Delete the columns spanned by the selection.
pub const DELETE_COLS: CommandId = CommandId("trame.col.delete");
/// Merge the selection into a single cell.
pub const MERGE: CommandId = CommandId("trame.merge");
/// Unmerge the merge containing the active cell.
pub const UNMERGE: CommandId = CommandId("trame.unmerge");
/// Sort the selection ascending on its first column (single-column shared sort).
pub const SORT_RANGE_ASC: CommandId = CommandId("trame.sort.range_asc");
/// Sort the selection descending on its first column.
pub const SORT_RANGE_DESC: CommandId = CommandId("trame.sort.range_desc");
/// Sort the whole sheet ascending by the active cell's column.
pub const SORT_SHEET_ASC: CommandId = CommandId("trame.sort.sheet_asc");
/// Sort the whole sheet descending by the active cell's column.
pub const SORT_SHEET_DESC: CommandId = CommandId("trame.sort.sheet_desc");
/// Open the shared filter popover for the active cell's column (sec.5.D). A
/// UI-only intent: it has no `AppCommand` mapping; the launcher intercepts it to
/// open the popover and request the column's distinct values.
pub const FILTER_COLUMN: CommandId = CommandId("trame.filter.column");
/// Clear the active sheet's shared filter (sec.5.D).
pub const CLEAR_FILTER: CommandId = CommandId("trame.filter.clear");

/// Every command Trame contributes to the registry. The launcher builds the
/// `CommandStates` store and the keymap from this list, naming no command itself.
pub const COMMANDS: &[CommandId] = &[
    BOLD,
    ITALIC,
    UNDO,
    REDO,
    INSERT_ROW_ABOVE,
    INSERT_ROW_BELOW,
    INSERT_COL_LEFT,
    INSERT_COL_RIGHT,
    DELETE_ROWS,
    DELETE_COLS,
    MERGE,
    UNMERGE,
    SORT_RANGE_ASC,
    SORT_RANGE_DESC,
    SORT_SHEET_ASC,
    SORT_SHEET_DESC,
    FILTER_COLUMN,
    CLEAR_FILTER,
];

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
    } else if id == INSERT_ROW_ABOVE {
        TrameCommand::InsertRowAbove
    } else if id == INSERT_ROW_BELOW {
        TrameCommand::InsertRowBelow
    } else if id == INSERT_COL_LEFT {
        TrameCommand::InsertColLeft
    } else if id == INSERT_COL_RIGHT {
        TrameCommand::InsertColRight
    } else if id == DELETE_ROWS {
        TrameCommand::DeleteSelectedRows
    } else if id == DELETE_COLS {
        TrameCommand::DeleteSelectedCols
    } else if id == MERGE {
        TrameCommand::MergeSelection
    } else if id == UNMERGE {
        TrameCommand::UnmergeActive
    } else if id == SORT_RANGE_ASC {
        TrameCommand::SortSelectionAsc
    } else if id == SORT_RANGE_DESC {
        TrameCommand::SortSelectionDesc
    } else if id == SORT_SHEET_ASC {
        TrameCommand::SortSheetAsc
    } else if id == SORT_SHEET_DESC {
        TrameCommand::SortSheetDesc
    } else if id == CLEAR_FILTER {
        TrameCommand::ClearActiveFilter
    } else {
        // FILTER_COLUMN is a UI-only intent handled by the launcher (it opens the
        // popover and requests the column values); it has no AppCommand here.
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
        TrameEvent::FilterApplied { hidden_row_count, .. } => {
            toasts.info(format!("Filter applied ({hidden_row_count} rows hidden)"));
        }
        TrameEvent::FilterCleared => toasts.info("Filter cleared"),
        TrameEvent::SortComplete => toasts.info("Range sorted"),
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
        // Structural / merge ops (sec.5.C). Always listed; the Logic thread
        // guards them (Merge on a single cell is a silent no-op, Merge over a
        // non-empty covered cell replies MergeRejected -> toast, Unmerge outside
        // a merge is a no-op).
        MenuEntry::new(INSERT_ROW_ABOVE, "Insert row above"),
        MenuEntry::new(INSERT_ROW_BELOW, "Insert row below"),
        MenuEntry::new(INSERT_COL_LEFT, "Insert column left"),
        MenuEntry::new(INSERT_COL_RIGHT, "Insert column right"),
        MenuEntry::new(DELETE_ROWS, "Delete rows"),
        MenuEntry::new(DELETE_COLS, "Delete columns"),
        MenuEntry::new(MERGE, "Merge cells"),
        MenuEntry::new(UNMERGE, "Unmerge cells"),
        // Single-column shared sort over the selection (sec.5.C). Reference
        // column = the selection's first column; the Logic thread no-ops an
        // empty/single-row range.
        MenuEntry::new(SORT_RANGE_ASC, "Sort range ascending"),
        MenuEntry::new(SORT_RANGE_DESC, "Sort range descending"),
        // Whole-sheet sort by the active column (whole-column selection, sec.5.C):
        // sorts the full used range so rows stay aligned across all columns.
        MenuEntry::new(SORT_SHEET_ASC, "Sort sheet by column (asc)"),
        MenuEntry::new(SORT_SHEET_DESC, "Sort sheet by column (desc)"),
        // Shared filter on the active cell's column (sec.5.D). "Filter by column"
        // opens the popover (values / condition); "Clear filter" drops the active
        // sheet's filter. The Logic thread no-ops a clear when nothing is filtered.
        MenuEntry::new(FILTER_COLUMN, "Filter by column"),
        MenuEntry::new(CLEAR_FILTER, "Clear filter"),
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
                    // Ctrl/Cmd+Enter fills the whole selection with the value.
                    if ev.ctrl_key() || ev.meta_key() {
                        on_action.run(FormulaAction::CommitFill);
                    } else {
                        on_action.run(FormulaAction::Commit(CommitDirection::Down));
                    }
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

// -----------------------------------------------------------------------------
// Shared filter popover (`-ui.md` sec.5.D)
// -----------------------------------------------------------------------------

/// Open state of the shared filter popover: which grid column it targets and the
/// viewport (client-space) anchor point under which it is drawn. `None` means
/// closed. The launcher derives the anchor from the active cell's published
/// screen position (the active cell is in the filtered column).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterPopoverState {
    /// Grid column (0-based) the filter targets.
    pub col: u32,
    /// Client-space anchor (CSS px): the popover's top-left.
    pub x: f64,
    pub y: f64,
}

/// The condition operators offered by the popover, paired with their labels.
/// The `<select>` index maps back to the [`FilterOp`] on apply.
const FILTER_OPS: &[(FilterOp, &str)] = &[
    (FilterOp::Eq, "equals"),
    (FilterOp::Ne, "not equals"),
    (FilterOp::Gt, "greater than"),
    (FilterOp::Lt, "less than"),
    (FilterOp::Gte, "greater or equal"),
    (FilterOp::Lte, "less or equal"),
    (FilterOp::Contains, "contains"),
    (FilterOp::NotContains, "does not contain"),
    (FilterOp::StartsWith, "starts with"),
    (FilterOp::IsEmpty, "is empty"),
    (FilterOp::IsNotEmpty, "is not empty"),
];

/// A1-style column label for a 0-based grid column index (0 -> "A", 26 -> "AA").
fn col_label(mut col: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// The shared filter popover: anchored under the active column header, it offers
/// the "by values" (checkbox list of the column's distinct visible values) and a
/// minimal "by condition" (operator + value) modes (`-ui.md` sec.5.D). Applying
/// fires `on_apply` with the chosen [`FilterKind`]; "Clear filter" routes through
/// `on_command` (the shared command path). `values` is fed by the launcher from
/// [`TrameEvent::ColumnValues`].
#[component]
pub fn FilterPopover(
    state: RwSignal<Option<FilterPopoverState>>,
    values: RwSignal<Vec<String>>,
    on_apply: Callback<FilterKind>,
    on_command: Callback<CommandId>,
) -> impl IntoView {
    // true = "by values", false = "by condition".
    let by_values = RwSignal::new(true);
    // The set of currently-checked (visible) values in the "by values" mode.
    let checked = RwSignal::new(std::collections::BTreeSet::<String>::new());
    // Whenever a fresh value list arrives (popover (re)opened on a column), check
    // every value by default -- the unfiltered state.
    Effect::new(move |_| {
        let vs = values.get();
        checked.set(vs.into_iter().collect());
    });
    // Condition-mode operator + operand.
    let op_idx = RwSignal::new(0usize);
    let cond_value = RwSignal::new(String::new());

    let close = move || state.set(None);

    let apply = move |_| {
        let kind = if by_values.get_untracked() {
            FilterKind::Values(checked.get_untracked().into_iter().collect())
        } else {
            let (op, _) = FILTER_OPS[op_idx.get_untracked().min(FILTER_OPS.len() - 1)];
            FilterKind::Condition { op, value: cond_value.get_untracked() }
        };
        on_apply.run(kind);
        state.set(None);
    };

    move || {
        state.get().map(|st| {
            let style = format!(
                "position: fixed; left: {}px; top: {}px; z-index: 50;",
                st.x, st.y
            );
            view! {
                <div class="ambre-filter-popover" style=style>
                    <div class="ambre-filter-head">
                        {format!("Filter column {}", col_label(st.col))}
                    </div>
                    <div class="ambre-filter-modes">
                        <button
                            class="mode"
                            aria-pressed=move || by_values.get().then_some("true")
                            on:click=move |_| by_values.set(true)
                        >
                            "By values"
                        </button>
                        <button
                            class="mode"
                            aria-pressed=move || (!by_values.get()).then_some("true")
                            on:click=move |_| by_values.set(false)
                        >
                            "By condition"
                        </button>
                    </div>

                    <Show
                        when=move || by_values.get()
                        fallback=move || view! {
                            <div class="ambre-filter-cond">
                                <select on:change=move |ev| {
                                    op_idx.set(event_target_value(&ev).parse().unwrap_or(0));
                                }>
                                    {FILTER_OPS
                                        .iter()
                                        .enumerate()
                                        .map(|(i, (_, label))| {
                                            view! { <option value=i.to_string()>{label.to_string()}</option> }
                                        })
                                        .collect_view()}
                                </select>
                                <input
                                    type="text"
                                    placeholder="value"
                                    on:input=move |ev| cond_value.set(event_target_value(&ev))
                                />
                            </div>
                        }
                    >
                        <div class="ambre-filter-values">
                            <For
                                each=move || values.get()
                                key=|v| v.clone()
                                children=move |v: String| {
                                    let v_for_check = v.clone();
                                    let v_for_toggle = v.clone();
                                    let is_on = move || checked.get().contains(&v_for_check);
                                    view! {
                                        <label class="ambre-filter-value">
                                            <input
                                                type="checkbox"
                                                prop:checked=is_on
                                                on:change=move |ev| {
                                                    let on = event_target_checked(&ev);
                                                    checked.update(|set| {
                                                        if on {
                                                            set.insert(v_for_toggle.clone());
                                                        } else {
                                                            set.remove(&v_for_toggle);
                                                        }
                                                    });
                                                }
                                            />
                                            <span>{v}</span>
                                        </label>
                                    }
                                }
                            />
                        </div>
                    </Show>

                    <div class="ambre-filter-actions">
                        <button class="primary" on:click=apply>"Apply"</button>
                        <button on:click=move |_| {
                            on_command.run(CLEAR_FILTER);
                            close();
                        }>"Clear filter"</button>
                        <button on:click=move |_| close()>"Cancel"</button>
                    </div>
                </div>
            }
        })
    }
}
