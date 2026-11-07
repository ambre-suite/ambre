//! Trame (spreadsheet) event surface (Logic thread -> UI Thread).
//!
//! The `TrameEvent` family plus its display value type [`DisplayCell`], carried
//! by move inside `AppEvent::Trame` of the Logic runtime. The command-side
//! value types it reuses (`CellStyle`, `HAlign`, `VAlign`, `Borders`, ...) arrive
//! by glob from the sibling command modules of `crate::message`.

use std::cmp::Ordering;

use super::{CellPos, CellRect, Col, EditSession, MergeSpan, Row, SheetName};
use ambre_bus::token::Token;
use ambre_core::id::{FilterView, Id, Sheet, User};
use ambre_core::index::{Axis, Len, Pos};
use ambre_core::{Bounded, Bounds, Count, Doc, Px, RankKind, Refusal, Rgb};

// -----------------------------------------------------------------------------
// Trame (spreadsheet) -- events (Logic -> UI)
// -----------------------------------------------------------------------------

/// A number a cell holds, as it crosses a public struct or the bus:
/// `Bounded<CellFloat>`, always finite. The formula engine computes in
/// `f64`, and a NaN or an infinity it yields, or a literal it parses, is no
/// cell number (`trame-status-bar`).
///
/// The bounds are the lowest and the highest finite `f64`. `Bounded::new`
/// refuses a NaN, which compares false with both, and each infinity, which
/// lies past one. So the bounds check is the finite check. `Debug` keeps the
/// name of the hand-written `CellNumber`, and a refusal keeps its message.
pub enum CellFloat {}

impl Bounds for CellFloat {
    type Repr = f64;
    const MIN: f64 = f64::MIN;
    const MAX: f64 = f64::MAX;
    const NAME: &'static str = "CellNumber";
    const REFUSAL: Refusal = Refusal::new("", " is not a finite number");
}

/// The totals of the numeric cells of a selection (`trame-status-bar`). There
/// is always at least one number: a selection with none has no totals, so no
/// aggregate is ever an empty one's zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumericStats {
    count: Count<Cells>,
    /// The total `of_all` was handed. Its caller passes the exact total of
    /// the numbers, `None` when it is past the largest finite number.
    sum: Option<Bounded<CellFloat>>,
    min: Bounded<CellFloat>,
    max: Bounded<CellFloat>,
}

impl NumericStats {
    /// The totals of `numbers`, `None` when there is none.
    ///
    /// The caller passes in `sum` the exact total of `numbers`, as the Logic
    /// thread's formulas compute it. It passes `None` for a total past the
    /// largest finite number. The band and a formula over the same cells then read
    /// one total (`TRAME-FEAT-029`). This crate cannot see that sum, so the
    /// type does not check it: `of_all` keeps the `sum` it is handed.
    #[must_use]
    pub fn of_all(numbers: &[Bounded<CellFloat>], sum: Option<Bounded<CellFloat>>) -> Option<Self> {
        let (&first, rest) = numbers.split_first()?;
        // A cell number is never NaN, so `partial_cmp` always orders two.
        let (min, max) = rest.iter().fold((first, first), |(min, max), &n| {
            (
                if n.partial_cmp(&min) == Some(Ordering::Less) {
                    n
                } else {
                    min
                },
                if n.partial_cmp(&max) == Some(Ordering::Greater) {
                    n
                } else {
                    max
                },
            )
        });
        Some(Self {
            count: Count::<Cells>::of(numbers.len()),
            sum,
            min,
            max,
        })
    }

    /// How many numbers were totalled, never zero.
    #[must_use]
    pub const fn count(&self) -> Count<Cells> {
        self.count
    }

    /// The sum, `None` when it overflows past the largest finite number.
    #[must_use]
    pub const fn sum(&self) -> Option<Bounded<CellFloat>> {
        self.sum
    }

    /// The sum over the count, `None` when the sum overflows. A finite sum
    /// over a count of at least one stays finite.
    #[must_use]
    pub fn avg(&self) -> Option<Bounded<CellFloat>> {
        self.sum
            .and_then(|sum| Bounded::new(sum.get() / self.count.as_f64()))
    }

    #[must_use]
    pub const fn min(&self) -> Bounded<CellFloat> {
        self.min
    }

    #[must_use]
    pub const fn max(&self) -> Bounded<CellFloat> {
        self.max
    }
}

