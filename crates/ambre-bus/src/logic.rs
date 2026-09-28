//! Logic-bound protocol -- messages addressed to the Logic thread from the UI.
//!
//! Holds the [`LogicCommand`] envelope (UI Thread -> Logic thread) plus
//! [`AppCommand`], the closed sum over the document types it carries. Each
//! per-app command enum lives with that app's whole data plane under
//! [`crate::app`] (for example `crate::app::trame::TrameCommand`); `AppCommand` is a
//! thin dispatch enum referencing them.
//!
//! Carried over a `conduit` queue in shared memory -- typed Rust values by move,
//! no serialization. The symmetric Logic -> UI direction (events) lives in
//! [`crate::ui`].
//!
//! # Three planes, one envelope
//! - **Content** ([`LogicCommand::App`]): document edits / viewport requests,
//!   routed by `DocumentId` inside the Logic thread. [`AppCommand`] is a closed
//!   sum: adding a document type is adding a variant. No trait object, no `Any`,
//!   no crate cycle -- this module depends only on `ambre-core`.
//! - **Service** ([`LogicCommand::Rest`]): the correlatable request/response
//!   family, defined once in [`crate::rest`] and forwarded verbatim to Sync.
//! - **Control** (`OpenDocument`, `CloseDocument`, `OpenChannel`, `CloseChannel`,
//!   `SendChannelMessage`): fire-and-forget session/content-send commands with
//!   no single response to correlate -- kept explicit.

use crate::{ChannelId, DocType, DocumentId, RequestId, RestCommand};

// The typed application data plane carried by `LogicCommand::App` lives per-app under
// `crate::app`, command + event + value types grouped by document type. This envelope
// module only holds the `LogicCommand` sum and the `AppCommand` dispatch enum that
// references those app modules.

// -----------------------------------------------------------------------------
// LogicCommand -- UI Thread -> Logic thread
// -----------------------------------------------------------------------------

/// Message sent from the UI Thread to the Logic thread.
///
/// - [`App`](LogicCommand::App) (**P2**): user edits / viewport requests, < 50 ms
///   budget (scroll requests < 10 ms). Routed to the document's state.
/// - [`Rest`](LogicCommand::Rest) (**P3**): a [`RestCommand`] service operation,
///   forwarded to the Sync thread as [`SyncCommand::Rest`]. A small Logic hook
///   covers the few ops that also mutate Logic state, for example `Logout`.
/// - Control variants (**P3**): forwarded to the Sync thread as their explicit
///   `SyncCommand` counterparts.
///
/// [`SyncCommand::Rest`]: crate::SyncCommand::Rest
#[derive(Debug, Clone)]
pub enum LogicCommand {
    // -- P2 -- Content plane ----------------------------------------------------
    /// Application-specific command for an open document (cell edit, viewport
    /// scroll in Trame; text insertion in Prose; element move in Toile; ...).
    ///
    /// The Logic dispatcher routes [`command`](AppCommand) to the state owning
    /// `document_id` and replies with [`crate::ui::UiEvent::Viewport`] /
    /// [`crate::ui::UiEvent::Presence`] events.
    App {
        document_id: DocumentId,
        /// Typed application command (schema owned by [`crate::logic`]).
        command: AppCommand,
    },

    /// Restore a document's CRDT state from a locally persisted snapshot
    /// (IndexedDB), dispatched by the Main thread at startup. Local-only: it
    /// never reaches Sync or the server -- it lets an edit survive a refresh
    /// without a network round-trip.
    RestoreLocalSnapshot {
        document_id: DocumentId,
        /// Document kind, known from the boot URL route (`/trame/<id>`). Lets the
        /// handler materialise the live document if the restore wins the startup
        /// race against [`OpenDocument`](LogicCommand::OpenDocument).
        doc_type: DocType,
        /// Full Loro snapshot (`ExportMode::Snapshot`) read back from local
        /// storage, when one was saved.
        snapshot: Option<Vec<u8>>,
        /// The deltas logged since that snapshot, in no set order. A Loro
        /// import does not depend on it.
        deltas: Vec<Vec<u8>>,
    },

