// ambre-trame/src/chrome/commands.rs
// ============================================================================
// Chrome half of Trame's command surface: the wasm-only pieces that need the
// DOM or a Leptos signal.
//
//   - the context-menu layouts, built from the registry;
//   - the keymap aliases the registry cannot hold;
//   - the inbound `UiEvent` mirroring (toggle state, toasts).
//
// The registry itself is `crate::ui::commands`: the identities, the descriptions
// and the `StaticKey<Commands> -> AppCommand` mapping. It is host-compiled, so a test
// reaches a label without a browser. The identities are re-exported here so the
// launcher and the chrome components keep naming `chrome::BOLD`
// (`ambre-spec-commands.md`).
// ============================================================================

use crate::message::TrameEvent;
use ambre_bus::{DocType, Embed, UiEvent};
use ambre_chrome::commands::{Keymap, Locale};
use ambre_chrome::{CommandStates, MenuEntry, Toasts};

use super::icons;
use crate::ui::commands as registry;

pub use crate::ui::commands::{
    BOLD, CLEAR_FILTER, COMMANDS, COPY, CUT, DELETE_COLS, DELETE_ROWS, EXPORT_CSV, FILTER_COLUMN,
    FIND, IMPORT_CSV, IMPORT_ODS, IMPORT_XLSX, INSERT_COL_LEFT, INSERT_COL_RIGHT, INSERT_ROW_ABOVE,
    INSERT_ROW_BELOW, ITALIC, MERGE, PALETTE, PASTE, REDO, REPLACE, SAVE_FILE, SAVE_SEALED_FILE,
    SORT_RANGE_ASC, SORT_RANGE_DESC, SORT_RANGE_MULTI, SORT_SHEET_ASC, SORT_SHEET_DESC, UNDO,
    UNMERGE, command_to_app,
};
pub use crate::ui::grid::CtxScope;

/// Bind Trame's command chords into the launcher's keymap.
///
/// The table is the one declaration site, chords included. The seeding is
/// therefore host-compiled beside it (`registry::bind_keys`), and a test
/// resolves a chord without a browser. Nothing is bound here on the side.
pub fn bind_keys(keymap: &mut Keymap) {
    registry::bind_keys(keymap);
}

/// Mirror authoritative toggle state from an inbound `UiEvent` into the command
/// store, so the chrome's `aria-pressed` buttons reflect the Logic thread. A
/// no-op for any event that is not a Trame format-state notification; the
/// launcher calls this for every event it relays, naming no Trame variant.
pub fn mirror_event<S: Embed<TrameEvent>, P: ambre_bus::ChannelPlane>(
    store: &CommandStates,
    ev: &UiEvent<S, P>,
) {
    if let Some(te) = ev.viewport_event::<TrameEvent>() {
        match te {
            // Loro `UndoManager` stack availability -> the Undo/Redo affordances'
            // enabled state. The Logic thread is the single source of truth.
            TrameEvent::UndoState { undo, redo } => {
                store.set_enabled(UNDO, *undo);
                store.set_enabled(REDO, *redo);
            }
            // Every other variant carries no command-toggle state. Listed
            // explicitly so a future variant is a compile error here, not a
            // silent drop.
            TrameEvent::ViewportData { .. }
            | TrameEvent::StyleApplied { .. }
            | TrameEvent::DataReset
            | TrameEvent::CellsChanged { .. }
            | TrameEvent::SelectRange { .. }
            | TrameEvent::MergeRejected { .. }
            | TrameEvent::MergeConflictDetected { .. }
            | TrameEvent::SheetList { .. }
            | TrameEvent::LocaleState { .. }
            | TrameEvent::Error { .. }
            | TrameEvent::EditTargetMoved { .. }
            | TrameEvent::EditTargetGone { .. }
            | TrameEvent::DocumentRefused { .. }
            | TrameEvent::UndecodableRecords
            | TrameEvent::SortComplete
            | TrameEvent::SortRange { .. }
            | TrameEvent::ColumnContent { .. }
            | TrameEvent::FilterApplied { .. }
            | TrameEvent::ColumnValues { .. }
            | TrameEvent::SelectionStats { .. }
            | TrameEvent::FilterCleared
            | TrameEvent::FindResult { .. }
            | TrameEvent::ReplaceDone { .. }
            | TrameEvent::PasteRejected { .. }
            | TrameEvent::FilterViewSaved { .. }
            | TrameEvent::CondFormatsUpdated { .. }
            | TrameEvent::ValidationError { .. }
            | TrameEvent::ValidationWarning { .. }
            | TrameEvent::Interop(_)
            | TrameEvent::ProtectionDenied { .. }
            | TrameEvent::Clipboard(_)
            | TrameEvent::ProtectionViolationWarning { .. } => {}
        }
    }
}

