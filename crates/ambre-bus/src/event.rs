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
use crate::{ChannelId, DocType, DocumentId, RequestId, RestEvent, Role, UserId};

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

    /// A batch of decrypted CRDT deltas to import as one merge+reseed. Emitted by
    /// a server history replay (`WsServerMsg::MissingUpdates` on (re)open): the
    /// Logic thread imports them all before repainting, so a cell that only
    /// appears in the last delta does not flicker in across N intermediate
    /// re-renders (one per delta). Order is preserved (server `seq` order).
    RemoteUpdateBatch {
        document_id: DocumentId,
        payloads: Vec<Vec<u8>>,
    },

    /// Decrypted and decompressed CRDT snapshot.
    RemoteSnapshot {
        document_id: DocumentId,
        payload: Vec<u8>,
    },

    /// Decrypted ephemeral data (cursor, collaborator presence). `from_user_id`
    /// is the sender's canonical `UserId`, resolved from the authenticated
    /// `pub_key` against the document's member list before delivery, so presence
    /// is keyed by a spoof-proof per-user identity (`sync`, "Presence
    /// identity").
    RemoteEphemeral {
        document_id: DocumentId,
        from_user_id: UserId,
        payload: Vec<u8>,
    },

    /// A peer stopped watching a document (`PRESENCE-014`): they unsubscribed,
    /// or their connection closed. `user_id` is resolved from the departing
    /// `pub_key` the relay named, by the same path an ephemeral takes, so a
    /// departure cannot be attributed to a peer who did not leave.
    ///
    /// Best-effort, like presence itself: a lost message leaves the peer to age
    /// out of the live set instead.
    PeerLeft {
        document_id: DocumentId,
        user_id: UserId,
    },

    /// Initial hydration of a freshly opened document is complete: the Sync
    /// thread has fetched members and the latest snapshot (if any) and emitted
    /// it ahead of this event. Fires exactly once per open, for both the
    /// snapshot and the empty/brand-new paths, so the UI can dismiss its
    /// document-loading overlay (an empty document never produces a snapshot,
    /// so the overlay cannot key off `RemoteSnapshot`).
    DocumentReady {
        document_id: DocumentId,
        /// This client's own role on the document, resolved by Sync from the
        /// members response (`architecture`). Logic keeps it beside
        /// the live `DocState` and drops mutating commands when it is not
        /// writable. The gate is UX only; the server remains the sole authority.
        role: Role,
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

    /// A document was shared with this user (server push on `user:{id}`,
    /// translated from `WsServerMsg::DocumentShared`). The Logic thread forwards
    /// it to the UI, which refreshes its document list and notifies the user.
    DocumentShared {
        document_id: DocumentId,
        doc_type: DocType,
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
    ///
    /// `id` echoes the originating [`RequestId`] when the failed command carried
    /// one (a `SyncCommand::Rest`), so the dispatcher can route a correlated
    /// failure back to the document that issued it (`toile-media`: a
    /// failed media upload must reach the pending import that is waiting on it).
    /// `None` for a spontaneous error with no originating request.
    ApiError {
        id: Option<RequestId>,
        request: String,
        code: u16,
        message: String,
    },
}
