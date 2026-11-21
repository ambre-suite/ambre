//! Logic-bound protocol -- messages addressed to the Logic thread from the UI.
//!
//! Holds the [`LogicCommand`] envelope (UI Thread -> Logic thread), generic
//! over the document command it carries. Each application declares its own
//! command enum in its crate (`BUS-032`). The Logic runtime declares
//! `AppCommand`, the closed sum over them, downstream of every application. A
//! view sends `LogicCommand<ItsCommand>`, and the launcher embeds it into the
//! sum through [`crate::Embed`].
//!
//! Carried over a `conduit` queue in shared memory -- typed Rust values by move,
//! no serialization. The symmetric Logic -> UI direction (events) lives in
//! [`crate::ui`].
//!
//! # Three planes, one envelope
//! - **Content** ([`LogicCommand::App`], [`LogicCommand::Channel`]): document
//!   edits / viewport requests, routed by `Id<Document>` inside the Logic thread,
//!   and channel commands, routed by `Id<Channel>`. The document command is the
//!   first type parameter, and the channel types are the second, a
//!   [`ChannelPlane`]. No trait object, no `Any`, no crate cycle -- this module
//!   depends only on `ambre-core`.
//! - **Service** ([`LogicCommand::Rest`]): the correlatable request/response
//!   family, defined once in [`crate::rest`] and forwarded verbatim to Sync.
//! - **Control** (`OpenDocument`, `CloseDocument`, `OpenChannel`, `CloseChannel`):
//!   fire-and-forget session commands with no single response to correlate --
//!   kept explicit.

use crate::{Authored, DirectMember, DocType, Instant, Request, RestCommand};
use ambre_core::id::{Channel, Document, Id, Space};
use ambre_core::{FixedBytes, Seq, SessionNonce};
use std::convert::Infallible;

use crate::{ChannelPlane, Embed, NoChannels, PlaneOf};

// The document command carried by `LogicCommand::App` is the type parameter: each
// application declares its own, and `AppCommand` sums them in the Logic runtime.

// -----------------------------------------------------------------------------
// LogicCommand -- UI Thread -> Logic thread
// -----------------------------------------------------------------------------

/// Whether the browser store takes the unsent changes of a document
/// (`RESID-018`). Main tells Logic each time the answer changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StoreRoom {
    #[default]
    Free,
    Full,
}

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
///
/// `A` is the document command `App` carries. A sender that carries none, such
/// as the launcher's own service requests, leaves it at [`Infallible`]. `P` is
/// the channel plane the channel and space variants carry, [`NoChannels`] for
/// every sender but Murmure's.
#[derive(Debug, Clone)]
pub enum LogicCommand<A = Infallible, P: ChannelPlane = NoChannels> {
    // -- P2 -- Content plane ----------------------------------------------------
    /// Application-specific command for an open document (cell edit, viewport
    /// scroll in Trame; text insertion in Prose; element move in Toile; ...).
    ///
    /// The Logic dispatcher routes `command` to the state owning
    /// `document_id` and replies with [`crate::ui::UiEvent::Viewport`] /
    /// [`crate::ui::UiEvent::Presence`] events.
    App {
        document_id: Id<Document>,
        /// Typed application command, declared by its application's crate.
        command: A,
    },

    /// Command for an open Murmure channel (`BUS-027`).
    ///
    /// Addressed to the channel, never to a document. A channel is a set of
    /// documents (the Murmure engine spec), and the engine picks the one a
    /// command writes. A typed variant in place of an opaque payload, so no envelope
    /// carries a channel message as bytes (`BUS-028`).
    Channel {
        channel_id: Id<Channel>,
        command: P::Command,
    },

    /// Command for an open Murmure space: its catalogue, and the public
    /// channels it lists. Addressed to the space, which is one document.
    Space {
        space_id: Id<Space>,
        command: P::SpaceCommand,
    },