/// Display events emitted by the Trame logic to the UI Thread.
#[derive(Clone)]
pub enum TrameEvent {
    /// Cell content for a requested viewport region.
    ///
    /// `row_start` / `col_start` are the top-left corner of the batch; `cells`
    /// holds all non-empty cells within the requested range. The UI Thread
    /// merges this batch into its local cache. `version` echoes the originating
    /// [`TrameCommand::ScrollViewport`](super::TrameCommand::ScrollViewport)
    /// so stale responses can be dropped.
    ViewportData {
        version: ambre_core::Seq<ambre_bus::ViewportVersion>,
        row_start: Pos<Row>,
        col_start: Pos<Col>,
        cells: Vec<DisplayCell>,
        /// Per-column widths (document px, before the zoom) for the requested column window,
        /// `col_widths[i]` being the width of column `col_start + i`
        /// (`trame-resize`, `trame-engine`). A column never resized carries
        /// the uniform default, so the UI can feed the slice straight into the
        /// variable-metric `GridLayout`. Empty only on an out-of-range request.
        col_widths: Vec<Px<Doc>>,
        /// Per-row heights (document px, before the zoom) for the requested row window, aligned to
        /// `row_start` the same way [`col_widths`](Self::ViewportData::col_widths)
        /// is to `col_start`.
        row_heights: Vec<Px<Doc>>,
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

    /// The cells whose shown value a cell write or a remote batch changed, as
    /// one rectangle (`TRAME-284`). The UI Thread re-fetches the part of its
    /// window the rectangle covers, and nothing else (`TRAME-UI-184`).
    /// `version` is the last viewport request the Logic thread answered before
    /// the change (`TRAME-UI-005`). The answer to any later request already
    /// shows the change.
    CellsChanged {
        version: ambre_core::Seq<ambre_bus::ViewportVersion>,
        range: CellRect,
    },

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
        master_row: Pos<Row>,
        master_col: Pos<Col>,
        covered_row: Pos<Row>,
        covered_col: Pos<Col>,
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
        active: Id<Sheet>,
    },

    /// The workbook's locale and the reader's half it is read under. Sent on
    /// load, on a set, on a peer's set and on a change of the reader's half.
    /// A view says so when the locale it shows is not the workbook's
    /// (`LOCALE-021`).
    LocaleState {
        document: ambre_core::DocumentLocale,
        reader: ambre_core::ReaderLocale,
    },

    /// A command was rejected for a reason the user should see (for example deleting the
    /// last remaining sheet). Purely informational; no CRDT mutation occurred.
    Error { message: String },

    /// A peer's insert, delete or move carried the cell of edit `session` to
    /// `(row, col)` on the active sheet. The open editor follows it, so the
    /// cell it shows is the cell its commit reaches.
    EditTargetMoved {
        session: Token<EditSession>,
        row: Pos<Row>,
        col: Pos<Col>,
    },

    /// A peer deleted the sheet, row or column under edit `session`. The Logic
    /// thread dropped the session, so its commit writes nothing; the editor
    /// closes and the user is told the cell is gone.
    EditTargetGone { session: Token<EditSession> },

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
    SortRange {
        r1: Pos<Row>,
        c1: Pos<Col>,
        r2: Pos<Row>,
        c2: Pos<Col>,
    },

    /// Every non-empty cell of grid column `col`, all rows, replying to
    /// [`RequestColumnContent`](super::TrameCommand::RequestColumnContent). The
    /// canvas measures them for an auto-fit (`TRAME-UI-128`).
    ColumnContent {
        col: Pos<Col>,
        cells: Vec<DisplayCell>,
    },

    /// A local [`ApplyFilter`](super::TrameCommand::ApplyFilter) finished
    /// recomputing the hidden rows (`trame-shared-filter`).
    /// `hidden_row_count` is how many rows the filter hides. The accompanying
    /// [`DataReset`](Self::DataReset) prompts the UI to re-fetch the viewport,
    /// where the hidden rows now carry [`DisplayCell::is_filtered`]. A no-op
    /// filter (non-active sheet / virtual column) emits nothing. `col` is the
    /// grid column the filter targets, so the UI can draw the funnel badge on its
    /// header (`trame-shared-filter`).
    FilterApplied {
        hidden_row_count: Len<Row>,
        col: Pos<Col>,
    },

    /// The distinct visible values of a column, replying to
    /// [`RequestActiveColumnValues`](super::TrameCommand::RequestActiveColumnValues).
    /// Feeds the shared filter popover's "by values" checkbox list
    /// (`trame-shared-filter`). `col` is the grid column the values belong to;
    /// `values` is the deduplicated, sorted display text of every non-hidden row.
    ColumnValues { col: Pos<Col>, values: Vec<String> },

