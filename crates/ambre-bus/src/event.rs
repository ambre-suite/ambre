//! `LogicEvent` – events sent from the Sync thread to the Logic thread.
//!
//! This enum contains every kind of response, notification, or error that the
//! Sync thread can send back to the Logic thread after processing a command
//! or receiving data from the network.

use crate::{ChannelId, DocType, DocumentId, MeetingId, Role, UserId};

// ─────────────────────────────────────────────────────────────────────────────
// Business‑shared types (zero crypto)
// ─────────────────────────────────────────────────────────────────────────────

/// Summary of a document (for lists and navigation).
///
/// The `name` is decrypted by the Sync thread – the Logic thread sees it in plaintext.
#[derive(Debug, Clone)]
pub struct DocumentSummary {
    pub document_id: DocumentId,
    pub doc_type: DocType,
    /// Decrypted from `name_enc` by the Sync thread.
    pub name: Option<String>,
    pub role: Role,
    /// Epoch milliseconds.
    pub updated_at: u64,
}

/// Member of a document or channel.
#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub user_id: UserId,
    pub display_name: String,
    pub email: String,
    pub role: Role,
}

/// User (search result).
#[derive(Debug, Clone)]
pub struct UserInfo {
    pub user_id: UserId,
    pub display_name: String,
    pub email: String,
}

/// Summary of a Murmure channel.
#[derive(Debug, Clone)]
pub struct ChannelSummary {
    pub channel_id: ChannelId,
    /// Decrypted from `name_enc`.
    pub name: Option<String>,
    pub role: Role,
}

/// Unread message count per channel.
#[derive(Debug, Clone)]
pub struct UnreadCount {
    pub channel_id: ChannelId,
    pub count: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// LogicEvent
// ─────────────────────────────────────────────────────────────────────────────

/// Event sent from the Sync thread to the Logic thread.
#[derive(Debug, Clone)]
pub enum LogicEvent {
    // ── Session ──────────────────────────────────────────────────────────────
    /// Login successful – session ready, documents can be opened.
    SessionReady {
        user_id: UserId,
        display_name: String,
        email: String,
    },

    /// First‑time connection detected – no keys on the server.
    /// The Logic thread must ask the UI to display the SetupAccount flow.
    AccountSetupRequired,

    /// Account created – display the 24‑word recovery phrase **once**.
    AccountCreated {
        recovery_words: String,
    },

    /// Passphrase changed.
    PassphraseChanged,

    /// Account recovered via Recovery Key.
    AccountRecovered,

    /// Logged out.
    LoggedOut,

    /// Authentication error (incorrect passphrase, corrupted data, network).
    AuthError {
        message: String,
    },

    // ── Documents CRUD ───────────────────────────────────────────────────────
    /// List of documents accessible to the user.
    DocumentList {
        documents: Vec<DocumentSummary>,
    },

    /// Document created successfully.
    DocumentCreated {
        document_id: DocumentId,
        doc_type: DocType,
    },

    /// Document deleted.
    DocumentDeleted {
        document_id: DocumentId,
    },

    /// Document renamed.
    DocumentRenamed {
        document_id: DocumentId,
    },

    // ── Sync temps réel ──────────────────────────────────────────────────────
    /// Decrypted and decompressed CRDT delta, ready for `LoroDoc::import()`.
    RemoteUpdate {
        document_id: DocumentId,
        payload: Vec<u8>,
    },

    /// Decrypted and decompressed CRDT snapshot.
    RemoteSnapshot {
        document_id: DocumentId,
        payload: Vec<u8>,
    },

    /// Decrypted ephemeral data (cursor, collaborator presence).
    RemoteEphemeral {
        document_id: DocumentId,
        payload: Vec<u8>,
    },

    /// WebSocket connection status for a document.
    ConnectionStatus {
        document_id: DocumentId,
        connected: bool,
    },

    /// Non‑fatal warning (quarantine overflow, unknown version, etc.).
    SyncWarning {
        document_id: DocumentId,
        code: u16,
        message: String,
    },

