//! Logic-bound protocol — messages addressed to the Logic thread from the UI.
//!
//! Holds the [`LogicCommand`] envelope (UI Thread → Logic thread) plus the typed
//! application **command** data plane it carries: [`AppCommand`] (a closed sum
//! over the document types) and the per-app command enums (`TrameCommand`, …).
//!
//! Carried over a `flume` channel in shared memory — typed Rust values by move,
//! no serialization. The symmetric Logic → UI direction (events) lives in
//! [`crate::ui`].
//!
//! # Three planes, one envelope
//! - **Content** ([`LogicCommand::App`]): document edits / viewport requests,
//!   routed by `DocumentId` inside the Logic thread. [`AppCommand`] is a closed
//!   sum — adding a document type = adding a variant (no trait object, no `Any`,
//!   no crate cycle; this module depends only on `ambre-core`).
//! - **Service** ([`LogicCommand::Rest`]): the correlatable request/response
//!   family, defined once in [`crate::rest`] and forwarded verbatim to Sync.
//! - **Control** (`OpenDocument`, `CloseDocument`, `OpenChannel`, `CloseChannel`,
//!   `SendChannelMessage`): fire-and-forget session/content-send commands with
//!   no single response to correlate — kept explicit.

use crate::{ChannelId, DocumentId, RequestId, RestCommand};

// ─────────────────────────────────────────────────────────────────────────────
// LogicCommand — UI Thread → Logic thread
// ─────────────────────────────────────────────────────────────────────────────

/// Message sent from the UI Thread to the Logic thread.
///
/// - [`App`](LogicCommand::App) (**P2**): user edits / viewport requests, < 50 ms
///   budget (scroll requests < 10 ms). Routed to the document's state.
/// - [`Rest`](LogicCommand::Rest) (**P3**): a [`RestCommand`] service operation,
///   forwarded to the Sync thread as [`SyncCommand::Rest`] (with a small Logic
///   hook for the few ops that also mutate Logic state, e.g. `Logout`).
/// - Control variants (**P3**): forwarded to the Sync thread as their explicit
///   `SyncCommand` counterparts.
///
/// [`SyncCommand::Rest`]: crate::SyncCommand::Rest
#[derive(Debug, Clone)]
pub enum LogicCommand {
    // ── P2 — Content plane ────────────────────────────────────────────────────
    /// Application-specific command for an open document (cell edit, viewport
    /// scroll in Trame; text insertion in Prose; element move in Toile; …).
    ///
    /// The Logic dispatcher routes [`command`](AppCommand) to the state owning
    /// `document_id` and replies with [`crate::ui::UiEvent::Viewport`] /
    /// [`crate::ui::UiEvent::Presence`] events.
    App {
        document_id: DocumentId,
        /// Typed application command (schema owned by [`crate::logic`]).
        command: AppCommand,
    },

    // ── P3 — Service plane (relayed request/response) ─────────────────────────
    /// A [`RestCommand`] service operation. The Logic dispatcher applies any
    /// state hook (e.g. `Logout` clears the document registry) and forwards it
    /// to the Sync thread as [`SyncCommand::Rest`], echoing `id` on the response.
    ///
    /// [`SyncCommand::Rest`]: crate::SyncCommand::Rest
    Rest {
        id: RequestId,
        cmd: RestCommand,
    },

    // ── P3 — Control plane (fire-and-forget, no correlatable response) ─────────
    /// Open a real-time sync session for a document.
    ///
    /// The Sync thread fetches members + `doc_key_enc`, decrypts the DocumentKey,
    /// loads snapshot + deltas, and opens a WebSocket subscription. The first
    /// [`crate::ui::UiEvent::Viewport`] after this indicates the document is ready.
    OpenDocument {
        document_id: DocumentId,
    },

    /// Close the real-time sync session for a document. The Logic thread drops
    /// the in-memory state; the Sync thread unsubscribes and zeroizes the key.
    CloseDocument {
        document_id: DocumentId,
    },

