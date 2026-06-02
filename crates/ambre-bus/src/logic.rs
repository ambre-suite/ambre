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
//! # Closed world
//! [`AppCommand`] is a closed sum: adding a document type = adding a variant
//! here (one edit point, no trait object, no `Any`, no crate cycles — this
//! module depends only on `ambre-core`; the app crates depend on `ambre-bus`,
//! never the reverse). A future plugin surface would be a single
//! `Plugin(Box<dyn …>)` variant. Container documents embed others by recursion
//! (a `ToileCommand` carrying `Box<AppCommand>`), routed by `DocumentId` in the
//! Logic thread.

use crate::{ChannelId, DocType, DocumentId, MeetingId, Role, UserId};

// ─────────────────────────────────────────────────────────────────────────────
// LogicCommand — UI Thread → Logic thread
// ─────────────────────────────────────────────────────────────────────────────

/// Message sent from the UI Thread to the Logic thread.
///
/// Variants are grouped by processing priority:
/// - **P2** (`App`): user edits / viewport requests, < 50 ms budget (scroll
///   requests within < 10 ms).
/// - **P3** (everything else): session / document lifecycle, no strict latency.
///
/// Session and document lifecycle variants are forwarded as [`SyncCommand`]s to
/// the Sync thread by the Logic dispatcher (`handle_logic_command`). `App`
/// commands are routed to the document's state and handled within the Logic
/// thread.
///
/// [`SyncCommand`]: crate::SyncCommand
#[derive(Debug, Clone)]
pub enum LogicCommand {
    // ── P2 — Application command ──────────────────────────────────────────────

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

    // ── P3 — Session ──────────────────────────────────────────────────────────

    /// Bootstrap authentication after OIDC callback.
    ///
    /// Forwarded to the Sync thread as `SyncCommand::Unlock`. The Sync thread
    /// fetches `/api/keys/master`, derives the User Key via
    /// Argon2id(passphrase, salt), decrypts the MasterSeed, and derives keypairs.
    /// Responds with [`crate::ui::UiEvent::SessionReady`] or [`crate::ui::UiEvent::AuthError`].
    Unlock {
        /// UTF-8 passphrase encoded as bytes (zeroized by the Sync thread after use).
        passphrase: Vec<u8>,
    },

    /// First-time account setup.
    ///
    /// Forwarded to the Sync thread as `SyncCommand::SetupAccount`. The Sync thread
    /// generates a MasterSeed (CSPRNG), derives keypairs, encrypts with
    /// Argon2id(passphrase) → `PUT /api/keys/master`.
    /// Responds with [`crate::ui::UiEvent::AccountCreated`].
    SetupAccount {
        /// UTF-8 passphrase encoded as bytes.
        passphrase: Vec<u8>,
    },

    /// Change the account passphrase.
    ///
    /// The Sync thread re-derives the User Key and re-encrypts `master_key_enc`
    /// → `PUT /api/keys/master/rotate`.
    ChangePassphrase {
        /// Current passphrase (zeroized after use).
        current_passphrase: Vec<u8>,
        /// New passphrase (zeroized after use).
        new_passphrase: Vec<u8>,
    },

    /// Account recovery via 24-word BIP39 recovery phrase.
    ///
    /// The Sync thread reconstructs the MasterSeed and re-encrypts it
    /// → `PUT /api/keys/master/rotate`.
    RecoverAccount {
        /// Space-separated BIP39 recovery words.
        recovery_words: String,
        /// New passphrase to set after recovery (zeroized after use).
        new_passphrase: Vec<u8>,
    },

    /// Log out of the current session.
    ///
    /// The Logic thread clears all in-memory controllers. The Sync thread
    /// calls `POST /api/auth/logout` and zeroizes all in-memory keys.
    Logout,

    // ── P3 — Document CRUD ────────────────────────────────────────────────────

    /// Fetch the list of documents accessible to the authenticated user.
    ///
    /// Responds with [`crate::ui::UiEvent::DocumentList`].
    ListDocuments,

    /// Create a new document of the given type.
    ///
    /// The Sync thread generates a DocumentKey (CSPRNG), encrypts the name,
    /// `POST /api/documents`, and shares the key with the creator.
    /// Responds with [`crate::ui::UiEvent::DocumentCreated`].
    CreateDocument {
        doc_type: DocType,
        /// Plaintext document name (encrypted by the Sync thread before upload).
        name: String,
    },

    /// Permanently delete a document.
    ///
    /// The Logic thread drops the in-memory controller for `document_id`.
    /// The Sync thread calls `DELETE /api/documents/{id}`.
    DeleteDocument {
        document_id: DocumentId,
    },

    /// Rename a document.
    ///
    /// The Sync thread encrypts the new name → `PATCH /api/documents/{id}`.
    RenameDocument {
        document_id: DocumentId,
        /// New plaintext document name.
        name: String,
    },

    // ── P3 — Document sync session ────────────────────────────────────────────

    /// Open a real-time sync session for a document.
    ///
    /// The Sync thread fetches members + `doc_key_enc`, decrypts the DocumentKey,
    /// loads snapshot + deltas from S3, and opens a WebSocket.
    /// The first [`crate::ui::UiEvent::Viewport`] after this command indicates
    /// the document is ready to display.
    OpenDocument {
        document_id: DocumentId,
    },

    /// Close the real-time sync session for a document.
    ///
    /// The Logic thread drops the in-memory controller. The Sync thread closes
    /// the WebSocket and zeroizes the DocumentKey.
    CloseDocument {
        document_id: DocumentId,
    },

    // ── P3 — Sharing ──────────────────────────────────────────────────────────

