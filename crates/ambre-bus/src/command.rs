//! `SyncCommand` – commands sent from the Logic thread to the Sync thread.
//!
//! This enum contains every operation that the Sync thread can perform on behalf
//! of the Logic thread: session management, document CRUD, real‑time sync,
//! sharing, users, media, channels, meetings, and lifecycle.
//!
//! Binary fields (`Vec<u8>`) carry genuine **network** bytes the Sync thread
//! must interpret (CRDT deltas, snapshots, media blobs); every other field is a
//! typed value. The Sync thread is responsible for encryption, compression, and
//! network transmission.
//!
//! Carried over a `flume` channel in shared memory as a typed Rust value by
//! move — nothing here is serialized (postcard lives only on the network path,
//! in `ambre-protocol`).

use crate::{ChannelId, DocType, DocumentId, MeetingId, Role, UserId};

/// Command sent from the Logic thread to the Sync thread.
#[derive(Debug, Clone)]
pub enum SyncCommand {
    // ── Session ──────────────────────────────────────────────────────────────
    /// Bootstrap auth after OIDC callback.
    ///
    /// The Sync thread fetches `/api/keys/master` and `/api/auth/me`,
    /// derives the User Key via Argon2id(passphrase, salt), decrypts the
    /// MasterSeed, derives the keypairs.
    ///
    /// Responds with `SessionReady`, `AccountSetupRequired`, or `AuthError`.
    Unlock {
        passphrase: Vec<u8>,
    },

    /// First‑time account creation – generate MasterSeed and upload public keys.
    ///
    /// The Sync thread generates the MasterSeed (CSPRNG), derives the keypairs,
    /// encrypts with Argon2id(passphrase) → `PUT /api/keys/master`.
    ///
    /// Responds with `AccountCreated { recovery_words }`.
    SetupAccount {
        passphrase: Vec<u8>,
    },

    /// Change the account passphrase.
    ///
    /// The Sync thread re‑derives the User Key, re‑encrypts `master_key_enc`
    /// → `PUT /api/keys/master/rotate`.
    ChangePassphrase {
        current_passphrase: Vec<u8>,
        new_passphrase: Vec<u8>,
    },

    /// Account recovery via 24‑word BIP39 recovery phrase.
    ///
    /// The Sync thread reconstructs the MasterSeed → re‑encrypts
    /// → `PUT /api/keys/master/rotate`.
    RecoverAccount {
        recovery_words: String,
        new_passphrase: Vec<u8>,
    },

    /// Log out. `POST /api/auth/logout`, zeroize all keys.
    Logout,

    // ── Documents CRUD ───────────────────────────────────────────────────────
    /// List accessible documents.
    ///
    /// The Sync thread fetches `GET /api/documents` and decrypts `name_enc`
    /// for each document.
    ListDocuments,

    /// Create a new document.
    ///
    /// The Sync thread generates a DocumentKey (CSPRNG), encrypts `name_enc`,
    /// `POST /api/documents`, `POST /api/documents/{id}/share` (self).
    CreateDocument {
        doc_type: DocType,
        name: String,
    },

    /// Delete a document. `DELETE /api/documents/{id}`.
    DeleteDocument {
        document_id: DocumentId,
    },

    /// Rename a document.
    ///
    /// The Sync thread encrypts `name_enc` → `PATCH /api/documents/{id}`.
    RenameDocument {
        document_id: DocumentId,
        name: String,
    },

    // ── Sync temps réel ──────────────────────────────────────────────────────
    /// Open a sync session for a document.
    ///
    /// The Sync thread fetches members + `doc_key_enc`, decrypts the DocumentKey,
    /// loads snapshot + deltas, opens WebSocket. Also used for embedded documents
    /// (read‑only).
    OpenDocument {
        document_id: DocumentId,
    },

    /// Close the sync session. Close WebSocket, zeroize DocumentKey.
    CloseDocument {
        document_id: DocumentId,
    },

