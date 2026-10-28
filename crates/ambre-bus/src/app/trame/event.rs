//! Trame (spreadsheet) event surface (Logic thread -> UI Thread).
//!
//! The `TrameEvent` family plus its display value type [`DisplayCell`], carried
//! by move inside [`AppEvent::Trame`](crate::ui::AppEvent). The command-side
//! value types it reuses (`CellStyle`, `HAlign`, `VAlign`, `Borders`, ...) arrive
//! by glob from the sibling command modules of `crate::app::trame`.

use super::{CellPos, CellRect, Col, ColCount, EditSessionId, MergeSpan, Row, RowCount, SheetName};
use crate::UserId;
use ambre_core::{DocPx, Rgb};

// -----------------------------------------------------------------------------
// Trame (spreadsheet) -- events (Logic -> UI)
// -----------------------------------------------------------------------------

/// The totals of the numeric cells of a selection (`trame-status-bar`). There
/// is always at least one number: a selection with none has no totals, so no
/// aggregate is ever an empty one's zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumericStats {
    count: CellCount,
    sum: f64,
    min: f64,
    max: f64,
}

impl NumericStats {
    /// The totals of the single number `n`.
    #[must_use]
    pub fn of(n: f64) -> Self {
        Self {
            count: CellCount::new(1),
            sum: n,
            min: n,
            max: n,
        }
    }

    /// The totals with one more number, `n`.
    #[must_use]
    pub fn with(self, n: f64) -> Self {
        Self {
            count: self.count.incremented(),
            sum: self.sum + n,
            min: self.min.min(n),
            max: self.max.max(n),
        }
    }

    /// The totals of `numbers`, `None` when there is none.
    pub fn of_all(numbers: impl IntoIterator<Item = f64>) -> Option<Self> {
        numbers.into_iter().fold(None, |stats, n| {
            Some(stats.map_or_else(|| Self::of(n), |stats: Self| stats.with(n)))
        })
    }

    /// How many numbers were totalled, never zero.
    #[must_use]
    pub const fn count(&self) -> CellCount {
        self.count
    }

    #[must_use]
    pub const fn sum(&self) -> f64 {
        self.sum
    }

    /// The sum over the count.
    #[must_use]
    pub fn avg(&self) -> f64 {
        self.sum / self.count.as_f64()
    }

    #[must_use]
    pub const fn min(&self) -> f64 {
        self.min
    }

