//! Trame (spreadsheet) command surface (UI Thread -> Logic thread).
//!
//! The full `TrameCommand` vocabulary plus its parameter value types, carried by
//! move inside [`AppCommand::Trame`](crate::logic::AppCommand). Value types (styles,
//! clipboard, feature payloads) live in the sibling modules and arrive via glob.

use super::*;
use crate::UserId;

// -----------------------------------------------------------------------------
// Trame (spreadsheet) -- commands (UI -> Logic)
// -----------------------------------------------------------------------------

/// Shape of a Trame selection band (`trame-selection`), the bus-side mirror of
/// `ambre_trame`'s `SelKind`. `Cells` is the ordinary rectangle; `Cols`/`Rows`
/// are whole-axis bands the Logic thread resolves to the used range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionKind {
    #[default]
    Cells,
    Cols,
    Rows,
}

/// Commands sent from the UI Thread to the Trame logic for an open document.
#[derive(Debug, Clone)]
pub enum TrameCommand {
    /// Request cell data for a viewport region.
    ///
    /// The UI Thread computes the visible range (plus padding) from its scroll
    /// position and grid dimensions; the Logic thread replies with a
    /// [`TrameEvent::ViewportData`](super::TrameEvent::ViewportData) covering
    /// `[row_start..=row_end] x [col_start..=col_end]`.
    ///
    /// `version` is a monotonic counter: the UI Thread drops any response
    /// whose version is older than the last accepted one.
    ScrollViewport {
        version: crate::ViewportVersion,
        row_start: Row,
        row_end: Row,
        col_start: Col,
        col_end: Col,
    },

    /// Move the active cell to `(row, col)` and report the whole selection
    /// (`trame-selection`). The report holds the live rectangle's corners,
    /// every committed disjoint rectangle in `extra`, and the band shape in
    /// `kind`.
    ///
    /// Formatting ([`SetCellStyle`](TrameCommand::SetCellStyle)) targets whatever
    /// selection the Logic thread tracks, so the UI Thread must keep it informed
    /// when the cursor moves. The Logic thread replies with the active cell's
    /// resolved style ([`TrameEvent::StyleApplied`](super::TrameEvent::StyleApplied))
    /// so the formatting toolbar tracks the selection.
    ///
    /// The Logic thread normalizes the two corners into the live rectangle, and
    /// resolves a `Cols`/`Rows` band to the materialized used range across its
    /// axis. The parameterless structural intents (`MergeSelection`,
    /// `DeleteSelectedRows`, ...) then act on every reported rectangle. The
    /// selection is local view state -- never persisted, never synced
    /// (`trame-selection`).
    SetActiveCell {
        row: Row,
        col: Col,
        anchor_row: Row,
        anchor_col: Col,
        /// Committed disjoint rectangles (Ctrl+click), each normalized. The live
        /// rectangle (`anchor..=active`) is not here; the full selection is these
        /// plus that rectangle.
        extra: Vec<GridRange>,
        /// Band shape: `Cells`, `Cols` or `Rows`. A `Cols`/`Rows` band carries no
        /// extent across its axis; the Logic thread resolves it to the used range.
        kind: SelectionKind,
    },

    /// Set the plaintext content of cell `(row, col)`.
    ///
    /// Emitted when the UI commits an edit session on the transparent textarea
    /// pipe. The Logic thread stores the text in the document CRDT (the source
    /// of truth); an empty `text` clears the cell. The UI updates its own cache
    /// optimistically, so no echo event is required.
    SetCellText { row: Row, col: Col, text: String },

    /// Open edit session `session` on cell `(row, col)`. The Logic thread
    /// resolves the position once, to the sheet and the two lines under it, and
    /// writes nothing. A later commit reaches that cell wherever a peer moved it.
    /// A new session replaces the previous one.
    BeginCellEdit { session: u64, row: Row, col: Col },

    /// Commit session `session` with `text`, written to the cell it opened on.
    /// A session the Logic thread no longer holds writes nothing: its cell is
    /// gone, and [`EditTargetGone`](super::TrameEvent::EditTargetGone) said so.
    /// Opens the undo group [`CommitEdit`](TrameCommand::CommitEdit) seals.
    CommitCellEdit { session: u64, text: String },

    /// Close session `session` with no write: an Escape, or a fill that writes
    /// the selection instead. A no-op for a session the Logic thread no longer
    /// holds.
    EndCellEdit { session: u64 },

    /// Set the plaintext content of many cells in one shot, as `(row, col, text)`
    /// triples. Emitted for a multi-cell fill (Ctrl+Enter) or a clear over a
    /// range (Delete on a selection). The Logic thread applies every write inside
    /// a single `UndoManager` group, so the whole batch is one undo item. A
    /// stream of `SetCellText` differs: each write is its own item. An empty
    /// `text` clears that cell.
    SetCellsBatch { cells: Vec<(CellPos, String)> },

