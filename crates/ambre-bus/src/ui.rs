//! UI-bound protocol -- messages addressed to the UI Thread from the Logic thread.
//!
//! Holds the [`UiEvent`] envelope (Logic thread -> UI Thread) plus [`AppEvent`],
//! the closed sum over the document types. Each per-app event enum and its
//! display types live with that app's whole data plane under
//! [`crate::app`], such as `crate::app::trame::TrameEvent` and `DisplayCell`.
//! A document type is therefore defined in one place, whichever direction the
//! message travels.
//!
//! Carried over a `conduit` queue in shared memory -- typed Rust values by move,
//! no serialization. The symmetric UI -> Logic direction (commands) lives in
//! [`crate::logic`].
//!
//! # Three planes, one envelope
//! - **Content** ([`UiEvent::Viewport`] / [`UiEvent::Presence`]): typed
//!   [`AppEvent`]s computed by the Logic thread, routed by `Id<Document>`.
//! - **Service** ([`UiEvent::Rest`]): a [`RestEvent`] response forwarded from
//!   [`crate::LogicEvent::Rest`], echoing the originating [`Request`].
//! - **Push & status** (`ConnectionStatus`, `SyncWarning`, `MemberRevoked`,
//!   channel/meeting push, `AuthError`, `ApiError`, `FatalError`, `SetupRequired`,
//!   `UpdateRequired`): unsolicited.

use crate::sync_warning::SyncWarningCode;
use crate::{
    ChannelActivity, ChannelMatches, DirectMessageEvent, DocType, MemberChannel, MurmureEvent,
    Request, RestEvent, Role, SpaceEvent,
};
use ambre_core::id::{Channel, Document, Id, Meeting, Message, Segment, Space, User};

// -----------------------------------------------------------------------------
// UiEvent -- Logic thread -> UI Thread
// -----------------------------------------------------------------------------

ambre_core::define_flag! {
    /// Whether a new message names this user or the channel, as its sealed
    /// summary says (the Murmure notifications spec).
    pub enum MentionState {
        /// The message names neither this user nor the channel.
        Unmentioned,
        /// The message names this user or the channel.
        Mentioned,
    }
}

/// Message sent from the Logic thread to the UI Thread.
#[derive(Debug, Clone)]
pub enum UiEvent {
    // -- Content plane ---------------------------------------------------------
    /// Display event produced in response to an [`crate::logic::AppCommand`]
    /// (for example a viewport scroll) or a spontaneous refresh after a remote CRDT
    /// update. The UI Thread routes [`event`](AppEvent) to the view owning
    /// `document_id`.
    Viewport {
        document_id: Id<Document>,
        event: AppEvent,
    },

    /// Decrypted collaborator presence event (cursor position, selection, ...).
    Presence {
        document_id: Id<Document>,
        event: AppEvent,
    },

    // -- Service plane (relayed request/response) ------------------------------
    /// A [`RestEvent`] response, echoing the originating [`Request`] so the UI
    /// can route it to the call-site that issued the [`crate::RestCommand`].
    /// `None` for an answer no request asked for.
    Rest {
        id: Option<ambre_core::Seq<Request>>,
        evt: RestEvent,
    },

    // -- Connection / status (server push) -------------------------------------
    /// WebSocket connection status for a document (online / offline indicator).
    ConnectionStatus {
        document_id: Id<Document>,
        connected: ambre_core::LinkState,
    },

    /// The activity of a document not held in memory, for its badge
    /// (`RESID-014`). A reopen replaces it with the counts the document holds
    /// (`RESID-011`).
    Activity {
        document_id: Id<Document>,
        counters: ambre_core::activity::ActivityCounters,
    },

    /// Initial hydration of a freshly opened document is complete (snapshot, if
    /// any, already applied). The UI dismisses its document-loading overlay.
    /// Fires once per open, including for empty/brand-new documents.
    /// Carries this client's own role so the UI can withhold editing
    /// affordances on a read-only document (`architecture`).
    DocumentReady {
        document_id: Id<Document>,
        role: Role,
    },

    /// Non-fatal sync warning (quarantine overflow, decrypt failure, ...).
    SyncWarning {
        document_id: Id<Document>,
        code: SyncWarningCode,
        message: String,
    },

    /// A member was revoked and key rotation is complete (server-initiated).
    MemberRevoked {
        document_id: Id<Document>,
        revoked_user_id: Id<User>,
    },

    /// A document was just shared with this user (server-initiated). The UI
    /// refreshes its document list so the new document appears without a reload,
    /// and notifies the user.
    DocumentShared {
        document_id: Id<Document>,
        doc_type: DocType,
    },

    // -- Channels (Murmure) -- real-time push -----------------------------------
    /// What an open channel's engine tells its view (`BUS-027`): typed, never
    /// an opaque payload (`BUS-028`).
    Channel {
        channel_id: Id<Channel>,
        event: MurmureEvent,
    },

    /// Which channels hold something new: the relay's answer at start, or a
    /// channel just read (the Murmure unread spec). A channel absent here keeps the
    /// mark the view last drew for it.
    ChannelActivity { channels: Vec<ChannelActivity> },

    /// Another member wrote a message in a channel, as its sealed summary
    /// says (the Murmure notifications spec). `mention` is `Mentioned` when
    /// the summary names this user or the channel. The view decides whether it
    /// notifies: it alone knows the direct messages and what is on screen.
    /// `message` names the message and its segment when the channel is open.
    /// A closed channel is counted by the relay alone, so it names none.
    NewMessage {
        channel_id: Id<Channel>,
        mention: MentionState,
        message: Option<(Id<Segment>, Id<Message>)>,
    },

