//! UI-bound protocol — messages addressed to the UI Thread from the Logic thread.
//!
//! Holds the [`UiEvent`] envelope (Logic thread → UI Thread) plus the typed
//! application **event** data plane it carries: [`AppEvent`] (a closed sum over
//! the document types), the per-app event enums (`TrameEvent`, …) and their
//! display types (`DisplayCell`).
//!
//! Carried over a `flume` channel in shared memory — typed Rust values by move,
//! no serialization. The symmetric UI → Logic direction (commands) lives in
//! [`crate::logic`].
//!
//! # Three planes, one envelope
//! - **Content** ([`UiEvent::Viewport`] / [`UiEvent::Presence`]): typed
//!   [`AppEvent`]s computed by the Logic thread, routed by `DocumentId`.
//! - **Service** ([`UiEvent::Rest`]): a [`RestEvent`] response forwarded from
//!   [`crate::LogicEvent::Rest`], echoing the originating [`RequestId`].
//! - **Push & status** (`ConnectionStatus`, `SyncWarning`, `MemberRevoked`,
//!   channel/meeting push, `AuthError`, `ApiError`, `FatalError`): unsolicited.

use crate::sync_warning::SyncWarningCode;
use crate::{ChannelId, DocType, DocumentId, MeetingId, RequestId, RestEvent, UserId};

// ─────────────────────────────────────────────────────────────────────────────
// UiEvent — Logic thread → UI Thread
// ─────────────────────────────────────────────────────────────────────────────

/// Message sent from the Logic thread to the UI Thread.
#[derive(Debug, Clone)]
pub enum UiEvent {
    // ── Content plane ─────────────────────────────────────────────────────────
    /// Display event produced in response to an [`crate::logic::AppCommand`]
    /// (e.g. a viewport scroll) or a spontaneous refresh after a remote CRDT
    /// update. The UI Thread routes [`event`](AppEvent) to the view owning
    /// `document_id`.
    Viewport {
        document_id: DocumentId,
        event: AppEvent,
    },

    /// Decrypted collaborator presence event (cursor position, selection, …).
    Presence {
        document_id: DocumentId,
        event: AppEvent,
    },

    // ── Service plane (relayed request/response) ──────────────────────────────
    /// A [`RestEvent`] response, echoing the originating [`RequestId`] so the UI
    /// can route it to the call-site that issued the [`crate::RestCommand`].
    Rest {
        id: RequestId,
        evt: RestEvent,
    },

    // ── Connection / status (server push) ─────────────────────────────────────
    /// WebSocket connection status for a document (online / offline indicator).
    ConnectionStatus {
        document_id: DocumentId,
        connected: bool,
    },

    /// Non-fatal sync warning (quarantine overflow, decrypt failure, …).
    SyncWarning {
        document_id: DocumentId,
        code: SyncWarningCode,
        message: String,
    },

    /// A member was revoked and key rotation is complete (server-initiated).
    MemberRevoked {
        document_id: DocumentId,
        revoked_user_id: UserId,
    },

    /// A document was just shared with this user (server-initiated). The UI
    /// refreshes its document list so the new document appears without a reload,
    /// and notifies the user.
    DocumentShared {
        document_id: DocumentId,
        doc_type: DocType,
    },

    // ── Channels (Murmure) — real-time push ───────────────────────────────────
    /// Inbound channel message (decrypted by the Sync thread).
    ///
    /// `payload` is an opaque application byte blob (schema owned by
    /// `ambre-murmure`).
    ChannelMessage {
        channel_id: ChannelId,
        payload: Vec<u8>,
    },

    /// Inbound ephemeral channel event (typing indicator, presence, …).
    ChannelPresenceUpdate {
        channel_id: ChannelId,
        payload: Vec<u8>,
    },

    /// WebSocket connection status for a channel.
    ChannelConnectionStatus {
        channel_id: ChannelId,
        connected: bool,
    },