    /// Close the undo group opened by the cell edit, on cell exit (`trame-cell-editing`,
    /// `trame-undo`). An interactive edit can mutate the `LoroDoc` more than once: the cell
    /// write, plus derived side-effects such as a filtered-column recompute. Loro records one
    /// `UndoManager` item per commit. The Logic thread groups every mutation of the edit into a
    /// single undo item, and seals that group when this command arrives. The whole cell edit is
    /// then exactly one undo step (1 cell edit = 1 undo item).
    ///
    /// Emitted by the UI right after the committing [`SetCellText`](TrameCommand::SetCellText)
    /// when leaving the inline editor; produces no CRDT delta of its own. A no-op if no edit
    /// group is open.
    CommitEdit,

    /// Apply a sparse formatting [`StylePatch`] to every cell of the tracked
    /// selection (`trame-cell-styles`). The Logic thread resolves the geometry from
    /// the selection it keeps via [`SetActiveCell`](TrameCommand::SetActiveCell),
    /// the same parameterless-geometry contract as the structural intents. It
    /// merges the patch into each cell's stored style. The result is persisted in
    /// the per-sheet style containers, a synchronised CRDT mutation, hence
    /// undoable. It reformats display text for any number-format change. It replies
    /// with a [`TrameEvent::StyleApplied`](super::TrameEvent::StyleApplied)
    /// carrying the active cell's resulting style, plus a [`DataReset`] so the UI
    /// re-fetches the restyled viewport.
    ///
    /// [`DataReset`]: super::TrameEvent::DataReset
    SetCellStyle { patch: StylePatch },

    /// Insert a blank row at grid position `at`, shifting existing rows at
    /// `>= at` down by one. One `LoroMovableList` op; cells keyed by stable id
    /// pairs follow their row automatically (`trame-rows-and-columns`).
    /// A row inserted within a range formula is naturally included; references
    /// are never rewritten (`trame-formula-engine`). A no-op past the
    /// materialized used range (virtual rows are already blank).
    InsertRow { at: Row },

    /// Delete the row at grid position `at`, shifting rows at `> at` up by one.
    /// Its cells/formatting are dropped and any formula referencing the now
    /// removed row id resolves to `#REF!` (`trame-formula-engine`). A no-op in
    /// virtual space.
    DeleteRow { at: Row },

    /// Insert a blank column at grid position `at`, shifting existing columns at
    /// `>= at` right by one. Column analogue of [`InsertRow`](TrameCommand::InsertRow).
    InsertColumn { at: Col },

    /// Delete the column at grid position `at`, shifting columns at `> at` left
    /// by one. Column analogue of [`DeleteRow`](TrameCommand::DeleteRow).
    DeleteColumn { at: Col },

    /// Move the row at grid position `from` to grid position `to` (Excel-style
    /// drag reorder), shifting the rows in between to fill the gap. Realized as a
    /// single `LoroMovableList::mov` on `rows_{sheet}`. Cells keyed by the
    /// move-invariant `(row_uid, col_uid)` pair travel with their row, and no
    /// reference is rewritten. Every `A1` resolves to its row's new position
    /// (`trame-formula-engine`, the same guarantee as sort). Concurrent moves
    /// converge by LWW on the list order. A no-op if `from == to` or either index
    /// is outside the materialized used range.
    MoveRow { from: Row, to: Row },

    /// Move the column at grid position `from` to grid position `to`. Column
    /// analogue of [`MoveRow`](TrameCommand::MoveRow): a single `mov` on
    /// `cols_{sheet}`.
    MoveColumn { from: Col, to: Col },

    /// Merge the rectangular range `[row..=row_end] x [col..=col_end]` into a
    /// single cell, the top-left corner becoming the master
    /// (`trame-cell-merges`). The corners may be given in any order.
    /// Rejected locally with [`TrameEvent::MergeRejected`] if any covered cell
    /// (other than the master) is non-empty -- Excel/Sheets behavior. Stored in
    /// the per-sheet `merges_{sheet}` LWW map keyed by stable id pairs, so the
    /// merge follows its cells across structural ops.
    MergeCells {
        row: Row,
        col: Col,
        row_end: Row,
        col_end: Col,
    },

    /// Split the merge whose rectangle contains grid cell `(row, col)`, restoring
    /// the covered cells. Any value masked by the merge (a concurrent-edit
    /// conflict, `trame-cell-merges`) reappears. A no-op if `(row, col)` is not
    /// inside a merge.
    UnmergeCells { row: Row, col: Col },

