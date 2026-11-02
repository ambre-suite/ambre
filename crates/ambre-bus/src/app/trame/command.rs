//! Trame (spreadsheet) command surface (UI Thread -> Logic thread).
//!
//! The full `TrameCommand` vocabulary plus its parameter value types, carried by
//! move inside [`AppCommand::Trame`](crate::logic::AppCommand). Value types (styles,
//! clipboard, feature payloads) live in the sibling modules and arrive via glob.

use super::*;
use ambre_core::id::{Id, User};
use ambre_core::{Redacted, Rule, Validated};

// -----------------------------------------------------------------------------
// Trame (spreadsheet) -- commands (UI -> Logic)
// -----------------------------------------------------------------------------

/// The family of the tokens the UI knows an open cell-edit session by,
/// echoed in every event about it (`trame-cell-editing`). The view mints the
/// tokens, one per session, so an event about a closed session matches no
/// open editor.
pub enum EditSession {}

impl crate::token::TokenKind for EditSession {
    const NAME: &'static str = "EditSessionId";
}

/// A text refused as a defined name: empty, or not a letter or underscore
/// followed by letters, digits and underscores. `Debug` prints the length
/// alone; `Display` quotes the text, for the author who typed it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "{:?} is not a name: a letter or underscore, then letters, digits and underscores",
    .0.as_str()
)]
pub struct InvalidDefinedName(Redacted<String>);

impl InvalidDefinedName {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: String) -> Self {
        Self(Redacted::new(value))
    }

    /// Returns the raw text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// The name of a workbook named range, as a formula writes it (`TOTAL`), case
/// kept (`trame-named-ranges`). It is an ASCII letter or underscore, then ASCII
/// letters, digits and underscores, which is the shape of a formula bareword.
/// The grammar reads some such words as something else first, a cell
/// reference (`A1`) or a boolean (`TRUE`). `DefineName` refuses those.
/// `Debug` prints the length alone, so a log carries none of the name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefinedName(Validated<NameShape>);

/// The rule of a [`DefinedName`]: the shape of a formula bareword.
pub enum NameShape {}

impl Rule for NameShape {
    type Error = InvalidDefinedName;

    fn admit(text: String) -> Result<String, InvalidDefinedName> {
        let mut chars = text.chars();
        let starts = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        if starts && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
            Ok(text)
        } else {
            Err(InvalidDefinedName(Redacted::new(text)))
        }
    }
}

impl DefinedName {
    /// The name `text` is.
    ///
    /// # Errors
    ///
    /// [`InvalidDefinedName`] when `text` does not have the shape of a name.
    pub fn new(text: &str) -> Result<Self, InvalidDefinedName> {
        Validated::parse(text).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Display for DefinedName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// A name table is looked up by the key text the document stores.
impl std::borrow::Borrow<str> for DefinedName {
    fn borrow(&self) -> &str {
        self.0.as_str()
    }
}

/// Why a text cannot be a sheet name (`TRAME-234`). The rule is Excel's, so
/// a workbook written here opens there under the same names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SheetNameRefusal {
    /// Nothing is left once the spaces at both ends are trimmed.
    #[error("a sheet name cannot be empty")]
    Empty,
    /// More than [`SheetName::MAX_CHARS`] characters.
    #[error("a sheet name cannot be longer than 31 characters")]
    TooLong,
    /// A character a sheet name cannot hold.
    #[error("a sheet name cannot contain {0:?}")]
    Forbidden(char),
    /// An apostrophe as the first or the last character. A formula quotes a
    /// sheet name with apostrophes.
    #[error("a sheet name cannot start or end with an apostrophe")]
    EdgeApostrophe,
}

/// The display name of a Trame sheet, the label of its tab (`trame-sheets`).
/// A formula names a sheet by it (`'My Sheet'!A1`), and the document stores a
/// sheet by its id, never by it.
///
/// [`SheetName::new`] holds the rule a create or a rename obeys (`TRAME-234`).
/// [`SheetName::stored`] wraps a name a formula or the document holds, and
/// [`SheetName::numbered`] is the fallback a stored name the rule refuses
/// reads as (`TRAME-235`). Uniqueness compares [`SheetName::key`], which
/// ignores case (`TRAME-078`). `Debug` prints the length alone, so a log
/// carries none of the name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SheetName(Redacted<String>);

/// The comparison key of a [`SheetName`]: the name with its case folded. Two
/// names with one key are one name to the uniqueness rule and to a formula
/// qualifier (`TRAME-078`, `TRAME-236`). Its `Debug` form is the length too.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SheetNameKey(Redacted<String>);

impl SheetName {
    /// The most characters a sheet name holds.
    pub const MAX_CHARS: ambre_core::CharCount = ambre_core::CharCount::new(31);