    // ── Meetings (Rendez-Vous) — participant push ─────────────────────────────
    /// A participant joined the meeting.
    MeetingParticipantJoined {
        meeting_id: MeetingId,
        user_id: UserId,
    },

    /// A participant left the meeting.
    MeetingParticipantLeft {
        meeting_id: MeetingId,
        user_id: UserId,
    },

    // ── Errors & warnings ─────────────────────────────────────────────────────
    /// Authentication error (wrong passphrase, corrupted vault, network failure).
    AuthError {
        message: String,
    },

    /// REST or WebSocket operation error.
    ///
    /// `request` identifies the originating command so the UI Thread can
    /// correlate the error to the triggering action.
    ApiError {
        request: String,
        code: u16,
        message: String,
    },

    /// Unrecoverable failure in a worker thread — the application cannot
    /// continue and the UI should display a terminal error screen.
    ///
    /// This is the **graceful** path: a thread that detects an unrecoverable
    /// condition emits this before exiting. The **ungraceful** path (a panic,
    /// which is `panic=abort` under multi-threaded WASM) is observed instead as
    /// a channel *disconnect* on the UI Thread's inbound receiver — the UI
    /// treats that identically to a `FatalError`. See the "thread failure"
    /// section of `ambre-spec-architecture`.
    FatalError {
        message: String,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// AppEvent — typed application event family (content plane)
// ─────────────────────────────────────────────────────────────────────────────

/// Application-specific event, Logic thread → UI Thread.
///
/// Carried inside [`UiEvent::Viewport`] / [`UiEvent::Presence`]. The
/// `Command` / `Event` suffix marks the direction at both the envelope and the
/// payload level: UI→Logic is always a `Command` (see
/// [`AppCommand`](crate::logic::AppCommand)), Logic→UI always an `Event`.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Display event for a Trame (spreadsheet) document.
    Trame(TrameEvent),
}

// ─────────────────────────────────────────────────────────────────────────────
// Trame (spreadsheet) — events (Logic → UI)
// ─────────────────────────────────────────────────────────────────────────────

/// Display events emitted by the Trame logic to the UI Thread.
#[derive(Debug, Clone)]
pub enum TrameEvent {
    /// Cell content for a requested viewport region.
    ///
    /// `row_start` / `col_start` are the top-left corner of the batch; `cells`
    /// holds all non-empty cells within the requested range. The UI Thread
    /// merges this batch into its local cache. `version` echoes the originating
    /// [`TrameCommand::ScrollViewport`](crate::logic::TrameCommand::ScrollViewport)
    /// so stale responses can be dropped.
    ViewportData {
        version: u64,
        row_start: u32,
        col_start: u32,
        cells: Vec<DisplayCell>,
    },

    /// The resolved formatting of the **active cell** after a
    /// [`SetCellStyle`](crate::logic::TrameCommand::SetCellStyle) or a
    /// [`SetActiveCell`](crate::logic::TrameCommand::SetActiveCell), driving the
    /// whole formatting toolbar (bold/italic/underline toggles, font/size selects,
    /// colour swatches, align buttons, cell-format / border menus, wrap). Bold and
    /// italic ride alongside the [`CellStyle`] -- they are now part of the single
    /// style channel, not a separate per-flag event (`-ui.md` sec.5bis).
    ///
    /// [`CellStyle`]: crate::logic::CellStyle
    StyleApplied {
        style: crate::logic::CellStyle,
        bold: bool,
        italic: bool,
        /// Whether the active cell is part of a merge (a master or a covered
        /// cell), driving the toolbar's merge/unmerge toggle pressed state.
        merged: bool,
    },

    /// The document's content was replaced wholesale (local snapshot restore at
    /// startup). The UI Thread re-requests the visible region: the initial
    /// viewport response predated the restore, so the cache must be refreshed.
    DataReset,

    /// A local [`MergeCells`](crate::logic::TrameCommand::MergeCells) was refused
    /// because a covered cell is non-empty (`ambre-spec-trame-logic.md` §6). The
    /// chrome surfaces `reason` to the user; no CRDT mutation was produced.
    MergeRejected {
        reason: String,
    },

