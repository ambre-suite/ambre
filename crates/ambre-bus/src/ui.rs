//! UI-bound protocol -- messages addressed to the UI Thread from the Logic thread.
//!
//! Holds the [`UiEvent`] envelope (Logic thread -> UI Thread) plus [`AppEvent`],
//! the closed sum over the document types. Each per-app event enum and its
//! display types live with that app's whole data plane under
//! [`crate::app`], such as `crate::app::trame::TrameEvent` and `DisplayCell`.
//! A document type is therefore defined in one place, whichever direction the
//! message travels.
//!
//! Carried over a `conduit` queue in shared memory -- typed Rust values by move,
//! no serialization. The symmetric UI -> Logic direction (commands) lives in
//! [`crate::logic`].
//!
//! # Three planes, one envelope
//! - **Content** ([`UiEvent::Viewport`] / [`UiEvent::Presence`]): typed
//!   [`AppEvent`]s computed by the Logic thread, routed by `DocumentId`.
//! - **Service** ([`UiEvent::Rest`]): a [`RestEvent`] response forwarded from
//!   [`crate::LogicEvent::Rest`], echoing the originating [`RequestId`].
//! - **Push & status** (`ConnectionStatus`, `SyncWarning`, `MemberRevoked`,
//!   channel/meeting push, `AuthError`, `ApiError`, `FatalError`, `SetupRequired`):
//!   unsolicited.

use crate::sync_warning::SyncWarningCode;
use crate::{ChannelId, DocType, DocumentId, MeetingId, RequestId, RestEvent, Role, UserId};

// -----------------------------------------------------------------------------
// UiEvent -- Logic thread -> UI Thread
// -----------------------------------------------------------------------------

/// Message sent from the Logic thread to the UI Thread.
#[derive(Debug, Clone)]
pub enum UiEvent {
    // -- Content plane ---------------------------------------------------------
    /// Display event produced in response to an [`crate::logic::AppCommand`]
    /// (for example a viewport scroll) or a spontaneous refresh after a remote CRDT
    /// update. The UI Thread routes [`event`](AppEvent) to the view owning
    /// `document_id`.
    Viewport {
        document_id: DocumentId,
        event: AppEvent,
    },

    /// Decrypted collaborator presence event (cursor position, selection, ...).
    Presence {
        document_id: DocumentId,
        event: AppEvent,
    },

    // -- Service plane (relayed request/response) ------------------------------
    /// A [`RestEvent`] response, echoing the originating [`RequestId`] so the UI
    /// can route it to the call-site that issued the [`crate::RestCommand`].
    Rest { id: RequestId, evt: RestEvent },

    // -- Connection / status (server push) -------------------------------------
    /// WebSocket connection status for a document (online / offline indicator).
    ConnectionStatus {
        document_id: DocumentId,
        connected: bool,
    },

    /// Initial hydration of a freshly opened document is complete (snapshot, if
    /// any, already applied). The UI dismisses its document-loading overlay.
    /// Fires once per open, including for empty/brand-new documents.
    /// Carries this client's own role so the UI can withhold editing
    /// affordances on a read-only document (`architecture`).
    DocumentReady { document_id: DocumentId, role: Role },

    /// Non-fatal sync warning (quarantine overflow, decrypt failure, ...).
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

    // -- Channels (Murmure) -- real-time push -----------------------------------
    /// Inbound channel message (decrypted by the Sync thread).
    ///
    /// `payload` is an opaque application byte blob (schema owned by
    /// `ambre-murmure`).
    ChannelMessage {
        channel_id: ChannelId,
        payload: Vec<u8>,
    },

    /// Inbound ephemeral channel event (typing indicator, presence, ...).
    ChannelPresenceUpdate {
        channel_id: ChannelId,
        payload: Vec<u8>,
    },

    /// WebSocket connection status for a channel.
    ChannelConnectionStatus {
        channel_id: ChannelId,
        connected: bool,
    },

    // -- Meetings (Rendez-Vous) -- participant push -----------------------------
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

    // -- Errors & warnings -----------------------------------------------------
    /// Authentication error (wrong passphrase, corrupted vault, network failure).
    AuthError { message: String },

    /// REST or WebSocket operation error.
    ///
    /// `request` identifies the originating command so the UI Thread can correlate the
    /// error to the triggering action. `id` echoes the originating [`RequestId`] when the
    /// failed command carried one, so a UI-issued request can be matched to its failure. It
    /// is `None` for a spontaneous error.
    ApiError {
        id: Option<RequestId>,
        request: String,
        code: u16,
        message: String,
    },

    /// Unrecoverable failure in a worker thread -- the application cannot continue and the
    /// UI should display a terminal error screen.
    ///
    /// This is the **graceful** path: a thread that detects an unrecoverable condition
    /// emits this before exiting. The **ungraceful** path is a panic, which is
    /// `panic=abort` under multi-threaded WASM, and it never reaches a receiver at all.
    /// Nothing is dropped, so the sender leaks in shared memory and the queue reads empty
    /// rather than disconnected. That path goes through the page's fatal hook instead
    /// (`ambre-spec-architecture.md`, `ARCH-008`).
    FatalError { message: String },

    /// The relay refused a request or the socket because the account still owes
    /// its key setup. It calls for the setup, not for an error screen. Nothing
    /// is retried until the keys are stored (`ambre-spec-sync.md`, `SYNC-134`).
    SetupRequired,
}

impl UiEvent {
    /// The `DocumentId` this event is addressed to, for the per-document fan-out a nested
    /// `doc_embed` needs (`architecture` / `toile-embeds`). A Toile embedding another
    /// document owns a child view, whose Logic->UI stream must be demultiplexed out of the
    /// single shared queue. Only the variants a *view* consumes carry a routing id.
    /// Service-plane and app-global events (`Rest`, `DocumentShared`, channel/meeting push,
    /// errors, `FatalError`) return `None` and stay on the top-level path.
    pub fn routed_document_id(&self) -> Option<DocumentId> {
        match self {
            UiEvent::Viewport { document_id, .. }
            | UiEvent::Presence { document_id, .. }
            | UiEvent::ConnectionStatus { document_id, .. }
            | UiEvent::DocumentReady { document_id, .. }
            | UiEvent::SyncWarning { document_id, .. }
            | UiEvent::MemberRevoked { document_id, .. } => Some(*document_id),
            _ => None,
        }
    }
}

// -----------------------------------------------------------------------------
// AppEvent -- typed application event family (content plane)
// -----------------------------------------------------------------------------

/// Application-specific event, Logic thread -> UI Thread.
///
/// Carried inside [`UiEvent::Viewport`] / [`UiEvent::Presence`]. The `Command` / `Event`
/// suffix marks the direction at both the envelope and the payload level. UI->Logic is
/// always a `Command` (see [`AppCommand`](crate::logic::AppCommand)), Logic->UI always an
/// `Event`.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Display event for a Trame (spreadsheet) document.
    Trame(crate::app::trame::TrameEvent),
    /// Display event for a Toile (whiteboard) document.
    Toile(crate::app::toile::ToileEvent),
    /// Display event for a Prose (word processor) document.
    Prose(crate::app::prose::ProseEvent),
}