    /// Set the width (physical px) of column `col` on `sheet_id` (`trame-resize`,
    /// `trame-engine`). The Logic thread writes the value into the per-sheet
    /// `col_widths_{sheet}` LWW map keyed by the column's stable `Uid`, clamped to
    /// a minimum. It then replies with a [`DataReset`], so the UI re-fetches the
    /// viewport (whose [`ViewportData`] now carries the new per-column widths). A
    /// synchronised CRDT mutation, hence undoable; two peers resizing the same
    /// column converge by LWW, like any cell write.
    ///
    /// [`DataReset`]: super::TrameEvent::DataReset [`ViewportData`]:
    /// super::TrameEvent::ViewportData
    ResizeCol {
        version: crate::ViewportVersion,
        sheet_id: SheetId,
        col: Col,
        width_px: f32,
    },
    /// Set the height (physical px) of row `row` on `sheet_id`. Row analogue of
    /// [`ResizeCol`](TrameCommand::ResizeCol): writes `row_heights_{sheet}`.
    ResizeRow {
        version: crate::ViewportVersion,
        sheet_id: SheetId,
        row: Row,
        height_px: f32,
    },

    // -- Parameterless structural intents (grid context menu,
    // `trame-context-menu`).
    // These carry no geometry: the Logic thread resolves them against the active
    // cell / selection rectangle it tracks (kept current by `SetActiveCell`).
    // They map one-to-one to a registry `CommandId`, so the DOM context menu, the
    // toolbar and the keyboard can all trigger them through the same dispatch.
    /// Insert a blank row just above the active cell's row.
    InsertRowAbove,
    /// Insert a blank row just below the active cell's row.
    InsertRowBelow,
    /// Insert a blank column just left of the active cell's column.
    InsertColLeft,
    /// Insert a blank column just right of the active cell's column.
    InsertColRight,
    /// Delete every row spanned by the current selection (descending order, so
    /// earlier positions stay valid). The rows share one commit: one undo step
    /// and one delta.
    DeleteSelectedRows,
    /// Delete every column spanned by the current selection (descending order).
    DeleteSelectedCols,
    /// Merge the current selection rectangle (equivalent to [`MergeCells`] over
    /// `selection.bounds()`). Rejected with [`TrameEvent::MergeRejected`] on a
    /// single-cell selection or a non-empty covered cell.
    MergeSelection,
    /// Unmerge the merge containing the active cell (equivalent to
    /// [`UnmergeCells`] at the active cell). A no-op outside a merge.
    UnmergeActive,
    /// Sort the current selection rectangle ascending on its first column
    /// (equivalent to [`SortRange`] over `selection.bounds()` with `sort_col = 0`,
    /// `has_header = false`). The Logic thread resolves the geometry and the
    /// active sheet itself. Replies with [`TrameEvent::SortComplete`].
    ///
    /// [`SortRange`]: TrameCommand::SortRange
    /// [`TrameEvent::SortComplete`]: super::TrameEvent::SortComplete
    SortSelectionAsc,
    /// Sort the current selection rectangle descending on its first column
    /// (see [`SortSelectionAsc`](TrameCommand::SortSelectionAsc)).
    SortSelectionDesc,
    /// Sort the whole sheet (its materialized used range) ascending on the active
    /// cell's column -- the "sort by column" affordance for a whole-column
    /// selection (`trame-sort`). Equivalent to [`SortRange`] over the full
    /// used range with `sort_col` resolved to the active column. The Logic thread
    /// resolves the geometry and the active sheet itself. Replies with
    /// [`TrameEvent::SortComplete`].
    ///
    /// [`SortRange`]: TrameCommand::SortRange
    /// [`TrameEvent::SortComplete`]: super::TrameEvent::SortComplete
    SortSheetAsc,
    /// Sort the whole sheet descending on the active cell's column
    /// (see [`SortSheetAsc`](TrameCommand::SortSheetAsc)).
    SortSheetDesc,
    /// Multi-column sort of the current selection rectangle (`trame-sort`, the "Sort
    /// range..." dialog). Parameterless sibling of
    /// [`SortRangeMulti`](TrameCommand::SortRangeMulti). The Logic thread resolves the
    /// range and the active sheet from the tracked selection, then sorts the rows
    /// lexicographically by `sort_keys`. Priority decreases along that list, and each `col`
    /// is an index within the selection (`0` = the selection's left column). Replies with
    /// [`TrameEvent::SortComplete`](super::TrameEvent::SortComplete).
    SortSelectionMulti {
        sort_keys: Vec<SortKey>,
        has_header: bool,
    },

