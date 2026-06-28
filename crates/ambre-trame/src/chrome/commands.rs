// ambre-trame/src/chrome/commands.rs
// ============================================================================
// Command surface: the Trame `CommandId`s, their keybindings, the CommandId ->
// `AppCommand` IPC mapping, and the inbound `UiEvent` mirroring (toggle state,
// toasts, context menu). The launcher reaches in here for `COMMANDS`,
// `bind_keys`, `command_to_app`, `mirror_event`, ... naming no command itself.
// ============================================================================

use ambre_bus::{AppCommand, AppEvent, DocType, TrameCommand, TrameEvent, UiEvent};
use ambre_chrome::commands::{CommandId, KeyBinding, Keymap};
use ambre_chrome::{CommandStates, MenuEntry, Toasts};

pub use crate::ui::grid::CtxScope;

/// The Trame "Bold" command. A UI affordance id (ambre-commands); the launcher
/// intercepts it into a `SetCellStyle` patch toggled against the tracked style.
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

// Clipboard commands (toolbar + context menu, `-ui.md` sec.5.E). UI-only
// intents: copy/cut read the viewport cache and paste reads the OS clipboard, so
// they never map to an `AppCommand`. The launcher intercepts them, driving the
// browser clipboard (the keyboard chords Ctrl+C/X/V are serviced directly by the
// DOM `copy`/`cut`/`paste` listeners, so they are NOT bound in the keymap here).
// Find & Replace (`-ui.md` sec.5.G). UI-only intents: the launcher intercepts
// them to open the DOM `FindReplace` panel (find vs replace mode); neither maps
// to an `AppCommand` (the panel sends `FindNext`/`ReplaceAll` itself).
/// Open the find/replace panel in find mode (Ctrl+F).
pub const FIND: CommandId = CommandId("trame.find.open");
/// Open the find/replace panel in replace mode (Ctrl+H).
pub const REPLACE: CommandId = CommandId("trame.find.replace");

// Interop (`-interop.md` §4). UI-only intents the launcher intercepts: IMPORT_CSV
// opens a file picker and reads the bytes, EXPORT_CSV asks the Logic thread to
// serialize the active sheet (parameterless). Neither maps to an `AppCommand`.
/// Import a CSV file into a new sheet.
pub const IMPORT_CSV: CommandId = CommandId("trame.interop.import_csv");
/// Export the active sheet to a CSV file.
pub const EXPORT_CSV: CommandId = CommandId("trame.interop.export_csv");

/// Copy the selection to the clipboard.
pub const COPY: CommandId = CommandId("trame.clipboard.copy");
/// Cut the selection to the clipboard.
pub const CUT: CommandId = CommandId("trame.clipboard.cut");
/// Paste the clipboard at the active cell.
pub const PASTE: CommandId = CommandId("trame.clipboard.paste");

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
    FIND,
    REPLACE,
    COPY,
    CUT,
    PASTE,
    IMPORT_CSV,
    EXPORT_CSV,
];

/// Bind Trame's command chords into the launcher's keymap.
pub fn bind_keys(keymap: &mut Keymap) {
    keymap.bind(KeyBinding::ctrl('b'), BOLD);
    keymap.bind(KeyBinding::ctrl('i'), ITALIC);
    keymap.bind(KeyBinding::ctrl('z'), UNDO);
    // Ctrl+Shift+Z and Ctrl+Y both redo (Excel/Sheets convention).
    keymap.bind(KeyBinding::ctrl_shift('z'), REDO);
    keymap.bind(KeyBinding::ctrl('y'), REDO);
    // Find & Replace (`-ui.md` sec.5.G): Ctrl+F opens find, Ctrl+H opens replace.
    keymap.bind(KeyBinding::ctrl('f'), FIND);
    keymap.bind(KeyBinding::ctrl('h'), REPLACE);
}

/// Translate a Trame `CommandId` into the `AppCommand` IPC payload the Logic
/// thread expects. `None` for a command Trame does not own (the launcher logs it).
pub fn command_to_app(id: CommandId) -> Option<AppCommand> {
    let cmd = if id == UNDO {
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
    } else if id == EXPORT_CSV {
        // Parameterless: the Logic thread serializes the active sheet itself.
        TrameCommand::ExportActiveSheetCsv
    } else {
        // BOLD / ITALIC (style toggles), FILTER_COLUMN, FIND / REPLACE and the
        // clipboard commands are UI-only intents the launcher intercepts:
        // BOLD/ITALIC become a `SetCellStyle` patch toggled against the tracked
        // style state, FILTER_COLUMN opens the popover, FIND/REPLACE open the
        // find/replace panel. None has a direct AppCommand here.
        return None;
    };
    Some(AppCommand::Trame(cmd))
}

