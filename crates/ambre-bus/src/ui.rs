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
//! # Network payloads stay bytes
//! Variants that carry `payload: Vec<u8>` (CRDT deltas, Murmure channel
//! messages) are genuine **network** bytes (Loro binary format, encrypted, sent
//! over the WebSocket), not application types.

use crate::event::{ChannelSummary, DocumentSummary, MemberInfo, UnreadCount, UserInfo};
use crate::{ChannelId, DocType, DocumentId, MeetingId, Role, UserId};

// ─────────────────────────────────────────────────────────────────────────────
// UiEvent — Logic thread → UI Thread
// ─────────────────────────────────────────────────────────────────────────────

/// Message sent from the Logic thread to the UI Thread.
///
/// The Logic thread produces these messages in two ways:
/// 1. **Forwarding**: a [`LogicEvent`] received from the Sync thread is mapped
///    1-to-1 to the equivalent `UiEvent` variant and forwarded to the UI.
/// 2. **Local computation**: `Viewport` and `Presence` are produced by the
///    Logic thread itself (after applying a CRDT delta or computing the visible
///    region) without a Sync thread round-trip.
///
/// [`LogicEvent`]: crate::LogicEvent
#[derive(Debug, Clone)]
pub enum UiEvent {
    // ── Viewport ──────────────────────────────────────────────────────────────

    /// Display event produced in response to an [`crate::logic::AppCommand`] (e.g. a viewport
    /// scroll) or a spontaneous refresh after a remote CRDT update.
    ///
    /// The UI Thread routes [`event`](AppEvent) to the view owning
    /// `document_id`.
    Viewport {
        document_id: DocumentId,
        /// Typed application display event (schema owned by [`crate::ui`]).
        event: AppEvent,
    },

    /// Decrypted collaborator presence event (cursor position, selection, …).
    ///
    /// Produced when the Logic thread applies a [`crate::event::LogicEvent::RemoteEphemeral`]
    /// through the document's state.
    Presence {
        document_id: DocumentId,
        /// Typed application presence event (schema owned by [`crate::ui`]).
        event: AppEvent,
    },

    // ── Session ───────────────────────────────────────────────────────────────

    /// Authentication succeeded — session is ready.
    ///
    /// The UI Thread may now display the document list and open documents.
    SessionReady {
        user_id: UserId,
        display_name: String,
        email: String,
    },

    /// First-time connection: no keys found on the server.
    ///
    /// The UI Thread must display the `SetupAccount` onboarding flow and send
    /// [`crate::logic::LogicCommand::SetupAccount`] once the user enters a passphrase.
    AccountSetupRequired,

    /// Account created successfully.
    ///
    /// The UI Thread must display `recovery_words` **exactly once**. These words
    /// cannot be retrieved later.
    AccountCreated {
        /// Space-separated 24-word BIP39 recovery phrase.
        recovery_words: String,
    },

    /// Passphrase changed successfully.
    PassphraseChanged,

    /// Account recovered via recovery phrase.
    AccountRecovered,

    /// User has been logged out. The UI Thread must clear all state.
    LoggedOut,

    // ── Document CRUD ─────────────────────────────────────────────────────────

    /// List of documents accessible to the authenticated user.
    DocumentList {
        documents: Vec<DocumentSummary>,
    },

    /// A new document was created and is ready to open.
    DocumentCreated {
        document_id: DocumentId,
        doc_type: DocType,
    },

    /// A document was permanently deleted.
    DocumentDeleted {
        document_id: DocumentId,
    },

    /// A document was renamed.
    DocumentRenamed {
        document_id: DocumentId,
    },

    // ── Sharing ───────────────────────────────────────────────────────────────

    /// Invitation sent successfully.
    InviteSent {
        document_id: DocumentId,
        user_id: UserId,
        display_name: String,
    },

    /// A member was revoked and key rotation is complete.
    ///
    /// Forwarded from [`crate::event::LogicEvent::MemberRevoked`] **after** the Sync thread
    /// has completed the full rotation (new keys in DB). The Logic thread may
    /// take a snapshot to seal the CRDT history at this boundary.
    MemberRevoked {
        document_id: DocumentId,
        revoked_user_id: UserId,
    },

    /// Revocation initiated by this client is complete.
    RevocationComplete {
        document_id: DocumentId,
    },

    /// Current member list of a document.
    DocumentMembers {
        document_id: DocumentId,
        members: Vec<MemberInfo>,
    },

