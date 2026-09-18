//! Trame (spreadsheet) event surface (Logic thread -> UI Thread).
//!
//! The `TrameEvent` family plus its display value type [`DisplayCell`], carried
//! by move inside [`AppEvent::Trame`](crate::ui::AppEvent). The command-side
//! value types it reuses (`CellStyle`, `HAlign`, `VAlign`, `Borders`, ...) arrive
//! by glob from the sibling command modules of `crate::app::trame`.

use crate::UserId;

// -----------------------------------------------------------------------------
// Trame (spreadsheet) -- events (Logic -> UI)
// -----------------------------------------------------------------------------

/// Display events emitted by the Trame logic to the UI Thread.
#[derive(Debug, Clone)]
pub enum TrameEvent {
    /// Cell content for a requested viewport region.
    ///
    /// `row_start` / `col_start` are the top-left corner of the batch; `cells`
    /// holds all non-empty cells within the requested range. The UI Thread
    /// merges this batch into its local cache. `version` echoes the originating
    /// [`TrameCommand::ScrollViewport`](super::TrameCommand::ScrollViewport)
    /// so stale responses can be dropped.
    ViewportData {
        version: u64,
        row_start: u32,
        col_start: u32,
        cells: Vec<DisplayCell>,
        /// Per-column widths (physical px) for the requested column window,
        /// `col_widths[i]` being the width of column `col_start + i`
        /// (`trame-resize`, `trame-engine`). A column never resized carries
        /// the uniform default, so the UI can feed the slice straight into the
        /// variable-metric `GridLayout`. Empty only on an out-of-range request.
        col_widths: Vec<f32>,
        /// Per-row heights (physical px) for the requested row window, aligned to
        /// `row_start` the same way [`col_widths`](Self::ViewportData::col_widths)
        /// is to `col_start`.
        row_heights: Vec<f32>,
    },

    /// The resolved formatting of the **active cell** after a
    /// [`SetCellStyle`](super::TrameCommand::SetCellStyle) or a
    /// [`SetActiveCell`](super::TrameCommand::SetActiveCell). It drives the whole
    /// formatting toolbar: bold/italic/underline toggles, font/size selects, colour
    /// swatches, align buttons, cell-format / border menus, wrap. Bold and italic ride
    /// alongside the [`CellStyle`]. They are part of the single style channel, not a
    /// separate per-flag event (`trame-cell-styles`).
    ///
    /// [`CellStyle`]: super::CellStyle
    StyleApplied {
        style: super::CellStyle,
        bold: bool,
        italic: bool,
        /// Whether the active cell is part of a merge (a master or a covered
        /// cell), driving the toolbar's merge/unmerge toggle pressed state.
        merged: bool,
        /// Whether the active cell's display value parses as a number. Lets the toolbar
        /// resolve [`HAlign::Auto`](super::HAlign::Auto) the same way the renderer does,
        /// numbers right and text left. The alignment control then reflects the de-facto
        /// alignment of an `Auto` cell.
        numeric: bool,
    },

    /// The document's content was replaced wholesale (local snapshot restore at
    /// startup). The UI Thread re-requests the visible region: the initial
    /// viewport response predated the restore, so the cache must be refreshed.
    DataReset,

    /// Ask the UI Thread to replace the rendered grid selection with the
    /// rectangle `(r1,c1)`-`(r2,c2)` (inclusive, corners ordered). Emitted when a
    /// command changes the area the user should see selected and the canvas
    /// cannot derive it locally. An unmerge is the case: the freed cells must
    /// stay selected, though the merge that defined the span is gone.
    /// The active cell collapses onto the top-left corner.
    SelectRange { r1: u32, c1: u32, r2: u32, c2: u32 },

    /// A local [`MergeCells`](super::TrameCommand::MergeCells) was refused
    /// because a covered cell is non-empty (`trame-cell-merges`). The
    /// chrome surfaces `reason` to the user; no CRDT mutation was produced.
    MergeRejected { reason: String },

