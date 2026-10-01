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

use serde::{Deserialize, Serialize};

use crate::{ChannelId, DocType, DocumentId, RequestId, RestCommand};

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

    /// Raw CRDT delta produced by `LoroDoc::export_updates()`.
    SendUpdate {
        document_id: DocumentId,
        payload: Vec<u8>,
    },

    /// Full CRDT state for compaction (presigned URL -> S3).
    SendSnapshot {
        document_id: DocumentId,
        payload: Vec<u8>,
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

    // -- Channel sync session (control) ----------------------------------------
    OpenChannel {
        channel_id: ChannelId,
    },
    CloseChannel {
        channel_id: ChannelId,
    },
    /// The Sync thread manages segments internally (creation, sealing, routing);
    /// the Logic thread sends a raw payload routed into the active segment.
    SendChannelUpdate {
        channel_id: ChannelId,
        payload: Vec<u8>,
    },
    SendChannelEphemeral {
        channel_id: ChannelId,
        payload: Vec<u8>,
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