    /// Request the distinct visible values of the active cell's column, for the
    /// shared filter popover's "by values" checkbox list (`trame-shared-filter`).
    /// The Logic thread resolves the column and active sheet itself, evaluates every
    /// non-hidden materialized row through the formula cache, and replies with
    /// [`TrameEvent::ColumnValues`](super::TrameEvent::ColumnValues)
    /// (deduplicated, sorted). A read-only query: no CRDT delta.
    RequestActiveColumnValues,
    /// Request the aggregates (Sum / Average / Count / Min / Max) of the tracked selection,
    /// for the status bar (`trame-status-bar`). The selection can exceed the viewport, so
    /// only the Logic thread (which holds every cell) can compute them. Parameterless: the
    /// Logic thread resolves the active sheet and the tracked selection rectangle, the same
    /// convention as the sort / filter / [`RequestActiveColumnValues`] siblings. The spec
    /// sketch passed `sheet_id`/`range`, but this codebase resolves geometry Logic-side.
    ///
    /// Replies with [`TrameEvent::SelectionStats`](super::TrameEvent::SelectionStats). A
    /// read-only query: no CRDT delta. The UI sends it only on a multi-cell selection; a
    /// lone cell shows no aggregates.
    ///
    /// [`RequestActiveColumnValues`]: TrameCommand::RequestActiveColumnValues
    RequestSelectionStats,
    /// Resolve the range a parameterless sort would order (`trame-sort`), so the
    /// sort dialog offers exactly its columns (`TRAME-UI-101`). A band selection
    /// carries no extent across its other axis, and only the Logic thread knows
    /// the used range. Replies with
    /// [`TrameEvent::SortRange`](super::TrameEvent::SortRange). A read-only query:
    /// no CRDT delta.
    RequestSortRange,
    /// Every non-empty cell of grid column `col`, all rows, for a column
    /// auto-fit (`TRAME-UI-128`). The canvas caches the loaded window only. A
    /// column is measured whole or not resized. Replies with
    /// [`TrameEvent::ColumnContent`](super::TrameEvent::ColumnContent). A
    /// read-only query: no CRDT delta.
    RequestColumnContent { col: Col },
    /// Build the clipboard payload for the tracked selection (`trame-clipboard`). The
    /// parameterless-geometry sibling of copy. The Logic thread is the only holder of
    /// document data, so it serializes the selected cells (never culled to a viewport) into
    /// a [`ClipboardPayload`]. It replies with
    /// [`TrameEvent::Clipboard`](super::TrameEvent::Clipboard). Non-mutating.
    CopyRange,
    /// Apply a shared filter to the active cell's column (`trame-shared-filter`).
    /// The parameterless-geometry sibling of [`ApplyFilter`]: the Logic thread
    /// resolves `sheet_id`/`col` from the tracked active cell, so the UI only
    /// supplies the user's chosen [`FilterKind`]. Replies with
    /// [`TrameEvent::FilterApplied`](super::TrameEvent::FilterApplied).
    ///
    /// [`ApplyFilter`]: TrameCommand::ApplyFilter
    ApplyFilterToActiveColumn { kind: FilterKind },
    /// Clear the shared filter on the active sheet (`trame-shared-filter`). The
    /// parameterless sibling of [`ClearFilter`]: the Logic thread resolves the
    /// active sheet itself. Replies with
    /// [`TrameEvent::FilterCleared`](super::TrameEvent::FilterCleared).
    ///
    /// [`ClearFilter`]: TrameCommand::ClearFilter
    ClearActiveFilter,

    /// Paste a clipboard [`ClipboardPayload`] with its top-left landing on `(target_row,
    /// target_col)` (`trame-clipboard`). The Logic thread is the only holder of document
    /// data, so the paste happens here, relative-formula reindexing and style application
    /// included. Every pasted cell lands at `(target_row + dr, target_col + dc)`, a
    /// formula's relative references shift by `(target_row - src_row, target_col -
    /// src_col)`, and the carried [`CellStyle`] (plus bold/italic) is written verbatim. The
    /// whole paste is a single undo group. Replies with a [`DataReset`] so the UI
    /// re-fetches the pasted region; a paste blocked by a protected range (Post-MVP)
    /// replies [`TrameEvent::PasteRejected`](super::TrameEvent::PasteRejected).
    ///
    /// [`DataReset`]: super::TrameEvent::DataReset
    PasteRange {
        target_row: Row,
        target_col: Col,
        payload: ClipboardPayload,
    },

    /// Paste plain tab-separated `text` at the tracked active cell (`trame-clipboard`). The
    /// parameterless-geometry sibling of [`PasteRange`](TrameCommand::PasteRange). The
    /// Logic thread resolves the target from the active cell it keeps via
    /// [`SetActiveCell`](TrameCommand::SetActiveCell). A toolbar/menu Paste button reads
    /// the OS clipboard asynchronously and has no view state, so it pastes without knowing
    /// the cursor. Each field becomes a literal cell (no relative-formula reindexing, the
    /// source is external). Replies with a [`DataReset`](super::TrameEvent::DataReset).
    PasteText { text: String },

    /// Paste a typed [`ClipboardPayload`] at the tracked active cell (`trame-clipboard`).
    /// The parameterless-geometry sibling of [`PasteRange`](TrameCommand::PasteRange). The
    /// Logic thread resolves the target from the active cell it keeps. The toolbar/menu
    /// Paste button reads the OS clipboard asynchronously via the Clipboard API and has no
    /// view state. It gets the same styled, formula-reindexed paste as a native Ctrl+V. The
    /// reindex offset is `active - payload.src`.
    PasteClipboardAtActive { payload: ClipboardPayload },

