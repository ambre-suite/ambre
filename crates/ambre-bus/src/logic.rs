//! Logic-bound protocol — messages addressed to the Logic thread from the UI.
//!
//! Holds the [`LogicCommand`] envelope (UI Thread → Logic thread) plus the typed
//! application **command** data plane it carries: [`AppCommand`] (a closed sum
//! over the document types) and the per-app command enums (`TrameCommand`, …).
//!
//! Carried over a `flume` channel in shared memory — typed Rust values by move,
//! no serialization. The symmetric Logic → UI direction (events) lives in
//! [`crate::ui`].
//!
//! # Three planes, one envelope
//! - **Content** ([`LogicCommand::App`]): document edits / viewport requests,
//!   routed by `DocumentId` inside the Logic thread. [`AppCommand`] is a closed
//!   sum — adding a document type = adding a variant (no trait object, no `Any`,
//!   no crate cycle; this module depends only on `ambre-core`).
//! - **Service** ([`LogicCommand::Rest`]): the correlatable request/response
//!   family, defined once in [`crate::rest`] and forwarded verbatim to Sync.
//! - **Control** (`OpenDocument`, `CloseDocument`, `OpenChannel`, `CloseChannel`,
//!   `SendChannelMessage`): fire-and-forget session/content-send commands with
//!   no single response to correlate — kept explicit.

use crate::{ChannelId, DocType, DocumentId, RequestId, RestCommand, UserId};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// LogicCommand — UI Thread → Logic thread
// ─────────────────────────────────────────────────────────────────────────────

/// Message sent from the UI Thread to the Logic thread.
///
/// - [`App`](LogicCommand::App) (**P2**): user edits / viewport requests, < 50 ms
///   budget (scroll requests < 10 ms). Routed to the document's state.
/// - [`Rest`](LogicCommand::Rest) (**P3**): a [`RestCommand`] service operation,
///   forwarded to the Sync thread as [`SyncCommand::Rest`] (with a small Logic
///   hook for the few ops that also mutate Logic state, e.g. `Logout`).
/// - Control variants (**P3**): forwarded to the Sync thread as their explicit
///   `SyncCommand` counterparts.
///
/// [`SyncCommand::Rest`]: crate::SyncCommand::Rest
#[derive(Debug, Clone)]
pub enum LogicCommand {
    // ── P2 — Content plane ────────────────────────────────────────────────────
    /// Application-specific command for an open document (cell edit, viewport
    /// scroll in Trame; text insertion in Prose; element move in Toile; …).
    ///
    /// The Logic dispatcher routes [`command`](AppCommand) to the state owning
    /// `document_id` and replies with [`crate::ui::UiEvent::Viewport`] /
    /// [`crate::ui::UiEvent::Presence`] events.
    App {
        document_id: DocumentId,
        /// Typed application command (schema owned by [`crate::logic`]).
        command: AppCommand,
    },

    /// Restore a document's CRDT state from a locally persisted snapshot
    /// (IndexedDB), dispatched by the Main thread at startup. Local-only: it
    /// never reaches Sync or the server -- it lets an edit survive a refresh
    /// without a network round-trip.
    RestoreLocalSnapshot {
        document_id: DocumentId,
        /// Document kind, known from the boot URL route (`/trame/<id>`). Lets the
        /// handler materialise the live document if the restore wins the startup
        /// race against [`OpenDocument`](LogicCommand::OpenDocument).
        doc_type: DocType,
        /// Full Loro snapshot (`ExportMode::Snapshot`) read back from local
        /// storage.
        snapshot: Vec<u8>,
    },

    // ── P3 — Service plane (relayed request/response) ─────────────────────────
    /// A [`RestCommand`] service operation. The Logic dispatcher applies any
    /// state hook (e.g. `Logout` clears the document registry) and forwards it
    /// to the Sync thread as [`SyncCommand::Rest`], echoing `id` on the response.
    ///
    /// [`SyncCommand::Rest`]: crate::SyncCommand::Rest
    Rest {
        id: RequestId,
        cmd: RestCommand,
    },

    // ── P3 — Control plane (fire-and-forget, no correlatable response) ─────────
    /// Open a real-time sync session for a document.
    ///
    /// The Sync thread fetches members + `doc_key_enc`, decrypts the DocumentKey,
    /// loads snapshot + deltas, and opens a WebSocket subscription. The first
    /// [`crate::ui::UiEvent::Viewport`] after this indicates the document is ready.
    OpenDocument {
        document_id: DocumentId,
        /// Document kind, known from the URL route (`/trame/<id>`). Carried so a
        /// not-yet-on-server document can be lazily created with the right type
        /// on its first modification.
        doc_type: DocType,
    },

    /// Close the real-time sync session for a document. The Logic thread drops
    /// the in-memory state; the Sync thread unsubscribes and zeroizes the key.
    CloseDocument {
        document_id: DocumentId,
    },

    /// Open a real-time sync session for a channel (subscribe to messages).
    OpenChannel {
        channel_id: ChannelId,
    },

    /// Close the real-time sync session for a channel.
    CloseChannel {
        channel_id: ChannelId,
    },

