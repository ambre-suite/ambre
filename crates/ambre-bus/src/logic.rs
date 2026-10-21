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
//! - **Content** ([`LogicCommand::App`], [`LogicCommand::Channel`]): document
//!   edits / viewport requests, routed by `DocumentId` inside the Logic thread,
//!   and channel commands, routed by `ChannelId`. [`AppCommand`] and
//!   [`MurmureCommand`] are closed sums: adding a document type is adding a
//!   variant. No trait object, no `Any`, no crate cycle -- this module depends
//!   only on `ambre-core`.
//! - **Service** ([`LogicCommand::Rest`]): the correlatable request/response
//!   family, defined once in [`crate::rest`] and forwarded verbatim to Sync.
//! - **Control** (`OpenDocument`, `CloseDocument`, `OpenChannel`, `CloseChannel`):
//!   fire-and-forget session commands with no single response to correlate --
//!   kept explicit.

use crate::{
    ChannelId, DirectMember, DirectMessageDraft, DocType, DocumentId, MurmureCommand, RequestId,
    RestCommand, SpaceCommand, SpaceId, Timestamp,
};
use ambre_core::SessionNonce;

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

    /// Command for an open Murmure channel (`BUS-027`).
    ///
    /// Addressed to the channel, never to a document. A channel is a set of
    /// documents (the Murmure engine spec), and the engine picks the one a
    /// command writes. A typed variant in place of an opaque payload, so no envelope
    /// carries a channel message as bytes (`BUS-028`).
    Channel {
        channel_id: ChannelId,
        command: MurmureCommand,
    },

    /// Command for an open Murmure space: its catalogue, and the public
    /// channels it lists. Addressed to the space, which is one document.
    Space {
        space_id: SpaceId,
        command: SpaceCommand,
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
        /// The earlier sessions of this user on the document: each PeerID with
        /// the nonce it derives from, so what one left unsent travels
        /// (`AUTHOR-028`).
        sessions: Vec<(ambre_core::PeerId, SessionNonce)>,
    },

    /// The browser refused to store a change of `document_id` that the relay
    /// has not acknowledged, even after room was made (`RESID-018`). While
    /// `full`, no local edit is taken. Sent again with `full: false` once
    /// the change is stored or the relay holds it. Local-only, from the Main
    /// thread, like [`RestoreLocalSnapshot`](LogicCommand::RestoreLocalSnapshot).
    LocalStoreFull { document_id: DocumentId, full: bool },

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

    /// Open a channel: its `meta` document, then its live segments, each an
    /// ordinary document session (the Murmure engine spec).
    OpenChannel { channel_id: ChannelId },

    /// Close a channel: the Logic thread drops its state and closes each of
    /// its documents.
    CloseChannel { channel_id: ChannelId },

    /// Open the direct message of this user and `draft.members`, created
    /// when none holds exactly them (the Murmure channels spec). Answered by
    /// [`crate::UiEvent::DirectMessage`] under `draft.channel_id`.
    OpenDirectMessage {
        draft: DirectMessageDraft,
        /// The instant of a creation, stamped by the view.
        now: Timestamp,
    },

    /// Invite `invitee` into an open channel, as an editor, on its `meta`
    /// (the Murmure channels spec). The key is the one the user confirmed.
    /// Answered as a [`crate::RestCommand::InviteToDocument`] under `id`.
    InviteToChannel {
        id: RequestId,
        channel_id: ChannelId,
        invitee: DirectMember,
        /// The instant the user confirmed, which stamps a notice.
        now: Timestamp,
    },

    /// Create a space: a new space document, with the user as its owner and
    /// only member (the Murmure spaces spec). The space opens at once.
    CreateSpace {
        space_id: SpaceId,
        name: String,
        /// The instant of the creation, stamped by the view.
        now: Timestamp,
    },

    /// Open a space: its document, then its catalogue as a
    /// [`crate::SpaceEvent::Catalogue`].
    OpenSpace { space_id: SpaceId },

    /// Close a space and the session of its document.
    CloseSpace { space_id: SpaceId },

    /// Ask which channels hold something new, for the sidebar at start
    /// (the Murmure unread spec). Loads no document: answered by
    /// [`crate::UiEvent::ChannelActivity`].
    FetchActivity,

    /// Ask which channels this user belongs to, for the sidebar at start
    /// (the Murmure channels spec). Opens nothing: answered by
    /// [`crate::UiEvent::MemberChannels`].
    ListMemberChannels,

    /// Mark every channel read (the Murmure unread spec).
    MarkAllRead,

    /// Search the open channels for `query`, in the segments the client
    /// holds (the Murmure search spec). Nothing leaves the client, and
    /// nothing is loaded: answered by [`crate::UiEvent::MessageSearch`].
    SearchMessages { query: String },

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