    /// Autofill the `src` rectangle into `dest` (`trame-autofill`). `dest` contains `src`
    /// and extends it along a single axis (the fill handle locks to one direction); the
    /// cells of `dest \ src` are the fill region.
    ///
    /// The Logic thread reads the source cells from its document model. Per fill line, it
    /// either extrapolates a numeric arithmetic [`Series`](FillMode::Series) or
    /// tiles/[`Copy`](FillMode::Copy)s the source values. A copied formula's relative
    /// references are reindexed by its tile offset (`formula::shift`, as in
    /// [`PasteRange`](TrameCommand::PasteRange)), and each source cell's style is carried.
    ///
    /// The whole fill is a single undo group. With [`FillMode::Auto`] the thread picks
    /// Series when a line's source values form an arithmetic numeric progression, Copy
    /// otherwise. Replies with a [`DataReset`](super::TrameEvent::DataReset). A no-op when
    /// `dest == src`.
    FillRange {
        sheet_id: SheetId,
        src: GridRange,
        dest: GridRange,
        mode: FillMode,
    },

    /// Find the next cell whose displayed value matches `query`, starting from the tracked
    /// active cell and wrapping around the active sheet (`trame-find-replace`). Read-only:
    /// the Logic thread scans the materialized extent, comparing each cell's *displayed*
    /// value (formula results included). The comparison is case-folded unless `match_case`,
    /// by exact equality when `whole_cell`, else by substring. Produces a
    /// [`FindResult`](crate::TrameEvent::FindResult) with every match on the sheet plus the
    /// index of the one to focus; no CRDT delta is emitted. The UI moves the active cell
    /// onto the focused match, so a repeated `FindNext` advances.
    FindNext {
        query: String,
        match_case: bool,
        whole_cell: bool,
    },

    /// Replace every match of `query` with `replacement` across the active sheet
    /// (or all sheets when `all_sheets`) in a single undo group
    /// (`trame-find-replace`). Matching tests the *displayed* value
    /// (case-folded unless `match_case`, exact when `whole_cell` else substring),
    /// but the rewrite edits the *raw* cell text. Cells whose raw text is a
    /// formula (`=...`) are skipped unless `in_formulas`. Replies with
    /// [`ReplaceDone`](crate::TrameEvent::ReplaceDone) and a
    /// [`DataReset`](crate::TrameEvent::DataReset) so the UI re-fetches.
    ReplaceAll {
        query: String,
        replacement: String,
        match_case: bool,
        whole_cell: bool,
        in_formulas: bool,
        all_sheets: bool,
    },

    /// Replace the single match at the tracked active cell, then advance to the next match
    /// (`trame-find-replace`). If the active cell matches `query` it is rewritten in one
    /// undo group, under the same raw-text rules as
    /// [`ReplaceAll`](TrameCommand::ReplaceAll), `in_formulas` gating included. Whether or
    /// not it matched, the reply carries a fresh
    /// [`FindResult`](crate::TrameEvent::FindResult), so the UI moves to and highlights the
    /// following match. Replies with [`ReplaceDone`](crate::TrameEvent::ReplaceDone) (`0`
    /// or `1`).
    ReplaceOne {
        query: String,
        replacement: String,
        match_case: bool,
        whole_cell: bool,
        in_formulas: bool,
    },

    /// Undo the last local edit (`trame-undo`). Backed by Loro's `UndoManager`. It replays
    /// the inverse of this session's most recent commit as fresh CRDT ops (1 `TrameCommand`
    /// == 1 commit == 1 undo item). The inverse ops broadcast to peers like any other edit,
    /// so undo converges. A no-op when the undo stack is empty.
    Undo,

    /// Redo the edit most recently undone by [`Undo`](TrameCommand::Undo). A no-op
    /// when the redo stack is empty (it is cleared by any new local edit).
    Redo,

    /// Define (or redefine) a workbook named range.
    ///
    /// `name` is the case-sensitive identifier used in formulas (`=SUM(TOTAL)`);
    /// `reference` is its target written in A1 syntax, optionally sheet-qualified
    /// (`A1`, `A1:B10`, `Sheet2!A1:B10`). Stored in the shared `named_ranges`
    /// CRDT map (LWW) and resolved positionally against the current grid each
    /// recomputation (`trame-named-ranges`). A reference that
    /// fails to parse leaves any formula using the name as `#PARSE!`.
    DefineName { name: String, reference: String },

    /// Remove a named range. Formulas referencing the now-undefined name
    /// re-evaluate to a parse error on the next recomputation. A no-op if the
    /// name does not exist.
    DeleteName { name: String },