    #[must_use]
    pub const fn max(&self) -> f64 {
        self.max
    }
}

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
        version: crate::ViewportVersion,
        row_start: Row,
        col_start: Col,
        cells: Vec<DisplayCell>,
        /// Per-column widths (document px, before the zoom) for the requested column window,
        /// `col_widths[i]` being the width of column `col_start + i`
        /// (`trame-resize`, `trame-engine`). A column never resized carries
        /// the uniform default, so the UI can feed the slice straight into the
        /// variable-metric `GridLayout`. Empty only on an out-of-range request.
        col_widths: Vec<DocPx>,
        /// Per-row heights (document px, before the zoom) for the requested row window, aligned to
        /// `row_start` the same way [`col_widths`](Self::ViewportData::col_widths)
        /// is to `col_start`.
        row_heights: Vec<DocPx>,
    },

    /// The resolved formatting of the **active cell** after a
    /// [`SetCellStyle`](super::TrameCommand::SetCellStyle) or a
    /// [`SetActiveCell`](super::TrameCommand::SetActiveCell). It drives the whole
    /// formatting toolbar: bold/italic/underline toggles, font/size selects, colour
    /// swatches, align buttons, cell-format / border menus, wrap. Bold and italic ride
    /// alongside the [`CellStyle`]. They are part of the single style event, not a
    /// separate per-flag event (`trame-cell-styles`).
    ///
    /// [`CellStyle`]: super::CellStyle
    StyleApplied {
        style: super::CellStyle,
        bold: super::Weight,
        italic: super::Slant,
        /// Whether the active cell is part of a merge (a master or a covered
        /// cell), driving the toolbar's merge/unmerge toggle pressed state.
        merged: Merge,
        /// Whether the active cell's display value parses as a number. Lets the toolbar
        /// resolve [`HAlign::Auto`](super::HAlign::Auto) the same way the renderer does,
        /// numbers right and text left. The alignment control then reflects the de-facto
        /// alignment of an `Auto` cell.
        numeric: CellValueKind,
    },

    /// The document's content was replaced wholesale (local snapshot restore at
    /// startup). The UI Thread re-requests the visible region: the initial
    /// viewport response predated the restore, so the cache must be refreshed.
    DataReset,

    /// Ask the UI Thread to replace the rendered grid selection with the
    /// rectangle `rect`. Emitted when a
    /// command changes the area the user should see selected and the canvas
    /// cannot derive it locally. An unmerge is the case: the freed cells must
    /// stay selected, though the merge that defined the span is gone.
    /// The active cell collapses onto the top-left corner.
    SelectRange { rect: CellRect },

    /// A local [`MergeCells`](super::TrameCommand::MergeCells) was refused
    /// because a covered cell is non-empty (`trame-cell-merges`). The
    /// chrome surfaces `reason` to the user; no CRDT mutation was produced.
    MergeRejected { reason: String },

    /// A merge ended up covering a non-empty cell after a concurrent edit
    /// converged (`trame-cell-merges`). The covered value is masked (not
    /// destroyed) while the merge is active; it reappears on unmerge. Purely
    /// informational for the UI.
    MergeConflictDetected {
        master_row: Row,
        master_col: Col,
        covered_row: Row,
        covered_col: Col,
    },

    /// Availability of undo/redo after a local
    /// [`Undo`](super::TrameCommand::Undo) /
    /// [`Redo`](super::TrameCommand::Redo). Drives the enabled state of the
    /// chrome's Undo/Redo affordances. Reflects Loro's `UndoManager` stacks, whose
    /// scope is the current document session.
    UndoState {
        undo: ambre_core::Availability,
        redo: ambre_core::Availability,
    },

    /// The workbook's sheets, in tab-strip order.
    /// Emitted after any sheet-lifecycle command
    /// ([`CreateSheet`](super::TrameCommand::CreateSheet) /
    /// [`RenameSheet`](super::TrameCommand::RenameSheet) /
    /// [`MoveSheet`](super::TrameCommand::MoveSheet) /
    /// [`DeleteSheet`](super::TrameCommand::DeleteSheet)) and on load, so
    /// the tab strip stays in sync (`trame-sheets`). `active`
    /// is the id of the sheet currently shown.
    SheetList {
        sheets: Vec<SheetTab>,
        active: SheetId,
    },

    /// A command was rejected for a reason the user should see (for example deleting the
    /// last remaining sheet). Purely informational; no CRDT mutation occurred.
    Error { message: String },

    /// A peer's insert, delete or move carried the cell of edit `session` to
    /// `(row, col)` on the active sheet. The open editor follows it, so the
    /// cell it shows is the cell its commit reaches.
    EditTargetMoved {
        session: EditSessionId,
        row: Row,
        col: Col,
    },

    /// A peer deleted the sheet, row or column under edit `session`. The Logic
    /// thread dropped the session, so its commit writes nothing; the editor
    /// closes and the user is told the cell is gone.
    EditTargetGone { session: EditSessionId },

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

    /// The range a parameterless sort would order, replying to
    /// [`RequestSortRange`](super::TrameCommand::RequestSortRange): `(r1,c1)`-`(r2,c2)`,
    /// inclusive, corners ordered. The sort dialog opens over it (`TRAME-UI-101`).
    SortRange { r1: Row, c1: Col, r2: Row, c2: Col },

    /// Every non-empty cell of grid column `col`, all rows, replying to
    /// [`RequestColumnContent`](super::TrameCommand::RequestColumnContent). The
    /// canvas measures them for an auto-fit (`TRAME-UI-128`).
    ColumnContent { col: Col, cells: Vec<DisplayCell> },

    /// A local [`ApplyFilter`](super::TrameCommand::ApplyFilter) finished
    /// recomputing the hidden rows (`trame-shared-filter`).
    /// `hidden_row_count` is how many rows the filter hides. The accompanying
    /// [`DataReset`](Self::DataReset) prompts the UI to re-fetch the viewport,
    /// where the hidden rows now carry [`DisplayCell::is_filtered`]. A no-op
    /// filter (non-active sheet / virtual column) emits nothing. `col` is the
    /// grid column the filter targets, so the UI can draw the funnel badge on its
    /// header (`trame-shared-filter`).
    FilterApplied {
        hidden_row_count: RowCount,
        col: Col,
    },

    /// The distinct visible values of a column, replying to
    /// [`RequestActiveColumnValues`](super::TrameCommand::RequestActiveColumnValues).
    /// Feeds the shared filter popover's "by values" checkbox list
    /// (`trame-shared-filter`). `col` is the grid column the values belong to;
    /// `values` is the deduplicated, sorted display text of every non-hidden row.
    ColumnValues { col: Col, values: Vec<String> },

    /// Selection aggregates for the status bar (`trame-status-bar`), replying to
    /// [`RequestSelectionStats`](super::TrameCommand::RequestSelectionStats). `count` is
    /// the number of non-empty cells in the selection. `numbers` totals its numeric
    /// cells, and is `None` when it holds none: the UI then shows no numeric
    /// aggregate.
    SelectionStats {
        count: CellCount,
        numbers: Option<NumericStats>,
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
        matches: Vec<CellPos>,
        active: Option<crate::app::find::MatchIndex>,
    },

    /// A [`ReplaceAll`](super::TrameCommand::ReplaceAll) finished, having
    /// rewritten `replaced` cells (`trame-find-replace`). Paired with a
    /// [`DataReset`](Self::DataReset) that prompts the viewport re-fetch; the
    /// chrome surfaces the count as a toast.
    ReplaceDone {
        replaced: crate::app::find::MatchCount,
    },

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
    FilterViewSaved { view_id: FilterViewId },

    /// The sheet's conditional-formatting rules changed
    /// ([`AddCondFormat`](super::TrameCommand::AddCondFormat) /
    /// [`UpdateCondFormat`](super::TrameCommand::UpdateCondFormat) /
    /// [`RemoveCondFormat`](super::TrameCommand::RemoveCondFormat),
    /// `trame-cell-styles`).
    /// The accompanying [`DataReset`](Self::DataReset) prompts a viewport
    /// re-fetch with the new style overrides applied.
    CondFormatsUpdated { version: crate::ViewportVersion },

    /// A local edit was rejected by a [`ValidationOnInvalid::Reject`] rule
    /// (`trame-cell-editing`). No CRDT delta was produced; the
    /// chrome surfaces `message`. `version` echoes the originating edit.
    ///
    /// [`ValidationOnInvalid::Reject`]: super::ValidationOnInvalid::Reject
    ValidationError {
        version: crate::ViewportVersion,
        row: Row,
        col: Col,
        message: String,
    },

    /// A local edit passed through despite failing a
    /// [`ValidationOnInvalid::Warn`] rule (`trame-cell-editing`). The mutation was
    /// applied; the chrome surfaces `message` as a non-blocking warning.
    ///
    /// [`ValidationOnInvalid::Warn`]: super::ValidationOnInvalid::Warn
    ValidationWarning {
        version: crate::ViewportVersion,
        row: Row,
        col: Col,
        message: String,
    },

    /// A local edit was blocked by a range protection the current user may not
    /// edit (`trame-cell-editing`, Post-MVP). No CRDT delta was
    /// produced; the chrome surfaces `description`.
    ProtectionDenied {
        version: crate::ViewportVersion,
        row: Row,
        col: Col,
        description: String,
    },

    /// A [`ImportCsv`](super::TrameCommand::ImportCsv) finished writing
    /// the parsed rows into the workbook (`trame-import-export`).
    /// Rides alongside a [`DataReset`](Self::DataReset) so the UI re-fetches the
    /// populated region. `rows` / `cols` are the dimensions actually written;
    /// `sheet_id` is the (possibly freshly created) target sheet.
    ImportComplete {
        sheet_id: SheetId,
        rows: RowCount,
        cols: ColCount,
    },

    /// An export ([`ExportCsv`](super::TrameCommand::ExportCsv)) produced
    /// a file for download (`trame-import-export`). The Logic thread
    /// holds the document, so it serializes the bytes; the UI Thread receives them
    /// and triggers the browser download. `filename` already carries the right
    /// extension.
    ExportComplete {
        filename: ambre_core::ExportFilename,
        data: ambre_core::FileBytes,
    },

    /// A remote delta edited a protected range its author was not allowed to
    /// (`trame-cell-editing`). The delta IS applied (CRDT convergence is
    /// mandatory) but the UI is warned. `from_user_id` is the offending author's
    /// stable
    /// [`UserId`](crate::UserId).
    ProtectionViolationWarning {
        row: Row,
        col: Col,
        from_user_id: UserId,
        description: String,
    },
}