    /// Raw CRDT delta produced by `LoroDoc::export_updates()`.
    SendUpdate {
        document_id: DocumentId,
        payload: Vec<u8>,
    },

    /// Full CRDT state for compaction (presigned URL → S3).
    SendSnapshot {
        document_id: DocumentId,
        payload: Vec<u8>,
    },

    /// Cursor / presence data (encrypted by the Sync thread before sending).
    SendEphemeral {
        document_id: DocumentId,
        payload: Vec<u8>,
    },

    /// Reconnect WebSocket after a disconnect.
    Reconnect {
        document_id: DocumentId,
    },

    // ── Partage ──────────────────────────────────────────────────────────────
    /// Invite a user to a document.
    ///
    /// The Sync thread fetches the invitee's public keys, encrypts the
    /// DocumentKey for them → `POST /api/documents/{id}/share`.
    InviteToDocument {
        document_id: DocumentId,
        invitee_email: String,
        role: Role,
    },

    /// Revoke a member from a document.
    ///
    /// The Sync thread orchestrates the whole sequence:
    /// `DELETE /share/{uid}` → generate new DocumentKey → encrypt for remaining
    /// members → `POST /rotate-key`. The broadcast `WsServerMsg::KeyRotationRequired`
    /// is emitted by the server on `POST /rotate-key` (when the new keys are in DB).
    RevokeFromDocument {
        document_id: DocumentId,
        user_id: UserId,
    },

    /// List members of a document.
    ListDocumentMembers {
        document_id: DocumentId,
    },

    /// Update a member's role.
    UpdateDocumentMemberRole {
        document_id: DocumentId,
        user_id: UserId,
        role: Role,
    },

    // ── Utilisateurs ─────────────────────────────────────────────────────────
    /// Search users by email (for invitations).
    SearchUsers {
        query: String,
    },

    // ── Médias ───────────────────────────────────────────────────────────────
    /// Upload a media file.
    ///
    /// The Sync thread encrypts, fetches presigned URL, PUT S3, then
    /// `POST /api/documents/{id}/media` to confirm the upload (INSERT Postgres).
    UploadMedia {
        document_id: DocumentId,
        data: Vec<u8>,
        mime_type: String,
    },

    /// Download a media file.
    ///
    /// The Sync thread fetches presigned URL, GET S3, decrypts.
    DownloadMedia {
        document_id: DocumentId,
        content_hash: String,
    },

    // ── Channels (Murmure) ───────────────────────────────────────────────────
    ListChannels,
    CreateChannel {
        name: String,
    },
    DeleteChannel {
        channel_id: ChannelId,
    },
    InviteToChannel {
        channel_id: ChannelId,
        invitee_email: String,
        role: Role,
    },
    RevokeFromChannel {
        channel_id: ChannelId,
        user_id: UserId,
    },
    ListChannelMembers {
        channel_id: ChannelId,
    },
    UpdateChannelMemberRole {
        channel_id: ChannelId,
        user_id: UserId,
        role: Role,
    },
    OpenChannel {
        channel_id: ChannelId,
    },
    CloseChannel {
        channel_id: ChannelId,
    },
    /// The Sync thread manages segments internally (creation, sealing, routing).
    /// The Logic thread sends a raw payload – the Sync thread routes it into the
    /// active segment.
    SendChannelUpdate {
        channel_id: ChannelId,
        payload: Vec<u8>,
    },
    SendChannelEphemeral {
        channel_id: ChannelId,
        payload: Vec<u8>,
    },
    UpdateReadCursor {
        channel_id: ChannelId,
        last_seq: u64,
    },
    GetUnreadCounts,

    // ── Meetings (Rendez‑Vous) ───────────────────────────────────────────────
    CreateMeeting {
        name: String,
    },
    JoinMeeting {
        meeting_id: MeetingId,
    },
    LeaveMeeting {
        meeting_id: MeetingId,
    },
    EndMeeting {
        meeting_id: MeetingId,
    },

    // ── Lifecycle ────────────────────────────────────────────────────────────
    Shutdown,
}