/// Seed the command store for a freshly mounted Trame document. Undo and Redo
/// start disabled, an untouched document having an empty history, until the
/// first [`TrameEvent::UndoState`] arrives. Every other command stays enabled
/// by default. Called by the launcher right after it builds the store.
pub fn init_command_states(store: &CommandStates) {
    store.set_enabled(UNDO, ambre_chrome::Availability::Disabled);
    store.set_enabled(REDO, ambre_chrome::Availability::Disabled);
}

/// Surface a Trame `UiEvent` to the user as a transient toast, when it carries a
/// message the user should see. That means a rejected merge/edit, a
/// sheet-lifecycle error, or the completion of a shared sort/filter. A no-op for
/// every other event; the launcher calls this for each event it relays, naming
/// no Trame variant itself.
pub fn notify_event<S: Embed<TrameEvent>, P: ambre_bus::ChannelPlane>(
    toasts: &Toasts,
    ev: &UiEvent<S, P>,
) {
    let Some(te) = ev.viewport_event::<TrameEvent>() else {
        return;
    };
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
        // A peer deleted the cell an open session was writing to, and the
        // session closed with nothing written (`TRAME-UI-176`).
        TrameEvent::EditTargetGone { .. } => toasts.warning(
            "A collaborator deleted the cell you were editing. Your edit was not written.",
        ),
        // The workbook was written by a newer client (`SCHEMA-008`). An error,
        // not a warning: nothing opened, and the remedy is not in this workbook.
        TrameEvent::DocumentRefused { reason } => toasts.error(reason.clone()),
        // Part of the workbook could not be read and lost its formatting
        // (`SCHEMA-003`). The document is open and usable, so this is a
        // warning. But it must be said: a register that silently reads as
        // unformatted is a workbook that looks intact and is not.
        TrameEvent::UndecodableRecords => toasts.warning(
            "Some formatting in this workbook was written by a newer client and could not be read.",
        ),
        // A converged concurrent edit masked a covered cell on an active merge.
        // Purely informational: the value reappears on unmerge.
        TrameEvent::MergeConflictDetected {
            covered_row,
            covered_col,
            ..
        } => {
            toasts.info(format!(
                "A merge now covers a non-empty cell ({}, {}); its value is hidden until unmerge",
                covered_row.get(),
                covered_col.get()
            ));
        }
        // Shared sort/filter completion: confirm the action took effect (the data
        // refresh rides on the accompanying `DataReset`).
        TrameEvent::FilterApplied {
            hidden_row_count, ..
        } => {
            toasts.info(format!(
                "Filter applied ({} rows hidden)",
                hidden_row_count.get()
            ));
        }
        TrameEvent::FilterCleared => toasts.info("Filter cleared"),
        TrameEvent::SortComplete => toasts.info("Range sorted"),
        // A Replace All finished; confirm the rewritten-cell count (the data
        // refresh rides on the accompanying DataReset). `ui.rs` documents this
        // toast as the surface for ReplaceDone.
        TrameEvent::ReplaceDone { replaced } => {
            toasts.info(format!("{} cells replaced", replaced.get()));
        }
        // Interop (`trame-import-export`): confirm an import/export finished (the
        // download itself is triggered canvas-side on ExportComplete).
        // The shared sentence (`INTEROP-007`, `INTEROP-011`).
        TrameEvent::Interop(interop) => {
            if let Some(message) = interop.message() {
                if interop.is_warning() {
                    toasts.warning(message);
                } else {
                    toasts.info(message);
                }
            }
        }
        // A paste blocked by a protected range (Post-MVP); no CRDT mutation.
        TrameEvent::PasteRejected { reason } => {
            toasts.warning(format!("Paste rejected: {reason}"));
        }
        // Validation warning (`trame-cell-editing`): the edit was applied
        // despite failing a Warn rule -- non-blocking, unlike ValidationError.
        TrameEvent::ValidationWarning { message, .. } => {
            toasts.warning(format!("Validation warning: {message}"));
        }
        // Range protection (`trame-cell-editing`, Post-MVP): a local edit was
        // blocked, or a remote delta violated a protection. Both inform the user;
        // neither rolls back.
        TrameEvent::ProtectionDenied { description, .. } => {
            toasts.warning(format!("Edit blocked: {description}"));
        }
        TrameEvent::ProtectionViolationWarning { description, .. } => {
            toasts.warning(format!(
                "Protection violated by a collaborator: {description}"
            ));
        }
        // No toast for state-only events: cell data, toolbar style, sheet list,
        // undo-redo, find matches, filter values. Wholesale refreshes whose user
        // feedback rides elsewhere are silent too. Listed explicitly so a future
        // variant is a compile error here, not a silent drop.
        TrameEvent::ViewportData { .. }
        | TrameEvent::StyleApplied { .. }
        | TrameEvent::DataReset
        | TrameEvent::CellsChanged { .. }
        | TrameEvent::SelectRange { .. }
        | TrameEvent::UndoState { .. }
        | TrameEvent::SheetList { .. }
        | TrameEvent::LocaleState { .. }
        | TrameEvent::EditTargetMoved { .. }
        | TrameEvent::ColumnValues { .. }
        | TrameEvent::SortRange { .. }
        | TrameEvent::ColumnContent { .. }
        | TrameEvent::SelectionStats { .. }
        | TrameEvent::FindResult { .. }
        | TrameEvent::FilterViewSaved { .. }
        | TrameEvent::Clipboard(_)
        | TrameEvent::CondFormatsUpdated { .. } => {}
    }
}

