//! Message types for UI Thread ↔ Logic Worker communication.
//!
//! # Transport
//! Both enums cross the WASM/JS boundary via `postMessage` + postcard serialization
//! (see `ambre-spec-architecture.md §2.B`). The caller serializes with
//! `postcard::to_allocvec`, copies into a JS `ArrayBuffer`, and transfers it.
//! The receiver copies the `ArrayBuffer` back into WASM heap and deserializes.
//!
//! # Layering
//! ```text
//! UI Thread  ──UiEnvelope──▶  Logic Worker  ──SyncCommand──▶  Sync Worker
//! UI Thread  ◀─LogicEnvelope─  Logic Worker  ◀─LogicEvent───  Sync Worker
//! ```
//! `UiEnvelope` and `LogicEnvelope` carry the full application surface visible
//! to the UI layer. `SyncCommand` / `LogicEvent` (see `command.rs` / `event.rs`)
//! are the internal bus between Logic and Sync Workers and must never leak to
//! the UI Thread.
//!
//! # Opaque payloads
//! Variants that carry a `payload: Vec<u8>` hold postcard-serialized data whose
//! schema is owned by an application crate (`ambre-trame`, `ambre-prose`, …).
//! `ambre-bus` does not interpret these bytes; only the `AppController` for the
//! relevant document type does.

use crate::event::{ChannelSummary, DocumentSummary, MemberInfo, UnreadCount, UserInfo};
use crate::{DocType, Role, Uuid};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// UiEnvelope — UI Thread → Logic Worker
// ─────────────────────────────────────────────────────────────────────────────

