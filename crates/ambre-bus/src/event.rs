//! `LogicEvent` - events sent from the Sync thread to the Logic thread.
//!
//! Two groups:
//! - [`LogicEvent::Rest`] -- a [`RestEvent`] service response answering an earlier
//!   [`SyncCommand::Rest`], echoing its [`RequestId`]. Forwarded verbatim to the
//!   UI (with a small Logic hook, for example `DocumentCreated` records the doc type).
//! - **Server push & errors** -- unsolicited notifications (CRDT deltas,
//!   connection status, member revocation, meeting participants...) and failures
//!   (`AuthError` / `ApiError` / `SetupRequired` / `UpdateRequired`). No originating request, so no
//!   `RequestId`; each is handled explicitly by the Logic thread (apply CRDT,
//!   mutate state, ...).
//!
//! [`SyncCommand::Rest`]: crate::SyncCommand::Rest

use serde::{Deserialize, Serialize};

use crate::sync_warning::SyncWarningCode;
use crate::{ChannelId, DocType, DocumentId, RequestId, RestEvent, Role, SnapshotId, UserId};
use ambre_core::SessionNonce;

// -----------------------------------------------------------------------------
// LogicEvent
// -----------------------------------------------------------------------------

/// Event sent from the Sync thread to the Logic thread. Crosses the Sync-worker
/// `postMessage` boundary, hence postcard-serialized (`Serialize`/`Deserialize`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogicEvent {
    // -- Service plane (relayed request/response) ------------------------------
    /// A [`RestEvent`] service response echoing the originating [`RequestId`].
    Rest {
        id: RequestId,
        evt: RestEvent,
    },

    // -- Real-time sync (server push) ------------------------------------------
    /// The identity this session writes under in a document, sent once per open
    /// (`ambre-spec-op-authorship.md`). The Logic thread sets `peer` on the
    /// document, and writes `entry` into its peer registry with its first delta.
    /// It applies no mutating command to the document before this arrives.
    SessionIdentity {
        document_id: DocumentId,
        /// The PeerID, derived from the signing key and the session nonce.
        peer: u64,
        /// The encoded registry entry of `peer` (`ambre_core::PeerEntry`).
        entry: Vec<u8>,
        /// The nonce `peer` derives from. The Logic thread keeps it in the
        /// local store, so a later open can send what this session left
        /// unsent (`AUTHOR-028`).
        nonce: SessionNonce,
    },

    /// Decrypted and decompressed CRDT delta, with its signer, ready for `LoroDoc::import()`.
    RemoteUpdate {
        document_id: DocumentId,
        delta: SignedDelta,
    },

    /// A batch of decrypted CRDT deltas to import as one merge+reseed. Emitted by a server
    /// history replay (`WsServerMsg::MissingUpdates` on (re)open). The Logic thread imports
    /// them all before repainting. A cell that only appears in the last delta does not
    /// flicker in across N intermediate re-renders (one per delta). Order is preserved
    /// (server `seq` order).
    RemoteUpdateBatch {
        document_id: DocumentId,
        deltas: Vec<SignedDelta>,
    },

    /// Decrypted and decompressed CRDT snapshot.
    RemoteSnapshot {
        document_id: DocumentId,
        payload: Vec<u8>,
        /// The last delta `seq` it folds in, 0 when unknown. The Logic thread
        /// counts the deltas a later snapshot must hold from it (`AUTHOR-016`).
        covered_seq: u64,
    },

    /// A snapshot to check before anything trusts it
    /// (`ambre-spec-op-authorship.md`, `AUTHOR-016`, `AUTHOR-022`). The Logic
    /// thread compares it with the deltas it holds and answers with
    /// [`SyncCommand::SnapshotVerdict`](crate::SyncCommand::SnapshotVerdict).
    PendingSnapshot {
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        /// The last delta `seq` it claims to fold in.
        covered_seq: u64,
        /// Decrypted and decompressed, as in `RemoteSnapshot`.
        payload: Vec<u8>,
        purpose: SnapshotPurpose,
    },

    /// Decrypted ephemeral data (cursor, collaborator presence). `from_user_id` is the
    /// sender's canonical `UserId`, resolved from the authenticated `pub_key` against the
    /// document's member list before delivery. Presence is therefore keyed by a spoof-proof
    /// per-user identity (`sync`, "Presence identity").
    RemoteEphemeral {
        document_id: DocumentId,
        from_user_id: UserId,
        payload: Vec<u8>,
    },

    /// A peer stopped watching a document (`PRESENCE-014`): they unsubscribed, or their
    /// connection closed. `user_id` is resolved from the departing `pub_key` the relay
    /// named, by the same path an ephemeral takes. A departure therefore cannot be
    /// attributed to a peer who did not leave.
    ///
    /// Best-effort, like presence itself: a lost message leaves the peer to age out of the
    /// live set instead.
    PeerLeft {
        document_id: DocumentId,
        user_id: UserId,
    },

    /// Initial hydration of a freshly opened document is complete. The Sync thread has
    /// fetched members and the latest snapshot (if any), and emitted it ahead of this
    /// event. Fires exactly once per open, for both the snapshot and the empty/brand-new
    /// paths, so the UI can dismiss its document-loading overlay. An empty document never
    /// produces a snapshot, so the overlay cannot key off `RemoteSnapshot`.
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

    /// The activity counted for a document not held in memory, read from the
    /// summaries alone (`RESID-005`, `RESID-014`).
    Activity {
        document_id: DocumentId,
        counters: ambre_core::activity::ActivityCounters,
    },

    /// A released document turned into a light subscription: the relay
    /// acknowledged every frame this client sent of it (`RESID-009`). Its
    /// local copy holds nothing the relay lacks (`RESID-019`).
    Released {
        document_id: DocumentId,
    },

    /// Non-fatal warning, such as a quarantine overflow or a decrypt failure.
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

    // -- Channels (Murmure) -- real-time (server push) --------------------------
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

    // -- Meetings (Rendez-Vous) -- participant push -----------------------------
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

    // -- Errors ----------------------------------------------------------------
    /// Authentication error (incorrect passphrase, corrupted data, network).
    AuthError {
        message: String,
    },

    /// Generic error for any REST/WS operation. `request` identifies the source command
    /// (for example, "ListDocuments").
    ///
    /// `id` echoes the originating [`RequestId`] when the failed command carried one (a
    /// `SyncCommand::Rest`). The dispatcher can then route a correlated failure back to
    /// the issuing document. In `toile-media`, a failed media upload must reach the
    /// pending import waiting on it. `None` for a spontaneous error with no originating
    /// request.
    ApiError {
        id: Option<RequestId>,
        request: String,
        code: u16,
        message: String,
    },

    /// The relay refused a request or the socket because the account still owes
    /// its key setup. Stands in for the generic error such a refusal would
    /// otherwise raise (`ambre-spec-sync.md`, `SYNC-134`).
    SetupRequired,

    /// The relay speaks a newer wire version than this build. Only an update
    /// cures it, so the socket stays closed (`ambre-spec-sync.md`, `SYNC-137`).
    UpdateRequired,
}