    /// A merge ended up covering a non-empty cell after a concurrent edit
    /// converged (`trame-cell-merges`). The covered value is masked (not
    /// destroyed) while the merge is active; it reappears on unmerge. Purely
    /// informational for the UI.
    MergeConflictDetected {
        master_row: u32,
        master_col: u32,
        covered_row: u32,
        covered_col: u32,
    },

    /// Availability of undo/redo after a local
    /// [`Undo`](super::TrameCommand::Undo) /
    /// [`Redo`](super::TrameCommand::Redo). Drives the enabled state of the
    /// chrome's Undo/Redo affordances. Reflects Loro's `UndoManager` stacks, whose
    /// scope is the current document session.
    UndoState { can_undo: bool, can_redo: bool },

    /// The workbook's sheets, in tab-strip order: `(sheet_id, display_name)`.
    /// Emitted after any sheet-lifecycle command
    /// ([`CreateSheet`](super::TrameCommand::CreateSheet) /
    /// [`RenameSheet`](super::TrameCommand::RenameSheet) /
    /// [`MoveSheet`](super::TrameCommand::MoveSheet) /
    /// [`DeleteSheet`](super::TrameCommand::DeleteSheet)) and on load, so
    /// the tab strip stays in sync (`trame-sheets`). `active`
    /// is the id of the sheet currently shown.
    SheetList {
        sheets: Vec<(String, String)>,
        active: String,
    },

    /// A command was rejected for a reason the user should see (for example deleting the
    /// last remaining sheet). Purely informational; no CRDT mutation occurred.
    Error { message: String },

    /// The workbook was written by a newer client and was refused whole (`SCHEMA-008`).
    /// Nothing was applied, and the grid on screen is the one that was already open.
    /// Distinct from [`Error`](Self::Error), which reports a rejected *command*. This one
    /// reports a document the build cannot read at all, and the remedy is to update the
    /// client.
    DocumentRefused { reason: String },

    /// At least one record in the workbook could not be decoded and lost its formatting
    /// (`SCHEMA-003`). A fact rather than a count. The same unreadable register is met
    /// again on every later reseed. Any number reported here would be an artefact of how
    /// often the grid was rebuilt.
    UndecodableRecords,

    /// A local [`SortRange`](super::TrameCommand::SortRange) finished
    /// reordering the rows (`trame-sort`). The accompanying
    /// [`DataReset`](Self::DataReset) prompts the UI to re-fetch the viewport with
    /// the new ordering. This event is the explicit completion signal, which
    /// clears a "sorting..." affordance. A no-op sort emits nothing.
    SortComplete,

    /// A local [`ApplyFilter`](super::TrameCommand::ApplyFilter) finished
    /// recomputing the hidden rows (`trame-shared-filter`).
    /// `hidden_row_count` is how many rows the filter hides. The accompanying
    /// [`DataReset`](Self::DataReset) prompts the UI to re-fetch the viewport,
    /// where the hidden rows now carry [`DisplayCell::is_filtered`]. A no-op
    /// filter (non-active sheet / virtual column) emits nothing. `col` is the
    /// grid column the filter targets, so the UI can draw the funnel badge on its
    /// header (`trame-shared-filter`).
    FilterApplied { hidden_row_count: u32, col: u32 },

    /// The distinct visible values of a column, replying to
    /// [`RequestActiveColumnValues`](super::TrameCommand::RequestActiveColumnValues).
    /// Feeds the shared filter popover's "by values" checkbox list
    /// (`trame-shared-filter`). `col` is the grid column the values belong to;
    /// `values` is the deduplicated, sorted display text of every non-hidden row.
    ColumnValues { col: u32, values: Vec<String> },

    /// Selection aggregates for the status bar (`trame-status-bar`), replying to
    /// [`RequestSelectionStats`](super::TrameCommand::RequestSelectionStats). `count` is
    /// the number of non-empty cells in the selection. `count_num` is the number of numeric
    /// cells, and drives whether the UI shows `sum`/`avg`/`min`/`max` (they are
    /// meaningless, and left at `0.0`, when `count_num == 0`). `avg` is `sum / count_num`
    /// (0.0 for no numeric cell).
    SelectionStats {
        sum: f64,
        avg: f64,
        count: u64,
        count_num: u64,
        min: f64,
        max: f64,
    },