    /// Send a message to a channel.
    ///
    /// `payload` is an opaque application byte blob (schema owned by
    /// `ambre-murmure`); the Sync thread routes it into the active segment.
    ///
    /// TODO: migrate the Murmure channel surface to a typed `AppCommand::Murmure(…)`
    /// like the document surface (Murmure is currently a stub).
    SendChannelMessage {
        channel_id: ChannelId,
        /// Opaque application byte blob (schema owned by `ambre-murmure`).
        payload: Vec<u8>,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// AppCommand — typed application command family (content plane)
// ─────────────────────────────────────────────────────────────────────────────

/// Application-specific command, UI Thread → Logic thread.
///
/// Carried inside [`LogicCommand::App`]. The Logic dispatcher matches the
/// variant, routes it to the `DocumentId`'s document state, and produces zero or
/// more [`AppEvent`](crate::ui::AppEvent)s.
#[derive(Debug, Clone)]
pub enum AppCommand {
    /// Command for a Trame (spreadsheet) document.
    Trame(TrameCommand),
    // Future document types: Prose(ProseCommand), Toile(ToileCommand),
    // Murmure(MurmureCommand), Rendezvous(RendezvousCommand), …
    // Future plugin surface: Plugin(Box<dyn Plugin>).
}

// ─────────────────────────────────────────────────────────────────────────────
// Trame (spreadsheet) — commands (UI → Logic)
// ─────────────────────────────────────────────────────────────────────────────

/// Commands sent from the UI Thread to the Trame logic for an open document.
#[derive(Debug, Clone)]
pub enum TrameCommand {
    /// Request cell data for a viewport region.
    ///
    /// The UI Thread computes the visible range (plus padding) from its scroll
    /// position and grid dimensions; the Logic thread replies with a
    /// [`TrameEvent::ViewportData`](crate::ui::TrameEvent::ViewportData) covering
    /// `[row_start..=row_end] × [col_start..=col_end]`.
    ///
    /// `version` is a monotonic counter: the UI Thread discards any response
    /// whose version is older than the last accepted one.
    ScrollViewport {
        version: u64,
        row_start: u32,
        row_end: u32,
        col_start: u32,
        col_end: u32,
    },

    /// Move the active cell to `(row, col)` and report the selection rectangle
    /// whose opposite (anchor) corner is `(anchor_row, anchor_col)`.
    ///
    /// Formatting ([`SetCellStyle`](TrameCommand::SetCellStyle)) targets whatever
    /// selection the Logic thread tracks, so the UI Thread must keep it informed
    /// when the cursor moves. The Logic thread replies with the active cell's
    /// resolved style ([`TrameEvent::StyleApplied`](crate::ui::TrameEvent::StyleApplied))
    /// so the formatting toolbar tracks the selection.
    ///
    /// `(anchor_row, anchor_col)` is the selection's other corner (equal to
    /// `(row, col)` for a single-cell selection). The Logic thread normalizes the
    /// two corners into the selection rectangle that the parameterless structural
    /// intents (`MergeSelection`, `DeleteSelectedRows`, ...) resolve against. The
    /// selection is local view state -- never persisted, never synced
    /// (`-ui.md` sec.16).
    SetActiveCell {
        row: u32,
        col: u32,
        anchor_row: u32,
        anchor_col: u32,
    },

    /// Set the plaintext content of cell `(row, col)`.
    ///
    /// Emitted when the UI commits an edit session on the transparent textarea
    /// pipe. The Logic thread stores the text in the document CRDT (the source
    /// of truth); an empty `text` clears the cell. The UI updates its own cache
    /// optimistically, so no echo event is required.
    SetCellText {
        row: u32,
        col: u32,
        text: String,
    },

    /// Set the plaintext content of many cells in one shot, as `(row, col,
    /// text)` triples. Emitted for a multi-cell fill (Ctrl+Enter) or a clear over
    /// a range (Delete on a selection). The Logic thread applies every write
    /// inside a single `UndoManager` group, so the whole batch is one undo item
    /// (unlike a stream of `SetCellText`, each of which is its own item). An
    /// empty `text` clears that cell.
    SetCellsBatch {
        cells: Vec<(u32, u32, String)>,
    },

    /// Close the undo group opened by the cell edit, on cell exit
    /// (`ambre-spec-trame-ui.md` §6.C, `-logic.md` §8). An interactive edit can
    /// mutate the `LoroDoc` more than once -- the cell write plus derived
    /// side-effects such as a filtered-column recompute -- and Loro records one
    /// `UndoManager` item per commit. The Logic thread groups every mutation of
    /// the edit into a single undo item and seals that group when this command
    /// arrives, so the whole cell edit is exactly one undo step (1 cell edit = 1
    /// undo item). Emitted by the UI right after the committing
    /// [`SetCellText`](TrameCommand::SetCellText) when leaving the inline editor;
    /// produces no CRDT delta of its own. A no-op if no edit group is open.
    CommitEdit,

    /// Apply a sparse formatting [`StylePatch`] to every cell of the tracked
    /// selection (`-ui.md` sec.5bis). The Logic thread resolves the geometry from
    /// the selection it keeps via [`SetActiveCell`](TrameCommand::SetActiveCell)
    /// -- the same parameterless-geometry contract as the structural intents --
    /// merges the patch into each cell's stored style, persists the result in the
    /// per-sheet style containers (a synchronised CRDT mutation, hence undoable),
    /// reformats display text for any number-format change, and replies with a
    /// [`TrameEvent::StyleApplied`](crate::ui::TrameEvent::StyleApplied) carrying
    /// the active cell's resulting style plus a [`DataReset`] so the UI re-fetches
    /// the restyled viewport.
    ///
    /// [`DataReset`]: crate::ui::TrameEvent::DataReset
    SetCellStyle {
        patch: StylePatch,
    },

    /// Insert a blank row at grid position `at`, shifting existing rows at
    /// `>= at` down by one. One `LoroMovableList` op; cells keyed by stable id
    /// pairs follow their row automatically (`ambre-spec-trame-logic.md` §2).
    /// A row inserted within a range formula is naturally included; references
    /// are never rewritten (§12). A no-op past the materialized used range
    /// (virtual rows are already blank).
    InsertRow {
        at: u32,
    },

    /// Delete the row at grid position `at`, shifting rows at `> at` up by one.
    /// Its cells/formatting are dropped and any formula referencing the now
    /// removed row id resolves to `#REF!` (§12). A no-op in virtual space.
    DeleteRow {
        at: u32,
    },

    /// Insert a blank column at grid position `at`, shifting existing columns at
    /// `>= at` right by one. Column analogue of [`InsertRow`](TrameCommand::InsertRow).
    InsertColumn {
        at: u32,
    },

    /// Delete the column at grid position `at`, shifting columns at `> at` left
    /// by one. Column analogue of [`DeleteRow`](TrameCommand::DeleteRow).
    DeleteColumn {
        at: u32,
    },

    /// Merge the rectangular range `[row..=row_end] x [col..=col_end]` into a
    /// single cell, the top-left corner becoming the master
    /// (`ambre-spec-trame-logic.md` §6). The corners may be given in any order.
    /// Rejected locally with [`TrameEvent::MergeRejected`] if any covered cell
    /// (other than the master) is non-empty -- Excel/Sheets behavior. Stored in
    /// the per-sheet `merges_{sheet}` LWW map keyed by stable id pairs, so the
    /// merge follows its cells across structural ops.
    MergeCells {
        row: u32,
        col: u32,
        row_end: u32,
        col_end: u32,
    },

    /// Split the merge whose rectangle contains grid cell `(row, col)`, restoring
    /// the covered cells. Any value masked by the merge (a concurrent-edit
    /// conflict, §6) reappears. A no-op if `(row, col)` is not inside a merge.
    UnmergeCells {
        row: u32,
        col: u32,
    },

    // -- Parameterless structural intents (grid context menu, `-ui.md` sec.5.C).
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
    /// earlier positions stay valid). One `DeleteRow` per row internally.
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
    /// [`TrameEvent::SortComplete`]: crate::ui::TrameEvent::SortComplete
    SortSelectionAsc,
    /// Sort the current selection rectangle descending on its first column
    /// (see [`SortSelectionAsc`](TrameCommand::SortSelectionAsc)).
    SortSelectionDesc,
    /// Sort the whole sheet (its materialized used range) ascending on the active
    /// cell's column -- the "sort by column" affordance for a whole-column
    /// selection (`-ui.md` sec.5.C). Equivalent to [`SortRange`] over the full
    /// used range with `sort_col` resolved to the active column. The Logic thread
    /// resolves the geometry and the active sheet itself. Replies with
    /// [`TrameEvent::SortComplete`].
    ///
    /// [`SortRange`]: TrameCommand::SortRange
    /// [`TrameEvent::SortComplete`]: crate::ui::TrameEvent::SortComplete
    SortSheetAsc,
    /// Sort the whole sheet descending on the active cell's column
    /// (see [`SortSheetAsc`](TrameCommand::SortSheetAsc)).
    SortSheetDesc,

    /// Request the distinct visible values of the active cell's column, for the
    /// shared filter popover's "by values" checkbox list (`-ui.md` sec.5.D). The
    /// Logic thread resolves the column and active sheet itself, evaluates every
    /// non-hidden materialized row through the formula cache, and replies with
    /// [`TrameEvent::ColumnValues`](crate::ui::TrameEvent::ColumnValues)
    /// (deduplicated, sorted). A read-only query: no CRDT delta.
    RequestActiveColumnValues,
    /// Apply a shared filter to the active cell's column (`-ui.md` sec.5.D). The
    /// parameterless-geometry sibling of [`ApplyFilter`]: the Logic thread
    /// resolves `sheet_id`/`col` from the tracked active cell, so the UI only
    /// supplies the user's chosen [`FilterKind`]. Replies with
    /// [`TrameEvent::FilterApplied`](crate::ui::TrameEvent::FilterApplied).
    ///
    /// [`ApplyFilter`]: TrameCommand::ApplyFilter
    ApplyFilterToActiveColumn {
        kind: FilterKind,
    },
    /// Clear the shared filter on the active sheet (`-ui.md` sec.5.D). The
    /// parameterless sibling of [`ClearFilter`]: the Logic thread resolves the
    /// active sheet itself. Replies with
    /// [`TrameEvent::FilterCleared`](crate::ui::TrameEvent::FilterCleared).
    ///
    /// [`ClearFilter`]: TrameCommand::ClearFilter
    ClearActiveFilter,

    /// Paste a clipboard [`ClipboardPayload`] with its top-left landing on
    /// `(target_row, target_col)` (`-ui.md` sec.5.E). The Logic thread is the
    /// only holder of document data, so the paste -- including relative-formula
    /// reindexing and style application -- happens here: every pasted cell lands
    /// at `(target_row + dr, target_col + dc)`, a formula's relative references
    /// shift by `(target_row - src_row, target_col - src_col)`, and the carried
    /// [`CellStyle`] (plus bold/italic) is written verbatim. The whole paste is a
    /// single undo group. Replies with a [`DataReset`] so the UI re-fetches the
    /// pasted region; a paste blocked by a protected range (Post-MVP) replies
    /// [`TrameEvent::PasteRejected`](crate::ui::TrameEvent::PasteRejected).
    ///
    /// [`DataReset`]: crate::ui::TrameEvent::DataReset
    PasteRange {
        target_row: u32,
        target_col: u32,
        payload: ClipboardPayload,
    },

    /// Paste plain tab-separated `text` at the tracked active cell (`-ui.md`
    /// sec.5.E). The parameterless-geometry sibling of
    /// [`PasteRange`](TrameCommand::PasteRange): the Logic thread resolves the
    /// target from the active cell it keeps via
    /// [`SetActiveCell`](TrameCommand::SetActiveCell), so a toolbar/menu Paste
    /// button -- which reads the OS clipboard asynchronously and has no view
    /// state -- can paste without knowing the cursor. Each field becomes a
    /// literal cell (no relative-formula reindexing, the source is external).
    /// Replies with a [`DataReset`](crate::ui::TrameEvent::DataReset).
    PasteText {
        text: String,
    },

    /// Paste a typed [`ClipboardPayload`] at the tracked active cell (`-ui.md`
    /// sec.5.E). The parameterless-geometry sibling of
    /// [`PasteRange`](TrameCommand::PasteRange): the Logic thread resolves the
    /// target from the active cell it keeps, so the toolbar/menu Paste button --
    /// which reads the OS clipboard asynchronously via the Clipboard API and has
    /// no view state -- gets the same styled, formula-reindexed paste as a native
    /// Ctrl+V. The reindex offset is `active - payload.src`.
    PasteClipboardAtActive {
        payload: ClipboardPayload,
    },

    /// Autofill the `src` rectangle into `dest` (`-ui.md` sec.5.F). `dest`
    /// contains `src` and extends it along a single axis (the fill handle locks to
    /// one direction); the cells of `dest \ src` are the fill region. The Logic
    /// thread reads the source cells from its document model and, per fill line,
    /// either extrapolates a numeric arithmetic [`Series`](FillMode::Series) or
    /// tiles/[`Copy`](FillMode::Copy)s the source values, reindexing a copied
    /// formula's relative references by its tile offset (`formula::shift`, as in
    /// [`PasteRange`](TrameCommand::PasteRange)) and carrying each source cell's
    /// style. The whole fill is a single undo group. With [`FillMode::Auto`] the
    /// thread picks Series when a line's source values form an arithmetic numeric
    /// progression, Copy otherwise. Replies with a
    /// [`DataReset`](crate::ui::TrameEvent::DataReset). A no-op when `dest == src`.
    FillRange {
        sheet_id: String,
        src: GridRange,
        dest: GridRange,
        mode: FillMode,
    },

    /// Find the next cell whose displayed value matches `query`, starting from
    /// the tracked active cell and wrapping around the active sheet
    /// (`ambre-spec-trame-ui.md` sec.5.G). Read-only: the Logic thread scans the
    /// materialized extent comparing each cell's *displayed* value (formula
    /// results included), case-folded unless `match_case`, by exact equality when
    /// `whole_cell` else by substring. Produces a
    /// [`FindResult`](crate::TrameEvent::FindResult) with every match on the sheet
    /// plus the index of the one to focus; no CRDT delta is emitted. The UI moves
    /// the active cell onto the focused match, so a repeated `FindNext` advances.
    FindNext {
        query: String,
        match_case: bool,
        whole_cell: bool,
    },

    /// Replace every match of `query` with `replacement` across the active sheet
    /// (or all sheets when `all_sheets`) in a single undo group
    /// (`ambre-spec-trame-ui.md` sec.5.G). Matching tests the *displayed* value
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

    /// Replace the single match at the tracked active cell, then advance to the
    /// next match (`ambre-spec-trame-ui.md` sec.5.G). If the active cell matches
    /// `query` it is rewritten (same raw-text rules as
    /// [`ReplaceAll`](TrameCommand::ReplaceAll), `in_formulas` gating included) in
    /// one undo group; whether or not it matched, the reply carries a fresh
    /// [`FindResult`](crate::TrameEvent::FindResult) so the UI moves to and
    /// highlights the following match. Replies with
    /// [`ReplaceDone`](crate::TrameEvent::ReplaceDone) (`0` or `1`).
    ReplaceOne {
        query: String,
        replacement: String,
        match_case: bool,
        whole_cell: bool,
        in_formulas: bool,
    },

    /// Undo the last local edit (`ambre-spec-trame-logic.md` §8). Backed by Loro's
    /// `UndoManager`: it replays the inverse of this session's most recent commit
    /// as fresh CRDT ops (1 `TrameCommand` == 1 commit == 1 undo item). The
    /// inverse ops broadcast to peers like any other edit, so undo converges. A
    /// no-op when the undo stack is empty.
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
    /// recomputation (`ambre-spec-trame-logic.md` §8, Phase 2). A reference that
    /// fails to parse leaves any formula using the name as `#PARSE!`.
    DefineName {
        name: String,
        reference: String,
    },

    /// Remove a named range. Formulas referencing the now-undefined name
    /// re-evaluate to a parse error on the next recomputation. A no-op if the
    /// name does not exist.
    DeleteName {
        name: String,
    },

    /// Switch the sheet the UI is looking at to `sheet_id`. The Logic thread
    /// rebuilds its active-sheet derived caches (index, merges, formatting,
    /// formula engine) and replies with a [`DataReset`] so the UI re-fetches the
    /// new sheet's viewport (`ambre-spec-trame-logic.md` §2.D). A no-op if the id
    /// is unknown.
    ///
    /// [`DataReset`]: crate::ui::TrameEvent::DataReset
    ActivateSheet {
        sheet_id: String,
    },

    /// Add a new sheet named `name`. Mints a peer-scoped stable `sheet_id`
    /// (`{peer:016x}_{counter:08x}`) so concurrent creates on two peers never
    /// collide, registers it in `sheets`, appends it to `sheet_order`, and
    /// replies with a refreshed [`SheetList`](crate::ui::TrameEvent::SheetList)
    /// (`ambre-spec-trame-logic.md` §2.D). Cells are created lazily on first write.
    CreateSheet {
        name: String,
    },

    /// Rename `sheet_id`'s display label (LWW on the name field). The stable id
    /// is untouched, so same-name references keep resolving; cross-sheet formula
    /// text bound to the old display name is the known MVP gap (§2.D).
    RenameSheet {
        sheet_id: String,
        name: String,
    },

    /// Move `sheet_id` to position `to_index` in the tab strip (one
    /// `LoroMovableList` move op). Pure presentation: never touches a reference
    /// (§2.D). A no-op if the id is unknown or already at that position.
    MoveSheet {
        sheet_id: String,
        to_index: u32,
    },

    /// Sort the rows of the rectangular range `[row_start..=row_end] x
    /// [col_start..=col_end]` by the values in the column `col_start + sort_col`
    /// (`ambre-spec-trame-features.md` §3, single-column sort). Whole rows are
    /// reordered, so cells outside `[col_start..=col_end]` move with their row --
    /// matching Google Sheets "sort range". `has_header` keeps the first range row
    /// pinned in place. Realized as `LoroMovableList` move ops on
    /// `rows_{sheet_id}`: cells keyed by stable id pairs follow their row, so no
    /// cell key is rewritten and references are preserved (§12). LWW on the row
    /// list resolves concurrent sorts deterministically. A no-op when the range is
    /// empty/virtual. Replies with [`TrameEvent::SortComplete`].
    ///
    /// [`TrameEvent::SortComplete`]: crate::ui::TrameEvent::SortComplete
    SortRange {
        sheet_id: String,
        row_start: u32,
        row_end: u32,
        col_start: u32,
        col_end: u32,
        /// Column to sort on, as an index within the range (`0` = `col_start`).
        sort_col: u32,
        ascending: bool,
        /// When `true`, the first range row is treated as a header and not sorted.
        has_header: bool,
    },

    /// Delete `sheet_id` and its per-sheet containers (§2.D). Rejected locally
    /// with [`TrameEvent::Error`](crate::ui::TrameEvent::Error) if it is the last
    /// remaining sheet (a workbook always keeps >= 1). If the active sheet is
    /// deleted, the active sheet falls back to a neighbour. Cross-sheet formulas
    /// targeting the gone id resolve to `#REF!`.
    DeleteSheet {
        sheet_id: String,
    },

    /// Apply a shared filter to column `col` of `sheet_id`
    /// (`ambre-spec-trame-features.md` §1.A). The filter is stored in the document
    /// CRDT (`filters_{sheet}` + `filter_hidden_{sheet}`), so every collaborator
    /// sees the same hidden rows. The Logic thread evaluates the column through
    /// the formula cache, writes the failing rows' stable ids into
    /// `filter_hidden_{sheet}` (keyed by `row_uid`, so the hidden set follows the
    /// data across a sort or insert), and broadcasts the resulting delta. Rows
    /// hidden by a filter are excluded from the viewport (flagged
    /// [`DisplayCell::is_filtered`](crate::ui::DisplayCell::is_filtered)) but stay
    /// in calculations (`=SUM` still totals them). A filter is a plain CRDT
    /// mutation, hence undoable. Replies with
    /// [`TrameEvent::FilterApplied`](crate::ui::TrameEvent::FilterApplied). A no-op
    /// for a non-active sheet or a virtual (never-materialized) column.
    ApplyFilter {
        sheet_id: String,
        /// Grid column the filter is applied to (0-based).
        col: u32,
        kind: FilterKind,
    },

    /// Clear the shared filter on `sheet_id`: removes every `filter_hidden_{sheet}`
    /// entry and the stored `filters_{sheet}` definition, then broadcasts the
    /// delta (`ambre-spec-trame-features.md` §1.A). Replies with
    /// [`TrameEvent::FilterCleared`](crate::ui::TrameEvent::FilterCleared). A no-op
    /// when no filter is active.
    ClearFilter {
        sheet_id: String,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Deferred feature surface (Phase 2+)
    //
    // The variants below complete the `TrameCommand` surface mandated by
    // `ambre-spec-trame-features.md §10` ("all feature variants are in the
    // enums"). They are part of the typed data plane now so the UI Thread and
    // the Logic dispatcher compile against a stable command vocabulary; their
    // Logic-thread handlers are no-op stubs until the corresponding phase lands
    // (personal filter views / multi-column sort -> Phase 2, conditional
    // formatting & validation -> Phase 2/3, range protection -> Post-MVP).
    // ─────────────────────────────────────────────────────────────────────────

    /// Persist a personal (non-shared) filter view in local storage only
    /// (`ambre-spec-trame-features.md §1.B`). Never touches the `LoroDoc`, so it
    /// emits no CRDT delta. Replies with
    /// [`TrameEvent::FilterViewSaved`](crate::ui::TrameEvent::FilterViewSaved).
    SaveFilterView {
        name: String,
        sheet_id: String,
        /// Grid column the personal view filters on (0-based).
        filter_col: u32,
        kind: FilterKind,
    },

    /// Activate a previously saved personal filter view (`§1.B`). Loads it from
    /// local storage, recomputes its hidden rows against the current doc state,
    /// and re-sends the viewport without them -- a purely local view change, no
    /// CRDT delta. A no-op if `view_id` is unknown.
    ActivateFilterView {
        view_id: String,
    },

    /// Sort the rows of a range by several keys in decreasing priority
    /// (`ambre-spec-trame-features.md §2`, multi-column sort). Like
    /// [`SortRange`](TrameCommand::SortRange) but with a lexicographic
    /// comparison over `sort_keys`. Realized as `LoroMovableList` move ops on
    /// `rows_{sheet}`; references follow the data (§12).
    SortRangeMulti {
        sheet_id: String,
        row_start: u32,
        row_end: u32,
        col_start: u32,
        col_end: u32,
        /// Decreasing priority: `[primary, secondary, ...]`. Each `col` is an
        /// index within the range (`0` = `col_start`).
        sort_keys: Vec<SortKey>,
        has_header: bool,
    },

    /// Add a conditional-formatting rule to `sheet_id` over `range`
    /// (`ambre-spec-trame-features.md §3`). Stored in `cond_formats_{sheet}`
    /// (LoroList), evaluated per-cell after each recompute into a
    /// `DisplayCell.style` override (never mutates `styles_{sheet}`). `priority`
    /// orders evaluation (smaller = higher); the first matching rule wins.
    /// Replies with
    /// [`TrameEvent::CondFormatsUpdated`](crate::ui::TrameEvent::CondFormatsUpdated).
    AddCondFormat {
        sheet_id: String,
        range: GridRange,
        rule: CondFormatKind,
        style: CondStyle,
        priority: i64,
    },

    /// Replace the rule identified by `id` on `sheet_id` (`§3`). A no-op if no
    /// rule with that id exists.
    UpdateCondFormat {
        sheet_id: String,
        id: String,
        range: GridRange,
        rule: CondFormatKind,
        style: CondStyle,
        priority: i64,
    },

    /// Remove the conditional-formatting rule `id` from `sheet_id` (`§3`). A
    /// no-op if it does not exist.
    RemoveCondFormat {
        sheet_id: String,
        id: String,
    },

    /// Attach a data-validation rule to `range` of `sheet_id`
    /// (`ambre-spec-trame-features.md §5`). Stored in `validations_{sheet}`,
    /// checked before a local edit commits: `Reject` blocks the mutation,
    /// `Warn` lets it through with a warning, `Info` is a visual indicator only.
    /// Validations are an input aid, not a CRDT integrity barrier (a remote
    /// delta can still violate them).
    AddValidation {
        sheet_id: String,
        range: GridRange,
        kind: ValidationKind,
        on_invalid: ValidationOnInvalid,
        message: String,
    },

    /// Remove the validation rule covering `range` on `sheet_id` (`§5`). A no-op
    /// if none is attached there.
    RemoveValidation {
        sheet_id: String,
        range: GridRange,
    },

    /// Protect `range` of `sheet_id` so only `allowed_user_ids` (plus the owner)
    /// may edit it (`ambre-spec-trame-features.md §6`, Post-MVP). Soft
    /// client-side enforcement: an official client respects it, the Blind Relay
    /// cannot. Empty `allowed_user_ids` means owner-only. Identified by stable
    /// [`UserId`](crate::UserId), not `pub_key` (survives key rotation).
    SetProtection {
        sheet_id: String,
        range: GridRange,
        allowed_user_ids: Vec<UserId>,
        description: String,
    },

    /// Remove the protection covering `range` on `sheet_id` (`§6`). A no-op if
    /// none is set there.
    RemoveProtection {
        sheet_id: String,
        range: GridRange,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared filter — definition (UI → Logic)
// ─────────────────────────────────────────────────────────────────────────────

/// How a shared filter decides whether a row's value in the filtered column is
/// kept (visible) or hidden (`ambre-spec-trame-features.md` §1.A). MVP covers
/// the "by values" and "by condition" modes; colour filters are Phase 2.
#[derive(Debug, Clone)]
pub enum FilterKind {
    /// Keep only rows whose filtered-column display text is one of these values
    /// (Google Sheets "filter by values": the listed values are the *checked*,
    /// i.e. visible, ones).
    Values(Vec<String>),
    /// Keep only rows whose filtered-column value satisfies the condition.
    Condition {
        op: FilterOp,
        /// Comparison operand (parsed as a number when both sides are numeric,
        /// compared as text otherwise). Ignored by the unary operators
        /// ([`IsEmpty`](FilterOp::IsEmpty) / [`IsNotEmpty`](FilterOp::IsNotEmpty)).
        value: String,
    },
}

/// Condition operators for a [`FilterKind::Condition`]
/// (`ambre-spec-trame-features.md` §1.A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
    Contains,
    NotContains,
    StartsWith,
    IsEmpty,
    IsNotEmpty,
}

// ─────────────────────────────────────────────────────────────────────────────
// Manual cell formatting (`ambre-spec-trame-ui.md` sec.5bis)
//
// The toolbar's typed style vocabulary. `CellStyle` is the resolved formatting of
// a single cell (the toolbar reflects the active cell's); `StylePatch` is the
// sparse delta a control emits over the selection (`SetCellStyle`). Both travel
// by move inside `AppCommand::Trame` / `AppEvent::Trame` -- no `serde`, like the
// rest of the data plane. Colours are packed `0xRRGGBB`.
// ─────────────────────────────────────────────────────────────────────────────

/// Horizontal text alignment of a cell (`-ui.md` sec.5bis). [`Auto`](Self::Auto)
/// is the spreadsheet default: numbers align right, everything else left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HAlign {
    #[default]
    Auto,
    Left,
    Center,
    Right,
}

/// Vertical text alignment of a cell (`-ui.md` sec.5bis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// A cell number-format family (`-ui.md` sec.5bis). The number of decimal places
/// rides alongside in [`CellStyle::decimals`]; [`Custom`](Self::Custom) carries
/// its own pattern string (applied verbatim by the Logic thread).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CellFormat {
    /// No reformatting: the raw value / formula result is shown as-is.
    #[default]
    General,
    Number,
    Currency,
    Percent,
    Scientific,
    Fraction,
    /// Date (ISO `YYYY-MM-DD`) -- the numeric value is a day serial (MVP stub
    /// keeps the raw text when it is not numeric).
    Date,
    /// Time (`HH:MM:SS`).
    Time,
    /// Force textual interpretation (no numeric coercion).
    Text,
    Custom(String),
}

impl CellFormat {
    /// Whether this format honours a decimal-places count (i.e. the
    /// Increase/Decrease-decimals controls are meaningful). Only the numeric
    /// families render with [`CellStyle::decimals`]; `General`, `Fraction`,
    /// `Date`, `Time`, `Text` and `Custom` ignore it (see `format_number`).
    pub fn has_decimals(&self) -> bool {
        matches!(
            self,
            CellFormat::Number
                | CellFormat::Currency
                | CellFormat::Percent
                | CellFormat::Scientific
        )
    }
}

/// A border preset naming a set of edges within the selection (`-ui.md`
/// sec.5bis). Resolved per cell (merge-aware) by the Logic thread into the
/// concrete edges of [`Borders`] the chosen stroke is applied to. Every preset
/// except [`BorderPreset::None`] is additive: it sets only its own edges and
/// leaves the others untouched; `None` clears all four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderPreset {
    #[default]
    None,
    /// Border around the outside of the selection rectangle only.
    Outline,
    /// Border on every cell edge in the selection.
    All,
    /// All interior edges of the selection (between adjacent selected cells).
    Inner,
    /// Interior horizontal edges only (between stacked rows).
    InnerHorizontal,
    /// Interior vertical edges only (between side-by-side columns).
    InnerVertical,
    Top,
    Bottom,
    Left,
    Right,
}

/// Border edge bits for [`CellStyle::border_edges`] / [`DisplayCell::border_edges`]
/// (`-ui.md` sec.5bis). A [`BorderPreset`] applied over a selection is resolved by
/// the Logic thread into the per-cell set of edges these bits represent, so a
/// selection-level "Outline" lands only on the perimeter cells, not the interior.
pub const BORDER_EDGE_TOP: u8 = 1 << 0;
pub const BORDER_EDGE_RIGHT: u8 = 1 << 1;
pub const BORDER_EDGE_BOTTOM: u8 = 1 << 2;
pub const BORDER_EDGE_LEFT: u8 = 1 << 3;
/// All four edges (the `All` preset, per cell).
pub const BORDER_EDGE_ALL: u8 =
    BORDER_EDGE_TOP | BORDER_EDGE_RIGHT | BORDER_EDGE_BOTTOM | BORDER_EDGE_LEFT;

/// The stroke style of a cell border, conflating line pattern and weight into a
/// single choice (`-ui.md` sec.5bis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BorderStyle {
    /// A solid line border.
    #[default]
    Solid,
    /// A dashed line border.
    Dashed,
    /// A dotted line border.
    Dotted,
    /// A thicker line border.
    Medium,
    /// A very thick line border.
    Thick,
    /// A double line border.
    Double,
}