    /// Selection aggregates for the status bar (`trame-status-bar`), replying to
    /// [`RequestSelectionStats`](super::TrameCommand::RequestSelectionStats). `count` is
    /// the number of non-empty cells in the selection. `numbers` totals its numeric
    /// cells, and is `None` when it holds none: the UI then shows no numeric
    /// aggregate.
    SelectionStats {
        count: Count<Cells>,
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
        active: Option<Pos<ambre_bus::app::find::Match>>,
    },

    /// A [`ReplaceAll`](super::TrameCommand::ReplaceAll) finished, having
    /// rewritten `replaced` cells (`trame-find-replace`). Paired with a
    /// [`DataReset`](Self::DataReset) that prompts the viewport re-fetch; the
    /// chrome surfaces the count as a toast.
    ReplaceDone {
        replaced: Len<ambre_bus::app::find::Match>,
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
    FilterViewSaved { view_id: Id<FilterView> },

    /// The sheet's conditional-formatting rules changed
    /// ([`AddCondFormat`](super::TrameCommand::AddCondFormat) /
    /// [`UpdateCondFormat`](super::TrameCommand::UpdateCondFormat) /
    /// [`RemoveCondFormat`](super::TrameCommand::RemoveCondFormat),
    /// `trame-cell-styles`).
    /// The accompanying [`DataReset`](Self::DataReset) prompts a viewport
    /// re-fetch with the new style overrides applied.
    CondFormatsUpdated {
        version: ambre_core::Seq<ambre_bus::ViewportVersion>,
    },

    /// A local edit was rejected by a [`ValidationOnInvalid::Reject`] rule
    /// (`trame-cell-editing`). No CRDT delta was produced; the
    /// chrome surfaces `message`. `version` echoes the originating edit.
    ///
    /// [`ValidationOnInvalid::Reject`]: super::ValidationOnInvalid::Reject
    ValidationError {
        version: ambre_core::Seq<ambre_bus::ViewportVersion>,
        row: Pos<Row>,
        col: Pos<Col>,
        message: String,
    },

    /// A local edit passed through despite failing a
    /// [`ValidationOnInvalid::Warn`] rule (`trame-cell-editing`). The mutation was
    /// applied; the chrome surfaces `message` as a non-blocking warning.
    ///
    /// [`ValidationOnInvalid::Warn`]: super::ValidationOnInvalid::Warn
    ValidationWarning {
        version: ambre_core::Seq<ambre_bus::ViewportVersion>,
        row: Pos<Row>,
        col: Pos<Col>,
        message: String,
    },

    /// A local edit was blocked by a range protection the current user may not
    /// edit (`trame-cell-editing`, Post-MVP). No CRDT delta was
    /// produced; the chrome surfaces `description`.
    ProtectionDenied {
        version: ambre_core::Seq<ambre_bus::ViewportVersion>,
        row: Pos<Row>,
        col: Pos<Col>,
        description: String,
    },

    /// A [`ImportCsv`](super::TrameCommand::ImportCsv) finished writing
    /// the parsed rows into the workbook (`trame-import-export`).
    /// Rides alongside a [`DataReset`](Self::DataReset) so the UI re-fetches the
    /// populated region. `rows` / `cols` are the dimensions actually written;
    /// `sheet_id` is the (possibly freshly created) target sheet.
    ImportComplete {
        sheet_id: Id<Sheet>,
        rows: Len<Row>,
        cols: Len<Col>,
    },

    /// An export ([`ExportCsv`](super::TrameCommand::ExportCsv)) produced
    /// a file for download (`trame-import-export`). The Logic thread
    /// holds the document, so it serializes the bytes; the UI Thread receives them
    /// and triggers the browser download. `filename` already carries the right
    /// extension.
    ExportComplete {
        filename: ambre_core::ExportFilename,
        data: ambre_core::Payload<ambre_core::FileBytes>,
    },

    /// A remote delta edited a protected range its author was not allowed to
    /// (`trame-cell-editing`). The delta IS applied (CRDT convergence is
    /// mandatory) but the UI is warned. `from_user_id` is the offending author's
    /// stable
    /// [`Id<User>`](Id).
    ProtectionViolationWarning {
        row: Pos<Row>,
        col: Pos<Col>,
        from_user_id: Id<User>,
        description: String,
    },
}

// A cell value, a validation message and a protection note are the author's,
// and print their length alone. An error message may quote a sheet or a name
// the author typed, and prints its length alone too. A refusal the engine
// words from positions and counts prints whole.
impl std::fmt::Debug for TrameEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ambre_core::RedactedChars;
        macro_rules! fields {
            ($name:literal $(, $field:ident)*) => {
                f.debug_struct($name)$(.field(stringify!($field), $field))*.finish()
            };
        }
        macro_rules! at_cell {
            ($name:literal, $version:expr, $row:expr, $col:expr, $label:literal, $text:expr) => {
                f.debug_struct($name)
                    .field("version", $version)
                    .field("row", $row)
                    .field("col", $col)
                    .field($label, &RedactedChars::of($text))
                    .finish()
            };
        }
        match self {
            Self::ViewportData {
                version,
                row_start,
                col_start,
                cells,
                col_widths,
                row_heights,
            } => fields!(
                "ViewportData",
                version,
                row_start,
                col_start,
                cells,
                col_widths,
                row_heights
            ),
            Self::StyleApplied {
                style,
                bold,
                italic,
                merged,
                numeric,
            } => fields!("StyleApplied", style, bold, italic, merged, numeric),
            Self::DataReset => f.write_str("DataReset"),
            Self::CellsChanged { version, range } => fields!("CellsChanged", version, range),
            Self::SelectRange { rect } => fields!("SelectRange", rect),
            Self::MergeRejected { reason } => fields!("MergeRejected", reason),
            Self::MergeConflictDetected {
                master_row,
                master_col,
                covered_row,
                covered_col,
            } => fields!(
                "MergeConflictDetected",
                master_row,
                master_col,
                covered_row,
                covered_col
            ),
            Self::UndoState { undo, redo } => fields!("UndoState", undo, redo),
            Self::SheetList { sheets, active } => fields!("SheetList", sheets, active),
            Self::LocaleState { document, reader } => fields!("LocaleState", document, reader),
            Self::Error { message } => f
                .debug_struct("Error")
                .field("message", &RedactedChars::of(message))
                .finish(),
            Self::EditTargetMoved { session, row, col } => {
                fields!("EditTargetMoved", session, row, col)
            }
            Self::EditTargetGone { session } => fields!("EditTargetGone", session),
            Self::DocumentRefused { reason } => fields!("DocumentRefused", reason),
            Self::UndecodableRecords => f.write_str("UndecodableRecords"),
            Self::SortComplete => f.write_str("SortComplete"),
            Self::SortRange { r1, c1, r2, c2 } => fields!("SortRange", r1, c1, r2, c2),
            Self::ColumnContent { col, cells } => fields!("ColumnContent", col, cells),
            Self::FilterApplied {
                hidden_row_count,
                col,
            } => fields!("FilterApplied", hidden_row_count, col),
            Self::ColumnValues { col, values } => f
                .debug_struct("ColumnValues")
                .field("col", col)
                .field(
                    "values",
                    &values
                        .iter()
                        .map(|text| RedactedChars::of(text))
                        .collect::<Vec<_>>(),
                )
                .finish(),
            Self::SelectionStats { count, numbers } => fields!("SelectionStats", count, numbers),
            Self::FilterCleared => f.write_str("FilterCleared"),
            Self::FindResult { matches, active } => fields!("FindResult", matches, active),
            Self::ReplaceDone { replaced } => fields!("ReplaceDone", replaced),
            Self::PasteRejected { reason } => fields!("PasteRejected", reason),
            Self::Clipboard(payload) => f.debug_tuple("Clipboard").field(payload).finish(),
            Self::FilterViewSaved { view_id } => fields!("FilterViewSaved", view_id),
            Self::CondFormatsUpdated { version } => fields!("CondFormatsUpdated", version),
            Self::ValidationError {
                version,
                row,
                col,
                message,
            } => at_cell!("ValidationError", version, row, col, "message", message),
            Self::ValidationWarning {
                version,
                row,
                col,
                message,
            } => at_cell!("ValidationWarning", version, row, col, "message", message),
            Self::ProtectionDenied {
                version,
                row,
                col,
                description,
            } => at_cell!(
                "ProtectionDenied",
                version,
                row,
                col,
                "description",
                description
            ),
            Self::ImportComplete {
                sheet_id,
                rows,
                cols,
            } => fields!("ImportComplete", sheet_id, rows, cols),
            Self::ExportComplete { filename, data } => fields!("ExportComplete", filename, data),
            Self::ProtectionViolationWarning {
                row,
                col,
                from_user_id,
                description,
            } => f
                .debug_struct("ProtectionViolationWarning")
                .field("row", row)
                .field("col", col)
                .field("from_user_id", from_user_id)
                .field("description", &RedactedChars::of(description))
                .finish(),
        }
    }
}