    /// A local [`ClearFilter`](super::TrameCommand::ClearFilter) removed the
    /// active shared filter (`trame-shared-filter`). The accompanying
    /// [`DataReset`](Self::DataReset) prompts a viewport re-fetch with every row
    /// visible again. A no-op clear (no active filter) emits nothing.
    FilterCleared,

    /// The matches of a [`FindNext`](super::TrameCommand::FindNext) on the active sheet
    /// (`trame-find-replace`). `matches` lists every matching cell as a `(row, col)` grid
    /// pair in scan order. `active` is the index within `matches` of the cell to focus
    /// (`None` when there is no match). Local view state: the UI highlights the set and
    /// scrolls the focused cell into view; it is never synced. An empty `matches` means
    /// "not found".
    FindResult {
        matches: Vec<(u32, u32)>,
        active: Option<usize>,
    },

    /// A [`ReplaceAll`](super::TrameCommand::ReplaceAll) finished, having
    /// rewritten `replaced` cells (`trame-find-replace`). Paired with a
    /// [`DataReset`](Self::DataReset) that prompts the viewport re-fetch; the
    /// chrome surfaces the count as a toast.
    ReplaceDone { replaced: u32 },

    /// A local [`PasteRange`](super::TrameCommand::PasteRange) was refused
    /// because the destination overlaps a protected range the current user may
    /// not edit (`trame-clipboard`, range protection is Post-MVP). No CRDT
    /// mutation occurred; the chrome surfaces `reason` as a toast.
    PasteRejected { reason: String },

    /// The authoritative clipboard payload for a
    /// [`CopyRange`](super::TrameCommand::CopyRange) (`trame-clipboard`). Built
    /// Logic-side from the full document (no viewport limit); the UI stashes it in
    /// memory and mirrors it to the OS clipboard. Non-mutating, never synced.
    Clipboard(super::ClipboardPayload),

    // -------------------------------------------------------------------------
    // Deferred feature surface (Phase 2+)
    //
    // Completes the `TrameEvent` surface mandated by
    // `trame-engine` ("all feature variants are in the
    // enums"). Emitted once the matching command handlers land (personal filter
    // views / conditional formatting -> Phase 2/3, validation -> Phase 2/3,
    // range protection -> Post-MVP).
    // -------------------------------------------------------------------------
    /// A personal filter view was persisted to local storage
    /// ([`SaveFilterView`](super::TrameCommand::SaveFilterView),
    /// `trame-shared-filter`). `view_id` is its stable local id.
    FilterViewSaved { view_id: String },

    /// The sheet's conditional-formatting rules changed
    /// ([`AddCondFormat`](super::TrameCommand::AddCondFormat) /
    /// [`UpdateCondFormat`](super::TrameCommand::UpdateCondFormat) /
    /// [`RemoveCondFormat`](super::TrameCommand::RemoveCondFormat),
    /// `trame-cell-styles`).
    /// The accompanying [`DataReset`](Self::DataReset) prompts a viewport
    /// re-fetch with the new style overrides applied.
    CondFormatsUpdated { version: u64 },

    /// A local edit was rejected by a [`ValidationOnInvalid::Reject`] rule
    /// (`trame-cell-editing`). No CRDT delta was produced; the
    /// chrome surfaces `message`. `version` echoes the originating edit.
    ///
    /// [`ValidationOnInvalid::Reject`]: super::ValidationOnInvalid::Reject
    ValidationError {
        version: u64,
        row: u32,
        col: u32,
        message: String,
    },

    /// A local edit passed through despite failing a
    /// [`ValidationOnInvalid::Warn`] rule (`trame-cell-editing`). The mutation was
    /// applied; the chrome surfaces `message` as a non-blocking warning.
    ///
    /// [`ValidationOnInvalid::Warn`]: super::ValidationOnInvalid::Warn
    ValidationWarning {
        version: u64,
        row: u32,
        col: u32,
        message: String,
    },