/// Stable identity of a sheet, peer-scoped where it is minted
/// (`{peer:016x}_{counter:08x}`). A rename leaves it unchanged
/// (`trame-sheets`).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct SheetId(pub String);

impl SheetId {
    /// The id as the CRDT container names spell it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A map keyed by sheet id answers a lookup by the bare text, as a map keyed
/// by `String` does: both hash the same bytes.
impl std::borrow::Borrow<str> for SheetId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SheetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable local identity of a personal filter view, minted where the view is
/// saved and never shared (`trame-shared-filter`).
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct FilterViewId(pub String);

/// Stable identity of one conditional-formatting rule on a sheet, which an
/// update or a removal names (`trame-cell-styles`).
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct CondFormatId(pub String);

/// Where a conditional-formatting rule falls in evaluation order: the
/// smaller is evaluated first, and the first rule that matches wins
/// (`trame-cell-styles`). It orders rules and counts nothing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct CondFormatPriority(i64);

impl CondFormatPriority {
    /// The rank `rank`, negative ranks first.
    #[must_use]
    pub const fn new(rank: i64) -> Self {
        Self(rank)
    }

    /// The rank itself.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

pub use ambre_core::CellCount;

ambre_core::define_index! {
    /// A place in the sheet tab strip, 0-based from the left. It orders the
    /// tabs and names no sheet.
    pub struct SheetTabPos(u32);
    /// A number of tabs.
    pub struct SheetTabCount;
    /// The signed gap between two tabs.
    pub struct SheetTabDelta(i64);
    /// A run of tabs.
    pub struct SheetTabSpan;
    /// A list indexed by tab.
    pub struct SheetTabVec;
}

/// One tab of the sheet strip: the sheet's id and its display name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SheetTab {
    pub id: SheetId,
    pub name: SheetName,
}

/// A single cell ready for display.
///
/// Flat, data-oriented. The UI Thread stores these in a
/// `HashMap<CellPos, DisplayCell>`; missing entries
/// are blank (white background, no text), so only non-empty cells are sent.
#[derive(Debug, Clone, Default)]
pub struct DisplayCell {
    pub row: Row,
    pub col: Col,
    /// Display text (formula result or raw string).
    pub text: String,
    /// Raw cell text as stored in the CRDT (for example `=SUM(A1:B1)` for a formula).
    /// Used to seed the in-cell editor so editing a formula shows the formula,
    /// not its computed result. Equals `text` for plain cells.
    pub raw: String,
    /// Bold run applied to this cell (pinned to the cell, not the selection).
    pub bold: super::Weight,
    /// Italic run applied to this cell.
    pub italic: super::Slant,
    /// Underline run applied to this cell (`trame-cell-styles`).
    pub underline: super::Underline,
    /// Strikethrough run applied to this cell.
    pub strikethrough: super::Strikethrough,
    /// Horizontal text alignment.
    pub h_align: super::HAlign,
    /// The cell's computed value is a number, before any format renders it.
    /// `HAlign::Auto` aligns such a cell right. The Logic thread decides it
    /// once, and the toolbar reads the same test (`TRAME-UI-093`, `TRAME-186`).
    /// The formatted `text` of a currency or a fraction no longer parses as one.
    pub numeric: CellValueKind,
    /// Vertical text alignment.
    pub v_align: super::VAlign,
    /// Wrap long text onto multiple lines inside the cell.
    pub wrap: super::WordBreak,
    /// Text colour, packed `0xRRGGBB` (`None` = theme default).
    pub fg: Option<Rgb>,
    /// Fill colour, packed `0xRRGGBB` (`None` = transparent).
    pub bg: Option<Rgb>,
    /// Font family (`None` = grid default).
    pub font: Option<ambre_core::FontFamily>,
    /// Font size (`None` = grid default).
    pub size: Option<ambre_core::FontSize>,
    /// Per-edge border (style + colour each); `None` edges are undrawn.
    pub borders: super::Borders,
    /// Part of a merged range (`trame-cell-merges`). `true` for both
    /// the master cell and the cells it covers; the UI distinguishes them via
    /// [`merge_span`](DisplayCell::merge_span).
    pub is_merged: Merge,
    /// `Some(span)` on the master (top-left) cell of a merge, giving how many
    /// rows and columns it spans. `None` on a covered cell (rendered
    /// blank, swallowed by the master) and on any non-merged cell.
    pub merge_span: Option<MergeSpan>,
    /// The cell's row is hidden by the active shared filter
    /// (`trame-shared-filter`). The UI Thread collapses such
    /// rows; the value is still present, and the aggregates such as `=SUM` still
    /// count it.
    pub is_filtered: RowFilter,
}

ambre_core::define_flag! {
    /// Whether a cell is part of a merged range (`trame-cell-merges`): the
    /// master cell or a cell it covers.
    pub enum Merge {
        /// A cell of its own.
        Single,
        /// The master of a merge, or a cell it covers.
        Merged,
    }

    /// Whether the active shared filter hides a cell's row
    /// (`trame-shared-filter`).
    pub enum RowFilter {
        /// The row is drawn.
        Shown,
        /// The row is collapsed; its values still count in an aggregate.
        Filtered,
    }

    /// What a cell's computed value is, before any format renders it.
    /// `HAlign::Auto` aligns a number right and text left (`TRAME-UI-093`).
    pub enum CellValueKind {
        /// Text, an error, an empty value or a number that is not finite.
        Text,
        /// A number.
        Number,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Totals exist only over at least one number, and the average is the sum
    /// over the count.
    #[test]
    fn numeric_totals_need_a_number_and_average_over_their_count() {
        assert_eq!(NumericStats::of_all([]), None);
        let totals = NumericStats::of_all([10.0, -4.0, 6.0]).expect("three numbers");
        assert_eq!(totals.count().get(), 3);
        assert_eq!(totals.sum(), 12.0);
        assert_eq!(totals.avg(), 4.0);
        assert_eq!((totals.min(), totals.max()), (-4.0, 10.0));
    }
}