/// Mirror authoritative toggle state from an inbound `UiEvent` into the command
/// store, so the chrome's `aria-pressed` buttons reflect the Logic thread. A
/// no-op for any event that is not a Trame format-state notification; the
/// launcher calls this for every event it relays, naming no Trame variant.
pub fn mirror_event(store: &CommandStates, ev: &UiEvent) {
    if let UiEvent::Viewport {
        event: AppEvent::Trame(te),
        ..
    } = ev
    {
        match te {
            // Loro `UndoManager` stack availability -> the Undo/Redo affordances'
            // enabled state. The Logic thread is the single source of truth.
            TrameEvent::UndoState { can_undo, can_redo } => {
                store.set_enabled(UNDO, *can_undo);
                store.set_enabled(REDO, *can_redo);
            }
            // Every other variant carries no command-toggle state. Listed
            // explicitly so a future variant is a compile error here, not a
            // silent drop.
            TrameEvent::ViewportData { .. }
            | TrameEvent::StyleApplied { .. }
            | TrameEvent::DataReset
            | TrameEvent::SelectRange { .. }
            | TrameEvent::MergeRejected { .. }
            | TrameEvent::MergeConflictDetected { .. }
            | TrameEvent::SheetList { .. }
            | TrameEvent::Error { .. }
            | TrameEvent::SortComplete
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
            | TrameEvent::ImportComplete { .. }
            | TrameEvent::ExportComplete { .. }
            | TrameEvent::ProtectionDenied { .. }
            | TrameEvent::ProtectionViolationWarning { .. } => {}
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
    let UiEvent::Viewport {
        event: AppEvent::Trame(te),
        ..
    } = ev
    else {
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
        // A converged concurrent edit masked a covered cell on an active merge.
        // Purely informational: the value reappears on unmerge.
        TrameEvent::MergeConflictDetected {
            covered_row,
            covered_col,
            ..
        } => {
            toasts.info(format!(
                "A merge now covers a non-empty cell ({covered_row}, {covered_col}); its value is hidden until unmerge"
            ));
        }
        // Shared sort/filter completion: confirm the action took effect (the data
        // refresh rides on the accompanying `DataReset`).
        TrameEvent::FilterApplied {
            hidden_row_count, ..
        } => {
            toasts.info(format!("Filter applied ({hidden_row_count} rows hidden)"));
        }
        TrameEvent::FilterCleared => toasts.info("Filter cleared"),
        TrameEvent::SortComplete => toasts.info("Range sorted"),
        // A Replace All finished; confirm the rewritten-cell count (the data
        // refresh rides on the accompanying DataReset). `ui.rs` documents this
        // toast as the surface for ReplaceDone.
        TrameEvent::ReplaceDone { replaced } => {
            toasts.info(format!("{replaced} cells replaced"));
        }
        // Interop (`-interop.md` §4): confirm an import/export finished (the
        // download itself is triggered canvas-side on ExportComplete).
        TrameEvent::ImportComplete { rows, cols, .. } => {
            toasts.info(format!("Imported {rows} rows x {cols} columns"));
        }
        TrameEvent::ExportComplete { filename, .. } => {
            toasts.info(format!("Exported {filename}"));
        }
        // A paste blocked by a protected range (Post-MVP); no CRDT mutation.
        TrameEvent::PasteRejected { reason } => {
            toasts.warning(format!("Paste rejected: {reason}"));
        }
        // Validation warning (§5): the edit was applied despite failing a Warn
        // rule -- non-blocking, unlike ValidationError above.
        TrameEvent::ValidationWarning { message, .. } => {
            toasts.warning(format!("Validation warning: {message}"));
        }
        // Range protection (§6, Post-MVP): a local edit was blocked, or a remote
        // delta violated a protection. Both inform the user; neither rolls back.
        TrameEvent::ProtectionDenied { description, .. } => {
            toasts.warning(format!("Edit blocked: {description}"));
        }
        TrameEvent::ProtectionViolationWarning { description, .. } => {
            toasts.warning(format!(
                "Protection violated by a collaborator: {description}"
            ));
        }
        // No toast: state-only events (cell data / toolbar style / sheet list /
        // undo-redo / find matches / filter values) or wholesale refreshes whose
        // user feedback rides elsewhere. Listed explicitly so a future variant is
        // a compile error here, not a silent drop.
        TrameEvent::ViewportData { .. }
        | TrameEvent::StyleApplied { .. }
        | TrameEvent::DataReset
        | TrameEvent::SelectRange { .. }
        | TrameEvent::UndoState { .. }
        | TrameEvent::SheetList { .. }
        | TrameEvent::ColumnValues { .. }
        | TrameEvent::SelectionStats { .. }
        | TrameEvent::FindResult { .. }
        | TrameEvent::FilterViewSaved { .. }
        | TrameEvent::CondFormatsUpdated { .. } => {}
    }
}

// -- Structural-op group (shared by the cell and header menus, sec.5.C) --------
fn structural_group() -> [MenuEntry; 6] {
    [
        MenuEntry::with_icon(
            INSERT_ROW_ABOVE,
            "Insert row above",
            icondata::MdiTableRowPlusBefore,
        ),
        MenuEntry::with_icon(
            INSERT_ROW_BELOW,
            "Insert row below",
            icondata::MdiTableRowPlusAfter,
        ),
        MenuEntry::with_icon(
            INSERT_COL_LEFT,
            "Insert column left",
            icondata::MdiTableColumnPlusBefore,
        ),
        MenuEntry::with_icon(
            INSERT_COL_RIGHT,
            "Insert column right",
            icondata::MdiTableColumnPlusAfter,
        ),
        MenuEntry::with_icon(DELETE_ROWS, "Delete rows", icondata::MdiTableRowRemove),
        MenuEntry::with_icon(
            DELETE_COLS,
            "Delete columns",
            icondata::MdiTableColumnRemove,
        ),
    ]
}

// -- Sort + filter group (shared by the cell and header menus, sec.5.C/5.D) ----
//
// Single-column shared sort over the selection (reference column = the
// selection's first column; the Logic thread no-ops an empty/single-row range)
// and the whole-sheet sort by the active column (whole-column selection: sorts
// the full used range so rows stay aligned). Filter "by column" opens the
// popover (values / condition); "Clear filter" drops the active sheet's filter
// (a no-op when nothing is filtered).
fn sort_filter_group() -> [MenuEntry; 6] {
    [
        MenuEntry::with_icon(
            SORT_RANGE_ASC,
            "Sort range ascending",
            icondata::MdiSortAscending,
        ),
        MenuEntry::with_icon(
            SORT_RANGE_DESC,
            "Sort range descending",
            icondata::MdiSortDescending,
        ),
        MenuEntry::with_icon(
            SORT_SHEET_ASC,
            "Sort sheet by column (asc)",
            icondata::MdiSort,
        ),
        MenuEntry::with_icon(
            SORT_SHEET_DESC,
            "Sort sheet by column (desc)",
            icondata::MdiSort,
        ),
        MenuEntry::with_icon(FILTER_COLUMN, "Filter by column", icondata::MdiFilter),
        MenuEntry::with_icon(CLEAR_FILTER, "Clear filter", icondata::MdiFilterOff),
    ]
}

/// The right-click context-menu entries Trame contributes, grouped, iconed and
/// separated (`-ui.md` sec.5.C). `scope` selects the cell menu (clipboard +
/// structural + sort/filter + merge) or the header-scoped menu (structural +
/// sort/filter only -- the cell-only clipboard and merge groups are dropped).
pub fn context_entries_for(scope: CtxScope) -> Vec<MenuEntry> {
    let mut entries = Vec::new();
    // Clipboard (sec.5.E): cell-only. The launcher drives the browser clipboard.
    if scope == CtxScope::Cell {
        entries.push(MenuEntry::with_icon(CUT, "Cut", icondata::MdiContentCut));
        entries.push(MenuEntry::with_icon(COPY, "Copy", icondata::MdiContentCopy));
        entries.push(MenuEntry::with_icon(
            PASTE,
            "Paste",
            icondata::MdiContentPaste,
        ));
        entries.push(MenuEntry::separator());
    }
    // Insert / delete rows+columns. The Logic thread resolves the geometry
    // against the tracked active cell / selection.
    entries.extend(structural_group());
    entries.push(MenuEntry::separator());
    // Sort + shared filter (sec.5.C / 5.D).
    entries.extend(sort_filter_group());
    // Merge (sec.2): cell-only. Always listed; the Logic thread guards it (Merge
    // on a single cell is a silent no-op, Merge over a non-empty covered cell
    // replies MergeRejected -> toast, Unmerge outside a merge is a no-op).
    if scope == CtxScope::Cell {
        entries.push(MenuEntry::separator());
        entries.push(MenuEntry::with_icon(
            MERGE,
            "Merge cells",
            icondata::MdiTableMergeCells,
        ));
        entries.push(MenuEntry::with_icon(
            UNMERGE,
            "Unmerge cells",
            icondata::MdiTableSplitCell,
        ));
    }
    entries
}

/// The default (cell-scoped) right-click context menu. Kept as the launcher's
/// initial layout; a header right-click swaps in [`context_entries_for`]
/// `(CtxScope::Header)`.
pub fn context_entries() -> Vec<MenuEntry> {
    context_entries_for(CtxScope::Cell)
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