    /// The characters a sheet name cannot hold. A `:` or a bracket would read
    /// as reference syntax, and the rest are Excel's.
    pub const FORBIDDEN: [char; 7] = [':', '\\', '/', '?', '*', '[', ']'];

    /// The name `text` is, trimmed of its spaces at both ends.
    ///
    /// # Errors
    ///
    /// [`SheetNameRefusal`] when the trimmed text breaks the rule.
    pub fn new(text: &str) -> Result<Self, SheetNameRefusal> {
        let text = text.trim();
        if text.is_empty() {
            return Err(SheetNameRefusal::Empty);
        }
        if ambre_core::char_len(text) > Self::MAX_CHARS {
            return Err(SheetNameRefusal::TooLong);
        }
        if let Some(c) = text.chars().find(|c| Self::FORBIDDEN.contains(c)) {
            return Err(SheetNameRefusal::Forbidden(c));
        }
        if text.starts_with('\'') || text.ends_with('\'') {
            return Err(SheetNameRefusal::EdgeApostrophe);
        }
        Ok(Self(Redacted::new(text.to_owned())))
    }

    /// The name a document or a formula already holds, kept verbatim. An
    /// older build, a concurrent peer or an import may have stored a name the
    /// rule refuses; [`SheetName::obeyed`] tells which. Only a create or a
    /// rename goes through [`SheetName::new`].
    pub fn stored(text: String) -> Self {
        Self(Redacted::new(text))
    }

    /// Whether this name obeys the rule of a create or a rename as it is
    /// spelled, spaces included (`TRAME-234`).
    pub fn obeyed(&self) -> bool {
        Self::new(self.as_str()).is_ok_and(|name| name.0 == self.0)
    }

    /// The name `Sheet<n>`, the fallback a stored name the rule refuses reads
    /// as, `n` the position of its tab from 1 (`TRAME-235`). It obeys the
    /// rule for any `n`.
    pub fn numbered(n: usize) -> Self {
        Self(Redacted::new(format!("Sheet{n}")))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The key uniqueness and qualifier lookup compare (`TRAME-078`).
    pub fn key(&self) -> SheetNameKey {
        SheetNameKey(Redacted::new(self.0.as_str().to_lowercase()))
    }

    /// The first ` (n)` variant of this name, from `n = 2`, whose key `taken`
    /// refuses. The name is cut first where the suffix would pass
    /// [`SheetName::MAX_CHARS`], so a valid name yields a valid variant.
    pub fn free_variant(&self, taken: impl Fn(&SheetNameKey) -> bool) -> Self {
        (2_u64..)
            .map(|n| {
                let suffix = format!(" ({n})");
                let room = Self::MAX_CHARS.saturating_sub(ambre_core::char_len(&suffix));
                let base: String = self.0.as_str().chars().take(room.to_usize()).collect();
                Self(Redacted::new(format!("{}{suffix}", base.trim_end())))
            })
            .find(|candidate| !taken(&candidate.key()))
            .unwrap_or_else(|| self.clone())
    }
}

/// The name of the sheet a new workbook holds, `Sheet1`.
impl Default for SheetName {
    fn default() -> Self {
        Self::numbered(1)
    }
}

impl std::fmt::Display for SheetName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// A byte refused as a CSV field separator: the double quote, CR, LF, or a
/// byte outside ASCII.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("byte {0:#04x} cannot separate CSV fields")]
pub struct InvalidCsvDelimiter(u8);

impl InvalidCsvDelimiter {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw value.
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// The byte that separates the fields of a CSV record (`trame-import-export`).
/// It is an ASCII byte other than the double quote, which opens a quoted
/// field, and CR and LF, which end a record. A byte outside ASCII would cut a
/// UTF-8 character in two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CsvDelimiter(u8);

impl CsvDelimiter {
    pub const COMMA: Self = Self(b',');
    pub const SEMICOLON: Self = Self(b';');
    pub const TAB: Self = Self(b'\t');