    /// A local edit was blocked by a range protection the current user may not
    /// edit (`trame-cell-editing`, Post-MVP). No CRDT delta was
    /// produced; the chrome surfaces `description`.
    ProtectionDenied {
        version: u64,
        row: u32,
        col: u32,
        description: String,
    },

    /// A [`ImportCsv`](super::TrameCommand::ImportCsv) finished writing
    /// the parsed rows into the workbook (`trame-import-export`).
    /// Rides alongside a [`DataReset`](Self::DataReset) so the UI re-fetches the
    /// populated region. `rows` / `cols` are the dimensions actually written;
    /// `sheet_id` is the (possibly freshly created) target sheet.
    ImportComplete {
        sheet_id: String,
        rows: u32,
        cols: u32,
    },

    /// An export ([`ExportCsv`](super::TrameCommand::ExportCsv)) produced
    /// a file for download (`trame-import-export`). The Logic thread
    /// holds the document, so it serializes the bytes; the UI Thread receives them
    /// and triggers the browser download. `filename` already carries the right
    /// extension.
    ExportComplete { filename: String, data: Vec<u8> },

    /// A remote delta edited a protected range its author was not allowed to
    /// (`trame-cell-editing`). The delta IS applied (CRDT convergence is
    /// mandatory) but the UI is warned. `from_user_id` is the offending author's
    /// stable
    /// [`UserId`](crate::UserId).
    ProtectionViolationWarning {
        row: u32,
        col: u32,
        from_user_id: UserId,
        description: String,
    },
}

/// A single cell ready for display.
///
/// Flat, data-oriented. The UI Thread stores these in a
/// `HashMap<(u32, u32), DisplayCell>` keyed by `(row, col)`; missing entries
/// are blank (white background, no text), so only non-empty cells are sent.
#[derive(Debug, Clone, Default)]
pub struct DisplayCell {
    pub row: u32,
    pub col: u32,
    /// Display text (formula result or raw string).
    pub text: String,
    /// Raw cell text as stored in the CRDT (for example `=SUM(A1:B1)` for a formula).
    /// Used to seed the in-cell editor so editing a formula shows the formula,
    /// not its computed result. Equals `text` for plain cells.
    pub raw: String,
    /// Bold run applied to this cell (pinned to the cell, not the selection).
    pub bold: bool,
    /// Italic run applied to this cell.
    pub italic: bool,
    /// Underline run applied to this cell (`trame-cell-styles`).
    pub underline: bool,
    /// Strikethrough run applied to this cell.
    pub strikethrough: bool,
    /// Horizontal text alignment.
    pub h_align: super::HAlign,
    /// Vertical text alignment.
    pub v_align: super::VAlign,
    /// Wrap long text onto multiple lines inside the cell.
    pub wrap: bool,
    /// Text colour, packed `0xRRGGBB` (`None` = theme default).
    pub fg: Option<u32>,
    /// Fill colour, packed `0xRRGGBB` (`None` = transparent).
    pub bg: Option<u32>,
    /// Font family name (`None` = grid default).
    pub font: Option<String>,
    /// Font size in points (`None` = grid default).
    pub size: Option<u16>,
    /// Per-edge border (style + colour each); `None` edges are undrawn.
    pub borders: super::Borders,
    /// Part of a merged range (`trame-cell-merges`). `true` for both
    /// the master cell and the cells it covers; the UI distinguishes them via
    /// [`merge_span`](DisplayCell::merge_span).
    pub is_merged: bool,
    /// `Some((col_span, row_span))` on the master (top-left) cell of a merge,
    /// giving how many columns/rows it spans. `None` on a covered cell (rendered
    /// blank, swallowed by the master) and on any non-merged cell.
    pub merge_span: Option<(u32, u32)>,
    /// The cell's row is hidden by the active shared filter
    /// (`trame-shared-filter`). The UI Thread collapses such
    /// rows; the value is still present, and the aggregates such as `=SUM` still
    /// count it.
    pub is_filtered: bool,
}