    /// Switch the sheet the UI is looking at to `sheet_id`. The Logic thread rebuilds its
    /// active-sheet derived caches (index, merges, formatting, formula engine). It then
    /// replies with a [`DataReset`], so the UI re-fetches the new sheet's viewport
    /// (`trame-sheets`). A no-op if the id is unknown.
    ///
    /// [`DataReset`]: super::TrameEvent::DataReset
    ActivateSheet { sheet_id: SheetId },

    /// Add a new sheet named `name`. Mints a peer-scoped stable `sheet_id`
    /// (`{peer:016x}_{counter:08x}`), so concurrent creates on two peers never collide. The
    /// new sheet is registered in `sheets` and appended to `sheet_order`, and the reply is
    /// a refreshed [`SheetList`](super::TrameEvent::SheetList) (`trame-sheets`). Cells are
    /// created lazily on first write.
    CreateSheet { name: String },

    /// Rename `sheet_id`'s display label (LWW on the name field). The stable id
    /// is untouched, so same-name references keep resolving; cross-sheet formula
    /// text bound to the old display name is the known MVP gap (`trame-sheets`).
    RenameSheet { sheet_id: SheetId, name: String },

    /// Move `sheet_id` to position `to_index` in the tab strip (one
    /// `LoroMovableList` move op). Pure presentation: never touches a reference
    /// (`trame-sheets`). A no-op if the id is unknown or already at that position.
    MoveSheet { sheet_id: SheetId, to_index: u32 },

    /// Sort the rows of the rectangular range `[row_start..=row_end] x
    /// [col_start..=col_end]` by the values in the column `col_start + sort_col`
    /// (`trame-sort`, single-column sort). Whole rows are reordered, so cells outside
    /// `[col_start..=col_end]` move with their row -- matching Google Sheets "sort range".
    /// `has_header` keeps the first range row pinned in place.
    ///
    /// Realized as `LoroMovableList` move ops on `rows_{sheet_id}`. Cells keyed by stable
    /// id pairs follow their row, so no cell key is rewritten and references are preserved
    /// (`trame-formula-engine`). LWW on the row list resolves concurrent sorts
    /// deterministically. A no-op when the range is empty/virtual. Replies with
    /// [`TrameEvent::SortComplete`].
    ///
    /// [`TrameEvent::SortComplete`]: super::TrameEvent::SortComplete
    SortRange {
        sheet_id: SheetId,
        row_start: Row,
        row_end: Row,
        col_start: Col,
        col_end: Col,
        /// Column to sort on, as an index within the range (`0` = `col_start`).
        sort_col: ColCount,
        ascending: bool,
        /// When `true`, the first range row is treated as a header and not sorted.
        has_header: bool,
    },

    /// Delete `sheet_id` and its per-sheet containers (`trame-sheets`). Rejected
    /// locally with [`TrameEvent::Error`](super::TrameEvent::Error) if it is the last
    /// remaining sheet (a workbook always keeps >= 1). If the active sheet is
    /// deleted, the active sheet falls back to a neighbour. Cross-sheet formulas
    /// targeting the gone id resolve to `#REF!`.
    DeleteSheet { sheet_id: SheetId },

    /// Apply a shared filter to column `col` of `sheet_id` (`trame-shared-filter`). The
    /// filter is stored in the document CRDT (`filters_{sheet}` + `filter_hidden_{sheet}`),
    /// so every collaborator sees the same hidden rows.
    ///
    /// The Logic thread evaluates the column through the formula cache. It writes the
    /// failing rows' stable ids into `filter_hidden_{sheet}`, keyed by `row_uid`, so the
    /// hidden set follows the data across a sort or insert. The resulting delta is
    /// broadcast.
    ///
    /// Rows hidden by a filter are excluded from the viewport (flagged
    /// [`DisplayCell::is_filtered`](super::DisplayCell::is_filtered)) but stay in
    /// calculations (`=SUM` still totals them). A filter is a plain CRDT mutation, hence
    /// undoable. Replies with
    /// [`TrameEvent::FilterApplied`](super::TrameEvent::FilterApplied). A no-op for a
    /// non-active sheet or a virtual (never-materialized) column.
    ApplyFilter {
        sheet_id: SheetId,
        /// Grid column the filter is applied to (0-based).
        col: Col,
        kind: FilterKind,
    },

    /// Clear the shared filter on `sheet_id`: removes every `filter_hidden_{sheet}`
    /// entry and the stored `filters_{sheet}` definition, then broadcasts the
    /// delta (`trame-shared-filter`). Replies with
    /// [`TrameEvent::FilterCleared`](super::TrameEvent::FilterCleared). A no-op
    /// when no filter is active.
    ClearFilter { sheet_id: SheetId },