    /// Restore a document's CRDT state from a locally persisted snapshot
    /// (IndexedDB), dispatched by the Main thread at startup. Local-only: it
    /// never reaches Sync or the server -- it lets an edit survive a refresh
    /// without a network round-trip.
    RestoreLocalSnapshot {
        document_id: Id<Document>,
        /// Document kind, known from the boot URL route (`/trame/<id>`). Lets the
        /// handler materialise the live document if the restore wins the startup
        /// race against [`OpenDocument`](LogicCommand::OpenDocument).
        doc_type: DocType,
        /// Full Loro snapshot (`ExportMode::Snapshot`) read back from local
        /// storage, when one was saved.
        snapshot: Option<ambre_core::Payload<ambre_core::SnapshotBytes>>,
        /// The deltas logged since that snapshot, in no set order. A Loro
        /// import does not depend on it.
        deltas: Vec<ambre_core::Payload<ambre_core::DeltaBytes>>,
        /// The earlier sessions of this user on the document: each PeerID with
        /// the nonce it derives from, so what one left unsent travels
        /// (`AUTHOR-028`).
        sessions: Vec<(ambre_core::PeerId, FixedBytes<SessionNonce>)>,
    },

    /// The browser refused to store a change of `document_id` that the relay
    /// has not acknowledged, even after room was made (`RESID-018`). While
    /// [`StoreRoom::Full`], no local edit is taken. Sent again with
    /// [`StoreRoom::Free`] once the change is stored or the relay holds it.
    /// Local-only, from the Main thread, like
    /// [`RestoreLocalSnapshot`](LogicCommand::RestoreLocalSnapshot).
    LocalStoreFull {
        document_id: Id<Document>,
        room: StoreRoom,
    },

    // -- P3 -- Service plane (relayed request/response) -------------------------
    /// A [`RestCommand`] service operation. The Logic dispatcher applies any
    /// state hook (for example `Logout` clears the document registry) and forwards it
    /// to the Sync thread as [`SyncCommand::Rest`], echoing `id` on the response.
    ///
    /// [`SyncCommand::Rest`]: crate::SyncCommand::Rest
    Rest { id: Seq<Request>, cmd: RestCommand },

    // -- P3 -- Control plane (fire-and-forget, no correlatable response) ---------
    /// Open a real-time sync session for a document.
    ///
    /// The Sync thread fetches members + `doc_key_enc`, decrypts the DocumentKey,
    /// loads snapshot + deltas, and opens a WebSocket subscription. The first
    /// [`crate::ui::UiEvent::Viewport`] after this indicates the document is ready.
    OpenDocument {
        document_id: Id<Document>,
        /// Document kind, known from the URL route (`/trame/<id>`). Carried so a
        /// not-yet-on-server document can be lazily created with the right type
        /// on its first modification.
        doc_type: DocType,
    },

    /// Pin the inviter that the invite link of `document_id` names
    /// (`ambre-spec-membership.md`, `SYNC-205`). Sent before the open the
    /// link drives, so the Sync worker checks that open against it. The
    /// Logic thread forwards it and reads nothing of it.
    PinInviter {
        document_id: Id<Document>,
        inviter: ambre_core::FixedBytes<ambre_core::InviterPin>,
    },

    /// Hold where the log of `document_id` stood in an earlier session
    /// (`SYNC-290`). The Logic thread forwards it and reads nothing of it.
    KeepLog {
        document_id: Id<Document>,
        kept: crate::KeptLog,
    },

    /// Pin the inviter that a Murmure invite link names (`SYNC-236`). The
    /// Logic thread forwards it and reads nothing of it.
    PinMurmureInviter {
        inviter: ambre_core::FixedBytes<ambre_core::InviterPin>,
    },

    /// Close the real-time sync session for a document. The Logic thread drops
    /// the in-memory state; the Sync thread unsubscribes and zeroizes the key.
    CloseDocument { document_id: Id<Document> },