    /// A member's role was updated.
    MemberRoleUpdated {
        document_id: DocumentId,
        user_id: UserId,
        role: Role,
    },

    // ── Users ─────────────────────────────────────────────────────────────────

    /// Results of a user search (for invitation autocomplete).
    UserSearchResults {
        users: Vec<UserInfo>,
    },

    // ── Media ─────────────────────────────────────────────────────────────────

    /// Media file uploaded successfully.
    MediaUploaded {
        document_id: DocumentId,
        /// Content-addressable hash of the uploaded blob (use as a reference in the CRDT).
        content_hash: String,
    },

    /// Media file downloaded and decrypted.
    MediaDownloaded {
        document_id: DocumentId,
        content_hash: String,
        /// Decrypted raw media bytes, ready for display.
        data: Vec<u8>,
    },

    // ── Channels (Murmure) ────────────────────────────────────────────────────

    /// List of channels accessible to the authenticated user.
    ChannelList {
        channels: Vec<ChannelSummary>,
    },

    /// A new channel was created.
    ChannelCreated {
        channel_id: ChannelId,
    },

    /// A channel was deleted.
    ChannelDeleted {
        channel_id: ChannelId,
    },

    /// Invitation sent to a channel member.
    ChannelInviteSent {
        channel_id: ChannelId,
        user_id: UserId,
    },

    /// A channel member was revoked and key rotation is complete.
    ChannelMemberRevoked {
        channel_id: ChannelId,
        revoked_user_id: UserId,
    },

    /// Channel revocation initiated by this client is complete.
    ChannelRevocationComplete {
        channel_id: ChannelId,
    },

    /// Current member list of a channel.
    ChannelMembers {
        channel_id: ChannelId,
        members: Vec<MemberInfo>,
    },

    /// A channel member's role was updated.
    ChannelMemberRoleUpdated {
        channel_id: ChannelId,
        user_id: UserId,
        role: Role,
    },

    /// Inbound channel message (decrypted by the Sync thread).
    ///
    /// `payload` is an opaque application byte blob whose schema is owned by
    /// `ambre-murmure`.
    ///
    /// TODO: migrate the Murmure channel surface to a typed `AppEvent::Murmure(…)`
    /// like the document surface — it still carries `Vec<u8>` inter-thread (stub).
    ChannelMessage {
        channel_id: ChannelId,
        /// Opaque application byte blob (schema owned by `ambre-murmure`).
        payload: Vec<u8>,
    },

    /// Inbound ephemeral channel event (typing indicator, presence, …).
    ChannelPresenceUpdate {
        channel_id: ChannelId,
        /// Opaque application ephemeral byte blob (schema owned by `ambre-murmure`).
        payload: Vec<u8>,
    },

    /// WebSocket connection status for a channel.
    ChannelConnectionStatus {
        channel_id: ChannelId,
        connected: bool,
    },

    /// Unread message counts for all subscribed channels.
    UnreadCounts {
        counts: Vec<UnreadCount>,
    },

    // ── Meetings (Rendez-Vous) ────────────────────────────────────────────────

    /// Meeting room created.
    MeetingCreated {
        meeting_id: MeetingId,
    },

    /// Successfully joined a meeting.
    MeetingJoined {
        meeting_id: MeetingId,
    },

    /// Left a meeting.
    MeetingLeft {
        meeting_id: MeetingId,
    },

    /// Meeting ended (host ended the session for all participants).
    MeetingEnded {
        meeting_id: MeetingId,
    },

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

    // ── Connection ────────────────────────────────────────────────────────────

    /// WebSocket connection status for a document.
    ///
    /// The UI Thread may display an online / offline indicator.
    ConnectionStatus {
        document_id: DocumentId,
        connected: bool,
    },

    // ── Errors & warnings ─────────────────────────────────────────────────────

    /// Authentication error (wrong passphrase, corrupted vault, network failure).
    AuthError {
        message: String,
    },

    /// REST or WebSocket operation error.
    ///
    /// `request` identifies the originating command (e.g., `"ListDocuments"`,
    /// `"InviteToDocument"`) so the UI Thread can correlate the error to the
    /// triggering action.
    ApiError {
        /// Name of the originating `LogicCommand` variant.
        request: String,
        /// HTTP status code or internal error code.
        code: u16,
        message: String,
    },

    /// Non-fatal sync warning (quarantine overflow, unknown schema version, …).
    SyncWarning {
        document_id: DocumentId,
        code: u16,
        message: String,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// AppEvent — typed application event family
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
}