    /// Import CSV bytes into the workbook (`trame-import-export`). All conversion happens
    /// in the Logic thread (Zero-Knowledge): the bytes never leave the client. When
    /// `target_sheet_id` is `None` the Logic thread creates a fresh sheet. Otherwise it
    /// writes into the named sheet starting at `(start_row, start_col)`, extending the used
    /// range as needed.
    ///
    /// `delimiter` is auto-detected (comma vs semicolon) when `None`. Each field becomes a
    /// literal cell (no formula reindexing -- the source is external). Replies with
    /// [`TrameEvent::ImportComplete`](super::TrameEvent::ImportComplete) plus a
    /// [`DataReset`](super::TrameEvent::DataReset); the whole import is a single undo
    /// group.
    ImportCsv {
        data: Vec<u8>,
        /// Field separator; `None` auto-detects comma vs semicolon.
        delimiter: Option<u8>,
        /// When `true` the first record is treated as a header row and skipped.
        has_header: bool,
        /// Target sheet; `None` creates a new "CSV Import" sheet.
        target_sheet_id: Option<SheetId>,
        start_row: Row,
        start_col: Col,
    },

    /// Export the **active** sheet to CSV (`trame-import-export`). Parameterless sibling of
    /// [`ExportCsv`](TrameCommand::ExportCsv). The Logic thread resolves the active sheet
    /// itself, the same convention as the sort / filter / selection-stats intents. A
    /// toolbar/menu "Export CSV" button therefore needs no view state. Comma delimiter,
    /// whole used range. Replies with
    /// [`TrameEvent::ExportComplete`](super::TrameEvent::ExportComplete).
    ExportActiveSheetCsv,

    /// Export a sheet (or a sub-range of it) to CSV bytes
    /// (`trame-import-export`). Read-only: no CRDT
    /// delta. Cells are exported by their *displayed* value (formula results
    /// included). `delimiter` defaults to comma; `range` of `None` exports the
    /// whole materialized used range. Replies with
    /// [`TrameEvent::ExportComplete`](super::TrameEvent::ExportComplete)
    /// carrying the file bytes for the UI Thread to download.
    ExportCsv {
        sheet_id: SheetId,
        delimiter: Option<u8>,
        range: Option<GridRange>,
    },

    /// Write the whole workbook to a native `.ambre` file and hand the bytes back
    /// (`ambre-spec-docfile.md`). Read-only, and served by the Logic thread rather than by
    /// the workbook. The container's header carries the document's identity, which the
    /// workbook does not know: only the registry that keyed it does. Answered by
    /// [`ExportComplete`](super::TrameEvent::ExportComplete).
    ///
    /// Unlike the CSV export, this one drops nothing: the payload **is** the document
    /// (`DOCFILE-001`). It is also plaintext, which the surface that offers it must say
    /// (`DOCFILE-009`).
    ExportAmbreFile,
    /// Write the whole workbook to a native `.ambre` file **sealed under
    /// `passphrase`**, and hand the sealed bytes back
    /// (`ambre-spec-docfile.md section 5`). Read-only, and answered by the same
    /// [`ExportComplete`](super::TrameEvent::ExportComplete) as the plaintext
    /// export -- what differs is the bytes, which the surface never has to tell
    /// apart.
    ///
    /// Served neither by the workbook nor by the Logic thread alone. The Logic
    /// thread writes the plaintext container, the Sync worker seals it
    /// (`ARCH-004`), and the answer comes back through the service plane. A
    /// failure answers [`Error`](super::TrameEvent::Error) and leaves no file.
    /// That is the only safe direction: a partial seal is not a file the user
    /// can open.
    ExportAmbreFileSealed {
        /// The passphrase the user typed for this file, and not the account's:
        /// nothing on this path can reach a session (`DOCFILE-014`).
        passphrase: crate::rest::SecretBytes,
    },

    // -------------------------------------------------------------------------
    // Deferred feature surface (Phase 2+)
    //
    // The variants below complete the `TrameCommand` surface mandated by `trame-engine`
    // ("all feature variants are in the enums"). They are part of the typed data plane
    // now, so the UI Thread and the Logic dispatcher compile against a stable command
    // vocabulary. Their Logic-thread handlers are no-op stubs until the corresponding
    // phase lands. Personal filter views and multi-column sort land in Phase 2,
    // conditional formatting and validation in Phase 2/3, range protection Post-MVP.
    // -------------------------------------------------------------------------
    //
    /// Persist a personal (non-shared) filter view in local storage only
    /// (`trame-shared-filter`). Never touches the `LoroDoc`, so it
    /// emits no CRDT delta. Replies with
    /// [`TrameEvent::FilterViewSaved`](super::TrameEvent::FilterViewSaved).
    SaveFilterView {
        name: String,
        sheet_id: SheetId,
        /// Grid column the personal view filters on (0-based).
        filter_col: Col,
        kind: FilterKind,
    },

    /// Activate a previously saved personal filter view (`trame-shared-filter`). Loads it
    /// from local storage, recomputes its hidden rows against the current doc state, and
    /// re-sends the viewport without them. A purely local view change, no CRDT delta. A
    /// no-op if `view_id` is unknown.
    ActivateFilterView { view_id: FilterViewId },

