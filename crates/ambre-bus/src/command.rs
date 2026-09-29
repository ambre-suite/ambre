//! `SyncCommand` - commands sent from the Logic thread to the Sync thread.
//!
//! Three groups:
//! - [`SyncCommand::Rest`] -- the correlatable service family ([`RestCommand`]),
//!   forwarded verbatim from [`LogicCommand::Rest`]. The Sync thread performs the
//!   REST / crypto work and answers with [`LogicEvent::Rest`].
//! - **Control & content-send** -- document/channel session control and CRDT /
//!   ephemeral byte payloads (`Vec<u8>` = genuine network bytes the Sync thread
//!   encrypts/compresses/transmits).
//! - **Flow control** -- `HoldInbound` and `ReleaseInbound`, which the Main
//!   relay posts when the path back to the Logic thread fills and drains.
//! - **Lifecycle** -- `Shutdown`.
//!
//! Crosses a `postMessage` boundary from the Main instance to the separate Sync
//! worker, so it is postcard-serialized on that hop (hence `Serialize`/
//! `Deserialize`). The `Vec<u8>` payloads are genuine network bytes; large ones
//! ride a Transferable `ArrayBuffer` to stay zero-copy.
//!
//! [`LogicCommand::Rest`]: crate::LogicCommand::Rest
//! [`LogicEvent::Rest`]: crate::LogicEvent::Rest

use ambre_core::SessionNonce;
use serde::{Deserialize, Serialize};

use crate::{
    ActivitySummary, ChannelId, DocType, DocumentId, RequestId, RestCommand, SnapshotId,
    SnapshotPurpose, SnapshotVerdict,
};

/// Command sent from the Logic thread to the Sync thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncCommand {
    // -- Service plane (relayed request/response) ------------------------------
    /// A [`RestCommand`] service operation forwarded from the Logic thread. The
    /// Sync thread answers with [`crate::LogicEvent::Rest`] echoing `id`.
    Rest {
        id: RequestId,
        cmd: RestCommand,
    },

    // -- Document sync session (control) ---------------------------------------
    /// Open a sync session for a document.
    ///
    /// The Sync thread fetches members + `doc_key_enc`, decrypts the DocumentKey,
    /// loads snapshot + deltas, opens a WebSocket subscription.
    OpenDocument {
        document_id: DocumentId,
        /// Document kind, used to lazily create the document server-side (with the
        /// right type) on its first modification if it does not yet exist.
        doc_type: DocType,
    },

    /// Close the sync session. Unsubscribe, zeroize the DocumentKey.
    CloseDocument {
        document_id: DocumentId,
    },

    /// The Logic thread evicted a document from memory (`RESID-006`). Sync
    /// keeps a light subscription in place of the full session, which counts
    /// its activity and resumes a later open after `applied_seq`
    /// (`RESID-004`, `RESID-010`).
    ReleaseDocument {
        document_id: DocumentId,
        /// The highest relay number the evicted copy holds.
        applied_seq: u64,
    },

    /// Raw CRDT delta produced by `LoroDoc::export_updates()`.
    SendUpdate {
        document_id: DocumentId,
        payload: Vec<u8>,
        /// What the delta holds, when the application wrote a summary
        /// (`ambre-spec-residency.md`). A Murmure message carries one
        /// (the Murmure messages spec). The Sync worker seals it inside the ciphertext
        /// (`SYNC-113`), or drops it while that seal is not built.
        summary: Option<ActivitySummary>,
    },

    /// The operations an earlier session of this user wrote and never sent.
    /// They are sealed with that session's nonce, so a reader derives the
    /// PeerID that wrote them (`AUTHOR-028`).
    SendUnsent {
        document_id: DocumentId,
        payload: Vec<u8>,
        nonce: SessionNonce,
    },

    /// Full CRDT state for compaction (presigned URL -> S3).
    SendSnapshot {
        document_id: DocumentId,
        payload: Vec<u8>,
        /// The last delta `seq` the Logic thread holds with no gap below it.
        /// The snapshot claims to fold in every delta up to it (`SRV-083`).
        covered_seq: u64,
    },

    /// The Logic thread's verdict on a [`LogicEvent::PendingSnapshot`]
    /// (`ambre-spec-op-authorship.md`, `AUTHOR-016`, `AUTHOR-017`).
    ///
    /// [`LogicEvent::PendingSnapshot`]: crate::LogicEvent::PendingSnapshot
    SnapshotVerdict {
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        purpose: SnapshotPurpose,
        verdict: SnapshotVerdict,
    },

    /// Cursor / presence data (encrypted by the Sync thread before sending).
    SendEphemeral {
        document_id: DocumentId,
        payload: Vec<u8>,
    },

    /// Reconnect the WebSocket after a disconnect.
    Reconnect {
        document_id: DocumentId,
    },

    // -- Channel (control) -----------------------------------------------------
    /// List the documents a channel groups. Answered by
    /// [`crate::LogicEvent::ChannelDocuments`], or by
    /// [`crate::LogicEvent::ChannelOpenFailed`]. Each document then opens
    /// through [`SyncCommand::OpenDocument`]: a channel has no session of its
    /// own (`SRV-102`).
    OpenChannel {
        channel_id: ChannelId,
    },

    // -- Flow control (posted by the Main relay, never by Logic) ---------------
    /// The path back to the Logic thread is full: stop reading the socket
    /// until [`SyncCommand::ReleaseInbound`] (`SYNC-059`).
    HoldInbound,
    /// The path back has room again: reopen the socket and catch up.
    ReleaseInbound,

    // -- Lifecycle -------------------------------------------------------------
    Shutdown,
}
