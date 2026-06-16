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
    /// Formatting commands ([`ToggleBold`](TrameCommand::ToggleBold) /
    /// [`ToggleItalic`](TrameCommand::ToggleItalic)) target whatever cell is
    /// active in the Logic thread, so the UI Thread must keep it informed when
    /// the cursor moves. The Logic thread replies with the active cell's current
    /// formatting ([`TrameEvent::BoldState`](crate::ui::TrameEvent::BoldState) /
    /// [`ItalicState`](crate::ui::TrameEvent::ItalicState)) so the chrome toggle
    /// buttons track the selection.
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

    /// Toggle bold formatting for the active cell (set by
    /// [`SetActiveCell`](TrameCommand::SetActiveCell)).
    ToggleBold,

    /// Toggle italic formatting for the active cell (set by
    /// [`SetActiveCell`](TrameCommand::SetActiveCell)).
    ToggleItalic,

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