/// The evaluation order of conditional-formatting rules: the smaller rank
/// is evaluated first, negative ranks first, and the first rule that matches
/// wins (`trame-cell-styles`). It orders rules and counts nothing.
pub enum CondFormatPriority {}

impl RankKind for CondFormatPriority {
    type Repr = i64;
    const NAME: &'static str = "CondFormatPriority";
}

pub use ambre_core::Cells;

/// A place in the sheet tab strip, 0-based from the left. It orders the
/// tabs and names no sheet.
#[derive(PartialEq, Eq)]
pub enum TabStrip {}

impl Axis for TabStrip {
    const NAME: &'static str = "SheetTab";
}

/// One tab of the sheet strip: the sheet's id and its display name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SheetTab {
    pub id: Id<Sheet>,
    pub name: SheetName,
}

/// A single cell ready for display.
///
/// Flat, data-oriented. The UI Thread stores these in a
/// `HashMap<CellPos, DisplayCell>`; missing entries
/// are blank (white background, no text), so only non-empty cells are sent.
/// `Debug` prints the length of `text` and `raw` alone, so a log carries
/// none of the cell's content.
#[derive(Clone, Default)]
pub struct DisplayCell {
    pub row: Pos<Row>,
    pub col: Pos<Col>,
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

impl std::fmt::Debug for DisplayCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayCell")
            .field("row", &self.row)
            .field("col", &self.col)
            .field("text", &ambre_core::RedactedChars::of(&self.text))
            .field("raw", &ambre_core::RedactedChars::of(&self.raw))
            .field("bold", &self.bold)
            .field("italic", &self.italic)
            .field("underline", &self.underline)
            .field("strikethrough", &self.strikethrough)
            .field("h_align", &self.h_align)
            .field("numeric", &self.numeric)
            .field("v_align", &self.v_align)
            .field("wrap", &self.wrap)
            .field("fg", &self.fg)
            .field("bg", &self.bg)
            .field("font", &self.font)
            .field("size", &self.size)
            .field("borders", &self.borders)
            .field("is_merged", &self.is_merged)
            .field("merge_span", &self.merge_span)
            .field("is_filtered", &self.is_filtered)
            .finish()
    }
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
    use crate::message::CellText;

    fn numbers(values: &[f64]) -> Vec<Bounded<CellFloat>> {
        values.iter().map(|&v| number(v)).collect()
    }

    fn number(value: f64) -> Bounded<CellFloat> {
        Bounded::new(value).expect("a finite number")
    }

    /// A display cell's `Debug` form holds the length of its text and of its
    /// raw text, and none of either.
    #[test]
    fn a_display_cell_prints_the_length_of_its_text_alone() {
        let cell = DisplayCell {
            text: "4200".to_owned(),
            raw: "=SALARY*12".to_owned(),
            ..DisplayCell::default()
        };
        let printed = format!("{cell:?}");
        assert!(
            printed.contains("text: 4 chars, raw: 10 chars"),
            "{printed}"
        );
        assert!(!printed.contains("4200") && !printed.contains("SALARY"));
    }

    /// Every command, event and rule that holds a text the author typed prints
    /// its length in `Debug`, and none of the text.
    #[test]
    fn an_authored_text_prints_its_length_alone() {
        use super::super::{
            CellFormat, ClipboardCell, ClipboardPayload, CondFormatKind, FilterKind, FilterOp,
            SourceForm, TrameCommand, ValidationKind,
        };
        let secret = "Q3 layoffs".to_owned();
        let at = (Pos::<Row>::ZERO, Pos::<Col>::ZERO);
        let payload = ClipboardPayload {
            cells: vec![ClipboardCell {
                raw: secret.clone(),
                ..ClipboardCell::default()
            }],
            ..ClipboardPayload::default()
        };
        let printed = [
            format!(
                "{:?}",
                TrameCommand::SetCellText {
                    row: at.0,
                    col: at.1,
                    text: secret.clone(),
                }
            ),
            format!(
                "{:?}",
                TrameCommand::SetCellsBatch {
                    cells: vec![(CellPos::new(at.0, at.1), secret.clone())],
                }
            ),
            format!(
                "{:?}",
                TrameCommand::PasteText {
                    text: secret.clone(),
                }
            ),
            format!(
                "{:?}",
                TrameCommand::ReplaceAll {
                    query: None,
                    replacement: secret.clone(),
                }
            ),
            format!(
                "{:?}",
                TrameCommand::PasteClipboardAtActive {
                    payload: payload.clone(),
                }
            ),
            format!("{:?}", TrameEvent::Clipboard(payload)),
            format!(
                "{:?}",
                TrameEvent::ColumnValues {
                    col: at.1,
                    values: vec![secret.clone()],
                }
            ),
            format!(
                "{:?}",
                TrameEvent::ProtectionViolationWarning {
                    row: at.0,
                    col: at.1,
                    from_user_id: Id::<User>::mint(),
                    description: secret.clone(),
                }
            ),
            format!("{:?}", CondFormatKind::Contains(secret.clone())),
            format!(
                "{:?}",
                CondFormatKind::Formula(CellText::<SourceForm>::new(secret.clone()))
            ),
            format!("{:?}", ValidationKind::Regex(secret.clone())),
            format!("{:?}", ValidationKind::List(vec![secret.clone()])),
            format!("{:?}", FilterKind::Values(vec![secret.clone()])),
            format!(
                "{:?}",
                FilterKind::Condition {
                    op: FilterOp::Eq,
                    value: secret.clone(),
                }
            ),
            format!("{:?}", CellFormat::Custom(secret.clone())),
            format!(
                "{:?}",
                TrameEvent::Error {
                    message: secret.clone(),
                }
            ),
        ];
        for line in printed {
            assert!(line.contains("10 chars"), "{line}");
            assert!(!line.contains("Q3 layoffs"), "{line}");
        }
        assert_eq!(format!("{:?}", CellFormat::Date), "Date");
    }

    /// Totals exist only over at least one number, and the average is the sum
    /// over the count.
    #[test]
    fn numeric_totals_need_a_number_and_average_over_their_count() {
        assert_eq!(NumericStats::of_all(&[], Some(number(0.0))), None);
        let totals = NumericStats::of_all(&numbers(&[10.0, -4.0, 6.0]), Some(number(12.0)))
            .expect("three numbers");
        assert_eq!(totals.count().get(), 3);
        assert_eq!(totals.sum().map(Bounded::get), Some(12.0));
        assert_eq!(totals.avg().map(Bounded::get), Some(4.0));
        assert_eq!((totals.min().get(), totals.max().get()), (-4.0, 10.0));
    }

    /// A cell number is finite: NaN and both infinities are refused, the
    /// refusal echoes the value, and both finite extremes are admitted.
    #[test]
    fn a_cell_number_refuses_a_value_that_is_not_finite() {
        let get = |v: f64| Bounded::<CellFloat>::try_new(v).map(Bounded::get);
        assert_eq!(get(-0.5), Ok(-0.5));
        assert_eq!(get(f64::MAX), Ok(f64::MAX));
        assert_eq!(get(f64::MIN), Ok(f64::MIN));
        assert_eq!(get(f64::MIN_POSITIVE), Ok(f64::MIN_POSITIVE));
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(Bounded::<CellFloat>::new(value), None);
            let refused = Bounded::<CellFloat>::try_new(value).expect_err("not finite");
            assert_eq!(refused.refused().to_bits(), value.to_bits());
        }
        let refused = Bounded::<CellFloat>::try_new(f64::INFINITY).expect_err("not finite");
        assert_eq!(refused.to_string(), "inf is not a finite number");
        let refused = Bounded::<CellFloat>::try_new(f64::NEG_INFINITY).expect_err("not finite");
        assert_eq!(refused.to_string(), "-inf is not a finite number");
        assert_eq!(format!("{:?}", number(1.5)), "CellNumber(1.5)");
        assert_eq!(number(1.5).to_string(), "1.5");
    }

    /// A sum past the largest finite number is an overflow, never an
    /// infinity, and so is its average. The extremes stay exact
    /// (`TRAME-UI-179`).
    #[test]
    fn an_overflowed_sum_leaves_no_average_and_exact_extremes() {
        let totals = NumericStats::of_all(&numbers(&[f64::MAX, f64::MAX, -f64::MAX]), None)
            .expect("numbers");
        assert_eq!(totals.sum(), None);
        assert_eq!(totals.avg(), None);
        assert_eq!(totals.count().get(), 3);
        assert_eq!(
            (totals.min().get(), totals.max().get()),
            (-f64::MAX, f64::MAX)
        );
    }
}
