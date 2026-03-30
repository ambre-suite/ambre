//! `LogicEvent` – events sent from the Sync Worker to the Logic Worker.
//!
//! This enum contains every kind of response, notification, or error that the
//! Sync Worker can send back to the Logic Worker after processing a command
//! or receiving data from the network.

use crate::{DocType, Role, Uuid};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Business‑shared types (zero crypto)
// ─────────────────────────────────────────────────────────────────────────────

/// Summary of a document (for lists and navigation).
///
/// The `name` is decrypted by the Sync Worker – the Logic Worker sees it in plaintext.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub document_id: Uuid,
    pub doc_type: DocType,
    /// Decrypted from `name_enc` by the Sync Worker.
    pub name: Option<String>,
    pub role: Role,
    /// Epoch milliseconds.
    pub updated_at: u64,
}

/// Member of a document or channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberInfo {
    pub user_id: Uuid,
    pub display_name: String,
    pub email: String,
    pub role: Role,
}

/// User (search result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub user_id: Uuid,
    pub display_name: String,
    pub email: String,
}

/// Summary of a Murmure channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSummary {
    pub channel_id: Uuid,
    /// Decrypted from `name_enc`.
    pub name: Option<String>,
    pub role: Role,
}

/// Unread message count per channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnreadCount {
    pub channel_id: Uuid,
    pub count: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// LogicEvent
// ─────────────────────────────────────────────────────────────────────────────

/// Event sent from the Sync Worker to the Logic Worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogicEvent {
    // ── Session ──────────────────────────────────────────────────────────────
    /// Login successful – session ready, documents can be opened.
    SessionReady {
        user_id: Uuid,
        display_name: String,
        email: String,
    },

    /// First‑time connection detected – no keys on the server.
    /// The Logic Worker must ask the UI to display the SetupAccount flow.
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
        document_id: Uuid,
        doc_type: DocType,
    },

    /// Document deleted.
    DocumentDeleted {
        document_id: Uuid,
    },

    /// Document renamed.
    DocumentRenamed {
        document_id: Uuid,
    },

    // ── Sync temps réel ──────────────────────────────────────────────────────
    /// Decrypted and decompressed CRDT delta, ready for `LoroDoc::import()`.
    RemoteUpdate {
        document_id: Uuid,
        payload: Vec<u8>,
    },

    /// Decrypted and decompressed CRDT snapshot.
    RemoteSnapshot {
        document_id: Uuid,
        payload: Vec<u8>,
    },

    /// Decrypted ephemeral data (cursor, collaborator presence).
    RemoteEphemeral {
        document_id: Uuid,
        payload: Vec<u8>,
    },

    /// WebSocket connection status for a document.
    ConnectionStatus {
        document_id: Uuid,
        connected: bool,
    },

    /// Non‑fatal warning (quarantine overflow, unknown version, etc.).
    SyncWarning {
        document_id: Uuid,
        code: u16,
        message: String,
    },

    /// The server requests a snapshot (too many accumulated deltas).
    /// The Logic Worker must produce `LoroDoc::export_snapshot()` and respond
    /// with `SyncCommand::SendSnapshot { document_id, payload }`.
    SnapshotNeeded {
        document_id: Uuid,
    },

    // ── Partage ──────────────────────────────────────────────────────────────
    /// Invitation sent successfully.
    InviteSent {
        document_id: Uuid,
        user_id: Uuid,
        display_name: String,
    },

    /// A member has been revoked and consolidation is complete.
    /// Translated from `WsServerMsg::KeyRotationRequired` by the Sync Worker
    /// **after** the rotation is complete (new keys are in DB).
    /// The Logic Worker may trigger a snapshot to seal the state.
    MemberRevoked {
        document_id: Uuid,
        revoked_user_id: Uuid,
    },

    /// Revocation initiated by **this** client is complete.
    RevocationComplete {
        document_id: Uuid,
    },

    /// List of members of a document.
    DocumentMembers {
        document_id: Uuid,
        members: Vec<MemberInfo>,
    },

    /// Member role updated.
    MemberRoleUpdated {
        document_id: Uuid,
        user_id: Uuid,
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
        document_id: Uuid,
        content_hash: String,
    },

    /// Media downloaded and decrypted – raw data ready for display.
    MediaDownloaded {
        document_id: Uuid,
        content_hash: String,
        data: Vec<u8>,
    },

    // ── Channels (Murmure) ───────────────────────────────────────────────────
    ChannelList {
        channels: Vec<ChannelSummary>,
    },
    ChannelCreated {
        channel_id: Uuid,
    },
    ChannelDeleted {
        channel_id: Uuid,
    },
    ChannelInviteSent {
        channel_id: Uuid,
        user_id: Uuid,
    },
    ChannelMemberRevoked {
        channel_id: Uuid,
        revoked_user_id: Uuid,
    },
    ChannelRevocationComplete {
        channel_id: Uuid,
    },
    ChannelMembers {
        channel_id: Uuid,
        members: Vec<MemberInfo>,
    },
    ChannelMemberRoleUpdated {
        channel_id: Uuid,
        user_id: Uuid,
        role: Role,
    },
    ChannelRemoteUpdate {
        channel_id: Uuid,
        payload: Vec<u8>,
    },
    ChannelRemoteEphemeral {
        channel_id: Uuid,
        payload: Vec<u8>,
    },
    ChannelConnectionStatus {
        channel_id: Uuid,
        connected: bool,
    },
    ChannelSnapshotNeeded {
        channel_id: Uuid,
    },
    UnreadCounts {
        counts: Vec<UnreadCount>,
    },

    // ── Meetings (Rendez‑Vous) ───────────────────────────────────────────────
    MeetingCreated {
        meeting_id: Uuid,
    },
    MeetingJoined {
        meeting_id: Uuid,
    },
    MeetingLeft {
        meeting_id: Uuid,
    },
    MeetingEnded {
        meeting_id: Uuid,
    },
    MeetingParticipantJoined {
        meeting_id: Uuid,
        user_id: Uuid,
    },
    MeetingParticipantLeft {
        meeting_id: Uuid,
        user_id: Uuid,
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