/// A single cell edge's border: its stroke style and colour. The edge's
/// presence is carried by the enclosing `Option<CellBorder>` in [`Borders`]
/// (`None` = no border on that edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CellBorder {
    pub style: BorderStyle,
    /// Border colour, packed `0xRRGGBB`. `None` = the default grid-line colour.
    pub color: Option<u32>,
}

/// The four independent edges of a cell border (`-ui.md` sec.5bis). Each edge
/// carries its own stroke style and colour, so a cell can mix, e.g., a thick
/// red top with a dotted blue left. `None` on an edge means no border there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Borders {
    pub top: Option<CellBorder>,
    pub right: Option<CellBorder>,
    pub bottom: Option<CellBorder>,
    pub left: Option<CellBorder>,
}

/// The resolved manual formatting of a single cell, the value the toolbar
/// reflects (`-ui.md` sec.5bis). Bold/italic are tracked by their dedicated
/// toggle commands and reported separately ([`TrameEvent::StyleApplied`]); the
/// underline run lives here with the rest of the patchable attributes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CellStyle {
    pub underline: bool,
    /// Strikethrough run drawn through the glyphs (`-ui.md` sec.5bis).
    pub strikethrough: bool,
    pub h_align: HAlign,
    pub v_align: VAlign,
    pub wrap: bool,
    /// Text colour, packed `0xRRGGBB`. `None` = theme default (adapts to the
    /// light/dark theme).
    pub fg: Option<u32>,
    /// Fill colour, packed `0xRRGGBB`. `None` = transparent (shows the themed
    /// sheet background).
    pub bg: Option<u32>,
    /// Font family name. `None` = the grid default font.
    pub font: Option<String>,
    /// Font size in points. `None` = the grid default size.
    pub size: Option<u16>,
    pub format: CellFormat,
    /// Decimal places for the active number format. `None` = the format default.
    pub decimals: Option<u8>,
    /// Per-edge border (style + colour each), resolved per cell by the Logic
    /// thread from the selection-level [`StylePatch::border`] edit.
    pub borders: Borders,
}