    /// The separator `byte` is.
    ///
    /// # Errors
    ///
    /// [`InvalidCsvDelimiter`] for the double quote, CR, LF and any byte
    /// outside ASCII.
    pub const fn new(byte: u8) -> Result<Self, InvalidCsvDelimiter> {
        match byte {
            b'"' | b'\r' | b'\n' | 0x80.. => Err(InvalidCsvDelimiter(byte)),
            _ => Ok(Self(byte)),
        }
    }

    /// The byte a CSV reader or writer is configured with.
    pub const fn byte(self) -> u8 {
        self.0
    }
}

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
#[derive(Clone)]
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
        version: ambre_core::Generation<crate::ViewportVersion>,
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
        extra: Vec<CellRect>,
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
    BeginCellEdit {
        session: crate::token::Token<EditSession>,
        row: Row,
        col: Col,
    },

    /// Commit session `session` with `text`, written to the cell it opened on.
    /// A session the Logic thread no longer holds writes nothing: its cell is
    /// gone, and [`EditTargetGone`](super::TrameEvent::EditTargetGone) said so.
    /// Opens the undo group [`CommitEdit`](TrameCommand::CommitEdit) seals.
    CommitCellEdit {
        session: crate::token::Token<EditSession>,
        text: String,
    },

    /// Close session `session` with no write: an Escape, or a fill that writes
    /// the selection instead. A no-op for a session the Logic thread no longer
    /// holds.
    EndCellEdit {
        session: crate::token::Token<EditSession>,
    },

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

    /// Merge the rectangle `rect` into a single cell, its top-left corner
    /// becoming the master (`trame-cell-merges`).
    /// Rejected locally with [`TrameEvent::MergeRejected`] if any covered cell
    /// (other than the master) is non-empty -- Excel/Sheets behavior. Stored in
    /// the per-sheet `merges_{sheet}` LWW map keyed by stable id pairs, so the
    /// merge follows its cells across structural ops.
    MergeCells { rect: CellRect },

    /// Split the merge whose rectangle contains grid cell `(row, col)`, restoring
    /// the covered cells. Any value masked by the merge (a concurrent-edit
    /// conflict, `trame-cell-merges`) reappears. A no-op if `(row, col)` is not
    /// inside a merge.
    UnmergeCells { row: Row, col: Col },

    /// Set the width (document px) of column `col` on `sheet_id` (`trame-resize`,
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
        version: ambre_core::Generation<crate::ViewportVersion>,
        sheet_id: SheetId,
        col: Col,
        width: ambre_core::Px<ambre_core::Doc>,
    },
    /// Set the height (document px, before the zoom) of row `row` on `sheet_id`. Row analogue of
    /// [`ResizeCol`](TrameCommand::ResizeCol): writes `row_heights_{sheet}`.
    ResizeRow {
        version: ambre_core::Generation<crate::ViewportVersion>,
        sheet_id: SheetId,
        row: Row,
        height: ambre_core::Px<ambre_core::Doc>,
    },

    // -- Parameterless structural intents (grid context menu,
    // `trame-context-menu`).
    // These carry no geometry: the Logic thread resolves them against the active
    // cell / selection rectangle it tracks (kept current by `SetActiveCell`).
    // They map one-to-one to a registry `StaticKey<Commands>`, so the DOM context menu, the
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
    /// `header = HeaderRow::Absent`). The Logic thread resolves the geometry and the
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
        header: HeaderRow,
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
        src: CellRect,
        dest: CellRect,
        mode: FillMode,
    },

    /// Find the next cell whose displayed value matches `query`, starting from the tracked
    /// active cell and wrapping around the active sheet (`trame-find-replace`). Read-only:
    /// the Logic thread scans the materialized extent, comparing each cell's *displayed*
    /// value (formula results included). The scan reads only the case and the extent of
    /// `options`: it is case-folded unless [`CaseMatch::Sensitive`](super::CaseMatch), by
    /// exact equality for [`MatchExtent::WholeCell`](super::MatchExtent), else by substring.
    /// Produces a [`FindResult`](crate::TrameEvent::FindResult) with every match on the
    /// sheet plus the index of the one to focus; no CRDT delta is emitted. The UI moves the
    /// active cell onto the focused match, so a repeated `FindNext` advances. `None` is the
    /// emptied field: the reply carries no match, which clears the highlight (`FIND-005`).
    FindNext { query: Option<super::FindQuery> },

