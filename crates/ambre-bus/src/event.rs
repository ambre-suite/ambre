//! `LogicEvent` – events sent from the Sync thread to the Logic thread.
//!
//! Two groups:
//! - [`LogicEvent::Rest`] — a [`RestEvent`] service response answering an earlier
//!   [`SyncCommand::Rest`], echoing its [`RequestId`]. Forwarded verbatim to the
//!   UI (with a small Logic hook, e.g. `DocumentCreated` records the doc type).
//! - **Server push & errors** — unsolicited notifications (CRDT deltas,
//!   connection status, member revocation, meeting participants…) and failures
//!   (`AuthError` / `ApiError`). No originating request, so no `RequestId`; each
//!   is handled explicitly by the Logic thread (apply CRDT, mutate state, …).
//!
//! [`SyncCommand::Rest`]: crate::SyncCommand::Rest

use serde::{Deserialize, Serialize};

use crate::sync_warning::SyncWarningCode;
use crate::{ChannelId, DocumentId, RequestId, RestEvent, UserId};

// ─────────────────────────────────────────────────────────────────────────────
// LogicEvent
// ─────────────────────────────────────────────────────────────────────────────

/// Event sent from the Sync thread to the Logic thread. Crosses the Sync-worker
/// `postMessage` boundary, hence postcard-serialized (`Serialize`/`Deserialize`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogicEvent {
    // ── Service plane (relayed request/response) ──────────────────────────────
    /// A [`RestEvent`] service response echoing the originating [`RequestId`].
    Rest {
        id: RequestId,
        evt: RestEvent,
    },

    // ── Real-time sync (server push) ──────────────────────────────────────────
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

    /// Non-fatal warning (quarantine overflow, decrypt failure, etc.).
    SyncWarning {
        document_id: DocumentId,
        code: SyncWarningCode,
        message: String,
    },

    /// The server requests a snapshot (too many accumulated deltas).
    /// The Logic thread must produce `LoroDoc::export_snapshot()` and respond
    /// with `SyncCommand::SendSnapshot { document_id, payload }`.
    SnapshotNeeded {
        document_id: DocumentId,
    },

    /// A member has been revoked and consolidation is complete.
    /// Translated from `WsServerMsg::KeyRotationRequired` by the Sync thread
    /// **after** the rotation is complete (new keys in DB). Server-initiated:
    /// the Logic thread may trigger a snapshot to seal the state.
    MemberRevoked {
        document_id: DocumentId,
        revoked_user_id: UserId,
    },

    // ── Channels (Murmure) — real-time (server push) ──────────────────────────
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

    // ── Meetings (Rendez-Vous) — participant push ─────────────────────────────
    /// Another participant joined the meeting.
    MeetingParticipantJoined {
        meeting_id: crate::MeetingId,
        user_id: UserId,
    },
    /// Another participant left the meeting.
    MeetingParticipantLeft {
        meeting_id: crate::MeetingId,
        user_id: UserId,
    },

    // ── Errors ────────────────────────────────────────────────────────────────
    /// Authentication error (incorrect passphrase, corrupted data, network).
    AuthError {
        message: String,
    },

    /// Generic error for any REST/WS operation.
    /// `request` identifies the source command (e.g., "ListDocuments").
    ApiError {
        request: String,
        code: u16,
        message: String,
    },
}