/// A sparse formatting delta emitted by a toolbar control over the current
/// selection ([`TrameCommand::SetCellStyle`], `-ui.md` sec.5bis). Every `None`
/// field leaves the existing cell value intact; a `Some` overwrites it. The
/// nested `Option<Option<_>>` colour/font/size fields distinguish "leave alone"
/// (`None`) from "clear" (`Some(None)`) and "set" (`Some(Some(v))`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StylePatch {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
    pub h_align: Option<HAlign>,
    pub v_align: Option<VAlign>,
    pub wrap: Option<bool>,
    pub fg: Option<Option<u32>>,
    pub bg: Option<Option<u32>>,
    pub font: Option<Option<String>>,
    pub size: Option<Option<u16>>,
    pub format: Option<CellFormat>,
    pub decimals: Option<Option<u8>>,
    pub border: Option<BorderEdit>,
}

/// A border edit emitted by the toolbar's borders popover: apply the chosen
/// stroke `style` and `color` to the edges named by `preset` (or, for
/// [`BorderPreset::None`], clear every edge). Resolved per cell, merge-aware,
/// by the Logic thread (`-ui.md` sec.5bis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BorderEdit {
    pub preset: BorderPreset,
    pub style: BorderStyle,
    /// Border colour, packed `0xRRGGBB`. `None` = the default grid-line colour.
    pub color: Option<u32>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Clipboard (`ambre-spec-trame-ui.md` sec.5.E)
