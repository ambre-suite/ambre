//! Relayed service plane — the correlatable request/response family.
//!
//! Most P3 traffic is a pure relay: the UI asks for an operation, the Logic
//! thread forwards it verbatim to the Sync thread (which performs REST / crypto
//! / WebSocket work), and the response travels back the same way. Modelling
//! each step as its own enum variant produced four near-identical enums linked
//! by ~300 lines of hand-written 1:1 mapping (the "P3 triplication").
//!
//! Instead, this family is defined **once** here and carried by thin envelopes:
//!
//! ```text
//! LogicCommand::Rest { id, cmd: RestCommand }   // UI    → Logic
//! SyncCommand ::Rest { id, cmd: RestCommand }   // Logic → Sync
//! LogicEvent  ::Rest { id, evt: RestEvent }     // Sync  → Logic
//! UiEvent     ::Rest { id, evt: RestEvent }     // Logic → UI
//! ```
//!
//! The Logic dispatcher collapses to a single forwarding arm (plus a tiny hook
//! for the few service ops that also mutate Logic state, e.g. `Logout` clears
//! the document registry). Adding a service op = one `RestCommand` variant +
//! one `RestEvent` variant, edited in one place.
//!
//! # What is *not* here
//! - **Content plane** (`AppCommand` / `AppEvent`): document edits and viewport
//!   data, routed by `DocumentId` inside the Logic thread — see [`crate::logic`].
//! - **Control & content-send** (`OpenDocument`, `CloseDocument`, `OpenChannel`,
//!   `CloseChannel`, `SendChannelMessage`): fire-and-forget, no single response
//!   to correlate — kept as explicit envelope variants.
//! - **Server push** (`Remote*`, `ConnectionStatus`, `SyncWarning`,
//!   `MemberRevoked`, `*ParticipantJoined/Left`): unsolicited, no originating
//!   request — kept explicit.
//! - **Errors** (`AuthError`, `ApiError`): may be either a response or a
//!   spontaneous server error; kept explicit for now (correlating errors by
//!   `RequestId` is a later refinement).

use core::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::{ChannelId, DocType, DocumentId, MeetingId, Role, UserId};

// ─────────────────────────────────────────────────────────────────────────────
// RequestId — request/response correlation token
// ─────────────────────────────────────────────────────────────────────────────

/// Correlation token tying a [`RestCommand`] to the [`RestEvent`] that answers
/// it.
///
/// Allocated by the **originator** (the UI Thread) via [`RequestId::next`] and
/// echoed unchanged by the Logic and Sync threads. Lets a UI with several P3
/// operations in flight route each response to its call-site and apply
/// per-request timeout / retry — impossible with the previous fire-and-forget,
/// stringly-correlated model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(pub u64);

impl RequestId {
    /// Allocate the next process-wide-unique id.
    ///
    /// Backed by a single `AtomicU64` shared across the UI / Logic / Sync
    /// threads (legal: the shared-memory threading model enables wasm atomics).
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        RequestId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Business-shared response types (zero crypto — names already decrypted by Sync)
// ─────────────────────────────────────────────────────────────────────────────

/// Summary of a document (for lists and navigation).
///
/// The `name` is decrypted by the Sync thread — the Logic and UI threads see it
/// in plaintext.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberInfo {
    pub user_id: UserId,
    pub display_name: String,
    pub email: String,
    pub role: Role,
}

/// User (search result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub user_id: UserId,
    pub display_name: String,
    pub email: String,
}

/// Summary of a Murmure channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSummary {
    pub channel_id: ChannelId,
    /// Decrypted from `name_enc`.
    pub name: Option<String>,
    pub role: Role,
}

/// Unread message count per channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnreadCount {
    pub channel_id: ChannelId,
    pub count: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// RestCommand — relayed service request (UI → Logic → Sync)
// ─────────────────────────────────────────────────────────────────────────────