    /// Invite a collaborator to a document.
    ///
    /// The Sync thread fetches the invitee's public keys, encrypts the
    /// DocumentKey for them → `POST /api/documents/{id}/share`.
    /// Responds with [`crate::ui::UiEvent::InviteSent`].
    InviteToDocument {
        document_id: DocumentId,
        invitee_email: String,
        role: Role,
    },

    /// Revoke a member from a document.
    ///
    /// The Sync thread orchestrates the full revocation sequence:
    /// `DELETE /share/{uid}` → generate new DocumentKey → encrypt for remaining
    /// members → `POST /rotate-key`. The server broadcasts
    /// `WsServerMsg::KeyRotationRequired` to all members after the rotation.
    /// Responds with [`crate::ui::UiEvent::MemberRevoked`].
    RevokeFromDocument {
        document_id: DocumentId,
        user_id: UserId,
    },

    /// List current members of a document.
    ///
    /// Responds with [`crate::ui::UiEvent::DocumentMembers`].
    ListDocumentMembers {
        document_id: DocumentId,
    },

    /// Update a member's role on a document.
    ///
    /// Responds with [`crate::ui::UiEvent::MemberRoleUpdated`].
    UpdateDocumentMemberRole {
        document_id: DocumentId,
        user_id: UserId,
        role: Role,
    },

    // ── P3 — Users ────────────────────────────────────────────────────────────

    /// Search users by email prefix (for invitation autocomplete).
    ///
    /// Responds with [`crate::ui::UiEvent::UserSearchResults`].
    SearchUsers {
        query: String,
    },

    // ── P3 — Media ────────────────────────────────────────────────────────────

    /// Upload a media file attached to a document.
    ///
    /// The Sync thread encrypts `data`, fetches a presigned S3 URL, PUT S3,
    /// then `POST /api/documents/{id}/media` to confirm.
    /// Responds with [`crate::ui::UiEvent::MediaUploaded`].
    UploadMedia {
        document_id: DocumentId,
        /// Raw (unencrypted) media bytes.
        data: Vec<u8>,
        mime_type: String,
    },

    /// Download a media file from a document.
    ///
    /// The Sync thread fetches a presigned S3 URL, GET S3, decrypts.
    /// Responds with [`crate::ui::UiEvent::MediaDownloaded`].
    DownloadMedia {
        document_id: DocumentId,
        /// Content-addressable hash identifying the media blob.
        content_hash: String,
    },

    // ── P3 — Channels (Murmure) ───────────────────────────────────────────────

    /// List channels accessible to the authenticated user.
    ///
    /// Responds with [`crate::ui::UiEvent::ChannelList`].
    ListChannels,

    /// Create a new Murmure channel.
    ///
    /// Responds with [`crate::ui::UiEvent::ChannelCreated`].
    CreateChannel {
        /// Plaintext channel name.
        name: String,
    },

    /// Delete a Murmure channel.
    DeleteChannel {
        channel_id: ChannelId,
    },

    /// Open a real-time sync session for a channel (subscribe to messages).
    OpenChannel {
        channel_id: ChannelId,
    },

    /// Close the real-time sync session for a channel.
    CloseChannel {
        channel_id: ChannelId,
    },

    /// Invite a user to a Murmure channel.
    InviteToChannel {
        channel_id: ChannelId,
        invitee_email: String,
        role: Role,
    },

    /// Revoke a member from a Murmure channel.
    RevokeFromChannel {
        channel_id: ChannelId,
        user_id: UserId,
    },

    /// List current members of a channel.
    ListChannelMembers {
        channel_id: ChannelId,
    },

    /// Update a member's role in a channel.
    UpdateChannelMemberRole {
        channel_id: ChannelId,
        user_id: UserId,
        role: Role,
    },

    /// Send a message to a channel.
    ///
    /// `payload` is an opaque application byte blob (schema owned by
    /// `ambre-murmure`); the Sync thread routes it into the active segment
    /// (segment management is entirely internal to `ambre-sync`).
    ///
    /// TODO: the Murmure channel surface still carries opaque `Vec<u8>` on the
    /// inter-thread path — migrate it to a typed `AppCommand::Murmure(…)` like the
    /// document surface (Murmure is currently a stub).
    SendChannelMessage {
        channel_id: ChannelId,
        /// Opaque application byte blob (schema owned by `ambre-murmure`).
        payload: Vec<u8>,
    },

    /// Advance the read cursor for unread-count tracking.
    UpdateReadCursor {
        channel_id: ChannelId,
        /// Sequence number of the last message seen by the user.
        last_seq: u64,
    },

    /// Request the unread message counts for all subscribed channels.
    ///
    /// Responds with [`crate::ui::UiEvent::UnreadCounts`].
    GetUnreadCounts,

    // ── P3 — Meetings (Rendez-Vous) ───────────────────────────────────────────

    /// Create an ephemeral meeting room.
    ///
    /// Meetings are not persisted as documents; they exist only while participants
    /// are connected. Responds with [`crate::ui::UiEvent::MeetingCreated`].
    CreateMeeting {
        name: String,
    },

    /// Join an existing meeting by ID.
    JoinMeeting {
        meeting_id: MeetingId,
    },

    /// Leave a meeting (but do not end it for other participants).
    LeaveMeeting {
        meeting_id: MeetingId,
    },

    /// End a meeting for all participants (host action).
    EndMeeting {
        meeting_id: MeetingId,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// AppCommand — typed application command family
// ─────────────────────────────────────────────────────────────────────────────

/// Application-specific command, UI Thread → Logic thread.
///
/// Carried inside [`LogicCommand::App`]. The Logic dispatcher matches the
/// variant, routes it to the `DocumentId`'s document state, and produces zero or more
/// [`AppEvent`](crate::ui::AppEvent)s.
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
}