    /// Sort the rows of a range by several keys in decreasing priority
    /// (`trame-sort`, multi-column sort). Like
    /// [`SortRange`](TrameCommand::SortRange) but with a lexicographic
    /// comparison over `sort_keys`. Realized as `LoroMovableList` move ops on
    /// `rows_{sheet}`; references follow the data (`trame-formula-engine`).
    SortRangeMulti {
        sheet_id: SheetId,
        row_start: Row,
        row_end: Row,
        col_start: Col,
        col_end: Col,
        /// Decreasing priority: `[primary, secondary, ...]`. Each `col` is an
        /// index within the range (`0` = `col_start`).
        sort_keys: Vec<SortKey>,
        has_header: bool,
    },

    /// Add a conditional-formatting rule to `sheet_id` over `range`
    /// (`trame-cell-styles`). Stored in `cond_formats_{sheet}`
    /// (LoroList), evaluated per-cell after each recompute into a
    /// `DisplayCell.style` override (never mutates `styles_{sheet}`). `priority`
    /// orders evaluation (smaller = higher); the first matching rule wins.
    /// Replies with
    /// [`TrameEvent::CondFormatsUpdated`](super::TrameEvent::CondFormatsUpdated).
    AddCondFormat {
        sheet_id: SheetId,
        range: GridRange,
        rule: CondFormatKind,
        style: CondStyle,
        priority: i64,
    },

    /// Replace the rule identified by `id` on `sheet_id` (`trame-cell-styles`).
    /// A no-op if no
    /// rule with that id exists.
    UpdateCondFormat {
        sheet_id: SheetId,
        id: CondFormatId,
        range: GridRange,
        rule: CondFormatKind,
        style: CondStyle,
        priority: i64,
    },

    /// Remove the conditional-formatting rule `id` from `sheet_id`
    /// (`trame-cell-styles`). A no-op if it does not exist.
    RemoveCondFormat { sheet_id: SheetId, id: CondFormatId },

    /// Attach a data-validation rule to `range` of `sheet_id` (`trame-cell-editing`).
    /// Stored in `validations_{sheet}`, and checked before a local edit commits. `Reject`
    /// blocks the mutation, `Warn` lets it through with a warning, `Info` is a visual
    /// indicator only. Validations are an input aid, not a CRDT integrity barrier (a remote
    /// delta can still violate them).
    AddValidation {
        sheet_id: SheetId,
        range: GridRange,
        kind: ValidationKind,
        on_invalid: ValidationOnInvalid,
        message: String,
    },

    /// Remove the validation rule covering `range` on `sheet_id`
    /// (`trame-cell-editing`). A no-op
    /// if none is attached there.
    RemoveValidation { sheet_id: SheetId, range: GridRange },

    /// Protect `range` of `sheet_id` so only `allowed_user_ids` (plus the owner)
    /// may edit it (`trame-cell-editing`, Post-MVP). Soft
    /// client-side enforcement: an official client respects it, the Blind Relay
    /// cannot. Empty `allowed_user_ids` means owner-only. Identified by stable
    /// [`UserId`](crate::UserId), not `pub_key` (survives key rotation).
    SetProtection {
        sheet_id: SheetId,
        range: GridRange,
        allowed_user_ids: Vec<UserId>,
        description: String,
    },

    /// Remove the protection covering `range` on `sheet_id`
    /// (`trame-cell-editing`). A no-op if
    /// none is set there.
    RemoveProtection { sheet_id: SheetId, range: GridRange },
}

impl TrameCommand {
    /// Whether this command can change the sheet's shared CRDT state (`architecture`).
    ///
    /// Only the commands documented above as local-only or read-only are listed here;
    /// everything else is treated as mutating. That is the safe direction for a UX gate. A
    /// new variant left unclassified is withheld from a viewer rather than wrongly offered,
    /// and the server rejects the write regardless.
    pub fn is_mutating(&self) -> bool {
        !matches!(
            self,
            // Local view state: selection and scrolling are never persisted
            // (`trame-selection`), and switching sheet or personal filter view
            // only rebuilds derived caches.
            Self::ScrollViewport { .. }
                | Self::SetActiveCell { .. }
                | Self::ActivateSheet { .. }
                | Self::SaveFilterView { .. }
                | Self::ActivateFilterView { .. }
                // Read-only queries: they reply with data, emit no CRDT delta.
                | Self::RequestActiveColumnValues
                | Self::RequestSelectionStats
                | Self::RequestSortRange
                | Self::RequestColumnContent { .. }
                | Self::CopyRange
                | Self::FindNext { .. }
                | Self::ExportActiveSheetCsv
                | Self::ExportCsv { .. }
                | Self::ExportAmbreFile
                | Self::ExportAmbreFileSealed { .. }
        )
    }
}