//
// The serialized payload of a copied/cut range. Built by the UI from its
// viewport cache, written onto the OS clipboard (postcard, alongside a
// tab-separated text fallback for cross-app interop), read back on paste, and
// moved to the Logic thread inside [`TrameCommand::PasteRange`]. It derives
// `serde` (unlike the rest of the data plane) precisely so the UI can postcard
// it onto the clipboard.
// ─────────────────────────────────────────────────────────────────────────────

/// A copied/cut rectangular range: its source origin (top-left absolute grid
/// position, for relative-formula reindexing) plus its non-empty cells, each
/// keyed by an offset from that origin.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipboardPayload {
    /// Source range top-left absolute row (0-based). The paste reindex shift is
    /// `target_row - src_row`.
    pub src_row: u32,
    /// Source range top-left absolute column (0-based).
    pub src_col: u32,
    /// Source range height in rows (so a cut can clear the whole rectangle and
    /// the renderer can draw marching-ants over it).
    pub rows: u32,
    /// Source range width in columns.
    pub cols: u32,
    /// The range's non-empty cells. Blank cells are omitted; the paste leaves
    /// the corresponding destination cells untouched (Excel/Sheets behaviour).
    pub cells: Vec<ClipboardCell>,
}

impl ClipboardPayload {
    /// Lay the payload out as tab-separated rows (`\t` between columns, `\n`
    /// between rows), the lingua-franca a spreadsheet writes to the OS clipboard
    /// for cross-application paste. Cells are placed by their `(dr, dc)` offset;
    /// gaps (blank source cells, omitted from `cells`) render as empty fields. A
    /// formula cell contributes its raw text (e.g. `=A1*2`).
    pub fn to_tsv(&self) -> String {
        let mut grid = vec![vec![String::new(); self.cols.max(1) as usize]; self.rows.max(1) as usize];
        for cell in &self.cells {
            if let Some(row) = grid.get_mut(cell.dr as usize)
                && let Some(slot) = row.get_mut(cell.dc as usize)
            {
                *slot = cell.raw.clone();
            }
        }
        grid.into_iter().map(|r| r.join("\t")).collect::<Vec<_>>().join("\n")
    }
}