    /// Send every view of an open document its whole state again, as events
    /// of its application (`ambre-spec-message-bus.md`, `BUS-041`). The
    /// switchboard asks for it once a view that lost an event reads again.
    /// The document and its sessions are left as they are, and a document not
    /// open answers nothing.
    ResendState { document_id: Id<Document> },

    /// Open a channel: its `meta` document, then its live segments, each an
    /// ordinary document session (the Murmure engine spec).
    OpenChannel { channel_id: Id<Channel> },

    /// Close a channel: the Logic thread drops its state and closes each of
    /// its documents.
    CloseChannel { channel_id: Id<Channel> },

    /// Open the direct message of this user and `draft.members`, created
    /// when none holds exactly them (the Murmure channels spec). Answered by
    /// [`crate::UiEvent::DirectMessage`] under `draft.channel_id`.
    OpenDirectMessage {
        draft: P::DirectDraft,
        /// The instant of a creation, stamped by the view.
        now: Instant<Authored>,
    },

    /// Invite `invitee` into an open channel, as an editor, on its `meta`
    /// (the Murmure channels spec). The key is the one the user confirmed.
    /// Answered as a [`crate::RestCommand::InviteToDocument`] under `id`.
    InviteToChannel {
        id: Seq<Request>,
        channel_id: Id<Channel>,
        invitee: DirectMember,
        /// The instant the user confirmed, which stamps a notice.
        now: Instant<Authored>,
    },

    /// Create a space: a new space document, with the user as its owner and
    /// only member (the Murmure spaces spec). The space opens at once.
    CreateSpace {
        space_id: Id<Space>,
        name: ambre_core::Validated<ambre_core::Name>,
        /// The instant of the creation, stamped by the view.
        now: Instant<Authored>,
    },

    /// Open a space: its document, then its catalogue as a
    /// [`crate::SpaceEvent::Catalogue`].
    OpenSpace { space_id: Id<Space> },