/// A correlatable service request. Carried inside `LogicCommand::Rest` and
/// forwarded verbatim as `SyncCommand::Rest`; the Sync thread answers with the
/// matching [`RestEvent`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestCommand {
    // ── Session ──────────────────────────────────────────────────────────────
    /// Bootstrap auth after OIDC callback. → `SessionReady` / `AccountSetupRequired`.
    Unlock {
        /// UTF-8 passphrase bytes (zeroized by the Sync thread after use).
        passphrase: Vec<u8>,
    },
    /// First-time account setup. → `AccountCreated { recovery_words }`.
    SetupAccount { passphrase: Vec<u8> },
    /// Dev-only: authenticate as `username` (created on the fly server-side),
    /// deriving keys deterministically from the username. Triggered by the
    /// `?user=<name>` URL param to test multi-user behaviour on one machine.
    /// Rejected unless the server runs with `DEV_MODE`. → `SessionReady`.
    DevLogin { username: String },
    /// Change the account passphrase. → `PassphraseChanged`.
    ChangePassphrase {
        current_passphrase: Vec<u8>,
        new_passphrase: Vec<u8>,
    },
    /// Account recovery via 24-word BIP39 phrase. → `AccountRecovered`.
    RecoverAccount {
        recovery_words: String,
        new_passphrase: Vec<u8>,
    },
    /// Log out. The Logic thread clears its document registry on dispatch (hook),
    /// then the Sync thread zeroizes all keys. → `LoggedOut`.
    Logout,

    // ── Document CRUD ────────────────────────────────────────────────────────
    /// List accessible documents. → `DocumentList`.
    ListDocuments,
    /// Create a document. → `DocumentCreated`.
    CreateDocument {
        doc_type: DocType,
        /// Plaintext name (encrypted by the Sync thread before upload).
        name: String,
    },
    /// Delete a document. The Logic thread drops its in-memory state (hook). → `DocumentDeleted`.
    DeleteDocument { document_id: DocumentId },
    /// Rename a document. → `DocumentRenamed`.
    RenameDocument {
        document_id: DocumentId,
        name: String,
    },

    // ── Sharing ──────────────────────────────────────────────────────────────
    /// Invite a collaborator. → `InviteSent`.
    InviteToDocument {
        document_id: DocumentId,
        invitee_email: String,
        role: Role,
    },
    /// Revoke a member (triggers the full key-rotation sequence). → `RevocationComplete`.
    RevokeFromDocument {
        document_id: DocumentId,
        user_id: UserId,
    },
    /// List current members. → `DocumentMembers`.
    ListDocumentMembers { document_id: DocumentId },
    /// Update a member's role. → `MemberRoleUpdated`.
    UpdateDocumentMemberRole {
        document_id: DocumentId,
        user_id: UserId,
        role: Role,
    },

    // ── Users ────────────────────────────────────────────────────────────────
    /// Search users by email prefix. → `UserSearchResults`.
    SearchUsers { query: String },

    // ── Media ────────────────────────────────────────────────────────────────
    /// Upload a media file. → `MediaUploaded`.
    UploadMedia {
        document_id: DocumentId,
        /// Raw (unencrypted) media bytes.
        data: Vec<u8>,
        mime_type: String,
    },
    /// Download a media file. → `MediaDownloaded`.
    DownloadMedia {
        document_id: DocumentId,
        content_hash: String,
    },

    // ── Channels (Murmure) — CRUD & sharing ──────────────────────────────────
    /// List channels. → `ChannelList`.
    ListChannels,
    /// Create a channel. → `ChannelCreated`.
    CreateChannel { name: String },
    /// Delete a channel. → `ChannelDeleted`.
    DeleteChannel { channel_id: ChannelId },
    /// Invite a user to a channel. → `ChannelInviteSent`.
    InviteToChannel {
        channel_id: ChannelId,
        invitee_email: String,
        role: Role,
    },
    /// Revoke a channel member. → `ChannelRevocationComplete`.
    RevokeFromChannel {
        channel_id: ChannelId,
        user_id: UserId,
    },
    /// List channel members. → `ChannelMembers`.
    ListChannelMembers { channel_id: ChannelId },
    /// Update a channel member's role. → `ChannelMemberRoleUpdated`.
    UpdateChannelMemberRole {
        channel_id: ChannelId,
        user_id: UserId,
        role: Role,
    },
    /// Advance the read cursor for unread tracking. (Acknowledged implicitly.)
    UpdateReadCursor {
        channel_id: ChannelId,
        last_seq: u64,
    },
    /// Request unread counts for all subscribed channels. → `UnreadCounts`.
    GetUnreadCounts,

    // ── Meetings (Rendez-Vous) ───────────────────────────────────────────────
    /// Create an ephemeral meeting room. → `MeetingCreated`.
    CreateMeeting { name: String },
    /// Join a meeting. → `MeetingJoined`.
    JoinMeeting { meeting_id: MeetingId },
    /// Leave a meeting. → `MeetingLeft`.
    LeaveMeeting { meeting_id: MeetingId },
    /// End a meeting for all participants. → `MeetingEnded`.
    EndMeeting { meeting_id: MeetingId },
}