/// One cell of a [`ClipboardPayload`], positioned by its offset from the source
/// origin and carrying its raw text (formula source for a formula cell) plus its
/// full formatting.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipboardCell {
    /// Row offset from [`ClipboardPayload::src_row`].
    pub dr: u32,
    /// Column offset from [`ClipboardPayload::src_col`].
    pub dc: u32,
    /// Raw cell text (e.g. `=SUM(A1:B1)` for a formula), in display form. A
    /// formula's relative references are reindexed by the Logic thread on paste.
    pub raw: String,
    pub bold: bool,
    pub italic: bool,
    /// The cell's resolved rich formatting, written verbatim at the destination.
    pub style: CellStyle,
}

// ─────────────────────────────────────────────────────────────────────────────
// Deferred feature payloads (Phase 2+)
//
// Typed value types carried by the deferred `TrameCommand` variants above. They
// live in `ambre-bus` (re-exported by `ambre-trame`) and travel by move inside
// `AppCommand::Trame` -- no `serde`. Ranges are expressed in **grid
// coordinates** here because the engine-level `RangeDep` (`ambre-trame`) is not
// visible from the bus; the Logic thread resolves these positions to stable ids
// when each feature's handler lands.
// ─────────────────────────────────────────────────────────────────────────────

/// A rectangular grid range `[row..=row_end] x [col..=col_end]` (0-based,
/// inclusive). Corners may be given in any order; the handler normalizes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridRange {
    pub row: u32,
    pub col: u32,
    pub row_end: u32,
    pub col_end: u32,
}