/// Why a snapshot is checked (`ambre-spec-op-authorship.md` section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotPurpose {
    /// A pending snapshot, which a match confirms (`AUTHOR-016`). Every delta
    /// it covers must be held, or the check proves nothing.
    Confirm,
    /// A snapshot only its author confirmed, read on open (`AUTHOR-022`). The
    /// relay trimmed that author's deltas, so gaps in the seqs are expected.
    Open,
}

/// What the Logic thread found when it checked a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotVerdict {
    /// Every operation both sides hold is the same. `extras` names each PeerID
    /// only the snapshot holds, with its registry entry. The snapshot passes
    /// only if each entry resolves to its author (`AUTHOR-021`).
    Match { extras: Vec<(u64, Vec<u8>)> },
    /// The snapshot contradicts a delta it covers (`AUTHOR-017`).
    Mismatch,
    /// The client lacks what a verdict needs, and says nothing.
    Unchecked,
}

/// One opened update and what its signature proves about it
/// (`ambre-spec-sealed-envelope.md`, `SYNC-114`).
///
/// The Sync worker derives `peer` from the signer key and `nonce`. The Logic
/// thread refuses the delta when one of its operations carries another PeerID
/// (`ambre-spec-op-authorship.md`, `AUTHOR-012`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedDelta {
    /// The number the relay gave it. The Logic thread's snapshot check reads
    /// which deltas a horizon covers from it (`AUTHOR-016`).
    pub seq: u64,
    /// The Loro update, decrypted and decompressed.
    pub payload: Vec<u8>,
    /// The session nonce of its author, opened from inside the ciphertext.
    pub nonce: SessionNonce,
    /// The only PeerID its operations may carry.
    pub peer: u64,
    /// The member the signing key resolves to, `None` when the directory does
    /// not name it (`AUTHOR-013`).
    pub user: Option<UserId>,
}
