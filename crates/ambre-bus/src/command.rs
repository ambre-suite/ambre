//! `SyncCommand` – commands sent from the Logic Worker to the Sync Worker.
//!
//! This enum contains every operation that the Sync Worker can perform on behalf
//! of the Logic Worker: session management, document CRUD, real‑time sync,
//! sharing, users, media, channels, meetings, and lifecycle.
//!
//! All payloads are opaque `Vec<u8>` that the Sync Worker must interpret
//! (e.g., CRDT deltas, media blobs). The Sync Worker is responsible for
//! encryption, compression, and network transmission.

use crate::{DocType, Role, Uuid};
use serde::{Deserialize, Serialize};

/// Command sent from the Logic Worker to the Sync Worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncCommand {
    // ── Session ──────────────────────────────────────────────────────────────
    /// Bootstrap auth after OIDC callback.
    ///
    /// The Sync Worker fetches `/api/keys/master` and `/api/auth/me`,
    /// derives the User Key via Argon2id(passphrase, salt), decrypts the
    /// MasterSeed, derives the keypairs.
    ///
    /// Responds with `SessionReady`, `AccountSetupRequired`, or `AuthError`.
    Unlock {
        passphrase: Vec<u8>,
    },

    /// First‑time account creation – generate MasterSeed and upload public keys.
    ///
    /// The Sync Worker generates the MasterSeed (CSPRNG), derives the keypairs,
    /// encrypts with Argon2id(passphrase) → `PUT /api/keys/master`.
    ///
    /// Responds with `AccountCreated { recovery_words }`.
    SetupAccount {
        passphrase: Vec<u8>,
    },

    /// Change the account passphrase.
    ///
    /// The Sync Worker re‑derives the User Key, re‑encrypts `master_key_enc`
    /// → `PUT /api/keys/master/rotate`.
    ChangePassphrase {
        current_passphrase: Vec<u8>,
        new_passphrase: Vec<u8>,
    },

    /// Account recovery via 24‑word BIP39 recovery phrase.
    ///
    /// The Sync Worker reconstructs the MasterSeed → re‑encrypts
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
    /// The Sync Worker fetches `GET /api/documents` and decrypts `name_enc`
    /// for each document.
    ListDocuments,

    /// Create a new document.
    ///
    /// The Sync Worker generates a DocumentKey (CSPRNG), encrypts `name_enc`,
    /// `POST /api/documents`, `POST /api/documents/{id}/share` (self).
    CreateDocument {
        doc_type: DocType,
        name: String,
    },

    /// Delete a document. `DELETE /api/documents/{id}`.
    DeleteDocument {
        document_id: Uuid,
    },

    /// Rename a document.
    ///
    /// The Sync Worker encrypts `name_enc` → `PATCH /api/documents/{id}`.
    RenameDocument {
        document_id: Uuid,
        name: String,
    },

    // ── Sync temps réel ──────────────────────────────────────────────────────
    /// Open a sync session for a document.
    ///
    /// The Sync Worker fetches members + `doc_key_enc`, decrypts the DocumentKey,
    /// loads snapshot + deltas, opens WebSocket. Also used for embedded documents
    /// (read‑only).
    OpenDocument {
        document_id: Uuid,
    },

    /// Close the sync session. Close WebSocket, zeroize DocumentKey.
    CloseDocument {
        document_id: Uuid,
    },

    /// Raw CRDT delta produced by `LoroDoc::export_updates()`.
    SendUpdate {
        document_id: Uuid,
        payload: Vec<u8>,
    },

    /// Full CRDT state for compaction (presigned URL → S3).
    SendSnapshot {
        document_id: Uuid,
        payload: Vec<u8>,
    },

    /// Cursor / presence data (encrypted by the Sync Worker before sending).
    SendEphemeral {
        document_id: Uuid,
        payload: Vec<u8>,
    },

    /// Reconnect WebSocket after a disconnect.
    Reconnect {
        document_id: Uuid,
    },

    // ── Partage ──────────────────────────────────────────────────────────────
    /// Invite a user to a document.
    ///
    /// The Sync Worker fetches the invitee's public keys, encrypts the
    /// DocumentKey for them → `POST /api/documents/{id}/share`.
    InviteToDocument {
        document_id: Uuid,
        invitee_email: String,
        role: Role,
    },

    /// Revoke a member from a document.
    ///
    /// The Sync Worker orchestrates the whole sequence:
    /// `DELETE /share/{uid}` → generate new DocumentKey → encrypt for remaining
    /// members → `POST /rotate-key`. The broadcast `WsServerMsg::KeyRotationRequired`
    /// is emitted by the server on `POST /rotate-key` (when the new keys are in DB).
    RevokeFromDocument {
        document_id: Uuid,
        user_id: Uuid,
    },

    /// List members of a document.
    ListDocumentMembers {
        document_id: Uuid,
    },

    /// Update a member's role.
    UpdateDocumentMemberRole {
        document_id: Uuid,
        user_id: Uuid,
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
    /// The Sync Worker encrypts, fetches presigned URL, PUT S3, then
    /// `POST /api/documents/{id}/media` to confirm the upload (INSERT Postgres).
    UploadMedia {
        document_id: Uuid,
        data: Vec<u8>,
        mime_type: String,
    },

    /// Download a media file.
    ///
    /// The Sync Worker fetches presigned URL, GET S3, decrypts.
    DownloadMedia {
        document_id: Uuid,
        content_hash: String,
    },

    // ── Channels (Murmure) ───────────────────────────────────────────────────
    ListChannels,
    CreateChannel {
        name: String,
    },
    DeleteChannel {
        channel_id: Uuid,
    },
    InviteToChannel {
        channel_id: Uuid,
        invitee_email: String,
        role: Role,
    },
    RevokeFromChannel {
        channel_id: Uuid,
        user_id: Uuid,
    },
    ListChannelMembers {
        channel_id: Uuid,
    },
    UpdateChannelMemberRole {
        channel_id: Uuid,
        user_id: Uuid,
        role: Role,
    },
    OpenChannel {
        channel_id: Uuid,
    },
    CloseChannel {
        channel_id: Uuid,
    },
    /// The Sync Worker manages segments internally (creation, sealing, routing).
    /// The Logic Worker sends a raw payload – the Sync Worker routes it into the
    /// active segment.
    SendChannelUpdate {
        channel_id: Uuid,
        payload: Vec<u8>,
    },
    SendChannelEphemeral {
        channel_id: Uuid,
        payload: Vec<u8>,
    },
    UpdateReadCursor {
        channel_id: Uuid,
        last_seq: u64,
    },
    GetUnreadCounts,

    // ── Meetings (Rendez‑Vous) ───────────────────────────────────────────────
    CreateMeeting {
        name: String,
    },
    JoinMeeting {
        meeting_id: Uuid,
    },
    LeaveMeeting {
        meeting_id: Uuid,
    },
    EndMeeting {
        meeting_id: Uuid,
    },

    // ── Lifecycle ────────────────────────────────────────────────────────────
    Shutdown,
}