    /// Open a real-time sync session for a channel (subscribe to messages).
    OpenChannel {
        channel_id: ChannelId,
    },

    /// Close the real-time sync session for a channel.
    CloseChannel {
        channel_id: ChannelId,
    },

    /// Send a message to a channel.
    ///
    /// `payload` is an opaque application byte blob (schema owned by
    /// `ambre-murmure`); the Sync thread routes it into the active segment.
    ///
    /// TODO: migrate the Murmure channel surface to a typed `AppCommand::Murmure(…)`
    /// like the document surface (Murmure is currently a stub).
    SendChannelMessage {
        channel_id: ChannelId,
        /// Opaque application byte blob (schema owned by `ambre-murmure`).
        payload: Vec<u8>,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// AppCommand — typed application command family (content plane)
// ─────────────────────────────────────────────────────────────────────────────

/// Application-specific command, UI Thread → Logic thread.
///
/// Carried inside [`LogicCommand::App`]. The Logic dispatcher matches the
/// variant, routes it to the `DocumentId`'s document state, and produces zero or
/// more [`AppEvent`](crate::ui::AppEvent)s.
#[derive(Debug, Clone)]
pub enum AppCommand {
    /// Command for a Trame (spreadsheet) document.
    Trame(TrameCommand),
    // Future document types: Prose(ProseCommand), Toile(ToileCommand),
    // Murmure(MurmureCommand), Rendezvous(RendezvousCommand), …
    // Future plugin surface: Plugin(Box<dyn Plugin>).
}

// ─────────────────────────────────────────────────────────────────────────────
// Trame (spreadsheet) — commands (UI → Logic)
// ─────────────────────────────────────────────────────────────────────────────

/// Commands sent from the UI Thread to the Trame logic for an open document.
#[derive(Debug, Clone)]
pub enum TrameCommand {
    /// Request cell data for a viewport region.
    ///
    /// The UI Thread computes the visible range (plus padding) from its scroll
    /// position and grid dimensions; the Logic thread replies with a
    /// [`TrameEvent::ViewportData`](crate::ui::TrameEvent::ViewportData) covering
    /// `[row_start..=row_end] × [col_start..=col_end]`.
    ///
    /// `version` is a monotonic counter: the UI Thread discards any response
    /// whose version is older than the last accepted one.
    ScrollViewport {
        version: u64,
        row_start: u32,
        row_end: u32,
        col_start: u32,
        col_end: u32,
    },

    /// Move the active cell (selection) to `(row, col)`.
    ///
    /// Formatting commands ([`ToggleBold`](TrameCommand::ToggleBold) /
    /// [`ToggleItalic`](TrameCommand::ToggleItalic)) target whatever cell is
    /// active in the Logic thread, so the UI Thread must keep it informed when
    /// the cursor moves. The Logic thread replies with the active cell's current
    /// formatting ([`TrameEvent::BoldState`](crate::ui::TrameEvent::BoldState) /
    /// [`ItalicState`](crate::ui::TrameEvent::ItalicState)) so the chrome toggle
    /// buttons track the selection.
    SetActiveCell {
        row: u32,
        col: u32,
    },

    /// Set the plaintext content of cell `(row, col)`.
    ///
    /// Emitted when the UI commits an edit session on the transparent textarea
    /// pipe. The Logic thread stores the text in the document CRDT (the source
    /// of truth); an empty `text` clears the cell. The UI updates its own cache
    /// optimistically, so no echo event is required.
    SetCellText {
        row: u32,
        col: u32,
        text: String,
    },

    /// Toggle bold formatting for the active cell (set by
    /// [`SetActiveCell`](TrameCommand::SetActiveCell)).
    ToggleBold,

    /// Toggle italic formatting for the active cell (set by
    /// [`SetActiveCell`](TrameCommand::SetActiveCell)).
    ToggleItalic,
}