    /// Replace every match of `query` with `replacement` across the active sheet (or all
    /// sheets for [`SheetScope::All`](super::SheetScope)) in a single undo group
    /// (`trame-find-replace`). Matching tests the *displayed* value, by the case and the
    /// extent of `options`, but the rewrite edits the *raw* cell text. Cells whose raw text
    /// is a formula (`=...`) are skipped unless the scope is
    /// [`SearchScope::Formulas`](super::SearchScope). Replies with
    /// [`ReplaceDone`](crate::TrameEvent::ReplaceDone) and a
    /// [`DataReset`](crate::TrameEvent::DataReset) so the UI re-fetches. `None` replaces
    /// nothing.
    ReplaceAll {
        query: Option<super::FindQuery>,
        replacement: String,
    },

    /// Replace the single match at the tracked active cell, then advance to the next match
    /// (`trame-find-replace`). If the active cell matches `query` it is rewritten in one
    /// undo group, under the same raw-text rules as
    /// [`ReplaceAll`](TrameCommand::ReplaceAll), the scope gating included. The sheet
    /// scope is not read. Whether or not it matched, the reply carries a fresh
    /// [`FindResult`](crate::TrameEvent::FindResult), so the UI moves to and highlights the
    /// following match. Replies with [`ReplaceDone`](crate::TrameEvent::ReplaceDone) (`0`
    /// or `1`). `None` replaces nothing.
    ReplaceOne {
        query: Option<super::FindQuery>,
        replacement: String,
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
    /// `name` is the case-sensitive name used in formulas (`=SUM(TOTAL)`);
    /// `reference` is its target written in A1 syntax, optionally sheet-qualified
    /// (`A1`, `A1:B10`, `Sheet2!A1:B10`). Stored in the shared `named_ranges`
    /// CRDT map (LWW) and resolved positionally against the current grid each
    /// recomputation (`trame-named-ranges`). A reference that
    /// fails to parse leaves any formula using the name as `#PARSE!`.
    DefineName {
        name: DefinedName,
        reference: String,
    },

    /// Remove a named range. Formulas referencing the now-undefined name
    /// re-evaluate to a parse error on the next recomputation. A no-op if the
    /// name does not exist.
    DeleteName { name: DefinedName },

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
    CreateSheet { name: SheetName },

    /// Rename `sheet_id`'s display label (LWW on the name field). The stable id
    /// is untouched, so same-name references keep resolving; cross-sheet formula
    /// text bound to the old display name is the known MVP gap (`trame-sheets`).
    RenameSheet { sheet_id: SheetId, name: SheetName },

    /// Move `sheet_id` to position `to_index` in the tab strip (one
    /// `LoroMovableList` move op). Pure presentation: never touches a reference
    /// (`trame-sheets`). A no-op if the id is unknown or already at that position.
    MoveSheet {
        sheet_id: SheetId,
        to_index: SheetTabPos,
    },

    /// Sort the rows of the rectangle `rect` by the values in its column
    /// `sort_col`, counted from its left edge
    /// (`trame-sort`, single-column sort). Whole rows are reordered, so cells outside
    /// `rect`'s columns move with their row -- matching Google Sheets "sort range".
    /// A header row (`HeaderRow::Present`) stays pinned in place.
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
        rect: CellRect,
        /// Column to sort on, as an index within the range (`0` = its left column).
        sort_col: ColCount,
        order: SortOrder,
        /// A first range row that is a header is not sorted.
        header: HeaderRow,
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
    /// `delimiter` is auto-detected (comma, semicolon or tab) when `None`. Each field becomes a
    /// literal cell (no formula reindexing -- the source is external). Replies with
    /// [`TrameEvent::ImportComplete`](super::TrameEvent::ImportComplete) plus a
    /// [`DataReset`](super::TrameEvent::DataReset); the whole import is a single undo
    /// group.
    ImportCsv {
        data: ambre_core::Payload<ambre_core::FileBytes>,
        /// Field separator; `None` auto-detects it.
        delimiter: Option<CsvDelimiter>,
        /// A first record that is a header row is skipped.
        header: HeaderRow,
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
        delimiter: Option<CsvDelimiter>,
        range: Option<CellRect>,
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
        passphrase: ambre_core::Passphrase,
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
        rect: CellRect,
        /// Decreasing priority: `[primary, secondary, ...]`. Each `col` is an
        /// index within the range (`0` = its left column).
        sort_keys: Vec<SortKey>,
        header: HeaderRow,
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
        range: CellRect,
        rule: CondFormatKind,
        style: CondStyle,
        priority: CondFormatPriority,
    },

    /// Replace the rule identified by `id` on `sheet_id` (`trame-cell-styles`).
    /// A no-op if no
    /// rule with that id exists.
    UpdateCondFormat {
        sheet_id: SheetId,
        id: CondFormatId,
        range: CellRect,
        rule: CondFormatKind,
        style: CondStyle,
        priority: CondFormatPriority,
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
        range: CellRect,
        kind: ValidationKind,
        on_invalid: ValidationOnInvalid,
        message: String,
    },

    /// Remove the validation rule covering `range` on `sheet_id`
    /// (`trame-cell-editing`). A no-op
    /// if none is attached there.
    RemoveValidation { sheet_id: SheetId, range: CellRect },

    /// Protect `range` of `sheet_id` so only `allowed_user_ids` (plus the owner)
    /// may edit it (`trame-cell-editing`, Post-MVP). Soft
    /// client-side enforcement: an official client respects it, the Blind Relay
    /// cannot. Empty `allowed_user_ids` means owner-only. Identified by stable
    /// [`Id<User>`](Id), not `pub_key` (survives key rotation).
    SetProtection {
        sheet_id: SheetId,
        range: CellRect,
        allowed_user_ids: Vec<Id<User>>,
        description: String,
    },

    /// Remove the protection covering `range` on `sheet_id`
    /// (`trame-cell-editing`). A no-op if
    /// none is set there.
    RemoveProtection { sheet_id: SheetId, range: CellRect },
}

// A text a user typed prints its length alone, and every other field its own
// `Debug`. A log of a command carries none of the sheet's text.
impl std::fmt::Debug for TrameCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ambre_core::RedactedChars;
        macro_rules! fields {
            ($name:literal $(, $field:ident)*) => {
                f.debug_struct($name)$(.field(stringify!($field), $field))*.finish()
            };
        }
        match self {
            Self::ScrollViewport {
                version,
                row_start,
                row_end,
                col_start,
                col_end,
            } => fields!(
                "ScrollViewport",
                version,
                row_start,
                row_end,
                col_start,
                col_end
            ),
            Self::SetActiveCell {
                row,
                col,
                anchor_row,
                anchor_col,
                extra,
                kind,
            } => fields!(
                "SetActiveCell",
                row,
                col,
                anchor_row,
                anchor_col,
                extra,
                kind
            ),
            Self::SetCellText { row, col, text } => f
                .debug_struct("SetCellText")
                .field("row", row)
                .field("col", col)
                .field("text", &RedactedChars::of(text))
                .finish(),
            Self::BeginCellEdit { session, row, col } => {
                fields!("BeginCellEdit", session, row, col)
            }
            Self::CommitCellEdit { session, text } => f
                .debug_struct("CommitCellEdit")
                .field("session", session)
                .field("text", &RedactedChars::of(text))
                .finish(),
            Self::EndCellEdit { session } => fields!("EndCellEdit", session),
            Self::SetCellsBatch { cells } => f
                .debug_struct("SetCellsBatch")
                .field(
                    "cells",
                    &cells
                        .iter()
                        .map(|(pos, text)| (pos, RedactedChars::of(text)))
                        .collect::<Vec<_>>(),
                )
                .finish(),
            Self::CommitEdit => f.write_str("CommitEdit"),
            Self::SetCellStyle { patch } => fields!("SetCellStyle", patch),
            Self::InsertRow { at } => fields!("InsertRow", at),
            Self::DeleteRow { at } => fields!("DeleteRow", at),
            Self::InsertColumn { at } => fields!("InsertColumn", at),
            Self::DeleteColumn { at } => fields!("DeleteColumn", at),
            Self::MoveRow { from, to } => fields!("MoveRow", from, to),
            Self::MoveColumn { from, to } => fields!("MoveColumn", from, to),
            Self::MergeCells { rect } => fields!("MergeCells", rect),
            Self::UnmergeCells { row, col } => fields!("UnmergeCells", row, col),
            Self::ResizeCol {
                version,
                sheet_id,
                col,
                width,
            } => fields!("ResizeCol", version, sheet_id, col, width),
            Self::ResizeRow {
                version,
                sheet_id,
                row,
                height,
            } => fields!("ResizeRow", version, sheet_id, row, height),
            Self::InsertRowAbove => f.write_str("InsertRowAbove"),
            Self::InsertRowBelow => f.write_str("InsertRowBelow"),
            Self::InsertColLeft => f.write_str("InsertColLeft"),
            Self::InsertColRight => f.write_str("InsertColRight"),
            Self::DeleteSelectedRows => f.write_str("DeleteSelectedRows"),
            Self::DeleteSelectedCols => f.write_str("DeleteSelectedCols"),
            Self::MergeSelection => f.write_str("MergeSelection"),
            Self::UnmergeActive => f.write_str("UnmergeActive"),
            Self::SortSelectionAsc => f.write_str("SortSelectionAsc"),
            Self::SortSelectionDesc => f.write_str("SortSelectionDesc"),
            Self::SortSheetAsc => f.write_str("SortSheetAsc"),
            Self::SortSheetDesc => f.write_str("SortSheetDesc"),
            Self::SortSelectionMulti { sort_keys, header } => {
                fields!("SortSelectionMulti", sort_keys, header)
            }
            Self::RequestActiveColumnValues => f.write_str("RequestActiveColumnValues"),
            Self::RequestSelectionStats => f.write_str("RequestSelectionStats"),
            Self::RequestSortRange => f.write_str("RequestSortRange"),
            Self::RequestColumnContent { col } => fields!("RequestColumnContent", col),
            Self::CopyRange => f.write_str("CopyRange"),
            Self::ApplyFilterToActiveColumn { kind } => {
                fields!("ApplyFilterToActiveColumn", kind)
            }
            Self::ClearActiveFilter => f.write_str("ClearActiveFilter"),
            Self::PasteRange {
                target_row,
                target_col,
                payload,
            } => fields!("PasteRange", target_row, target_col, payload),
            Self::PasteText { text } => f
                .debug_struct("PasteText")
                .field("text", &RedactedChars::of(text))
                .finish(),
            Self::PasteClipboardAtActive { payload } => {
                fields!("PasteClipboardAtActive", payload)
            }
            Self::FillRange {
                sheet_id,
                src,
                dest,
                mode,
            } => fields!("FillRange", sheet_id, src, dest, mode),
            Self::FindNext { query } => fields!("FindNext", query),
            Self::ReplaceAll { query, replacement } => f
                .debug_struct("ReplaceAll")
                .field("query", query)
                .field("replacement", &RedactedChars::of(replacement))
                .finish(),
            Self::ReplaceOne { query, replacement } => f
                .debug_struct("ReplaceOne")
                .field("query", query)
                .field("replacement", &RedactedChars::of(replacement))
                .finish(),
            Self::Undo => f.write_str("Undo"),
            Self::Redo => f.write_str("Redo"),
            Self::DefineName { name, reference } => f
                .debug_struct("DefineName")
                .field("name", name)
                .field("reference", &RedactedChars::of(reference))
                .finish(),
            Self::DeleteName { name } => fields!("DeleteName", name),
            Self::ActivateSheet { sheet_id } => fields!("ActivateSheet", sheet_id),
            Self::CreateSheet { name } => fields!("CreateSheet", name),
            Self::RenameSheet { sheet_id, name } => fields!("RenameSheet", sheet_id, name),
            Self::MoveSheet { sheet_id, to_index } => fields!("MoveSheet", sheet_id, to_index),
            Self::SortRange {
                sheet_id,
                rect,
                sort_col,
                order,
                header,
            } => fields!("SortRange", sheet_id, rect, sort_col, order, header),
            Self::DeleteSheet { sheet_id } => fields!("DeleteSheet", sheet_id),
            Self::ApplyFilter {
                sheet_id,
                col,
                kind,
            } => fields!("ApplyFilter", sheet_id, col, kind),
            Self::ClearFilter { sheet_id } => fields!("ClearFilter", sheet_id),
            Self::ImportCsv {
                data,
                delimiter,
                header,
                target_sheet_id,
                start_row,
                start_col,
            } => fields!(
                "ImportCsv",
                data,
                delimiter,
                header,
                target_sheet_id,
                start_row,
                start_col
            ),
            Self::ExportActiveSheetCsv => f.write_str("ExportActiveSheetCsv"),
            Self::ExportCsv {
                sheet_id,
                delimiter,
                range,
            } => fields!("ExportCsv", sheet_id, delimiter, range),
            Self::ExportAmbreFile => f.write_str("ExportAmbreFile"),
            Self::ExportAmbreFileSealed { passphrase } => {
                fields!("ExportAmbreFileSealed", passphrase)
            }
            Self::SaveFilterView {
                name,
                sheet_id,
                filter_col,
                kind,
            } => f
                .debug_struct("SaveFilterView")
                .field("name", &RedactedChars::of(name))
                .field("sheet_id", sheet_id)
                .field("filter_col", filter_col)
                .field("kind", kind)
                .finish(),
            Self::ActivateFilterView { view_id } => fields!("ActivateFilterView", view_id),
            Self::SortRangeMulti {
                sheet_id,
                rect,
                sort_keys,
                header,
            } => fields!("SortRangeMulti", sheet_id, rect, sort_keys, header),
            Self::AddCondFormat {
                sheet_id,
                range,
                rule,
                style,
                priority,
            } => fields!("AddCondFormat", sheet_id, range, rule, style, priority),
            Self::UpdateCondFormat {
                sheet_id,
                id,
                range,
                rule,
                style,
                priority,
            } => fields!(
                "UpdateCondFormat",
                sheet_id,
                id,
                range,
                rule,
                style,
                priority
            ),
            Self::RemoveCondFormat { sheet_id, id } => fields!("RemoveCondFormat", sheet_id, id),
            Self::AddValidation {
                sheet_id,
                range,
                kind,
                on_invalid,
                message,
            } => f
                .debug_struct("AddValidation")
                .field("sheet_id", sheet_id)
                .field("range", range)
                .field("kind", kind)
                .field("on_invalid", on_invalid)
                .field("message", &RedactedChars::of(message))
                .finish(),
            Self::RemoveValidation { sheet_id, range } => {
                fields!("RemoveValidation", sheet_id, range)
            }
            Self::SetProtection {
                sheet_id,
                range,
                allowed_user_ids,
                description,
            } => f
                .debug_struct("SetProtection")
                .field("sheet_id", sheet_id)
                .field("range", range)
                .field("allowed_user_ids", allowed_user_ids)
                .field("description", &RedactedChars::of(description))
                .finish(),
            Self::RemoveProtection { sheet_id, range } => {
                fields!("RemoveProtection", sheet_id, range)
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{CsvDelimiter, DefinedName, SheetName, SheetNameRefusal};

    /// A sheet name, its key and a defined name print their length alone in
    /// `Debug`, and none of the text a user typed.
    #[test]
    fn a_name_of_a_workbook_prints_its_length_alone() {
        let sheet = SheetName::new("Payroll").unwrap();
        assert_eq!(format!("{sheet:?}"), "SheetName(7 chars)");
        assert_eq!(format!("{:?}", sheet.key()), "SheetNameKey(7 chars)");
        let name = DefinedName::new("TOTAL").unwrap();
        assert_eq!(format!("{name:?}"), "DefinedName(5 chars)");
        let refused = DefinedName::new("Q3 layoffs").unwrap_err();
        assert_eq!(format!("{refused:?}"), "InvalidDefinedName(10 chars)");
        assert!(refused.to_string().contains("Q3 layoffs"));
    }

    /// The Excel rule: 1 to 31 characters, none of `: \ / ? * [ ]`, and no
    /// apostrophe at either end (`TRAME-234`).
    #[test]
    fn a_sheet_name_follows_the_excel_rule() {
        for name in [
            "Sheet1",
            "My Sheet",
            "Bob's",
            "a",
            "x".repeat(31).as_str(),
            "\u{e9}t\u{e9} 2026",
        ] {
            assert_eq!(
                SheetName::new(name).map(|n| n.as_str().to_owned()),
                Ok(name.to_owned())
            );
        }
        assert_eq!(SheetName::new(""), Err(SheetNameRefusal::Empty));
        assert_eq!(SheetName::new("   "), Err(SheetNameRefusal::Empty));
        assert_eq!(
            SheetName::new(&"x".repeat(32)),
            Err(SheetNameRefusal::TooLong)
        );
        assert_eq!(
            SheetName::new(&"\u{e9}".repeat(32)),
            Err(SheetNameRefusal::TooLong)
        );
        for c in [':', '\\', '/', '?', '*', '[', ']'] {
            assert_eq!(
                SheetName::new(&format!("a{c}b")),
                Err(SheetNameRefusal::Forbidden(c)),
                "{c:?} is refused"
            );
        }
        assert_eq!(
            SheetName::new("'Data"),
            Err(SheetNameRefusal::EdgeApostrophe)
        );
        assert_eq!(
            SheetName::new("Data'"),
            Err(SheetNameRefusal::EdgeApostrophe)
        );
    }

    /// Two names that differ only in case share one key (`TRAME-078`).
    #[test]
    fn a_sheet_name_key_ignores_case() -> Result<(), SheetNameRefusal> {
        let data = SheetName::new("Data")?;
        assert_eq!(data.key(), SheetName::new("DATA")?.key());
        assert_eq!(data.key(), SheetName::new("data")?.key());
        assert_eq!(
            SheetName::new("\u{c9}t\u{e9}")?.key(),
            SheetName::new("\u{e9}T\u{c9}")?.key()
        );
        assert_ne!(data.key(), SheetName::new("Data2")?.key());
        Ok(())
    }

    /// A numbered variant stays within the 31 characters, so it is a name a
    /// create can store (`TRAME-234`).
    #[test]
    fn a_numbered_variant_is_a_valid_name() -> Result<(), SheetNameRefusal> {
        let short = SheetName::new("Data")?;
        let taken = [short.key()];
        assert_eq!(
            short.free_variant(|key| taken.contains(key)).as_str(),
            "Data (2)"
        );
        let long = SheetName::new(&"x".repeat(31))?;
        let variant = long.free_variant(|key| *key == long.key());
        assert_eq!(variant.as_str(), format!("{} (2)", "x".repeat(27)));
        assert_eq!(SheetName::new(variant.as_str()).as_ref(), Ok(&variant));
        Ok(())
    }

    /// The cap counts characters, not bytes: a name of 31 accented letters
    /// is a name, and its variant keeps 27 of them.
    #[test]
    fn the_cap_counts_characters_not_bytes() -> Result<(), SheetNameRefusal> {
        let accented = SheetName::new(&"\u{e9}".repeat(31))?;
        assert_eq!(
            SheetName::new(&"\u{e9}".repeat(32)),
            Err(SheetNameRefusal::TooLong)
        );
        let variant = accented.free_variant(|key| *key == accented.key());
        assert_eq!(variant.as_str(), format!("{} (2)", "\u{e9}".repeat(27)));
        Ok(())
    }

    #[test]
    fn a_name_is_a_letter_or_underscore_then_word_characters() {
        for name in ["TOTAL", "_TOTAL_2", "a", "_", "A1", "TRUE"] {
            assert!(
                DefinedName::new(name).is_ok(),
                "{name:?} has a name's shape"
            );
        }
        for text in ["", "1TOTAL", "MY TOTAL", "TOTAL!", "TÔTAL", "A-B"] {
            assert!(DefinedName::new(text).is_err(), "{text:?} is refused");
        }
    }

    /// A separator never opens a quote, ends a record or cuts a character
    /// (`TRAME-IO-049`).
    #[test]
    fn a_separator_is_an_ascii_byte_that_neither_quotes_nor_ends_a_record() {
        for byte in [b',', b';', b'\t', b'|', b' '] {
            assert_eq!(CsvDelimiter::new(byte).map(CsvDelimiter::byte), Ok(byte));
        }
        for byte in [b'"', b'\r', b'\n', 0x80, 0xff] {
            assert!(CsvDelimiter::new(byte).is_err(), "{byte:#04x} is refused");
        }
    }
}