    /// Close a space and the session of its document.
    CloseSpace { space_id: Id<Space> },

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
    SearchMessages {
        query: ambre_core::Validated<ambre_core::SearchQuery>,
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
    LoadFonts {
        faces: Vec<ambre_core::Payload<ambre_core::FontBytes>>,
    },
    /// The reader's own locale and convention, read from the device at boot
    /// and sent again when the reader changes them (`LOCALE-015`). Every open
    /// document takes it, and a document this reader creates is seeded with
    /// the locale (`LOCALE-019`). Local-only: no document stores it
    /// (`LOCALE-017`).
    SetReaderLocale {
        reader: ambre_client_core::ReaderLocale,
    },
}

// -----------------------------------------------------------------------------
// Retyping the document command
// -----------------------------------------------------------------------------

impl<A, P: ChannelPlane> LogicCommand<A, P> {
    /// The same command with its document command passed through `app` and
    /// each channel type through its own function, or the first refusal. The
    /// one exhaustive list of variants every retyping below shares.
    fn retype<B, Q: ChannelPlane, E>(
        self,
        app: impl FnOnce(A) -> Result<B, E>,
        channel: impl FnOnce(P::Command) -> Result<Q::Command, E>,
        space: impl FnOnce(P::SpaceCommand) -> Result<Q::SpaceCommand, E>,
        direct: impl FnOnce(P::DirectDraft) -> Result<Q::DirectDraft, E>,
    ) -> Result<LogicCommand<B, Q>, E> {
        Ok(match self {
            Self::App {
                document_id,
                command,
            } => LogicCommand::App {
                document_id,
                command: app(command)?,
            },
            Self::Channel {
                channel_id,
                command,
            } => LogicCommand::Channel {
                channel_id,
                command: channel(command)?,
            },
            Self::Space { space_id, command } => LogicCommand::Space {
                space_id,
                command: space(command)?,
            },
            Self::RestoreLocalSnapshot {
                document_id,
                doc_type,
                snapshot,
                deltas,
                sessions,
            } => LogicCommand::RestoreLocalSnapshot {
                document_id,
                doc_type,
                snapshot,
                deltas,
                sessions,
            },
            Self::LocalStoreFull { document_id, room } => {
                LogicCommand::LocalStoreFull { document_id, room }
            }
            Self::Rest { id, cmd } => LogicCommand::Rest { id, cmd },
            Self::OpenDocument {
                document_id,
                doc_type,
            } => LogicCommand::OpenDocument {
                document_id,
                doc_type,
            },
            Self::PinInviter {
                document_id,
                inviter,
            } => LogicCommand::PinInviter {
                document_id,
                inviter,
            },
            Self::KeepLog { document_id, kept } => LogicCommand::KeepLog { document_id, kept },
            Self::PinMurmureInviter { inviter } => LogicCommand::PinMurmureInviter { inviter },
            Self::CloseDocument { document_id } => LogicCommand::CloseDocument { document_id },
            Self::ResendState { document_id } => LogicCommand::ResendState { document_id },
            Self::OpenChannel { channel_id } => LogicCommand::OpenChannel { channel_id },
            Self::CloseChannel { channel_id } => LogicCommand::CloseChannel { channel_id },
            Self::OpenDirectMessage { draft, now } => LogicCommand::OpenDirectMessage {
                draft: direct(draft)?,
                now,
            },
            Self::InviteToChannel {
                id,
                channel_id,
                invitee,
                now,
            } => LogicCommand::InviteToChannel {
                id,
                channel_id,
                invitee,
                now,
            },
            Self::CreateSpace {
                space_id,
                name,
                now,
            } => LogicCommand::CreateSpace {
                space_id,
                name,
                now,
            },
            Self::OpenSpace { space_id } => LogicCommand::OpenSpace { space_id },
            Self::CloseSpace { space_id } => LogicCommand::CloseSpace { space_id },
            Self::FetchActivity => LogicCommand::FetchActivity,
            Self::ListMemberChannels => LogicCommand::ListMemberChannels,
            Self::MarkAllRead => LogicCommand::MarkAllRead,
            Self::SearchMessages { query } => LogicCommand::SearchMessages { query },
            Self::LoadFonts { faces } => LogicCommand::LoadFonts { faces },
            Self::SetReaderLocale { reader } => LogicCommand::SetReaderLocale { reader },
        })
    }

    /// The same command, its document command passed through `f`.
    pub fn map<B>(self, f: impl FnOnce(A) -> B) -> LogicCommand<B, P> {
        match self.retype(|command| Ok::<_, Infallible>(f(command)), Ok, Ok, Ok) {
            Ok(command) => command,
            Err(never) => match never {},
        }
    }

    /// The same command embedded into the sum `S` and the plane `Q`. The
    /// launcher puts a view's command on the Logic thread's queue with it.
    pub fn embed_into<S: Embed<A>, Q: PlaneOf<P>>(self) -> LogicCommand<S, Q> {
        let embedded = self.retype(
            |command| Ok::<_, Infallible>(S::embed(command)),
            |command| Ok(Q::embed_command(command)),
            |command| Ok(Q::embed_space_command(command)),
            |draft| Ok(Q::embed_draft(draft)),
        );
        match embedded {
            Ok(command) => command,
            Err(never) => match never {},
        }
    }

    /// The same command with its document command projected out of the sum
    /// `A` and its channel types out of the plane `P`. `None` when either
    /// holds another application's.
    pub fn project<B, Q: ChannelPlane>(self) -> Option<LogicCommand<B, Q>>
    where
        A: Embed<B>,
        P: PlaneOf<Q>,
    {
        self.retype(
            |command| command.project().map_err(drop),
            |command| P::project_command(command).ok_or(()),
            |command| P::project_space_command(command).ok_or(()),
            |draft| P::project_draft(draft).ok_or(()),
        )
        .ok()
    }
}