/// How [`TrameCommand::FillRange`] fills `dest \ src` (`-ui.md` sec.5.F).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillMode {
    /// The Logic thread picks `Series` when a fill line's source values form an
    /// arithmetic numeric progression, `Copy` otherwise. The fill handle default.
    #[default]
    Auto,
    /// Tile the source values verbatim across the fill region (relative formula
    /// references reindex by the tile offset).
    Copy,
    /// Extrapolate a numeric arithmetic progression from the source values; a
    /// line whose source is not numeric falls back to `Copy`.
    Series,
}

/// One key of a multi-column sort ([`TrameCommand::SortRangeMulti`],
/// `ambre-spec-trame-features.md §2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    /// Column to sort on, as an index within the range (`0` = `col_start`).
    pub col: u32,
    pub ascending: bool,
}

/// A conditional-formatting rule predicate (`ambre-spec-trame-features.md §3`).
/// `String` operands are either a literal value or a cell reference, resolved at
/// evaluation. MVP+ kinds (value/text/formula/presets) land in Phase 2; the
/// gradient / data-bar / top-bottom kinds are Phase 3.
#[derive(Debug, Clone)]
pub enum CondFormatKind {
    GreaterThan(String),
    LessThan(String),
    Between(String, String),
    Equal(String),
    Contains(String),
    StartsWith(String),
    /// Custom formula, e.g. `"=A1>AVERAGE(A:A)"` (relative refs resolved from
    /// the target cell).
    Formula(String),
    IsEmpty,
    IsNotEmpty,
    IsError,
    ColorScale2 {
        min_color: u32,
        max_color: u32,
    },
    ColorScale3 {
        min_color: u32,
        mid_color: u32,
        max_color: u32,
    },
    DataBar {
        color: u32,
    },
    TopN(u32),
    BottomN(u32),
    TopNPercent(f32),
}