    /// A merge ended up covering a non-empty cell after a concurrent edit
    /// converged (§6). The covered value is masked (not destroyed) while the
    /// merge is active; it reappears on unmerge. Purely informational for the UI.
    MergeConflictDetected {
        master_row: u32,
        master_col: u32,
        covered_row: u32,
        covered_col: u32,
    },

    /// Availability of undo/redo after a local
    /// [`Undo`](crate::logic::TrameCommand::Undo) /
    /// [`Redo`](crate::logic::TrameCommand::Redo). Drives the enabled state of the
    /// chrome's Undo/Redo affordances. Reflects Loro's `UndoManager` stacks, whose
    /// scope is the current document session.
    UndoState {
        can_undo: bool,
        can_redo: bool,
    },

    /// The workbook's sheets, in tab-strip order: `(sheet_id, display_name)`.
    /// Emitted after any sheet-lifecycle command
    /// ([`CreateSheet`](crate::logic::TrameCommand::CreateSheet) /
    /// [`RenameSheet`](crate::logic::TrameCommand::RenameSheet) /
    /// [`MoveSheet`](crate::logic::TrameCommand::MoveSheet) /
    /// [`DeleteSheet`](crate::logic::TrameCommand::DeleteSheet)) and on load, so
    /// the tab strip stays in sync (`ambre-spec-trame-logic.md` §2.D). `active`
    /// is the id of the sheet currently shown.
    SheetList {
        sheets: Vec<(String, String)>,
        active: String,
    },

    /// A command was rejected for a reason the user should see (e.g. deleting the
    /// last remaining sheet). Purely informational; no CRDT mutation occurred.
    Error {
        message: String,
    },

    /// A local [`SortRange`](crate::logic::TrameCommand::SortRange) finished
    /// reordering the rows (`ambre-spec-trame-features.md` §3). The accompanying
    /// [`DataReset`](Self::DataReset) prompts the UI to re-fetch the viewport with
    /// the new ordering; this event is the explicit completion signal (e.g. to
    /// clear a "sorting..." affordance). A no-op sort emits nothing.
    SortComplete,

    /// A local [`ApplyFilter`](crate::logic::TrameCommand::ApplyFilter) finished
    /// recomputing the hidden rows (`ambre-spec-trame-features.md` §1.A).
    /// `hidden_row_count` is how many rows the filter hides. The accompanying
    /// [`DataReset`](Self::DataReset) prompts the UI to re-fetch the viewport,
    /// where the hidden rows now carry [`DisplayCell::is_filtered`]. A no-op
    /// filter (non-active sheet / virtual column) emits nothing. `col` is the
    /// grid column the filter targets, so the UI can draw the funnel badge on its
    /// header (`-ui.md` sec.5.D).
    FilterApplied {
        hidden_row_count: u32,
        col: u32,
    },

    /// The distinct visible values of a column, replying to
    /// [`RequestActiveColumnValues`](crate::logic::TrameCommand::RequestActiveColumnValues).
    /// Feeds the shared filter popover's "by values" checkbox list (`-ui.md`
    /// sec.5.D). `col` is the grid column the values belong to; `values` is the
    /// deduplicated, sorted display text of every non-hidden row.
    ColumnValues {
        col: u32,
        values: Vec<String>,
    },

    /// A local [`ClearFilter`](crate::logic::TrameCommand::ClearFilter) removed the
    /// active shared filter (`ambre-spec-trame-features.md` §1.A). The accompanying
    /// [`DataReset`](Self::DataReset) prompts a viewport re-fetch with every row
    /// visible again. A no-op clear (no active filter) emits nothing.
    FilterCleared,

    // ─────────────────────────────────────────────────────────────────────────
    // Deferred feature surface (Phase 2+)
    //
    // Completes the `TrameEvent` surface mandated by
    // `ambre-spec-trame-features.md §7 / §10` ("all feature variants are in the
    // enums"). Emitted once the matching command handlers land (personal filter
    // views / conditional formatting -> Phase 2/3, validation -> Phase 2/3,
    // range protection -> Post-MVP).
    // ─────────────────────────────────────────────────────────────────────────