/// Message sent from the UI Thread to the Logic Worker via `postMessage`.
///
/// Variants are grouped by processing priority:
/// - **P1** (`ScrollTo`): must complete in < 15 ms end-to-end.
/// - **P2** (`AppCommand`): user edits, < 50 ms budget.
/// - **P3** (everything else): session / document lifecycle, no strict latency.
///
/// Session and document lifecycle variants are forwarded as [`SyncCommand`]s to
/// the Sync Worker by the Logic Worker dispatcher (`handle_ui_envelope`).
/// Viewport variants are handled entirely within the Logic Worker.
///
/// [`SyncCommand`]: crate::SyncCommand
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiEnvelope {
    // ── P1 — Viewport ─────────────────────────────────────────────────────────

    /// Scroll / resize request for an open document.
    ///
    /// The Logic Worker recomputes the visible region (cells, lines, elements)
    /// and replies with one or more [`LogicEnvelope::ViewportUpdate`] messages.
    /// This variant must be processed in < 15 ms to meet the display latency SLA.
    ScrollTo {
        document_id: Uuid,
        /// First visible row index (0-based).
        top_row: u32,
        /// First visible column index (0-based).
        left_col: u32,
        /// Number of visible rows.
        viewport_rows: u32,
        /// Number of visible columns.
        viewport_cols: u32,
    },

    // ── P2 — Application command ──────────────────────────────────────────────

    /// Application-specific edit command for an open document.
    ///
    /// The `payload` is a postcard-serialized command understood by the
    /// [`AppController`] registered for `document_id` (e.g., a cell edit in
    /// Trame, a text insertion in Prose, an element move in Toile).
    /// `ambre-bus` does not interpret the payload.
    ///
    /// [`AppController`]: crate::AppController
    AppCommand {
        document_id: Uuid,
        /// Postcard-serialized application command (schema owned by the app crate).
        payload: Vec<u8>,
    },

    // ── P3 — Session ──────────────────────────────────────────────────────────

    /// Bootstrap authentication after OIDC callback.
    ///
    /// Forwarded to the Sync Worker as `SyncCommand::Unlock`. The Sync Worker
    /// fetches `/api/keys/master`, derives the User Key via
    /// Argon2id(passphrase, salt), decrypts the MasterSeed, and derives keypairs.
    /// Responds with [`LogicEnvelope::SessionReady`] or [`LogicEnvelope::AuthError`].
    Unlock {
        /// UTF-8 passphrase encoded as bytes (zeroized by the Sync Worker after use).
        passphrase: Vec<u8>,
    },

    /// First-time account setup.
    ///
    /// Forwarded to the Sync Worker as `SyncCommand::SetupAccount`. The Sync Worker
    /// generates a MasterSeed (CSPRNG), derives keypairs, encrypts with
    /// Argon2id(passphrase) → `PUT /api/keys/master`.
    /// Responds with [`LogicEnvelope::AccountCreated`].
    SetupAccount {
        /// UTF-8 passphrase encoded as bytes.
        passphrase: Vec<u8>,
    },

    /// Change the account passphrase.
    ///
    /// The Sync Worker re-derives the User Key and re-encrypts `master_key_enc`
    /// → `PUT /api/keys/master/rotate`.
    ChangePassphrase {
        /// Current passphrase (zeroized after use).
        current_passphrase: Vec<u8>,
        /// New passphrase (zeroized after use).
        new_passphrase: Vec<u8>,
    },

    /// Account recovery via 24-word BIP39 recovery phrase.
    ///
    /// The Sync Worker reconstructs the MasterSeed and re-encrypts it
    /// → `PUT /api/keys/master/rotate`.
    RecoverAccount {
        /// Space-separated BIP39 recovery words.
        recovery_words: String,
        /// New passphrase to set after recovery (zeroized after use).
        new_passphrase: Vec<u8>,
    },

    /// Log out of the current session.
    ///
    /// The Logic Worker clears all in-memory controllers. The Sync Worker
    /// calls `POST /api/auth/logout` and zeroizes all in-memory keys.
    Logout,

    // ── P3 — Document CRUD ────────────────────────────────────────────────────

    /// Fetch the list of documents accessible to the authenticated user.
    ///
    /// Responds with [`LogicEnvelope::DocumentList`].
    ListDocuments,

    /// Create a new document of the given type.
    ///
    /// The Sync Worker generates a DocumentKey (CSPRNG), encrypts the name,
    /// `POST /api/documents`, and shares the key with the creator.
    /// Responds with [`LogicEnvelope::DocumentCreated`].
    CreateDocument {
        doc_type: DocType,
        /// Plaintext document name (encrypted by the Sync Worker before upload).
        name: String,
    },

    /// Permanently delete a document.
    ///
    /// The Logic Worker drops the in-memory controller for `document_id`.
    /// The Sync Worker calls `DELETE /api/documents/{id}`.
    DeleteDocument {
        document_id: Uuid,
    },

    /// Rename a document.
    ///
    /// The Sync Worker encrypts the new name → `PATCH /api/documents/{id}`.
    RenameDocument {
        document_id: Uuid,
        /// New plaintext document name.
        name: String,
    },

    // ── P3 — Document sync session ────────────────────────────────────────────

    /// Open a real-time sync session for a document.
    ///
    /// The Sync Worker fetches members + `doc_key_enc`, decrypts the DocumentKey,
    /// loads snapshot + deltas from S3, and opens a WebSocket.
    /// The first [`LogicEnvelope::ViewportUpdate`] after this command indicates
    /// the document is ready to display.
    OpenDocument {
        document_id: Uuid,
    },

    /// Close the real-time sync session for a document.
    ///
    /// The Logic Worker drops the in-memory controller. The Sync Worker closes
    /// the WebSocket and zeroizes the DocumentKey.
    CloseDocument {
        document_id: Uuid,
    },

    // ── P3 — Sharing ──────────────────────────────────────────────────────────

    /// Invite a collaborator to a document.
    ///
    /// The Sync Worker fetches the invitee's public keys, encrypts the
    /// DocumentKey for them → `POST /api/documents/{id}/share`.
    /// Responds with [`LogicEnvelope::InviteSent`].
    InviteToDocument {
        document_id: Uuid,
        invitee_email: String,
        role: Role,
    },

    /// Revoke a member from a document.
    ///
    /// The Sync Worker orchestrates the full revocation sequence:
    /// `DELETE /share/{uid}` → generate new DocumentKey → encrypt for remaining
    /// members → `POST /rotate-key`. The server broadcasts
    /// `WsServerMsg::KeyRotationRequired` to all members after the rotation.
    /// Responds with [`LogicEnvelope::MemberRevoked`].
    RevokeFromDocument {
        document_id: Uuid,
        user_id: Uuid,
    },

    /// List current members of a document.
    ///
    /// Responds with [`LogicEnvelope::DocumentMembers`].
    ListDocumentMembers {
        document_id: Uuid,
    },

    /// Update a member's role on a document.
    ///
    /// Responds with [`LogicEnvelope::MemberRoleUpdated`].
    UpdateDocumentMemberRole {
        document_id: Uuid,
        user_id: Uuid,
        role: Role,
    },

    // ── P3 — Users ────────────────────────────────────────────────────────────

    /// Search users by email prefix (for invitation autocomplete).
    ///
    /// Responds with [`LogicEnvelope::UserSearchResults`].
    SearchUsers {
        query: String,
    },

    // ── P3 — Media ────────────────────────────────────────────────────────────

    /// Upload a media file attached to a document.
    ///
    /// The Sync Worker encrypts `data`, fetches a presigned S3 URL, PUT S3,
    /// then `POST /api/documents/{id}/media` to confirm.
    /// Responds with [`LogicEnvelope::MediaUploaded`].
    UploadMedia {
        document_id: Uuid,
        /// Raw (unencrypted) media bytes.
        data: Vec<u8>,
        mime_type: String,
    },

    /// Download a media file from a document.
    ///
    /// The Sync Worker fetches a presigned S3 URL, GET S3, decrypts.
    /// Responds with [`LogicEnvelope::MediaDownloaded`].
    DownloadMedia {
        document_id: Uuid,
        /// Content-addressable hash identifying the media blob.
        content_hash: String,
    },

    // ── P3 — Channels (Murmure) ───────────────────────────────────────────────

    /// List channels accessible to the authenticated user.
    ///
    /// Responds with [`LogicEnvelope::ChannelList`].
    ListChannels,

    /// Create a new Murmure channel.
    ///
    /// Responds with [`LogicEnvelope::ChannelCreated`].
    CreateChannel {
        /// Plaintext channel name.
        name: String,
    },

    /// Delete a Murmure channel.
    DeleteChannel {
        channel_id: Uuid,
    },

    /// Open a real-time sync session for a channel (subscribe to messages).
    OpenChannel {
        channel_id: Uuid,
    },

    /// Close the real-time sync session for a channel.
    CloseChannel {
        channel_id: Uuid,
    },

    /// Invite a user to a Murmure channel.
    InviteToChannel {
        channel_id: Uuid,
        invitee_email: String,
        role: Role,
    },

    /// Revoke a member from a Murmure channel.
    RevokeFromChannel {
        channel_id: Uuid,
        user_id: Uuid,
    },

    /// List current members of a channel.
    ListChannelMembers {
        channel_id: Uuid,
    },

    /// Update a member's role in a channel.
    UpdateChannelMemberRole {
        channel_id: Uuid,
        user_id: Uuid,
        role: Role,
    },

    /// Send a message to a channel.
    ///
    /// The `payload` is a postcard-serialized message understood by the
    /// Murmure controller. The Sync Worker routes it into the active
    /// segment (segment management is entirely internal to `ambre-sync`).
    SendChannelMessage {
        channel_id: Uuid,
        /// Postcard-serialized channel message (schema owned by `ambre-murmure`).
        payload: Vec<u8>,
    },

    /// Advance the read cursor for unread-count tracking.
    UpdateReadCursor {
        channel_id: Uuid,
        /// Sequence number of the last message seen by the user.
        last_seq: u64,
    },

    /// Request the unread message counts for all subscribed channels.
    ///
    /// Responds with [`LogicEnvelope::UnreadCounts`].
    GetUnreadCounts,

    // ── P3 — Meetings (Rendez-Vous) ───────────────────────────────────────────

    /// Create an ephemeral meeting room.
    ///
    /// Meetings are not persisted as documents; they exist only while participants
    /// are connected. Responds with [`LogicEnvelope::MeetingCreated`].
    CreateMeeting {
        name: String,
    },

    /// Join an existing meeting by ID.
    JoinMeeting {
        meeting_id: Uuid,
    },

    /// Leave a meeting (but do not end it for other participants).
    LeaveMeeting {
        meeting_id: Uuid,
    },

    /// End a meeting for all participants (host action).
    EndMeeting {
        meeting_id: Uuid,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// LogicEnvelope — Logic Worker → UI Thread
// ─────────────────────────────────────────────────────────────────────────────

/// Message sent from the Logic Worker to the UI Thread via `postMessage`.
///
/// The Logic Worker produces these messages in two ways:
/// 1. **Forwarding**: a [`LogicEvent`] received from the Sync Worker is mapped
///    1-to-1 to the equivalent `LogicEnvelope` variant and forwarded to the UI.
/// 2. **Local computation**: `ViewportUpdate` and `PresenceUpdate` are produced
///    by the Logic Worker itself (after applying a CRDT delta or computing the
///    visible region) without a Sync Worker round-trip.
///
/// [`LogicEvent`]: crate::LogicEvent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogicEnvelope {
    // ── Viewport ──────────────────────────────────────────────────────────────

    /// Display list produced in response to [`UiEnvelope::ScrollTo`] or a
    /// spontaneous refresh after a remote CRDT update.
    ///
    /// The `payload` is a postcard-serialized display list whose schema is
    /// owned by the application crate for `document_id`. The Render Worker /
    /// UI Thread submits it directly to the GPU pipeline without parsing.
    ViewportUpdate {
        document_id: Uuid,
        /// Postcard-serialized display list (schema owned by the app crate).
        payload: Vec<u8>,
    },

    /// Decrypted collaborator presence data (cursor position, selection, …).
    ///
    /// Produced when the Logic Worker forwards a [`LogicEvent::RemoteEphemeral`]
    /// through its registered `AppController`. The `payload` schema is owned by
    /// the application crate for `document_id`.
    PresenceUpdate {
        document_id: Uuid,
        /// Postcard-serialized presence payload (schema owned by the app crate).
        payload: Vec<u8>,
    },

    // ── Session ───────────────────────────────────────────────────────────────

    /// Authentication succeeded — session is ready.
    ///
    /// The UI Thread may now display the document list and open documents.
    SessionReady {
        user_id: Uuid,
        display_name: String,
        email: String,
    },

    /// First-time connection: no keys found on the server.
    ///
    /// The UI Thread must display the `SetupAccount` onboarding flow and send
    /// [`UiEnvelope::SetupAccount`] once the user enters a passphrase.
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
        document_id: Uuid,
        doc_type: DocType,
    },

    /// A document was permanently deleted.
    DocumentDeleted {
        document_id: Uuid,
    },

    /// A document was renamed.
    DocumentRenamed {
        document_id: Uuid,
    },

    // ── Sharing ───────────────────────────────────────────────────────────────

    /// Invitation sent successfully.
    InviteSent {
        document_id: Uuid,
        user_id: Uuid,
        display_name: String,
    },

    /// A member was revoked and key rotation is complete.
    ///
    /// Forwarded from [`LogicEvent::MemberRevoked`] **after** the Sync Worker
    /// has completed the full rotation (new keys in DB). The Logic Worker may
    /// take a snapshot to seal the CRDT history at this boundary.
    MemberRevoked {
        document_id: Uuid,
        revoked_user_id: Uuid,
    },

    /// Revocation initiated by this client is complete.
    RevocationComplete {
        document_id: Uuid,
    },

    /// Current member list of a document.
    DocumentMembers {
        document_id: Uuid,
        members: Vec<MemberInfo>,
    },

    /// A member's role was updated.
    MemberRoleUpdated {
        document_id: Uuid,
        user_id: Uuid,
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
        document_id: Uuid,
        /// Content-addressable hash of the uploaded blob (use as a reference in the CRDT).
        content_hash: String,
    },

    /// Media file downloaded and decrypted.
    MediaDownloaded {
        document_id: Uuid,
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
        channel_id: Uuid,
    },

    /// A channel was deleted.
    ChannelDeleted {
        channel_id: Uuid,
    },

    /// Invitation sent to a channel member.
    ChannelInviteSent {
        channel_id: Uuid,
        user_id: Uuid,
    },

    /// A channel member was revoked and key rotation is complete.
    ChannelMemberRevoked {
        channel_id: Uuid,
        revoked_user_id: Uuid,
    },

    /// Channel revocation initiated by this client is complete.
    ChannelRevocationComplete {
        channel_id: Uuid,
    },

    /// Current member list of a channel.
    ChannelMembers {
        channel_id: Uuid,
        members: Vec<MemberInfo>,
    },

    /// A channel member's role was updated.
    ChannelMemberRoleUpdated {
        channel_id: Uuid,
        user_id: Uuid,
        role: Role,
    },

    /// Inbound channel message (decrypted by the Sync Worker).
    ///
    /// The `payload` is a postcard-serialized message whose schema is owned
    /// by `ambre-murmure`.
    ChannelMessage {
        channel_id: Uuid,
        /// Postcard-serialized message payload (schema owned by `ambre-murmure`).
        payload: Vec<u8>,
    },

    /// Inbound ephemeral channel event (typing indicator, presence, …).
    ChannelPresenceUpdate {
        channel_id: Uuid,
        /// Postcard-serialized ephemeral payload.
        payload: Vec<u8>,
    },

    /// WebSocket connection status for a channel.
    ChannelConnectionStatus {
        channel_id: Uuid,
        connected: bool,
    },

    /// Unread message counts for all subscribed channels.
    UnreadCounts {
        counts: Vec<UnreadCount>,
    },

    // ── Meetings (Rendez-Vous) ────────────────────────────────────────────────

    /// Meeting room created.
    MeetingCreated {
        meeting_id: Uuid,
    },

    /// Successfully joined a meeting.
    MeetingJoined {
        meeting_id: Uuid,
    },

    /// Left a meeting.
    MeetingLeft {
        meeting_id: Uuid,
    },

    /// Meeting ended (host ended the session for all participants).
    MeetingEnded {
        meeting_id: Uuid,
    },

    /// A participant joined the meeting.
    MeetingParticipantJoined {
        meeting_id: Uuid,
        user_id: Uuid,
    },

    /// A participant left the meeting.
    MeetingParticipantLeft {
        meeting_id: Uuid,
        user_id: Uuid,
    },

    // ── Connection ────────────────────────────────────────────────────────────

    /// WebSocket connection status for a document.
    ///
    /// The UI Thread may display an online / offline indicator.
    ConnectionStatus {
        document_id: Uuid,
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
        /// Name of the originating `UiEnvelope` variant.
        request: String,
        /// HTTP status code or internal error code.
        code: u16,
        message: String,
    },

    /// Non-fatal sync warning (quarantine overflow, unknown schema version, …).
    SyncWarning {
        document_id: Uuid,
        code: u16,
        message: String,
    },
}