/// Style applied when a conditional-formatting rule matches (`§3`). Layered on
/// top of the base style; absent fields leave the base intact. Colours are
/// packed `0xRRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CondStyle {
    pub fg: Option<u32>,
    pub bg: u32,
    pub bold: bool,
}

/// A data-validation predicate (`ambre-spec-trame-features.md §5`). `List` /
/// `NumberRange` / `TextLength` are Phase 2; `ListRange` / `DateRange` / `Regex`
/// are Phase 3.
#[derive(Debug, Clone)]
pub enum ValidationKind {
    /// Dropdown over a fixed value set.
    List(Vec<String>),
    /// Dropdown over the values of a range, given as an A1 reference (the
    /// engine-level `RangeDep` is not visible from the bus).
    ListRange(String),
    NumberRange {
        min: f64,
        max: f64,
        inclusive: bool,
    },
    NumberInteger,
    DateRange {
        min: Option<String>,
        max: Option<String>,
    },
    TextLength {
        min: Option<u32>,
        max: Option<u32>,
    },
    Regex(String),
    /// Custom formula that must return `TRUE`.
    Formula(String),
}

/// What happens when a [`ValidationKind`] rejects an input (`§5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationOnInvalid {
    /// Block the local mutation -> [`TrameEvent::ValidationError`](crate::ui::TrameEvent::ValidationError).
    Reject,
    /// Allow the mutation but warn -> [`TrameEvent::ValidationWarning`](crate::ui::TrameEvent::ValidationWarning).
    Warn,
    /// Visual indicator only.
    Info,
}