    /// The server requests a snapshot (too many accumulated deltas).
    /// The Logic thread must produce `LoroDoc::export_snapshot()` and respond
    /// with `SyncCommand::SendSnapshot { document_id, payload }`.
    SnapshotNeeded {
        document_id: DocumentId,
    },

    // ── Partage ──────────────────────────────────────────────────────────────
    /// Invitation sent successfully.
    InviteSent {
        document_id: DocumentId,
        user_id: UserId,
        display_name: String,
    },

    /// A member has been revoked and consolidation is complete.
    /// Translated from `WsServerMsg::KeyRotationRequired` by the Sync thread
    /// **after** the rotation is complete (new keys are in DB).
    /// The Logic thread may trigger a snapshot to seal the state.
    MemberRevoked {
        document_id: DocumentId,
        revoked_user_id: UserId,
    },

    /// Revocation initiated by **this** client is complete.
    RevocationComplete {
        document_id: DocumentId,
    },

    /// List of members of a document.
    DocumentMembers {
        document_id: DocumentId,
        members: Vec<MemberInfo>,
    },

    /// Member role updated.
    MemberRoleUpdated {
        document_id: DocumentId,
        user_id: UserId,
        role: Role,
    },

    // ── Utilisateurs ─────────────────────────────────────────────────────────
    /// User search results.
    UserSearchResults {
        users: Vec<UserInfo>,
    },

    // ── Médias ───────────────────────────────────────────────────────────────
    /// Media uploaded – returns the content‑addressable hash.
    MediaUploaded {
        document_id: DocumentId,
        content_hash: String,
    },

    /// Media downloaded and decrypted – raw data ready for display.
    MediaDownloaded {
        document_id: DocumentId,
        content_hash: String,
        data: Vec<u8>,
    },

    // ── Channels (Murmure) ───────────────────────────────────────────────────
    ChannelList {
        channels: Vec<ChannelSummary>,
    },
    ChannelCreated {
        channel_id: ChannelId,
    },
    ChannelDeleted {
        channel_id: ChannelId,
    },
    ChannelInviteSent {
        channel_id: ChannelId,
        user_id: UserId,
    },
    ChannelMemberRevoked {
        channel_id: ChannelId,
        revoked_user_id: UserId,
    },
    ChannelRevocationComplete {
        channel_id: ChannelId,
    },
    ChannelMembers {
        channel_id: ChannelId,
        members: Vec<MemberInfo>,
    },
    ChannelMemberRoleUpdated {
        channel_id: ChannelId,
        user_id: UserId,
        role: Role,
    },
    ChannelRemoteUpdate {
        channel_id: ChannelId,
        payload: Vec<u8>,
    },
    ChannelRemoteEphemeral {
        channel_id: ChannelId,
        payload: Vec<u8>,
    },
    ChannelConnectionStatus {
        channel_id: ChannelId,
        connected: bool,
    },
    ChannelSnapshotNeeded {
        channel_id: ChannelId,
    },
    UnreadCounts {
        counts: Vec<UnreadCount>,
    },

    // ── Meetings (Rendez‑Vous) ───────────────────────────────────────────────
    MeetingCreated {
        meeting_id: MeetingId,
    },
    MeetingJoined {
        meeting_id: MeetingId,
    },
    MeetingLeft {
        meeting_id: MeetingId,
    },
    MeetingEnded {
        meeting_id: MeetingId,
    },
    MeetingParticipantJoined {
        meeting_id: MeetingId,
        user_id: UserId,
    },
    MeetingParticipantLeft {
        meeting_id: MeetingId,
        user_id: UserId,
    },

    // ── Erreurs ──────────────────────────────────────────────────────────────
    /// Generic error for any REST/WS operation.
    /// `request` identifies the source command (e.g., "ListDocuments", "InviteToDocument").
    ApiError {
        request: String,
        code: u16,
        message: String,
    },
}