// -- Context-menu rendering (`trame-context-menu`) -----------------------------
//
// The layout is `ui::commands::context_layout`, host-tested. This half turns
// one row into the `MenuEntry` the DOM menu renders, reading the label and the
// icon from the registry (`UI-THREAD-019`). A menu declares an order and a
// grouping, never a word the user reads.

/// The menu entry for one layout row, labelled and iconed from the registry.
/// `None` for an identity the registry does not declare, which no layout
/// produces.
fn entry(row: registry::MenuRow) -> Option<MenuEntry> {
    let id = match row {
        registry::MenuRow::Separator => return Some(MenuEntry::separator()),
        registry::MenuRow::Command(id) => id,
    };
    let cmd = ambre_chrome::commands::find(COMMANDS, id)?;
    let entry = MenuEntry::command(cmd, &Locale::En);
    Some(match cmd.icon.and_then(icons::resolve) {
        Some(icon) => entry.icon(icon),
        None => entry,
    })
}

/// The right-click context-menu entries Trame contributes, grouped, iconed and
/// separated (`trame-context-menu`). `scope` and `filter` select the
/// layout; see [`registry::context_layout`].
pub fn context_entries_for(scope: CtxScope, filter: super::SheetFilter) -> Vec<MenuEntry> {
    registry::context_layout(scope, filter)
        .into_iter()
        .filter_map(entry)
        .collect()
}

/// The default (cell-scoped, no active filter) right-click context menu. Kept as
/// the launcher's initial layout; a header right-click swaps in
/// [`context_entries_for`] with the resolved scope and live filter state.
pub fn context_entries() -> Vec<MenuEntry> {
    context_entries_for(CtxScope::Cell, super::SheetFilter::Unfiltered)
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