    // -- P3 -- Service plane (relayed request/response) -------------------------
    /// A [`RestCommand`] service operation. The Logic dispatcher applies any
    /// state hook (for example `Logout` clears the document registry) and forwards it
    /// to the Sync thread as [`SyncCommand::Rest`], echoing `id` on the response.
    ///
    /// [`SyncCommand::Rest`]: crate::SyncCommand::Rest
    Rest { id: RequestId, cmd: RestCommand },

    // -- P3 -- Control plane (fire-and-forget, no correlatable response) ---------
    /// Open a real-time sync session for a document.
    ///
    /// The Sync thread fetches members + `doc_key_enc`, decrypts the DocumentKey,
    /// loads snapshot + deltas, and opens a WebSocket subscription. The first
    /// [`crate::ui::UiEvent::Viewport`] after this indicates the document is ready.
    OpenDocument {
        document_id: DocumentId,
        /// Document kind, known from the URL route (`/trame/<id>`). Carried so a
        /// not-yet-on-server document can be lazily created with the right type
        /// on its first modification.
        doc_type: DocType,
    },

    /// Close the real-time sync session for a document. The Logic thread drops
    /// the in-memory state; the Sync thread unsubscribes and zeroizes the key.
    CloseDocument { document_id: DocumentId },

    /// Open a real-time sync session for a channel (subscribe to messages).
    OpenChannel { channel_id: ChannelId },

    /// Close the real-time sync session for a channel.
    CloseChannel { channel_id: ChannelId },

    /// Send a message to a channel.
    ///
    /// `payload` is an opaque application byte blob (schema owned by
    /// `ambre-murmure`); the Sync thread routes it into the active segment.
    ///
    /// TODO: migrate the Murmure channel surface to a typed `AppCommand::Murmure(...)`
    /// like the document surface (Murmure is currently a stub).
    SendChannelMessage {
        channel_id: ChannelId,
        /// Opaque application byte blob (schema owned by `ambre-murmure`).
        payload: Vec<u8>,
    },

    // -- Local -- Logic-thread resource seeding (no Sync involvement) ------------
    /// Hand the Logic thread font faces the UI thread has fetched
    /// (`CANVAS-067`, `CANVAS-068`). It cannot fetch any itself: it blocks on
    /// its inbound selector and has no browser event loop. Every open Prose
    /// document takes them, so pagination measures on real glyphs
    /// (`PROSE-115`), and the PDF-export `FontContext` takes them too
    /// (`toile-pdf-export`). Sent at boot with the base faces, and again for
    /// each script face fetched on demand. Local-only -- it never reaches Sync
    /// or the server. Each entry is one font file's bytes.
    LoadFonts { faces: Vec<Vec<u8>> },
}

// -----------------------------------------------------------------------------
// AppCommand -- typed application command family (content plane)
// -----------------------------------------------------------------------------

/// Application-specific command, UI Thread -> Logic thread.
///
/// Carried inside [`LogicCommand::App`]. The Logic dispatcher matches the
/// variant, routes it to the `DocumentId`'s document state, and produces zero or
/// more [`AppEvent`](crate::ui::AppEvent)s.
#[derive(Debug, Clone)]
pub enum AppCommand {
    /// Command for a Trame (spreadsheet) document.
    Trame(crate::app::trame::TrameCommand),
    /// Command for a Toile (whiteboard) document.
    Toile(crate::app::toile::ToileCommand),
    /// Command for a Prose (word processor) document.
    Prose(crate::app::prose::ProseCommand),
    // Future document types: Murmure(MurmureCommand),
    // Rendezvous(RendezvousCommand), ...
    // Future plugin surface: Plugin(Box<dyn Plugin>).
}

impl AppCommand {
    /// Whether this command can change the document's shared CRDT state.
    ///
    /// Used by the Logic dispatcher to gate a read-only caller
    /// (`architecture`). Ephemeral presence and local view state are
    /// *not* mutating: a viewer still scrolls, selects, copies and broadcasts a
    /// cursor.
    pub fn is_mutating(&self) -> bool {
        match self {
            Self::Trame(cmd) => cmd.is_mutating(),
            Self::Toile(cmd) => cmd.is_mutating(),
            Self::Prose(cmd) => cmd.is_mutating(),
        }
    }
}