    /// A personal filter view was persisted to local storage
    /// ([`SaveFilterView`](crate::logic::TrameCommand::SaveFilterView),
    /// `ambre-spec-trame-features.md §1.B`). `view_id` is its stable local id.
    FilterViewSaved {
        view_id: String,
    },

    /// The sheet's conditional-formatting rules changed
    /// ([`AddCondFormat`](crate::logic::TrameCommand::AddCondFormat) /
    /// [`UpdateCondFormat`](crate::logic::TrameCommand::UpdateCondFormat) /
    /// [`RemoveCondFormat`](crate::logic::TrameCommand::RemoveCondFormat), §3).
    /// The accompanying [`DataReset`](Self::DataReset) prompts a viewport
    /// re-fetch with the new style overrides applied.
    CondFormatsUpdated {
        version: u64,
    },

    /// A local edit was rejected by a [`ValidationOnInvalid::Reject`] rule
    /// (`ambre-spec-trame-features.md §5`). No CRDT delta was produced; the
    /// chrome surfaces `message`. `version` echoes the originating edit.
    ///
    /// [`ValidationOnInvalid::Reject`]: crate::logic::ValidationOnInvalid::Reject
    ValidationError {
        version: u64,
        row: u32,
        col: u32,
        message: String,
    },

    /// A local edit passed through despite failing a
    /// [`ValidationOnInvalid::Warn`] rule (`§5`). The mutation was applied; the
    /// chrome surfaces `message` as a non-blocking warning.
    ///
    /// [`ValidationOnInvalid::Warn`]: crate::logic::ValidationOnInvalid::Warn
    ValidationWarning {
        version: u64,
        row: u32,
        col: u32,
        message: String,
    },

    /// A local edit was blocked by a range protection the current user may not
    /// edit (`ambre-spec-trame-features.md §6`, Post-MVP). No CRDT delta was
    /// produced; the chrome surfaces `description`.
    ProtectionDenied {
        version: u64,
        row: u32,
        col: u32,
        description: String,
    },

    /// A remote delta edited a protected range its author was not allowed to
    /// (`§6`). The delta IS applied (CRDT convergence is mandatory) but the UI
    /// is warned. `from_user_id` is the offending author's stable
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
    /// Raw cell text as stored in the CRDT (e.g. `=SUM(A1:B1)` for a formula).
    /// Used to seed the in-cell editor so editing a formula shows the formula,
    /// not its computed result. Equals `text` for plain cells.
    pub raw: String,
    /// Bold run applied to this cell (pinned to the cell, not the selection).
    pub bold: bool,
    /// Italic run applied to this cell.
    pub italic: bool,
    /// Underline run applied to this cell (`-ui.md` sec.5bis).
    pub underline: bool,
    /// Strikethrough run applied to this cell.
    pub strikethrough: bool,
    /// Horizontal text alignment.
    pub h_align: crate::logic::HAlign,
    /// Vertical text alignment.
    pub v_align: crate::logic::VAlign,
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
    pub borders: crate::logic::Borders,
    /// Part of a merged range (`ambre-spec-trame-logic.md` §6). `true` for both
    /// the master cell and the cells it covers; the UI distinguishes them via
    /// [`merge_span`](DisplayCell::merge_span).
    pub is_merged: bool,
    /// `Some((col_span, row_span))` on the master (top-left) cell of a merge,
    /// giving how many columns/rows it spans. `None` on a covered cell (rendered
    /// blank, swallowed by the master) and on any non-merged cell.
    pub merge_span: Option<(u32, u32)>,
    /// The cell's row is hidden by the active shared filter
    /// (`ambre-spec-trame-features.md` §1.A / §8). The UI Thread collapses such
    /// rows; the value is still present (and still counted by `=SUM`, etc.).
    pub is_filtered: bool,
}