// ─────────────────────────────────────────────────────────────────────────────
// RestEvent — relayed service response (Sync → Logic → UI)
// ─────────────────────────────────────────────────────────────────────────────

/// A correlatable service response. Carried inside `LogicEvent::Rest` and
/// forwarded verbatim as `UiEvent::Rest`, echoing the originating
/// [`RequestId`].
///
/// Success responses only: failures arrive as the explicit `AuthError` /
/// `ApiError` envelope variants (which may also be spontaneous).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestEvent {
    // ── Session ──────────────────────────────────────────────────────────────
    SessionReady {
        user_id: UserId,
        display_name: String,
        email: String,
    },
    /// First connection: no keys on the server — the UI must show onboarding.
    AccountSetupRequired,
    /// Display `recovery_words` **exactly once**; they cannot be retrieved later.
    AccountCreated {
        recovery_words: String,
    },
    PassphraseChanged,
    AccountRecovered,
    LoggedOut,

    // ── Document CRUD ────────────────────────────────────────────────────────
    DocumentList {
        documents: Vec<DocumentSummary>,
    },
    /// A document was created. The Logic thread records its `doc_type` (hook)
    /// until the first snapshot so it can build the matching state.
    DocumentCreated {
        document_id: DocumentId,
        doc_type: DocType,
    },
    DocumentDeleted {
        document_id: DocumentId,
    },
    DocumentRenamed {
        document_id: DocumentId,
    },

    // ── Sharing ──────────────────────────────────────────────────────────────
    InviteSent {
        document_id: DocumentId,
        user_id: UserId,
        display_name: String,
    },
    /// Revocation initiated by **this** client is complete.
    RevocationComplete {
        document_id: DocumentId,
    },
    DocumentMembers {
        document_id: DocumentId,
        members: Vec<MemberInfo>,
    },
    MemberRoleUpdated {
        document_id: DocumentId,
        user_id: UserId,
        role: Role,
    },

    // ── Users ────────────────────────────────────────────────────────────────
    UserSearchResults {
        users: Vec<UserInfo>,
    },

    // ── Media ────────────────────────────────────────────────────────────────
    MediaUploaded {
        document_id: DocumentId,
        content_hash: String,
    },
    MediaDownloaded {
        document_id: DocumentId,
        content_hash: String,
        /// Decrypted raw media bytes, ready for display.
        data: Vec<u8>,
    },

    // ── Channels (Murmure) — CRUD & sharing ──────────────────────────────────
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
    UnreadCounts {
        counts: Vec<UnreadCount>,
    },

    // ── Meetings (Rendez-Vous) ───────────────────────────────────────────────
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
}
