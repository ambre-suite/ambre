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
use crate::{ChannelId, DocumentId, MeetingId, RequestId, RestEvent, UserId};

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

    /// Bold formatting state of the **active cell**, emitted after a
    /// [`ToggleBold`] or [`SetActiveCell`] command. Drives the chrome's "Bold"
    /// toggle button (which always reflects the current selection).
    ///
    /// [`ToggleBold`]: crate::logic::TrameCommand::ToggleBold
    /// [`SetActiveCell`]: crate::logic::TrameCommand::SetActiveCell
    BoldState {
        enabled: bool,
    },

    /// Italic formatting state of the **active cell**, emitted after a
    /// [`ToggleItalic`] or [`SetActiveCell`] command. Drives the chrome's
    /// "Italic" toggle button.
    ///
    /// [`ToggleItalic`]: crate::logic::TrameCommand::ToggleItalic
    /// [`SetActiveCell`]: crate::logic::TrameCommand::SetActiveCell
    ItalicState {
        enabled: bool,
    },

    /// A single cell's formatting changed (after a toggle). The UI Thread
    /// updates that cell in its cache and repaints, so the bold/italic run stays
    /// pinned to the cell it was applied to -- independent of where the cursor
    /// moves next. Carries the full per-cell flags (not a delta) so it is
    /// order-independent.
    CellFormat {
        row: u32,
        col: u32,
        bold: bool,
        italic: bool,
    },

    /// The document's content was replaced wholesale (local snapshot restore at
    /// startup). The UI Thread re-requests the visible region: the initial
    /// viewport response predated the restore, so the cache must be refreshed.
    DataReset,
}

/// A single cell ready for display.
///
/// Flat, data-oriented. The UI Thread stores these in a
/// `HashMap<(u32, u32), DisplayCell>` keyed by `(row, col)`; missing entries
/// are blank (white background, no text), so only non-empty cells are sent.
#[derive(Debug, Clone)]
pub struct DisplayCell {
    pub row: u32,
    pub col: u32,
    /// Display text (formula result or raw string).
    pub text: String,
    /// Bold run applied to this cell (pinned to the cell, not the selection).
    pub bold: bool,
    /// Italic run applied to this cell.
    pub italic: bool,
}