    /// Every channel and every space this user is a member of, whole. It
    /// comes at start, and again each time the relay adds the user to one.
    /// None of them is open. `names` holds the name each named channel shows
    /// before it opens, and `peers` the other members of each direct message.
    MemberChannels {
        channels: Vec<MemberChannel>,
        spaces: Vec<Id<Space>>,
        names: Vec<crate::ChannelName>,
        peers: Vec<crate::DirectPeers>,
    },

    /// The answer to [`crate::LogicCommand::SearchMessages`]: one entry per
    /// open channel, each stating the segments it covered. A query with no
    /// word answers no channel.
    MessageSearch {
        query: ambre_core::Validated<ambre_core::SearchQuery>,
        channels: Vec<ChannelMatches>,
    },

    /// What an open space tells its view: its catalogue, and the outcome of
    /// a creation or a join.
    Space {
        space_id: Id<Space>,
        event: SpaceEvent,
    },

    /// The outcome of a direct message request, under the channel id the
    /// request minted.
    DirectMessage {
        requested: Id<Channel>,
        event: DirectMessageEvent,
    },

    // -- Meetings (Rendez-Vous) -- participant push -----------------------------
    /// A participant joined the meeting.
    MeetingParticipantJoined {
        meeting_id: Id<Meeting>,
        user_id: Id<User>,
    },

    /// A participant left the meeting.
    MeetingParticipantLeft {
        meeting_id: Id<Meeting>,
        user_id: Id<User>,
    },

    // -- Errors & warnings -----------------------------------------------------
    /// A document could not open, and nothing of it is held. The UI ends its
    /// loading screen and tells the user (`ambre-spec-message-bus.md`,
    /// `BUS-030`).
    DocumentOpenFailed {
        document_id: Id<Document>,
        message: String,
    },

    /// Authentication error (wrong passphrase, corrupted vault, network failure).
    AuthError { message: String },

    /// REST or WebSocket operation error.
    ///
    /// `request` identifies the originating command so the UI Thread can correlate the
    /// error to the triggering action. `failure` says whether the relay answered, with
    /// what status and code, or whether the request failed on this device. `id` echoes the originating [`Request`] when the
    /// failed command carried one, so a UI-issued request can be matched to its failure. It
    /// is `None` for a spontaneous error.
    ApiError {
        id: Option<ambre_core::Seq<Request>>,
        request: crate::ApiRequest,
        failure: ambre_core::ApiFailure,
        message: String,
    },

    /// Unrecoverable failure in a worker thread -- the application cannot continue and the
    /// UI should display a terminal error screen.
    ///
    /// This is the **graceful** path: a thread that detects an unrecoverable condition
    /// emits this before exiting. The **ungraceful** path is a panic, which is
    /// `panic=abort` under multi-threaded WASM, and it never reaches a receiver at all.
    /// Nothing is dropped, so the sender leaks in shared memory and the queue reads empty
    /// rather than disconnected. That path goes through the page's fatal hook instead
    /// (`ambre-spec-architecture.md`, `ARCH-008`).
    FatalError { message: String },

    /// The relay refused a request or the socket because the account still owes
    /// its key setup. It calls for the setup, not for an error screen. Nothing
    /// is retried until the keys are stored (`ambre-spec-sync.md`, `SYNC-134`).
    SetupRequired,

    /// The relay speaks a newer wire version than this build. It calls for a
    /// reload onto the new client, not for an error (`ambre-spec-sync.md`, `SYNC-137`).
    UpdateRequired,
}

impl UiEvent {
    /// The `Id<Document>` this event is addressed to, for the per-document fan-out a nested
    /// `doc_embed` needs (`architecture` / `toile-embeds`). A Toile embedding another
    /// document owns a child view, whose Logic->UI stream must be demultiplexed out of the
    /// single shared queue. Only the variants a *view* consumes carry a routing id.
    /// Service-plane and app-global events (`Rest`, `DocumentShared`, channel/meeting push,
    /// errors, `FatalError`) return `None` and stay on the top-level path.
    pub fn routed_document_id(&self) -> Option<Id<Document>> {
        match self {
            UiEvent::Viewport { document_id, .. }
            | UiEvent::Presence { document_id, .. }
            | UiEvent::ConnectionStatus { document_id, .. }
            | UiEvent::DocumentReady { document_id, .. }
            | UiEvent::DocumentOpenFailed { document_id, .. }
            | UiEvent::SyncWarning { document_id, .. }
            | UiEvent::MemberRevoked { document_id, .. } => Some(*document_id),
            _ => None,
        }
    }
}

// -----------------------------------------------------------------------------
// AppEvent -- typed application event family (content plane)
// -----------------------------------------------------------------------------

/// Application-specific event, Logic thread -> UI Thread.
///
/// Carried inside [`UiEvent::Viewport`] / [`UiEvent::Presence`]. The `Command` / `Event`
/// suffix marks the direction at both the envelope and the payload level. UI->Logic is
/// always a `Command` (see [`AppCommand`](crate::logic::AppCommand)), Logic->UI always an
/// `Event`.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Display event for a Trame (spreadsheet) document.
    Trame(crate::app::trame::TrameEvent),
    /// Display event for a Toile (whiteboard) document.
    Toile(crate::app::toile::ToileEvent),
    /// Display event for a Prose (word processor) document.
    Prose(crate::app::prose::ProseEvent),
}
