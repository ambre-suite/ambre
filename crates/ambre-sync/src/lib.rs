// ambre-sync/src/lib.rs
// ============================================================================
// Sync logic -- runs in its OWN wasm instance, a dedicated Web Worker with its
// own linear memory. It is separate from the Main instance (UI + Logic). The
// worker shell lives in `ambre-sync-worker`; this crate owns `SyncState` and
// the select loop ([`run_wasm`]).
//
// Responsibilities:
//   - Own ALL cryptographic material (MasterSeed, DocumentKeys) -- never leaves
//     this thread.
//   - Own the single session-level WebSocket (one WS, N document subscriptions).
//   - Perform HTTP REST calls (fetch via gloo-net).
//   - Encrypt/decrypt/compress/decompress every outbound and inbound payload.
//
// Out of scope:
//   - CRDT mutations -- exclusively in ambre-logic.
//   - Rendering, input -- exclusively in the UI Thread.
//
// Transport: inbound `SyncCommand`s arrive over `postMessage` from the Main
// instance, decoded by `ambre-sync-worker` and fed into [`run_wasm`] via an
// in-worker `ambre_bus::conduit::unbounded` queue. Outbound `LogicEvent`s are
// emitted through a [`LogicSink`] (the worker posts them back to Main via
// `postMessage`).
//
// This is a SINGLE worker: the internal event bus and the command queue both
// have their producer and consumer on it. The select loop awaits both queues'
// `recv`; the worker's JS event loop stays free for WebSocket / fetch.
//
// WebSocket topology (when implemented):
//   ONE WebSocket per session, opened at Unlock, closed at Logout.
//   OpenDocument / CloseDocument -> Subscribe / Unsubscribe on the shared WS.
//   All documents are demultiplexed by document_id on incoming WsServerMsg.
// ============================================================================

// A length, count, sequence or offset here comes from the relay, a peer or the
// local store. So an overflow, a wrap or an arithmetic panic is refused. A test
// asserts by panicking, so the lint stops at its boundary, as the panic lints
// of `clippy.toml` do (`ARCH-028`).
#![cfg_attr(not(test), deny(clippy::arithmetic_side_effects))]

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::WebSocket;
#[cfg(target_arch = "wasm32")]
use web_sys::{CloseEvent, MessageEvent};

use ambre_bus::ActivitySummary;
use ambre_bus::sync_warning::SyncWarningCode;
use ambre_bus::{
    ApiFailure, ApiRequest, Confidential, DocType, DocumentSummary, LogicEvent, MemberInfo,
    OpenFailure, Plaintext, RecoveryWords, Request, RestCommand, RestEvent, Role, SignedDelta,
    SnapshotPurpose, SnapshotVerdict, SyncCommand, UserInfo,
};
use ambre_core::id::{Channel, Document, Id, Lease, Message, Meta, Segment, Snapshot, Space, User};
use ambre_core::{Bounded, Count, FixedBytes, KeyId, Payload, PublicKey, SessionNonce};
use ambre_crypto::{DocumentKey, MasterSeed};
use ambre_protocol::messages::{DeltaAuthor, Ephemerals, Relay, Seq, Update};
use ambre_protocol::public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
use ambre_protocol::ws::{CloseCode, WsClientMsg, WsServerMsg, encode_client_msg};
#[cfg(target_arch = "wasm32")]
use ambre_protocol::ws::{WireFrame, decode_server_msg};
use ed25519_dalek::SigningKey;
use x25519_dalek::StaticSecret;

/// Sink for events flowing Sync -> Logic. Sync no longer shares memory with the
/// Logic thread, so it cannot hold a queue `Sender<LogicEvent>` directly:
/// instead it calls this type-erased closure. In the wasm worker the closure
/// postcard-encodes the event and `postMessage`s it to the Main instance. In
/// native tests it pushes into a `conduit` queue (see [`LogicSink::queue`]).
///
/// `Rc`-backed (clonable, `'static`) so it can be captured by the `spawn_local`
/// HTTP tasks and JS callbacks. Single-threaded by construction (one worker).
#[derive(Clone)]
pub struct LogicSink(std::rc::Rc<dyn Fn(LogicEvent)>);

/// Returned by [`LogicSink::send`]. The send is infallible in practice (the
/// closure always accepts the event); the type mirrors a queue `SendError` so
/// existing call sites are unchanged. The failed event is boxed so the `Err`
/// variant stays small, the `LogicEvent` payload being large. `send` is in
/// practice infallible, so this never allocates on the hot path.
pub struct SendError(Box<LogicEvent>);

impl SendError {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: Box<LogicEvent>) -> Self {
        Self(value)
    }

    /// Returns the wrapped value.
    pub fn into_inner(self) -> Box<LogicEvent> {
        self.0
    }
}

impl LogicSink {
    /// Wrap an arbitrary emit closure.
    pub fn new(f: impl Fn(LogicEvent) + 'static) -> Self {
        Self(std::rc::Rc::new(f))
    }

    /// A sink backed by a fresh `conduit` queue, returning the receiver so a
    /// (native) test can drain what Sync emits.
    pub fn channel() -> (Self, ambre_bus::conduit::Receiver<LogicEvent>) {
        let (tx, rx) = ambre_bus::conduit::unbounded();
        (
            Self::new(move |ev| {
                let _ = tx.send(ev);
            }),
            rx,
        )
    }

    /// Emit one event to the Logic thread.
    ///
    /// # Errors
    ///
    /// Never today. The sink owns a closure that swallows a closed queue, and
    /// the signature keeps the `Result` for a sink that can refuse.
    pub fn send(&self, event: LogicEvent) -> Result<(), SendError> {
        (self.0)(as_step_owed(event));
        Ok(())
    }
}

/// Tells a failure that asks the user for a step as that step, never as a
/// generic error. A key setup still owed reads as the setup (`SYNC-134`). An
/// answer naming a word this build does not know reads as an update to
/// install (`SYNC-152`). Every error leaves the worker through
/// [`LogicSink::send`], so this one mapping covers every request.
fn as_step_owed(event: LogicEvent) -> LogicEvent {
    match &event {
        LogicEvent::ApiError { failure, .. } => step_owed(*failure).unwrap_or(event),
        _ => event,
    }
}

/// The step `failure` asks the user for, if it asks for one.
fn step_owed(failure: ApiFailure) -> Option<LogicEvent> {
    match failure {
        ApiFailure::Relay {
            code: Some(ambre_core::RefusalCode::SetupRequired),
            ..
        } => Some(LogicEvent::SetupRequired),
        ApiFailure::Unreadable {
            cause: ambre_core::UnreadableCause::UnknownWord,
        } => Some(LogicEvent::UpdateRequired),
        _ => None,
    }
}

/// The event a sign-in or an unlock that `error` ended reads as: the step
/// it asks for, or an authentication error with its failure.
fn auth_failed(error: impl http::Reported) -> LogicEvent {
    step_owed(error.failure()).unwrap_or_else(|| {
        crate::log::warning!("Sync: authentication failed: {error}");
        LogicEvent::AuthError {
            cause: ambre_bus::AuthFailure::Api(error.failure()),
        }
    })
}

/// Runs `task` on the event loop of the worker.
///
/// The task is boxed here. So `spawn_local` keeps one copy of its scheduler
/// for the worker, not one per future type (`ambre-spec-budgets.md` section 5).
// A spawned task runs on the event loop of the browser alone, and no host
// test reaches one, so cargo-mutants skips the two functions.
#[cfg_attr(test, mutants::skip)]
fn spawn(task: impl std::future::Future<Output = ()> + 'static) {
    spawn_boxed(Box::pin(task));
}

/// The one call into `spawn_local`, over the boxed future of [`spawn`].
#[cfg_attr(test, mutants::skip)]
#[inline(never)]
fn spawn_boxed(task: std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>) {
    wasm_bindgen_futures::spawn_local(task);
}

mod auth;
mod call;
mod call_client;
mod call_link;
mod call_roster;
mod chain;
mod channels;
mod converter;
mod cursors;
mod docfile;
mod documents;
mod error;
mod finalize;
mod fold;
#[cfg(feature = "fuzz")]
pub mod fuzz;
mod generation;
mod http;
mod key_vouch;
mod lifecycle;
mod light;
mod local_store;
mod log;
mod media;
mod members;
mod membership_log;
pub mod mock;
pub mod mock_relay;
mod pipeline;
mod rekey;
mod renewal;
mod sharing;
mod snapshot_trust;
mod space_rotation;
mod updates;
mod websocket;
mod withheld;
/// Web Worker entry point (`#[wasm_bindgen(start)]`): the postMessage bridge to
/// the Main instance. wasm-only.
#[cfg(target_arch = "wasm32")]
mod worker;

/// The metered read of an answer body, for the wasm-tier test of `SYNC-278`.
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub use http::metered_body_len;
pub use http::{RelayHost, Url, UrlKind};

// -----------------------------------------------------------------------------
// Quarantine bounds (`ambre-spec-delta-order.md`)
// -----------------------------------------------------------------------------

/// Soft threshold: once this many updates are quarantined for a document, we ask
/// the server to resend the missing history (`RequestMissing`). The parent
/// snapshot can then arrive and release them.
const QUARANTINE_ALERT: Count<Quarantined> = Count::new(900);
/// Hard cap: beyond this, the oldest update (by `clock`) is dropped to bound the
/// shared heap, and a `SyncWarning` is raised.
const QUARANTINE_LIMIT: Count<Quarantined> = Count::new(1000);

/// Hard cap in bytes: past this many ciphertext bytes quarantined for one
/// document, the oldest update is dropped as past [`QUARANTINE_LIMIT`]. A
/// thousand slots of the relay's largest delta would otherwise hold
/// gigabytes (`SYNC-276`, audit `W206`, `ambre-spec-budgets.md` section 1).
const QUARANTINE_BYTES: Count<ambre_core::Octets> = Count::new(32 * 1024 * 1024);

/// The updates a document holds in quarantine, waiting for their parent.
pub(crate) enum Quarantined {}

impl ambre_core::NarrowCount for Quarantined {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("Quarantined");
}

/// How many of a document's latest sequence numbers the duplicate guard
/// remembers ([`SyncDocumentState::ingested`]).
const APPLIED_WINDOW: Count<Ingested> = Count::new(4096);

/// The sequence numbers the duplicate guard of a document remembers.
enum Ingested {}

impl ambre_core::NarrowCount for Ingested {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("Ingested");
}

/// The open a presence sample comes from: its author's signing key and the
/// session nonce its sealed body carries (`SYNC-287`). One author holds one
/// key for every open, so the key alone cannot tell two opens apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SampleSource {
    author: FixedBytes<PublicKey>,
    session: FixedBytes<SessionNonce>,
}
/// Hard cap on the per-document ephemeral anti-replay map
/// ([`SyncDocumentState::ephemeral_remote_counters`]). In practice the author set
/// is bounded by document membership, but a malicious peer can mint arbitrary
/// signing keys. The map is therefore capped to bound heap. At the cap a new
/// author evicts the least-active (lowest-counter) entry.
const MAX_EPHEMERAL_REMOTE_AUTHORS: Count<RemoteAuthors> = Count::new(512);

/// The document sync sessions a Sync worker holds open.
pub enum OpenDocuments {}

impl ambre_core::NarrowCount for OpenDocuments {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("OpenDocuments");
}

/// The sockets the native mock transport opened.
#[cfg(not(target_arch = "wasm32"))]
enum SocketOpens {}

#[cfg(not(target_arch = "wasm32"))]
impl ambre_core::NarrowCount for SocketOpens {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("SocketOpens");
}

/// The remote authors whose ephemeral counter a document keeps.
enum RemoteAuthors {}

impl ambre_core::NarrowCount for RemoteAuthors {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("RemoteAuthors");
}

// -----------------------------------------------------------------------------
// Internal event bus -- carries WebSocket frames and spawned-task results
// into the non-blocking select loop. Sent by JS callbacks (onmessage /
// onclose / onerror) and by HTTP tasks spawned via spawn_local.
// -----------------------------------------------------------------------------

#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        dead_code,
        reason = "only the wasm socket and request callbacks raise these events"
    )
)]
enum SyncEvent {
    WsMessage(WsServerMsg),
    /// The socket closed with `code`, or failed with no close frame, which
    /// carries no code (`SYNC-070`).
    WsDisconnected {
        code: Option<Bounded<CloseCode>>,
        reason: String,
    },
    WsError,
    /// A frame arrived under a newer wire version than this build's
    /// (`ambre-spec-sync.md`, `SYNC-137`).
    WireOutdated,
    /// Fired by a backoff timer scheduled after a disconnect: time to attempt a
    /// fresh WebSocket connection (see [`schedule_reconnect`](SyncState::schedule_reconnect)).
    ReconnectNow,
    /// Fired by the timer armed when the relay stopped a replay: ask again
    /// from the document's cursor (`SYNC-153`). A later stop or a reconnect
    /// makes the timer stale.
    ReplayRetry {
        document_id: Id<Document>,
        ticket: ambre_core::Seq<generation::ReplayTicket>,
    },
    /// Fired by the timer armed when the relay failed to sequence a delta:
    /// send it again with every later one (`SYNC-159`). A later failure or
    /// refusal makes the timer stale.
    ResendRetry {
        document_id: Id<Document>,
        ticket: ambre_core::Seq<generation::ResendTicket>,
    },
    /// Fired at the end of the period of the read cursors owed: send them,
    /// one request per document (`SYNC-269`).
    CursorsDue,
    /// Fired by the timer armed when a replay ask came inside the relay's
    /// window: send the ask it held (`SYNC-188`).
    ReplayAskDue {
        document_id: Id<Document>,
    },
    /// The WebSocket `onopen` fired: the socket is now OPEN, so (re)subscriptions
    /// can be sent (they would be dropped if sent while still CONNECTING).
    WsOpened,
    /// Result of a silent token refresh triggered by a WS auth rejection (close
    /// 4401). `ok` true means the access token was renewed and the socket can be
    /// reopened; false means the refresh failed and a fallback re-login is needed.
    Reauthenticated {
        ok: bool,
    },
    /// Fired by the timer armed when a token pair was stored: renew it
    /// ahead of its expiry (`SYNC-183`). A later pair makes the timer stale.
    RenewalDue {
        ticket: ambre_core::Seq<generation::RenewalTicket>,
    },
    /// A renewal replaced the pair in the store, on any path: arm the
    /// renewal of the new pair (`SYNC-183`).
    TokensRenewed,
    /// The membership log of the channel of the call was read (`CALL-060`). `None` for a
    /// log that stands on no root (`CALL-134`).
    CallLog {
        meta: Id<Document>,
        log: Option<membership_log::Log>,
    },
    /// The call socket of the relay opened.
    CallRelayOpened,
    /// A frame of the call socket of the relay.
    CallRelay(ambre_protocol::CallServerMsg),
    /// A socket of the call closed: the call ended for this session.
    CallRelayClosed,
    /// The socket of the SFU opened.
    CallSfuOpened,
    /// A message of the socket of the SFU.
    CallSfu(ambre_protocol::SfuMsg),
    /// The switch timer of the call fired (`CALL-027`).
    CallTick,
    /// The leave of this session waited for the answer of the relay as long as it may.
    CallLeaveDue,
    /// The relay answered an ask of who is in a call, `None` on a failed read. The log of the
    /// channel comes with it when the ask read it.
    CallRosterRead {
        call: Id<ambre_core::id::Call>,
        channel: Id<ambre_core::id::Channel>,
        meta: Id<Document>,
        roster: Option<ambre_protocol::signaling::CallRoster>,
        log: Option<membership_log::Log>,
    },
    /// The `meta` document of the channel of a waiting join was read, or `None` when the
    /// channel answered none.
    CallChannel {
        call: Id<ambre_core::id::Call>,
        meta: Option<Id<Document>>,
    },
    SessionReady {
        /// `None` for the silent re-login, which no request asked for.
        id: Option<Seq<Request>>,
        user_id: Id<User>,
        display_name: ambre_core::Validated<ambre_core::DisplayName>,
        email: ambre_core::Email,
        master_seed: MasterSeed,
        signing_key: SigningKey,
        exchange_key: StaticSecret,
        tokens: http::TokenPair,
    },
    /// A provider return was exchanged: the session's tokens, and whether the
    /// account holds its keys. No key is held yet, so no socket opens.
    ProviderSignedIn {
        id: Seq<Request>,
        user_id: Id<User>,
        display_name: ambre_core::Validated<ambre_core::DisplayName>,
        email: ambre_core::Email,
        keys_stored: ambre_bus::KeySetup,
        tokens: http::TokenPair,
    },
    /// The keys of an account were stored: by the setup of a new account
    /// (`SYNC-261`), or by the key setup of a signed-in one (`SYNC-255`). It
    /// carries the new seed and the pair the relay bound to its signing key.
    /// It also carries who the account is.
    KeysSetUp {
        id: Seq<Request>,
        recovery_words: ambre_bus::Confidential<ambre_bus::RecoveryWords>,
        master_seed: MasterSeed,
        signing_key: SigningKey,
        exchange_key: StaticSecret,
        tokens: http::TokenPair,
        user_id: Id<User>,
        display_name: ambre_core::Validated<ambre_core::DisplayName>,
        email: ambre_core::Email,
    },
    DocumentList {
        id: Seq<Request>,
        documents: Vec<DocumentSummary>,
    },
    DocumentCreated {
        id: Seq<Request>,
        document_id: Id<Document>,
        doc_type: DocType,
    },
    DocumentDeleted {
        id: Seq<Request>,
        document_id: Id<Document>,
    },
    DocumentRenamed {
        id: Seq<Request>,
        document_id: Id<Document>,
    },
    OpenDocumentReady {
        document_id: Id<Document>,
        doc_key: DocumentKey,
        /// The snapshot the open adopts whole, and the author its trust
        /// names (`AUTHOR-030`).
        snapshot_payload: Option<StartBlob>,
        snapshot_id: Id<Snapshot>,
        /// Highest delta `seq` folded into `snapshot_payload`, 0 when no snapshot
        /// was downloaded. The post-open `RequestMissing` starts after this seq,
        /// so the history already in the snapshot is not replayed.
        covered_seq: Seq<Relay>,
        /// The latest snapshot, when only its author confirmed it and that
        /// author is another user. `snapshot_payload` is then the reference,
        /// and this one is checked once the replay is done (`AUTHOR-022`).
        held_snapshot: Option<HeldSnapshot>,
        /// Document kind, carried so a not-yet-on-server doc can be lazily created
        /// with the right type on its first outbound update.
        doc_type: DocType,
        /// Whether the document already exists server-side. Local only for a
        /// doc the server answered 404 for: no WebSocket subscription yet,
        /// and the first update triggers lazy creation.
        existence: Existence,
        /// Members' Ed25519 signing keys -> canonical `Id<User>`, harvested from
        /// the same members response that carried the wrapped DocumentKey
        /// (`ambre-spec-membership.md`). Seeds `SyncDocumentState::signing_keys` so
        /// inbound ephemerals resolve to a spoof-proof author. Empty for a
        /// local-only open (no members response, hence no peers to resolve).
        signing_keys: Vec<(FixedBytes<PublicKey>, Id<User>)>,
        /// This client's own role on this document, read from its entry in the
        /// same members response (architecture). `Role::Owner` for a
        /// local-only open. Gates mutating commands in Logic as a UX measure;
        /// the server remains the sole authority.
        role: Role,
        /// The members answer says the document owes a rotation (`SYNC-128`).
        rotation_owed: ambre_core::RotationDebt,
        /// The document's key chain, as the relay serves it (`SYNC-117`).
        /// Empty for a local-only open.
        links: Vec<ambre_protocol::KeyLink>,
        /// The keys whose deltas and snapshots the document takes
        /// (`SYNC-211`). Unchecked for a local-only open.
        writers: membership_log::Writers,
        /// The place of the latest departure the open's log holds, which
        /// the key was checked against (`SYNC-222`, `SYNC-223`).
        departed: Option<Seq<ambre_protocol::LogPlaces>>,
    },
    /// An open accepted the log of a Murmure document. Its members become
    /// roots of every later Murmure open of the session (`SYNC-237`).
    RootsVouched {
        roots: Vec<FixedBytes<ambre_core::InviterPin>>,
    },
    /// The answer to a channel listing (`SyncCommand::OpenChannel`): its
    /// documents, or why the relay refused it.
    ChannelListed {
        channel_id: Id<Channel>,
        /// The space whose catalogue the Logic thread said lists the channel.
        listed_in: Option<Id<Space>>,
        result: Result<Vec<ambre_bus::ChannelDocument>, http::HttpError>,
    },
    /// The open walked the document's log. What it says lands on the session
    /// the open made, for the channels a space lists (the Murmure spaces
    /// spec).
    LogWalked {
        document_id: Id<Document>,
        word: membership_log::SpaceWord,
        /// The heads of the log the open read (`SYNC-207`).
        heads: membership_log::ServedHeads,
    },
    /// A walk this client trusts reached a later head of the log of
    /// `document_id`: an open, a listing, or a write this client signed. The
    /// next write stands on it (audits `P26`, `P27`).
    FootingAdvanced {
        document_id: Id<Document>,
        footing: key_vouch::Footing,
    },
    /// The answer to the rotation of one public channel of a space
    /// (`SyncCommand::RotateSpace`): its new key, sealed for the catalogue.
    ChannelRotated {
        space_id: Id<Space>,
        channel_id: Id<Channel>,
        /// The new entry, and where the log of the channel's `meta` stands
        /// once the re-key walked it.
        result: Result<(ambre_bus::SealedChannelKey, key_vouch::Footing), http::HttpError>,
    },
    /// The answer to a channel creation (`SyncCommand::CreateChannel`): the
    /// sealed key for the catalogue of a public channel.
    ChannelCreated {
        channel_id: Id<Channel>,
        result: Result<Option<ambre_bus::SealedChannelKey>, http::HttpError>,
    },
    /// The answer to a self-join (`SyncCommand::JoinChannel`).
    ChannelJoined {
        channel_id: Id<Channel>,
        result: Result<(), http::HttpError>,
    },
    /// The answer to a channel delete (`SyncCommand::DeleteChannel`).
    ChannelDeleted {
        channel_id: Id<Channel>,
        result: Result<(), http::HttpError>,
    },
    /// The answer to a channel leave (`SyncCommand::LeaveChannel`).
    ChannelLeft {
        channel_id: Id<Channel>,
        result: Result<(), http::HttpError>,
    },
    /// The answer to a segment close (`SyncCommand::CloseSegment`): the
    /// active segment after it, and whether it is the successor asked for.
    SegmentClosed {
        channel_id: Id<Channel>,
        closing: Id<Segment>,
        result: Result<http::ClosedSegmentResponse, http::HttpError>,
    },
    /// The answer to an amendments creation (`SyncCommand::CreateAmendments`):
    /// the document the relay holds for the segment.
    AmendmentsLinked {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        amendments: Id<Document>,
        result: Result<Id<Document>, http::HttpError>,
    },
    /// The answer to a thread creation (`SyncCommand::CreateThread`): the
    /// document the relay holds for the root.
    ThreadLinked {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        root: Id<Message>,
        thread: Id<Document>,
        result: Result<Id<Document>, http::HttpError>,
    },
    /// The answer to a finalize lease request
    /// (`SyncCommand::TakeFinalizeLease`).
    FinalizeLeaseAnswered {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        result: Result<ambre_bus::LeaseAnswer, http::HttpError>,
    },
    /// The answer to a finalize commit (`SyncCommand::FinalizeSegment`).
    SegmentFinalized {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        result: Result<(), http::HttpError>,
    },
    /// The answer to a fold lease request (`SyncCommand::TakeFoldLease`).
    FoldLeaseAnswered {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        result: Result<ambre_bus::FoldAnswer, http::HttpError>,
    },
    /// The answer to a fold commit (`SyncCommand::FoldSegment`).
    SegmentFolded {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        result: Result<(), http::HttpError>,
    },
    /// A confirmation of this client settled the fold of the segment
    /// `document_id`, and the relay erased `amendments` (`SRV-211`).
    AmendmentsFolded {
        document_id: Id<Document>,
        amendments: Id<Document>,
    },
    /// The answer to a direct message request
    /// (`SyncCommand::CreateDirectMessage`): the channel the relay created
    /// under the requested id, or the one that already held its members.
    DirectMessageOpened {
        requested: Id<Channel>,
        result: Result<Id<Channel>, http::HttpError>,
    },
    /// The answer to a listing of a space's public channels
    /// (`SyncCommand::ListSpaceChannels`).
    SpaceChannelsListed {
        space_id: Id<Space>,
        result: Result<Vec<Id<Channel>>, http::HttpError>,
    },
    /// The answer to a listing of the user's channels
    /// (`SyncCommand::ListMemberChannels`).
    MemberChannelsListed {
        /// The listing this answers, as `member_channels_asked` counted it.
        asked: ambre_core::Seq<generation::Listing>,
        result: Result<channels::MemberListing, http::HttpError>,
    },
    /// The relay's activity answer (`SyncCommand::FetchActivity`,
    /// `SyncCommand::MarkAllRead`).
    ActivityFetched {
        result: Result<Vec<ambre_bus::DocumentActivity>, http::HttpError>,
    },
    /// The converter module's bytes, or why none came
    /// (`SyncCommand::FetchConverter`).
    ConverterFetched {
        family: ambre_core::ConverterFamily,
        result: Result<ambre_core::Payload<converter::FetchedModule>, ambre_core::ConverterRefusal>,
    },
    /// Opening a server-side document failed because its wrapped DocumentKey
    /// could not be unwrapped (wrong exchange key, tampered server row, or a
    /// genuine authorization failure). Surfaced as an `AuthError` instead of
    /// silently opening with a random key (H3).
    OpenDocumentFailed {
        document_id: Id<Document>,
        cause: OpenFailure,
    },
    /// A local-only document was lazily materialized server-side. Flushes the
    /// buffered updates and opens the WebSocket subscription.
    DocumentMaterialized {
        document_id: Id<Document>,
    },
    /// Lazy creation failed. The `creating` guard is cleared so a later update
    /// retries; the buffered updates are kept.
    DocumentCreateFailed {
        document_id: Id<Document>,
        failure: http::Failed,
    },
    HttpError {
        id: Seq<Request>,
        failure: ApiFailure,
    },
    /// TOFU phase 1 (`ambre-spec-key-verification.md`): the invitee was resolved and
    /// their exchange key fetched, with a formatted fingerprint. No key wrapped
    /// yet -- the Main thread decides trust before phase 2 wraps.
    InviteeKeyReady {
        id: Seq<Request>,
        document_id: Id<Document>,
        user_id: Id<User>,
        display_name: ambre_core::Validated<ambre_core::DisplayName>,
        email: ambre_core::Email,
        /// Whether a provider proved the address the invite names.
        email_proof: ambre_core::EmailProof,
        exchange_pub_key: FixedBytes<ambre_core::ExchangePublicKey>,
        fingerprint: ambre_core::Opaque<ambre_core::KeyFingerprints>,
    },
    /// The key a user the client named is served with, and its fingerprint.
    /// Nothing is wrapped: the Main thread compares it with the pin first.
    MemberKeyReady {
        id: Seq<Request>,
        user_id: Id<User>,
        exchange_pub_key: FixedBytes<ambre_core::ExchangePublicKey>,
        fingerprint: ambre_core::Opaque<ambre_core::KeyFingerprints>,
    },
    /// Invite completed: the invitee's DocumentKey was wrapped via ECDH and the
    /// server accepted the share request.
    InviteSent {
        id: Seq<Request>,
        document_id: Id<Document>,
        user_id: Id<User>,
        display_name: ambre_core::Validated<ambre_core::DisplayName>,
        /// The pin of the inviter's signing key, for the invite link
        /// (`SYNC-208`).
        inviter: FixedBytes<ambre_core::InviterPin>,
        /// The pin of the signing key the entry names for the invitee. This
        /// client signed it, so it roots the later Murmure opens (`SYNC-237`).
        invitee: FixedBytes<ambre_core::InviterPin>,
    },
    /// Key rotation initiated by this client is finished: the server has the new
    /// `doc_key_enc` entries and the new key must be swapped locally.
    RevocationComplete {
        id: Seq<Request>,
        document_id: Id<Document>,
        new_doc_key: DocumentKey,
        /// The link the revoke left, from `new_doc_key` back to the key it
        /// replaced (`SYNC-116`).
        link: ambre_protocol::KeyLink,
    },
    /// A refresh of the member directory alone failed. A log head under
    /// check asks again at the next signed head (`SYNC-220`).
    DocumentMembersFailed {
        document_id: Id<Document>,
        /// Which listing of this document failed (`SYNC-037`).
        asked: ambre_core::Seq<generation::Listing>,
    },
    /// Member list for a document has been fetched from the server (decoded,
    /// plaintext names/emails).
    DocumentMembersReady {
        /// The request to answer, `None` for a refresh of the directory alone.
        id: Option<Seq<Request>>,
        /// Which listing of this document answered (`SYNC-037`).
        asked: ambre_core::Seq<generation::Listing>,
        document_id: Id<Document>,
        /// What the walked log says a channel listed in this space must heed.
        word: membership_log::SpaceWord,
        /// Whether the relay says the document owes a rotation (`SYNC-128`).
        rotation_owed: ambre_core::RotationDebt,
        members: Vec<MemberInfo>,
        /// Refreshed presence directory (`ambre-spec-membership.md`), harvested from
        /// the same response. Membership just changed often enough for someone to
        /// ask for the list. This is where a new member becomes resolvable and a
        /// revoked one stops. Ignored when the document is not open locally.
        signing_keys: Vec<(FixedBytes<PublicKey>, Id<User>)>,
        /// The heads of the log the same answer served (`SYNC-207`).
        heads: membership_log::ServedHeads,
        /// The keys whose deltas the document takes, from the same walk
        /// (`SYNC-211`). A newcomer's delta held for this answer lands here.
        writers: membership_log::Writers,
        /// The place of the latest departure the same walk holds. A writer
        /// whose key predates it owes a rotation, whatever the relay says
        /// of the debt (`SYNC-223`).
        departed: Option<Seq<ambre_protocol::LogPlaces>>,
        /// The latest revoke the same walk holds, and the key it installed.
        /// A writer whose key does not lead to it holds its edits
        /// (`SYNC-224`).
        revoked: Option<membership_log::NamedKey>,
        /// Where the same walk found the log starts, against the space that
        /// lists it (`SYNC-231`).
        root: membership_log::Root,
    },
    /// User search results from the server (decoded, plaintext).
    UserSearchReady {
        id: Seq<Request>,
        users: Vec<UserInfo>,
    },
    /// A document member's role was updated server-side.
    MemberRoleUpdatedEvent {
        id: Seq<Request>,
        document_id: Id<Document>,
        user_id: Id<User>,
        role: Role,
    },
    /// A remote key rotation was performed: the new DocumentKey has been fetched
    /// and unwrapped via ECDH; the local key cache must be swapped.
    KeyRotated {
        document_id: Id<Document>,
        new_doc_key: DocumentKey,
        /// The document's chain as the relay serves it after the rotation
        /// (`SYNC-117`).
        links: Vec<ambre_protocol::KeyLink>,
        /// The latest departure the key was checked against (`SYNC-223`).
        departed: Option<Seq<ambre_protocol::LogPlaces>>,
    },
    /// This user's departure landed: the relay removed their own share
    /// (`SYNC-127`). The document closes locally.
    DocumentLeft {
        id: Seq<Request>,
        document_id: Id<Document>,
    },
    /// The document owes a rotation: a member left and nobody re-keyed since
    /// (`SYNC-128`). Found in a members answer, never guessed.
    DebtFound {
        document_id: Id<Document>,
    },
    /// This client paid the owed rotation: the relay stored `new_doc_key` for
    /// every member, and it becomes the current key (`SYNC-128`).
    DebtSettled {
        document_id: Id<Document>,
        new_doc_key: DocumentKey,
        /// The link the rotation left (`SYNC-116`).
        link: ambre_protocol::KeyLink,
        /// The latest departure the log the payment read holds. The link
        /// signs a head past it (`SYNC-223`).
        departed: Option<Seq<ambre_protocol::LogPlaces>>,
    },
    /// The debt was paid by someone else before this client could. Their key
    /// is fetched like any other rotation's.
    DebtGone {
        document_id: Id<Document>,
    },
    /// Paying the debt failed. The next frame tries again.
    DebtUnsettled {
        document_id: Id<Document>,
        message: String,
    },
    /// Media upload completed: the encrypted blob was PUT to S3 and the server
    /// confirmed the insertion in the `media` table.
    MediaUploaded {
        id: Seq<Request>,
        document_id: Id<Document>,
        content_hash: ambre_core::Validated<ambre_core::ContentHash>,
    },
    /// The relay holds no upload of this user's for the blob any more.
    MediaErased {
        id: Seq<Request>,
        document_id: Id<Document>,
        content_hash: ambre_core::Validated<ambre_core::ContentHash>,
    },
    /// Snapshot upload completed: the sealed blob was PUT to S3 and the server
    /// confirmed the insertion (and broadcast `SnapshotAvailable`). The returned
    /// `snapshot_id` becomes this document's current parent for future deltas.
    SnapshotUploaded {
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
    },
    /// A snapshot a peer announced was fetched, or could not be (`None`). Only
    /// its contents reaching the Logic thread make it a known parent
    /// (`SYNC-030`).
    SnapshotFetched {
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        /// The last delta its author says it folds in.
        covered_seq: Seq<Relay>,
        blob: Option<Payload<pipeline::FetchedBlob>>,
    },
    /// Media download completed: the encrypted blob was fetched from S3 and
    /// decrypted. `data` is the raw plaintext bytes ready for display.
    MediaDownloaded {
        id: Seq<Request>,
        document_id: Id<Document>,
        content_hash: ambre_core::Validated<ambre_core::ContentHash>,
        data: ambre_core::Payload<ambre_core::MediaBytes>,
    },
}

impl SyncEvent {
    /// The failure of the service-plane request `id`, from a relay's answer
    /// or from this device.
    fn http_error(id: Seq<Request>, error: impl http::Reported) -> Self {
        crate::log::warning!("Sync: request {id} failed: {error}");
        Self::HttpError {
            id,
            failure: error.failure(),
        }
    }
}

// -----------------------------------------------------------------------------
// Per-document sync session
// -----------------------------------------------------------------------------

struct SyncDocumentState {
    document_id: Id<Document>,
    doc_key: DocumentKey,
    /// Document kind, used for lazy server creation on the first outbound update.
    doc_type: DocType,
    /// Whether the document row exists server-side. Starts local only for a
    /// local-only doc, and moves to the relay once lazily created, so the POST
    /// and the WebSocket subscription happen exactly once.
    existence: Existence,
    /// A lazy-creation POST is in flight. Guards against firing several POSTs
    /// when updates arrive back-to-back before the first one completes.
    creating: bool,
    /// Outbound update bodies, each carrying its session nonce, buffered while
    /// the document is being lazily created server-side. Flushed in order once
    /// it is materialized.
    pending_updates: Vec<pipeline::UpdateFrame>,
    /// Uploads asked for while the document is local-only. The relay signs no
    /// link for a document it does not hold, so each one waits for the lazy
    /// creation (`SYNC-145`).
    pending_uploads: Vec<(Seq<Request>, ambre_core::Payload<ambre_core::MediaBytes>)>,
    /// Invites asked for while the document is local-only. The relay holds
    /// no member log for it yet, so each one waits for the lazy creation
    /// (`SYNC-246`).
    pending_invites: Vec<members::PendingInvite>,
    /// Snapshot this document's updates are currently based on. Placeholder
    /// until the snapshot flow lands; carried in every `UpdatePublicData`.
    current_snapshot_id: Id<Snapshot>,
    local_clock: Seq<DeltaAuthor>,
    /// Monotonic counter for outbound ephemerals (anti-replay at the receiver).
    ephemeral_counter: Seq<Ephemerals>,
    /// Highest server-assigned `seq` seen for this document. Used as
    /// `after_seq` when requesting the missing history.
    last_seq: Seq<Relay>,
    /// The latest sequence numbers this client ingested, applied or held in
    /// quarantine. A reconnect replay can overlap a broadcast received during
    /// the same round trip, and the second copy is dropped here (`SYNC-087`).
    /// Bounded by [`APPLIED_WINDOW`].
    ingested: BTreeSet<Seq<Relay>>,
    /// Snapshots whose contents the Logic thread already holds. An inbound
    /// `Update` whose parent `snapshot_id` is here can be applied immediately;
    /// otherwise it is quarantined under that id.
    known_snapshots: HashSet<Id<Snapshot>>,
    /// Updates held back because their parent snapshot is not yet known, keyed by
    /// that parent `snapshot_id`. Stored sealed; decrypted only on release.
    /// Bounded by [`QUARANTINE_LIMIT`] (oldest-by-`clock` dropped past the cap).
    /// Each keeps the number the relay gave it, which the Logic thread's
    /// snapshot check reads.
    quarantine: HashMap<Id<Snapshot>, Vec<(Seq<Relay>, Update)>>,
    /// The missing history was asked for this gap: the quarantine crossed the
    /// soft threshold and has not dropped back under it since. One request per
    /// gap, never one per arrival (`SYNC-031`).
    gap_requested: bool,
    /// LRU cache of previous DocumentKeys (max 10). A frame that names one of
    /// them opens under it: it was sealed before its author learned of the
    /// rotation.
    old_keys: Vec<DocumentKey>,
    /// The links the rotations left: a key older than the cache opens through
    /// them, walked back from the active key (`SYNC-119`).
    chain: chain::Chain,
    /// Per-open ephemeral anti-replay counter. Keyed by the author's Ed25519
    /// verifying key and by the session nonce of the open the sample names.
    /// A sender counts from 1 at each open (`SYNC-287`, audit `P22`). An inbound
    /// ephemeral whose counter is <= the stored value is silently dropped.
    ephemeral_remote_counters: HashMap<SampleSource, Seq<Ephemerals>>,
    /// Directory of this document's members' Ed25519 signing keys, mapping an
    /// ephemeral author's verifying key to their canonical `Id<User>`
    /// (`ambre-spec-membership.md`). Built from the members response already fetched at open and
    /// rebuilt when membership changes. A key absent here is not resolved, so the
    /// sample is dropped rather than attributed to a guessed identity.
    ///
    /// This is a *membership* directory, not a *trust* store: it trusts the
    /// server's key mapping. Detecting a server that substitutes a member's keys
    /// is the TOFU trusted-keys store's job (architecture) and is orthogonal.
    signing_keys: HashMap<FixedBytes<PublicKey>, Id<User>>,
    /// The keys whose deltas and snapshots this document takes, from the
    /// latest walk of its log (`SYNC-211`).
    writers: membership_log::Writers,
    /// The latest departure the current key was checked against, or found
    /// owed for. A walk that holds a later one leaves the current key in
    /// the leaver's hands, so a writer owes a rotation (`SYNC-223`).
    departed: Option<Seq<ambre_protocol::LogPlaces>>,
    /// Deltas whose signer no writer of the walk held names, kept sealed
    /// until a listing asked after each arrived answers (`SYNC-211`).
    /// Bounded by [`QUARANTINE_LIMIT`], as the quarantine is.
    unvouched: Vec<Unvouched>,
    /// Deltas of a channel document that a listing refused as no writer's,
    /// kept sealed. A space that lists the channel can open later, or change
    /// its word. The next walk of the log decides them again (`SYNC-230`).
    /// The refusal stands until that walk. Bounded by
    /// [`QUARANTINE_LIMIT`], as the quarantine is.
    refused: Vec<Unvouched>,
    /// Whether the document may land a delta and seal a frame while its log
    /// walks again under the rule of a space (`SYNC-232`).
    gate: channels::LogGate,
    /// How many member listings of this document were asked, and which one
    /// the directory holds. Two listings can cross on the way, and the answer
    /// to the older one changes nothing. It cannot bring back the key of a
    /// member the newer one dropped (`SYNC-037`, `SYNC-094`).
    directory_asked: ambre_core::Issuer<generation::Listing>,
    directory_held: ambre_core::Seq<generation::Listing>,
    /// When the document is a space: what a channel it lists must heed, from
    /// the latest walk of its log (the Murmure spaces spec).
    word: membership_log::SpaceWord,
    /// The heads of the log the relay last served, and the check of a log
    /// head a peer signed past them (`SYNC-207`).
    head_watch: withheld::HeadWatch,
    /// Where the log the open accepted stands for this client. Each path
    /// that writes members or keys walks from it, and refuses a log that
    /// does not pass through its head (audits `P26`, `P27`). `None` until an
    /// open or a creation accepts the log.
    footing: Option<key_vouch::Footing>,
    /// This client's own role on this document, resolved from the members
    /// response at open. Carried here rather than emitted straight through
    /// because `DocumentReady` fires from two places: immediately for a
    /// local-only / offline open, or after the initial replay. Both must
    /// report the same role.
    ///
    /// Defaults to `Role::Owner`: a document with no members response is
    /// local-only, and its creator owns it. The server stays authoritative
    /// either way (architecture) -- this drives UX only.
    role: Role,
    /// True between sending the post-open `RequestMissing` and receiving its
    /// `MissingUpdates` reply. The initial catch-up replay is the point at which
    /// the document is fully hydrated: snapshot plus every delta the server
    /// already holds. `DocumentReady` is emitted when this clears, not at open.
    /// Opening would dismiss the UI loading overlay before delta-only documents
    /// have any content. Stays `false` for local-only / offline opens, which
    /// emit `DocumentReady` immediately (no replay to wait for).
    awaiting_initial_hydration: bool,
    /// Updates sent and not yet echoed, and the rotation this document owes
    /// (`SYNC-128`, `SYNC-129`).
    rekey: rekey::Rekey,
    /// The snapshots handed to the Logic thread to check, awaiting its
    /// verdict (`AUTHOR-016`, `AUTHOR-022`).
    checks: HashMap<Id<Snapshot>, PendingCheck>,
    /// A snapshot only its author confirmed, held until the open has replayed
    /// every delta the relay kept. It is checked then (`AUTHOR-022`).
    held_snapshot: Option<HeldSnapshot>,
    /// The Logic thread evicted the document at this horizon. The session
    /// becomes a light subscription once every delta sent is acknowledged
    /// (`RESID-009`).
    released: Option<Seq<Relay>>,
}

/// A delta held until the members are listed again (`SYNC-211`).
struct Unvouched {
    /// The latest listing asked when the delta arrived. Only the answer to a
    /// later one was read after the delta existed, so only it decides.
    seen: ambre_core::Seq<generation::Listing>,
    seq: Seq<Relay>,
    update: Update,
}

/// A snapshot the Logic thread is checking.
struct PendingCheck {
    /// The signing key its sealed blob verified under: who built it.
    author: FixedBytes<PublicKey>,
    covered_seq: Seq<Relay>,
    /// The digest of the sealed blob under check, which a confirmation
    /// names (`AUTHOR-031`, audit `W101`).
    blob: FixedBytes<ambre_core::SnapshotDigest>,
    /// The opened snapshot, kept on open only: a match adopts it.
    payload: Option<ambre_core::Payload<ambre_core::SnapshotBytes>>,
}

/// A snapshot only its author confirmed, still sealed (`SRV-128`).
/// Whether the relay holds the row of a document yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Existence {
    /// Local only: the relay answered 404 for its members, and the first
    /// update creates it.
    LocalOnly,
    /// The relay holds its row.
    OnRelay,
}

#[derive(Debug, Clone)]
pub(crate) struct HeldSnapshot {
    pub snapshot_id: Id<Snapshot>,
    pub covered_seq: Seq<Relay>,
    pub blob: Payload<pipeline::FetchedBlob>,
}

/// The snapshot an open adopts whole, still sealed, and the key it must
/// verify under: the author its confirmation names, or this user
/// (`AUTHOR-030`).
pub(crate) struct StartBlob {
    pub blob: Payload<pipeline::FetchedBlob>,
    pub author: FixedBytes<PublicKey>,
}

impl SyncDocumentState {
    fn new(
        document_id: Id<Document>,
        doc_key: DocumentKey,
        doc_type: DocType,
        existence: Existence,
    ) -> Self {
        // Anchor the base to the document's genesis snapshot, which every client
        // derives identically. Until a real snapshot is uploaded, the earliest
        // deltas reference this id. A fresh open (or a peer) then recognises their
        // parent instead of quarantining them forever. A later `OpenDocumentReady`
        // or `SnapshotAvailable` carrying a real snapshot id supersedes it.
        let current_snapshot_id = Id::<Snapshot>::genesis(document_id);
        Self {
            document_id,
            doc_key,
            doc_type,
            existence,
            creating: false,
            pending_updates: Vec::new(),
            pending_uploads: Vec::new(),
            pending_invites: Vec::new(),
            current_snapshot_id,
            local_clock: Seq::<DeltaAuthor>::new(0),
            ephemeral_counter: Seq::<Ephemerals>::new(0),
            last_seq: Seq::<Relay>::new(0),
            ingested: BTreeSet::new(),
            known_snapshots: HashSet::from([current_snapshot_id]),
            quarantine: HashMap::new(),
            gap_requested: false,
            old_keys: Vec::new(),
            chain: chain::Chain::new(document_id, Vec::new()),
            ephemeral_remote_counters: HashMap::new(),
            signing_keys: HashMap::new(),
            writers: membership_log::Writers::Unchecked,
            departed: None,
            unvouched: Vec::new(),
            refused: Vec::new(),
            gate: channels::LogGate::default(),
            directory_asked: ambre_core::Issuer::default(),
            directory_held: ambre_core::Seq::ZERO,
            word: membership_log::SpaceWord::default(),
            head_watch: withheld::HeadWatch::default(),
            footing: None,
            role: Role::Owner,
            awaiting_initial_hydration: false,
            rekey: rekey::Rekey::default(),
            checks: HashMap::new(),
            held_snapshot: None,
            released: None,
        }
    }

    /// Records `seq` as ingested, and says whether this is its first arrival.
    /// The window keeps the latest [`APPLIED_WINDOW`] numbers; a reconnect
    /// overlap is far shorter than that.
    fn first_arrival(&mut self, seq: Seq<Relay>) -> bool {
        if !self.ingested.insert(seq) {
            return false;
        }
        if Count::of(self.ingested.len()) > APPLIED_WINDOW {
            self.ingested.pop_first();
        }
        true
    }

    /// The ciphertext bytes quarantined across all parent snapshots.
    fn quarantine_bytes(&self) -> Count<ambre_core::Octets> {
        Count::of(
            self.quarantine
                .values()
                .flatten()
                .map(|(_, update)| update.ciphertext.as_bytes().len())
                .sum(),
        )
    }

    /// Whether the quarantine passes one of its two caps: the count or the
    /// bytes.
    fn quarantine_overflows(&self) -> bool {
        self.quarantine_len() >= QUARANTINE_LIMIT || self.quarantine_bytes() > QUARANTINE_BYTES
    }

    /// Total number of quarantined updates across all parent snapshots.
    fn quarantine_len(&self) -> Count<Quarantined> {
        Count::of(self.quarantine.values().map(Vec::len).sum())
    }

    /// The key a frame names: the active key, one in the [`old_keys`] cache,
    /// or one the chain reaches from the active key. `None` when this client
    /// reaches no such key (`SYNC-045`, `SYNC-119`).
    fn key_named(&self, id: FixedBytes<KeyId>) -> Option<DocumentKey> {
        chain::find_key(&self.doc_key, &self.old_keys, &self.chain, id)
    }

    /// The key each frame of this document is sealed under. Every seal under
    /// the document key reads it here. It is refused while the log walks
    /// again, and after a walk found the log rooted outside its space. A
    /// writer of the earlier walk may have handed the relay that key
    /// (`SYNC-232`, `SYNC-234`).
    pub(crate) fn sealing_key(&self) -> Result<&DocumentKey, error::SyncError> {
        if matches!(self.gate, channels::LogGate::Open) {
            Ok(&self.doc_key)
        } else {
            Err(error::SyncError::LogUnsettled)
        }
    }

    /// The keys of this document, for a task that opens a blob later.
    fn keyring(&self) -> chain::Keyring {
        chain::Keyring {
            document_id: self.document_id,
            current: self.doc_key.clone(),
            previous: self.old_keys.clone(),
            chain: self.chain.clone(),
        }
    }

    /// Opens an ephemeral with the one key it names, and with no other
    /// (`SYNC-045`).
    fn open_ephemeral_named(
        &self,
        eph: &ambre_protocol::Ephemeral,
    ) -> Result<ambre_core::Payload<ambre_core::EphemeralBytes>, pipeline::PipelineError> {
        let key = self
            .key_named(eph.public_data.key_id)
            .ok_or(pipeline::PipelineError::WrongKey)?;
        pipeline::open_ephemeral(eph, &key)
    }

    /// Anti-replay gate for an inbound ephemeral. Returns `true` when `counter`
    /// passes the last value seen for `author_pk` in the open `session` names,
    /// and records it. Returns `false` for a replay, which the caller drops.
    /// A reopened author names a new session, so a peer that stayed open
    /// takes its samples from counter 1 again (`SYNC-287`, audit `P22`).
    ///
    /// The counter map is bounded by [`MAX_EPHEMERAL_REMOTE_AUTHORS`]: when a new
    /// author would exceed the cap, the least-active entry (lowest counter) is
    /// evicted first. Eviction resets that author's baseline, so a later *stale*
    /// ephemeral from an evicted author could replay once. That is harmless for
    /// presence, and it cannot affect an active author whose counter keeps rising.
    fn accept_ephemeral_counter(
        &mut self,
        author_pk: &FixedBytes<PublicKey>,
        session: &FixedBytes<SessionNonce>,
        counter: Seq<Ephemerals>,
    ) -> bool {
        let source = SampleSource {
            author: *author_pk,
            session: *session,
        };
        // A sender counts from 1 at each open, so the floor of an open not
        // seen yet is the counter before its first send.
        let last = self
            .ephemeral_remote_counters
            .get(&source)
            .copied()
            .unwrap_or(Seq::<Ephemerals>::new(0));
        if counter <= last {
            return false;
        }
        let is_new = !self.ephemeral_remote_counters.contains_key(&source);
        if is_new && Count::of(self.ephemeral_remote_counters.len()) >= MAX_EPHEMERAL_REMOTE_AUTHORS
        {
            // Evict the least-active (lowest-counter) author to make room.
            let evict = self
                .ephemeral_remote_counters
                .iter()
                .min_by_key(|&(_, &c)| c)
                .map(|(k, _)| *k);
            if let Some(evict) = evict {
                self.ephemeral_remote_counters.remove(&evict);
            }
        }
        self.ephemeral_remote_counters.insert(source, counter);
        true
    }

    /// Replace the members' signing-key directory (`ambre-spec-membership.md`).
    /// Called with the members response fetched at open, and again whenever
    /// membership changes, so a revoked member stops resolving.
    ///
    /// A key that is not valid hex or not 32 bytes refused the members answer
    /// when it was read. So every entry here keys a member.
    fn set_signing_keys(
        &mut self,
        entries: impl IntoIterator<Item = (FixedBytes<PublicKey>, Id<User>)>,
    ) {
        self.signing_keys = entries.into_iter().collect();
    }

    /// Takes the writers a walk of the log granted (`SYNC-211`). A document
    /// of a channel walks the log of the `meta` its open names, so the
    /// writers it takes are the channel's (`SYNC-229`).
    fn set_writers(&mut self, writers: membership_log::Writers) {
        self.writers = writers;
    }

    /// Resolve the canonical `Id<User>` behind an ephemeral's authoring Ed25519
    /// key (`ambre-spec-membership.md`). Presence must be keyed by a per-user
    /// identity, not by the per-device signing key. The sender-supplied payload
    /// can never be trusted for identity, so the key is matched against the
    /// members directory, on this trusted side.
    ///
    /// `None` means the author is not a known member of this document (or the
    /// directory has not been filled yet). The caller drops the sample: presence
    /// is best-effort, so showing nothing is preferable to attributing a cursor
    /// to a guessed identity.
    fn resolve_ephemeral_author(&self, author_pk: &FixedBytes<PublicKey>) -> Option<Id<User>> {
        self.signing_keys.get(author_pk).copied()
    }

    /// Opens the snapshot an open adopts whole, under the one key it names. A
    /// snapshot sealed before a rotation opens with a key the chain recovers
    /// (`SYNC-119`). A blob another key signed than the author its trust
    /// names is refused, so a confirmation stands for one blob alone
    /// (`AUTHOR-030`).
    fn open_start_blob(
        &self,
        start: &StartBlob,
        served: pipeline::ServedSnapshot,
    ) -> Result<ambre_core::Payload<ambre_core::SnapshotBytes>, pipeline::PipelineError> {
        let (payload, signer) = self.open_signed_snapshot(&start.blob, served)?;
        if signer != start.author {
            return Err(pipeline::PipelineError::WrongAuthor);
        }
        Ok(payload)
    }

    /// Opens a snapshot blob, and names the signing key it verified under: the
    /// member who built it, which the snapshot check needs (`AUTHOR-021`).
    /// A blob that seals another identifier or another horizon than
    /// `served` names is refused before any decryption. So a relay cannot
    /// serve one snapshot for another (`SYNC-257`), nor skip or replay the
    /// deltas past it (`SYNC-259`).
    fn open_signed_snapshot(
        &self,
        blob: &Payload<pipeline::FetchedBlob>,
        served: pipeline::ServedSnapshot,
    ) -> Result<
        (
            ambre_core::Payload<ambre_core::SnapshotBytes>,
            FixedBytes<PublicKey>,
        ),
        pipeline::PipelineError,
    > {
        // No snapshot is adopted while the log walks again: its author may be
        // a writer the relay forged (`SYNC-234`).
        if !matches!(self.gate, channels::LogGate::Open) {
            return Err(pipeline::PipelineError::LogUnsettled);
        }
        let snapshot = pipeline::decode_snapshot_blob(blob, self.document_id, served)?;
        // A snapshot its author could not have written as a delta is refused
        // whole (`SYNC-211`).
        if !self.writers.admits(&snapshot.public_data.pub_key) {
            return Err(pipeline::PipelineError::NotAWriter);
        }
        let key = self
            .key_named(snapshot.public_data.key_id)
            .ok_or(pipeline::PipelineError::WrongKey)?;
        let payload = pipeline::open_snapshot(&snapshot, &key)?;
        Ok((payload, snapshot.public_data.pub_key))
    }

    /// Opens an update with the one key it names, and with no other
    /// (`SYNC-045`).
    fn open_update_named(
        &self,
        update: &ambre_protocol::Update,
    ) -> Result<pipeline::UpdateFrame, pipeline::PipelineError> {
        let key = self
            .key_named(update.public_data.key_id)
            .ok_or(pipeline::PipelineError::WrongKey)?;
        pipeline::open_update(update, &key)
    }

    /// Opens an update and names what its signature proves: the PeerID its
    /// signer and nonce derive, and the member the signer resolves to
    /// (`SYNC-114`, `AUTHOR-013`). The relay's number `seq` travels beside.
    /// A body too short for its parts is refused.
    fn open_signed_update(
        &self,
        update: &ambre_protocol::Update,
        seq: Seq<Relay>,
    ) -> Result<SignedDelta, pipeline::PipelineError> {
        let signer = &update.public_data.pub_key;
        // A signer the log never granted a writing role writes nothing, though
        // it holds a key the cache or the chain reaches (`SYNC-211`).
        if !self.writers.admits(signer) {
            return Err(pipeline::PipelineError::NotAWriter);
        }
        let body = pipeline::split_update_body(self.open_update_named(update)?)?;
        Ok(SignedDelta {
            seq,
            peer: ambre_crypto::peer::derive_peer_id(signer, &body.nonce),
            nonce: body.nonce,
            user: self.signing_keys.get(signer).copied(),
            summary: body.summary.map(Into::into),
            payload: body.delta,
        })
    }

    /// Replaces the active document key with `new_key`, pushing the previous
    /// active key onto the front of the [`old_keys`] LRU cache. The cache is
    /// bounded to 10 entries; excess keys are dropped and zeroized on
    /// `DocumentKey`'s `Drop`.
    fn rotate_key(&mut self, new_key: DocumentKey) {
        let old_key = std::mem::replace(&mut self.doc_key, new_key);
        self.old_keys.insert(0, old_key);
        if self.old_keys.len() > 10 {
            self.old_keys.truncate(10);
        }
    }
}

// -----------------------------------------------------------------------------
// SyncState
// -----------------------------------------------------------------------------

/// Whether a client frame carries a sealed payload, which only the seal may
/// build (`SYNC-001`).
fn carries_payload(msg: &WsClientMsg) -> bool {
    matches!(msg, WsClientMsg::Update(_) | WsClientMsg::Ephemeral(_))
}

pub struct SyncState {
    /// Outbound sink: Sync -> Logic thread (postMessage in the worker).
    logic_tx: LogicSink,
    /// Base URL for REST API calls (for example `"api.ambre.app"`).
    api_base: crate::http::Url<crate::http::RelayHost>,
    /// URL scheme: `Http` on localhost, `Https` otherwise.
    url_scheme: crate::http::UrlScheme,
    /// Access + refresh tokens returned by login/setup and rotated by HTTP 401
    /// retry helpers. Kept in worker memory only.
    tokens: http::TokenStore,
    /// Authenticated user id, set after Unlock/SetupAccount.
    user_id: Option<Id<User>>,

    // -- Session-level crypto (derived once at Unlock, zeroized at Logout) ----
    master_seed: Option<MasterSeed>,
    /// Ed25519 signing key, derived from the master seed. Signs every outbound
    /// envelope; its public half is the author identity in `public_data`.
    signing_key: Option<SigningKey>,
    /// X25519 exchange key, derived from the master seed. Used to wrap/unwrap
    /// document keys when sharing (invite + key rotation on revoke).
    exchange_key: Option<StaticSecret>,
    /// The `meta` documents of the public channels a catalogue lists, with
    /// that space, as the Logic thread reports them on each open. Their logs
    /// admit a self-join (`SYNC-106`) and heed their space; no other log
    /// does either.
    self_join_logs: HashMap<Id<Document>, Id<Space>>,
    /// The `meta` of each open document of a channel, as the Logic thread
    /// read it from the sealed state of the channel. That `meta`'s log and
    /// links vouch for the key and the writers of the document (`SYNC-229`).
    channel_metas: HashMap<Id<Document>, Id<Meta>>,
    /// The space whose catalogue lists each channel, as the Logic thread
    /// last said it at an open or a later mark (`SYNC-106`, `SYNC-230`).
    listed_channels: HashMap<Id<Channel>, Id<Space>>,
    /// The `meta` of each channel a listing answered, so a later mark finds
    /// the log it applies to (`SYNC-230`).
    listing_metas: HashMap<Id<Channel>, Id<Document>>,
    /// The `meta` and the space of each public channel this client is
    /// creating, until the relay answers. A created one's log admits a
    /// self-join at once, as its catalogue lists it (`SYNC-190`).
    creating_public: HashMap<Id<Channel>, (Id<Document>, Id<Space>)>,
    /// The rotations this admin client drives after a leave, per space
    /// document (the Murmure spaces spec).
    space_rotations: HashMap<Id<Document>, space_rotation::SpaceRotation>,
    /// Where each membership log stands for this client, per log document:
    /// the `meta` of a channel, or the document itself. Every footing a
    /// document lands also lands here, and outlives the close of that
    /// document. So a re-key of a channel this client walked earlier in the
    /// session stands on the latest head it walked or signed (audit `P27`).
    log_footings: HashMap<Id<Document>, key_vouch::Footing>,

    // -- Single multiplexed WebSocket -----------------------------------------
    ws: Option<WebSocket>,
    /// JS closures attached to the WebSocket (onmessage / onclose / onerror).
    /// Kept alive to prevent GC. Each is type-erased to `JsValue` because the
    /// callback signatures differ (MessageEvent / CloseEvent / Event).
    _onmessage: Option<Closure<dyn FnMut(JsValue)>>,
    _onclose: Option<Closure<dyn FnMut(JsValue)>>,
    _onerror: Option<Closure<dyn FnMut(JsValue)>>,
    _onopen: Option<Closure<dyn FnMut(JsValue)>>,
    /// Closure kept alive for the pending reconnect `setTimeout`, if any.
    _reconnect_timer: Option<Closure<dyn FnMut()>>,
    /// The socket's reconnect state: the backoff and its deadline. A relay
    /// that rejects the upgrade (for example a 429) is not hammered.
    lifecycle: lifecycle::Lifecycle,
    /// The wall clock the lifecycle reads. A host test swaps in one it
    /// advances by hand.
    clock: Rc<dyn Fn() -> ambre_core::Instant<ambre_core::UnixMs>>,
    /// The renewal of the access token ahead of its expiry (`SYNC-183`).
    renewal: renewal::Renewal,
    /// The gate every renewal passes, shared by each [`http::AuthContext`]
    /// this state hands out: two renewals never run at once (`SYNC-184`).
    renewing: Rc<futures_util::lock::Mutex<()>>,
    /// The tokens each renewal ahead of the expiry was asked for, on the
    /// native build, which sends no request: a host test reads them.
    #[cfg(not(target_arch = "wasm32"))]
    renewals_ahead: Vec<http::Credential<http::AccessToken>>,
    /// Set when the relay refused the socket for a key setup still owed, and
    /// cleared once a session with keys is stored. While set, no timer or
    /// renewal reopens the socket (`SYNC-133`).
    setup_owed: bool,
    /// Set when the relay speaks a newer wire version than this build. Never
    /// cleared: only a reload onto the new client cures it (`SYNC-137`).
    update_owed: bool,
    /// Set while the path back to the Logic thread is full. The socket stays
    /// closed until the Main relay releases it (`SYNC-059`).
    inbound_held: bool,
    /// Updates sealed while the socket is held closed, as wire frames in order.
    /// Sent once it reopens, so a hold loses no edit of this user.
    held_outbox: RefCell<Vec<Payload<ambre_protocol::WireFrame>>>,
    /// The stops of each document's replay since the socket last opened.
    /// Cleared on every open (`SYNC-153`, `SYNC-154`).
    replay_stops: HashMap<Id<Document>, websocket::ReplayStops>,
    /// The read cursors owed the relay, sent together at the end of each
    /// period (`SYNC-269`).
    owed_cursors: cursors::OwedCursors,
    /// The last replay of each document this socket asked, and the ask held
    /// past the relay's window. Cleared on every open (`SYNC-188`).
    replay_asks: RefCell<HashMap<Id<Document>, websocket::ReplayAsk>>,
    /// The ticket of the last replay timer armed. It only grows, so a timer
    /// armed before a reset never matches one armed after it.
    replay_ticket: ambre_core::Issuer<generation::ReplayTicket>,
    /// The ticket of the last resend timer armed. It only grows, like
    /// `replay_ticket` (`SYNC-159`).
    resend_ticket: ambre_core::Issuer<generation::ResendTicket>,
    /// Sockets opened on the native mock transport, which has no socket to
    /// inspect: a host test reads it to tell a retry from none.
    #[cfg(not(target_arch = "wasm32"))]
    socket_opens: Count<SocketOpens>,
    /// The listings of the user's channels asked so far. Each answer is
    /// whole, so only the latest one is handed on: an older one that
    /// crossed it would drop a channel just added.
    member_channels_asked: ambre_core::Issuer<generation::Listing>,
    /// The key of each channel's `meta` the latest listing wrapped for this
    /// user. A channel never opened is watched under it (the Murmure
    /// notifications spec).
    listed_keys: HashMap<Id<Channel>, (Id<Meta>, DocumentKey)>,
    /// The documents of spaces this client created and the relay does not
    /// hold yet. Their lazy creation marks them spaces (`SYNC-170`).
    new_spaces: HashSet<Id<Document>>,
    /// The public channel creations that wait for the document of their
    /// space to reach the relay, by that document. Each runs once the relay
    /// holds it, and fails if its creation fails (`SYNC-189`).
    held_creations: HashMap<Id<Document>, Vec<SyncCommand>>,
    /// The call this session is in, one at a time (`CALL-004`).
    call: Option<call_link::CallLink>,
    /// A join that waits for the `meta` document of its channel.
    call_joining: Option<call_link::Joining>,
    /// The ports the join commands brought, which the worker entry fills.
    call_ports: call_link::CallPorts,
    /// The membership log of each channel whose calls a reader asks about, by its `meta`
    /// document (the Murmure calls spec).
    roster_logs: HashMap<Id<Document>, membership_log::Log>,
    /// The calls the host build was asked about, in order, for a test to read.
    #[cfg(not(target_arch = "wasm32"))]
    roster_asks: Vec<Id<ambre_core::id::Call>>,

    // -- In-memory WS transport (test-only, off) -------------------------------
    mock_ws_tx: Option<ambre_bus::conduit::UnboundedSender<Payload<ambre_protocol::WireFrame>>>,

    // -- Internal event queue (carries WS frames + HTTP results into the     --
    // -- non-blocking select loop; producer and consumer are the same worker)  --
    event_tx: Option<ambre_bus::conduit::UnboundedSender<SyncEvent>>,
    /// The event queue of the next session, once a sign-out ended this one.
    /// The loop reads it from then on and drops the queue that ended, so a
    /// task of the ended session answers into nothing (audit `W204`).
    next_events: Option<ambre_bus::conduit::Receiver<SyncEvent>>,

    // -- Per-document sync sessions --------------------------------------------
    documents: HashMap<Id<Document>, SyncDocumentState>,

    // -- Document metadata (in-memory mirror for mock mode) --------------------
    doc_metadata: HashMap<Id<Document>, (DocType, ambre_core::Validated<ambre_core::Name>)>,

    /// Opens received before the session keys were ready (for example a deep-link
    /// refresh dispatches `OpenDocument` at frame 0, before `DevLogin` lands). Held
    /// here and replayed by [`flush_pending_opens`](Self::flush_pending_opens) once
    /// the session is established. The document then fetches its real key and
    /// hydrates instead of opening local-only with a throwaway key.
    pending_opens: Vec<(Id<Document>, DocType)>,

    /// The inviter each document's invite link pinned, for the session
    /// (`SYNC-205`). A pin outlives a close, so a later open of the document
    /// in the session checks the same log against it.
    pinned_inviters: HashMap<Id<Document>, FixedBytes<ambre_core::InviterPin>>,
    /// The roots of every Murmure open of the session (`SYNC-235`). They hold
    /// the inviter a Murmure link pinned (`SYNC-236`), and the members of
    /// each Murmure log an open accepted (`SYNC-237`). The set only grows.
    murmure_roots: HashSet<FixedBytes<ambre_core::InviterPin>>,

    /// The documents an open named under a type other than Murmure, for the
    /// session (`SYNC-260`). The relay writes the Murmure label, and a
    /// Murmure open checks its key against a channel's log, not the log of
    /// the document. So no Murmure open installs a key on one of these.
    typed_opens: HashSet<Id<Document>>,

    /// Outbound updates produced before the session keys / document were ready.
    /// The user edits a restored cell in the milliseconds after F5, while
    /// `DevLogin` is still in flight. Held here instead of dropped, and flushed
    /// once the document is open. A server-side doc flushes after its
    /// `RequestMissing` replay, so `local_clock` has caught up to the server and
    /// no CLOCK_MISMATCH occurs. A local-only doc flushes right when it opens.
    /// Each keeps the summary it was sent with.
    pending_pre_session_updates: Vec<(
        Id<Document>,
        ambre_core::Payload<ambre_core::DeltaBytes>,
        Option<ActivitySummary>,
    )>,

    /// The session nonce of each open document, drawn when it opens and dropped
    /// when it closes (`AUTHOR-002`). Every update this session seals for the
    /// document carries it (`SYNC-112`).
    session_nonces: HashMap<Id<Document>, FixedBytes<SessionNonce>>,

    /// The documents the Logic thread evicted, each kept as a light
    /// subscription (`RESID-004`).
    light: HashMap<Id<Document>, light::LightSubscription>,

    /// Last dev username seen on `DevLogin`, kept so the WS reauth path can
    /// silently re-login if the refresh token is also dead. That happens when
    /// the server restarts with a fresh JWT key and an empty Redis. `None` in a
    /// real (non-dev) session, where reauth relies solely on the refresh token.
    #[cfg(feature = "dev-login")]
    dev_username: Option<ambre_core::Validated<ambre_core::DisplayName>>,
}

impl SyncState {
    /// A signed-out worker state: the session's tokens arrive with a sign-in.
    pub fn new(logic_tx: LogicSink, api_base: Url<RelayHost>) -> Self {
        let url_scheme = crate::http::UrlScheme::of(&api_base);
        Self {
            logic_tx,
            api_base,
            url_scheme,
            tokens: Rc::new(RefCell::new(None)),
            user_id: None,
            master_seed: None,
            signing_key: None,
            exchange_key: None,
            self_join_logs: HashMap::new(),
            channel_metas: HashMap::new(),
            listed_channels: HashMap::new(),
            listing_metas: HashMap::new(),
            creating_public: HashMap::new(),
            space_rotations: HashMap::new(),
            log_footings: HashMap::new(),
            ws: None,
            _onmessage: None,
            _onclose: None,
            _onerror: None,
            _onopen: None,
            _reconnect_timer: None,
            lifecycle: lifecycle::Lifecycle::default(),
            clock: lifecycle::system_clock(),
            renewal: renewal::Renewal::default(),
            renewing: Rc::new(futures_util::lock::Mutex::new(())),
            #[cfg(not(target_arch = "wasm32"))]
            renewals_ahead: Vec::new(),
            setup_owed: false,
            update_owed: false,
            inbound_held: false,
            held_outbox: RefCell::new(Vec::new()),
            replay_stops: HashMap::new(),
            owed_cursors: cursors::OwedCursors::default(),
            replay_asks: RefCell::new(HashMap::new()),
            replay_ticket: ambre_core::Issuer::default(),
            resend_ticket: ambre_core::Issuer::default(),
            #[cfg(not(target_arch = "wasm32"))]
            socket_opens: Count::ZERO,
            member_channels_asked: ambre_core::Issuer::default(),
            listed_keys: HashMap::new(),
            new_spaces: HashSet::new(),
            held_creations: HashMap::new(),
            call: None,
            call_joining: None,
            call_ports: call_link::CallPorts::default(),
            roster_logs: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            roster_asks: Vec::new(),
            mock_ws_tx: None,
            event_tx: None,
            next_events: None,
            documents: HashMap::new(),
            doc_metadata: HashMap::new(),
            pending_opens: Vec::new(),
            pinned_inviters: HashMap::new(),
            murmure_roots: HashSet::new(),
            typed_opens: HashSet::new(),
            pending_pre_session_updates: Vec::new(),
            session_nonces: HashMap::new(),
            light: HashMap::new(),
            #[cfg(feature = "dev-login")]
            dev_username: None,
        }
    }

    /// Stores the sender half of the internal event queue. Called once by
    /// [`run_wasm`] before the select loop, so JS closures and HTTP tasks can
    /// push events into the main loop without blocking.
    pub(crate) fn set_event_tx(&mut self, tx: ambre_bus::conduit::UnboundedSender<SyncEvent>) {
        self.event_tx = Some(tx);
    }

    fn auth_context(&self) -> http::AuthContext {
        http::AuthContext {
            api_base: self.api_base.clone(),
            url_scheme: self.url_scheme,
            tokens: self.tokens.clone(),
            renewing: self.renewing.clone(),
            renewed: self.event_tx.clone(),
        }
    }

    // -- Outbound: Sync -> Logic ------------------------------------------------

    fn send_to_logic(&self, event: LogicEvent) {
        if let Err(SendError(ev)) = self.logic_tx.send(event) {
            crate::log::warning!(
                "Sync: Logic receiver dropped, discarding {:?}",
                std::mem::discriminant(&*ev),
            );
        }
    }

    /// Routes a decrypted inbound payload to the Logic thread, or raises a
    /// non-fatal `SyncWarning` on a decode/decrypt failure (drop semantics).
    /// `opened` is `None` when the document is not open in this session.
    fn deliver_remote<T>(
        &self,
        document_id: Id<Document>,
        opened: Option<Result<T, pipeline::PipelineError>>,
        make_event: impl FnOnce(T) -> LogicEvent,
    ) {
        match opened {
            None => {
                crate::log::warning!(
                    "Sync: inbound message for unopened doc {document_id:?}, dropping"
                )
            }
            Some(Ok(payload)) => self.send_to_logic(make_event(payload)),
            Some(Err(e)) => {
                crate::log::warning!("Sync: dropping inbound for {document_id:?}: {e}");
                self.send_to_logic(LogicEvent::SyncWarning {
                    document_id,
                    code: e.warning(),
                });
            }
        }
    }

    // -- Outbound: Sync -> Server (WebSocket) -----------------------------------

    /// Hook the in-memory mock transport onto this SyncState. In test mode the
    /// mock captures every WS message that would be sent, without touching
    /// JS / WASM APIs.
    pub fn set_mock_ws(
        &mut self,
        tx: ambre_bus::conduit::UnboundedSender<Payload<ambre_protocol::WireFrame>>,
    ) {
        self.mock_ws_tx = Some(tx);
    }

    /// Test seam: mark an already-open document as existing server-side, so the
    /// outbound path sends updates directly instead of buffering them behind a
    /// lazy-creation POST. In production this flag is set by the async
    /// `OpenDocumentReady` (server returned members) or `DocumentMaterialized`
    /// flows. Neither can complete in a synchronous native test.
    // Its two callers sign in by a bare name, which only a `dev-login` build
    // holds, and cargo-mutants builds without it, so it skips the seam.
    #[cfg_attr(test, mutants::skip)]
    pub fn mark_document_server_created(&mut self, document_id: Id<Document>) {
        if let Some(doc_state) = self.documents.get_mut(&document_id) {
            doc_state.existence = crate::Existence::OnRelay;
        }
    }

    /// Number of currently-open document sync sessions.
    pub fn documents_len(&self) -> Count<OpenDocuments> {
        Count::of(self.documents.len())
    }

    /// Sends a control frame: a subscription or a catch-up request.
    ///
    /// A frame that carries a payload is refused and dropped here. Only the
    /// seal builds one, and it reaches the socket through [`Self::ws_send_sealed`]
    /// (`SYNC-001`).
    fn ws_send(&self, msg: &WsClientMsg) {
        if carries_payload(msg) {
            crate::log::error!("[Sync->Server] a payload frame that skipped the seal, dropped");
            return;
        }
        if let WsClientMsg::RequestMissing {
            document_id,
            after_seq,
        } = msg
            && self.space_replay_ask(*document_id, *after_seq) == websocket::AskSlot::Held
        {
            return;
        }
        self.push_frame(msg);
    }

    /// Sends a frame the seal built (`SYNC-001`).
    fn ws_send_sealed(&self, frame: &pipeline::SealedFrame) {
        self.push_frame(frame.message());
    }

    /// Encode and push a client message onto the WebSocket.
    ///
    /// A missing socket or a failed `send` is a transient network condition, not
    /// a programmer error. It must **not** panic: a panic is `panic=abort` and
    /// would tear the Sync thread down silently. Callers already gate on
    /// `self.ws.is_some()`; this only logs if that race is lost. The reconnect
    /// path (onclose -> backoff) re-establishes the socket.
    fn push_frame(&self, msg: &WsClientMsg) {
        let wire = encode_client_msg(msg);
        let Some(ws) = self.ws.as_ref() else {
            if let Some(tx) = &self.mock_ws_tx {
                let _ = tx.send(wire);
                return;
            }
            if self.inbound_held && matches!(msg, WsClientMsg::Update(_)) {
                // A hold is ours, not the network's: keep the edit for the
                // reopen rather than lose it (`SYNC-059`).
                self.held_outbox.borrow_mut().push(wire);
                return;
            }
            crate::log::warning!("[Sync->Server] ws_send called with no socket, dropping message");
            return;
        };
        crate::log::debug!(
            "[Sync->Server] ws_send {:?} -> {} bytes wire",
            std::mem::discriminant(msg),
            wire.len().get()
        );
        self.send_wire(ws, wire);
    }

    /// Sends the frames kept while the socket was held closed, in order, on
    /// whichever transport is up. With none, they stay kept.
    fn flush_held_outbox(&self) {
        let held = std::mem::take(&mut *self.held_outbox.borrow_mut());
        for wire in held {
            if let Some(ws) = self.ws.as_ref() {
                self.send_wire(ws, wire);
            } else if let Some(tx) = &self.mock_ws_tx {
                let _ = tx.send(wire);
            } else {
                self.held_outbox.borrow_mut().push(wire);
            }
        }
    }

    /// Pushes one encoded frame onto the socket.
    fn send_wire(&self, ws: &WebSocket, wire: Payload<ambre_protocol::WireFrame>) {
        // `wire` lives in the wasm heap, which is a SharedArrayBuffer under the
        // shared-memory threading model. Browsers reject `WebSocket.send` on a
        // view backed by a SharedArrayBuffer, so copy into a fresh (non-shared)
        // ArrayBuffer first.
        let buf =
            js_sys::Uint8Array::new_with_length(wire.len().saturating_len().to_usize() as u32);
        buf.copy_from(wire.as_bytes());
        if let Err(e) = ws.send_with_array_buffer(&buf.buffer()) {
            crate::log::warning!(
                "[Sync->Server] WebSocket send failed ({e:?}); reconnect will retry"
            );
        }
        if let Some(tx) = &self.mock_ws_tx {
            let _ = tx.send(wire);
        }
    }

    // -- Inbound: Logic -> Sync -------------------------------------------------

    /// Whether a transport (real WebSocket or mock) is available.
    fn ws_connected(&self) -> bool {
        self.ws.is_some() || self.mock_ws_tx.is_some()
    }

    pub fn handle_command(&mut self, cmd: SyncCommand) {
        match cmd {
            // -- Service plane: the relayed request/response family ------------
            SyncCommand::Rest { id, cmd } => self.handle_rest(id, cmd),

            // -- Document sync session (control) -------------------------------
            SyncCommand::OpenDocument {
                document_id,
                doc_type,
                meta,
            } => {
                match meta {
                    Some(meta) => self.channel_metas.insert(document_id, meta),
                    None => self.channel_metas.remove(&document_id),
                };
                self.handle_open_document(document_id, doc_type);
            }
            SyncCommand::PinInviter {
                document_id,
                inviter,
            } => {
                self.pinned_inviters.insert(document_id, inviter);
            }
            SyncCommand::PinMurmureInviter { inviter } => {
                self.murmure_roots.insert(inviter);
            }
            SyncCommand::CloseDocument { document_id } => {
                // A space closed before the relay held it creates no channel.
                self.refuse_creations(document_id);
                // An invite that waited for the creation is refused, never
                // dropped in silence (`SYNC-247`).
                self.flush_pending_invites(
                    document_id,
                    Some(&http::Failed::local(
                        ambre_core::LocalFailure::DocumentNotOpen,
                    )),
                );
                self.handle_close_document(document_id);
            }
            SyncCommand::ReleaseDocument {
                document_id,
                applied_seq,
            } => self.handle_release_document(document_id, applied_seq),
            SyncCommand::SendUpdate {
                document_id,
                payload,
                summary,
            } => {
                crate::log::info!(
                    "[Sync] received SendUpdate doc={document_id} delta_size={}",
                    payload.len().get()
                );
                self.send_summarized_update(document_id, payload, summary);
            }
            SyncCommand::SendUnsent {
                document_id,
                payload,
                nonce,
            } => self.handle_send_unsent(document_id, payload, nonce),
            SyncCommand::SendSnapshot {
                document_id,
                payload,
                covered_seq,
            } => {
                self.handle_send_snapshot(document_id, payload, covered_seq);
            }
            SyncCommand::SnapshotVerdict {
                document_id,
                snapshot_id,
                purpose,
                verdict,
            } => {
                if let Some(vouch) =
                    self.handle_snapshot_verdict(document_id, snapshot_id, purpose, verdict)
                {
                    self.post_snapshot_vouch(document_id, snapshot_id, vouch);
                }
            }
            SyncCommand::SendEphemeral {
                document_id,
                payload,
            } => {
                self.handle_send_ephemeral(document_id, payload);
            }
            SyncCommand::Reconnect { document_id } => self.handle_reconnect(document_id),

            // -- Channel sync session (control) --------------------------------
            SyncCommand::OpenChannel {
                channel_id,
                listed_in,
            } => {
                match listed_in {
                    Some(space_id) => self.listed_channels.insert(channel_id, space_id),
                    None => self.listed_channels.remove(&channel_id),
                };
                self.handle_open_channel(channel_id, listed_in);
            }
            SyncCommand::MarkChannelListed {
                channel_id,
                space_id,
            } => self.mark_channel_listed(channel_id, space_id),
            SyncCommand::RotateSpace { space_id, channels } => {
                self.handle_rotate_space(space_id, channels);
            }
            SyncCommand::MarkRead { live, finals } => self.handle_mark_read(&live, finals),
            SyncCommand::MoveReadCursors { cursors } => self.handle_move_read_cursors(cursors),
            SyncCommand::MarkThreadRead { thread } => self.handle_mark_thread_read(thread),
            SyncCommand::MarkAllRead => self.handle_mark_all_read(),
            SyncCommand::FetchActivity => self.handle_fetch_activity(),
            SyncCommand::FetchConverter { family } => self.handle_fetch_converter(family),
            // A public channel seals its key under the space key, which the
            // relay must hold first. A creation sent while the space is still
            // on its way waits for it (`SYNC-189`).
            cmd @ SyncCommand::CreateChannel {
                space_id,
                visibility: ambre_core::Visibility::Public,
                ..
            } if self.space_on_its_way(space_id) => {
                self.held_creations
                    .entry(space_id.document_id())
                    .or_default()
                    .push(cmd);
            }
            SyncCommand::CreateChannel {
                channel_id,
                space_id,
                meta,
                segment,
                visibility,
                name,
            } => {
                if visibility == ambre_core::Visibility::Public {
                    self.creating_public
                        .insert(channel_id, (meta.document_id(), space_id));
                }
                self.handle_create_channel(channel_id, space_id, (meta, segment), visibility, name)
            }
            SyncCommand::NameChannel { meta, name } => self.handle_name_channel(meta, name),
            SyncCommand::JoinChannel {
                channel_id,
                space_id,
                key,
            } => self.handle_join_channel(channel_id, space_id, &key),
            SyncCommand::CloseSegment {
                channel_id,
                meta,
                closing,
                successor,
            } => self.handle_close_segment(channel_id, meta, closing, successor),
            SyncCommand::CreateAmendments {
                channel_id,
                meta,
                segment,
                amendments,
            } => self.handle_create_amendments(channel_id, meta, segment, amendments),
            SyncCommand::CreateThread {
                channel_id,
                meta,
                segment,
                root,
                thread,
            } => self.handle_create_thread(channel_id, meta, segment, root, thread),
            SyncCommand::TakeFinalizeLease {
                channel_id,
                segment,
            } => self.handle_take_finalize_lease(channel_id, segment),
            SyncCommand::FinalizeSegment {
                channel_id,
                segment,
                lease,
                horizon,
                payload,
            } => self.handle_finalize_segment(channel_id, segment, lease, horizon, &payload),
            SyncCommand::TakeFoldLease {
                channel_id,
                segment,
            } => self.handle_take_fold_lease(channel_id, segment),
            SyncCommand::FoldSegment {
                channel_id,
                segment,
                lease,
                horizon,
                amendments_seq,
                payload,
            } => {
                let reach = fold::FoldReach {
                    lease,
                    amendments_seq,
                };
                self.handle_fold_segment(channel_id, segment, reach, horizon, &payload);
            }
            SyncCommand::WatchSegment { segment, meta } => self.handle_watch_segment(segment, meta),
            SyncCommand::WatchThread {
                thread,
                meta,
                after,
            } => self.handle_watch_thread(thread, meta, after),
            SyncCommand::WatchChannel {
                channel_id,
                meta,
                meta_last,
                documents,
            } => self.handle_watch_channel(channel_id, (meta, meta_last), &documents),
            SyncCommand::DeleteChannel { channel_id } => self.handle_delete_channel(channel_id),
            SyncCommand::LeaveChannel { channel_id } => self.handle_leave_channel(channel_id),
            SyncCommand::CreateDirectMessage {
                channel_id,
                meta,
                segment,
                members,
            } => self.handle_create_direct_message(channel_id, (meta, segment), members),
            SyncCommand::ListSpaceChannels { space_id } => {
                self.handle_list_space_channels(space_id)
            }
            SyncCommand::ListMemberChannels => self.handle_list_member_channels(),
            SyncCommand::CreateSpace { space_id } => self.handle_create_space(space_id),

            // -- Calls -----------------------------------------------------------
            SyncCommand::JoinCall { call, channel } => {
                let target = call_client::CallTarget { call, channel };
                self.handle_join_call(target, call_client::Opening::Join);
            }
            SyncCommand::StartCall { call, channel } => {
                let target = call_client::CallTarget { call, channel };
                self.handle_join_call(target, call_client::Opening::Start);
            }
            SyncCommand::LeaveCall => self.leave_call(),
            SyncCommand::AskCallRoster {
                call,
                channel,
                meta,
            } => self.ask_call_roster(call, channel, meta),
            SyncCommand::CallDescription(description) => {
                self.handle_call_description(description);
            }
            SyncCommand::ShareStarted => self.handle_share_started(),
            SyncCommand::ChooseVideos(choice) => self.handle_choose_videos(choice),

            // -- Flow control --------------------------------------------------
            SyncCommand::HoldInbound => self.hold_inbound(),
            SyncCommand::ReleaseInbound => self.release_inbound(),

            // -- Lifecycle -----------------------------------------------------
            SyncCommand::Shutdown => self.handle_logout(),
        }
    }

    /// Reply to a service request, echoing its [`Seq<Request>`].
    fn reply(&self, id: Seq<Request>, evt: RestEvent) {
        self.send_to_logic(LogicEvent::Rest { id: Some(id), evt });
    }

    /// Dispatch a relayed [`RestCommand`]. Most arms are placeholders today; the
    /// ones that already produce data answer via [`reply`](Self::reply).
    fn handle_rest(&mut self, id: Seq<Request>, cmd: RestCommand) {
        match cmd {
            // -- Session -------------------------------------------------------
            RestCommand::Unlock { passphrase } => self.handle_unlock(id, passphrase),
            RestCommand::SetupAccount {
                passphrase,
                email,
                display_name,
            } => self.handle_setup_account(id, passphrase, email, display_name),
            RestCommand::SetUpKeys { passphrase } => self.handle_set_up_keys(id, passphrase),
            RestCommand::DevLogin { username } => self.handle_dev_login(Some(id), username),
            RestCommand::ProviderSignIn { provider } => self.handle_provider_sign_in(id, provider),
            RestCommand::ProviderReturn {
                provider,
                code,
                state,
            } => self.handle_provider_return(id, provider, code, state),
            RestCommand::ChangePassphrase {
                current_passphrase,
                new_passphrase,
            } => {
                self.handle_change_passphrase(id, current_passphrase, new_passphrase);
            }
            RestCommand::RecoverAccount {
                recovery_words,
                new_passphrase,
            } => {
                self.handle_recover_account(id, recovery_words, new_passphrase);
            }
            RestCommand::Logout => self.handle_sign_out(id),

            // -- Document CRUD -------------------------------------------------
            RestCommand::ListDocuments => self.handle_list_documents(id),
            RestCommand::CreateDocument { doc_type, name } => {
                self.handle_create_document(id, doc_type, name);
            }
            RestCommand::DeleteDocument { document_id } => {
                self.handle_delete_document(id, document_id);
            }
            RestCommand::RenameDocument { document_id, name } => {
                self.handle_rename_document(id, document_id, name);
            }

            // -- Sharing -------------------------------------------------------
            RestCommand::FetchInviteeKey {
                document_id,
                invitee_email,
            } => {
                self.handle_fetch_invitee_key(id, document_id, invitee_email);
            }
            RestCommand::FetchMemberKey { user_id } => self.handle_fetch_member_key(id, user_id),
            // The instant stamps a notice Logic already wrote; Sync has no
            // use for it.
            RestCommand::InviteToDocument {
                document_id,
                invitee_email,
                invitee,
                role,
                expect_exchange_pub_key,
                now: _,
            } => {
                self.handle_invite_to_document(
                    id,
                    document_id,
                    invitee_email,
                    invitee,
                    role,
                    expect_exchange_pub_key,
                );
            }
            RestCommand::RevokeFromDocument {
                document_id,
                user_id,
            } => {
                self.handle_revoke_from_document(id, document_id, user_id);
            }
            RestCommand::LeaveDocument { document_id } => {
                self.handle_leave_document(id, document_id);
            }
            RestCommand::ListDocumentMembers { document_id } => {
                self.handle_list_document_members(Some(id), document_id);
            }
            RestCommand::UpdateDocumentMemberRole {
                document_id,
                user_id,
                role,
            } => {
                self.handle_update_document_member_role(id, document_id, user_id, role);
            }

            // -- Users ---------------------------------------------------------
            RestCommand::SearchUsers { query } => self.handle_search_users(id, query),

            // -- Media ---------------------------------------------------------
            RestCommand::UploadMedia { document_id, data } => {
                self.handle_upload_media(id, document_id, data);
            }
            RestCommand::DownloadMedia {
                document_id,
                content_hash,
            } => {
                self.handle_download_media(id, document_id, content_hash);
            }
            RestCommand::EraseMedia {
                document_id,
                content_hash,
            } => {
                self.handle_erase_media(id, document_id, content_hash);
            }

            // -- The native file (`.ambre`) ------------------------------------
            RestCommand::SealDocFile {
                plaintext,
                passphrase,
            } => {
                self.handle_seal_doc_file(id, plaintext, passphrase);
            }
            RestCommand::OpenDocFile { sealed, passphrase } => {
                self.handle_open_doc_file(id, sealed, passphrase);
            }

            // -- The local store -----------------------------------------------
            RestCommand::SealLocalRow { place, plaintext } => {
                self.handle_seal_local_row(id, place, plaintext);
            }
            RestCommand::OpenLocalRows { rows } => self.handle_open_local_rows(id, rows),

            // -- Meetings ------------------------------------------------------
            RestCommand::CreateMeeting { name } => {
                crate::log::info!("Sync: CreateMeeting {name:?}")
            }
            RestCommand::JoinMeeting { meeting_id } => {
                crate::log::info!("Sync: JoinMeeting {meeting_id:?}");
            }
            RestCommand::LeaveMeeting { meeting_id } => {
                crate::log::info!("Sync: LeaveMeeting {meeting_id:?}");
            }
            RestCommand::EndMeeting { meeting_id } => {
                crate::log::info!("Sync: EndMeeting {meeting_id:?}");
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Entry points
// -----------------------------------------------------------------------------

/// Async entry point for the Sync worker (`ambre-sync-worker` calls this from
/// `spawn_local`). Single-worker model:
///
/// - `cmd_rx`: in-worker `conduit` queue fed by the worker's `onmessage` handler
///   (decoded `SyncCommand`s relayed from the Main instance via `postMessage`).
/// - `logic_sink`: emits `LogicEvent`s back to Main (postMessage).
///
/// Internally it creates the event bus (`event_tx`/`event_rx`) into which WS /
/// HTTP callbacks push `SyncEvent`s, then `select!`s over both `recv`
/// futures. The worker's JS event loop stays free for WebSocket / fetch
/// callbacks.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn run_wasm(
    cmd_rx: ambre_bus::conduit::Receiver<SyncCommand>,
    logic_sink: LogicSink,
    api_base: Url<RelayHost>,
    url_scheme: http::UrlScheme,
    call_ports: call_link::CallPorts,
) {
    crate::log::info!("Ambre Sync worker starting (wasm async)");
    let mut state = SyncState::new(logic_sink, api_base);
    state.url_scheme = url_scheme;
    state.call_ports = call_ports;
    run(cmd_rx, state).await;
    crate::log::info!("Ambre Sync worker exiting (command channel closed)");
}

/// The worker's loop: one wait over the commands relayed in and the events
/// of its own socket and requests. Between them it yields, so the tasks
/// beside it keep running (`SYNC-058`). It returns once the commands close.
#[cfg_attr(
    not(any(test, target_arch = "wasm32")),
    expect(
        dead_code,
        reason = "only the wasm worker entry point and the tests drive the loop"
    )
)]
async fn run(cmd_rx: ambre_bus::conduit::Receiver<SyncCommand>, mut state: SyncState) {
    use futures_util::future::FutureExt;

    let (event_tx, mut event_rx) = ambre_bus::conduit::unbounded::<SyncEvent>();
    state.set_event_tx(event_tx);

    loop {
        // A sign-out started the queue of the next session: the queue of
        // the session that ended is dropped unread (audit `W204`).
        if let Some(next) = state.next_events.take() {
            event_rx = next;
        }
        futures_util::select! {
            cmd = cmd_rx.recv().fuse() => match cmd {
                Ok(cmd) => state.handle_command(cmd),
                // Command queue closed: the Main instance is gone (teardown).
                Err(_) => break,
            },
            ev = event_rx.recv().fuse() => match ev {
                Ok(ev) => state.handle_sync_event(ev),
                // `state` holds the sender, so the queue closes only once it lets the
                // sender go. A closed queue stays ready and would spin the loop without a
                // yield, so the loop reads the commands alone from then on.
                Err(_) => {
                    while let Ok(cmd) = cmd_rx.recv().await {
                        state.handle_command(cmd);
                    }
                    break;
                }
            },
        }
    }
}

#[cfg(test)]
mod run_tests {
    use super::*;
    use std::cell::Cell;

    /// The worker keeps the sender of its event queue, so a socket callback or a request
    /// task reaches the loop through it.
    #[test]
    fn the_worker_keeps_the_sender_of_its_event_queue() {
        let (sink, _logic_rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let (event_tx, event_rx) = ambre_bus::conduit::unbounded::<SyncEvent>();
        state.set_event_tx(event_tx);
        let held = state.event_tx.clone().expect("the sender is kept");
        assert!(held.send(SyncEvent::CallTick).is_ok());
        assert!(matches!(event_rx.try_recv(), Ok(SyncEvent::CallTick)));
    }

    /// `SYNC-275`, audit `W204`: a sign-out drops the roots, the pins, the
    /// listed channel keys and the plaintext deltas buffered before an
    /// open. A task the ended session spawned answers into the queue of
    /// that session, which the next one does not read.
    #[test]
    fn a_sign_out_leaves_nothing_of_the_session_in_the_worker() {
        let (sink, _logic_rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let (event_tx, _ended) = ambre_bus::conduit::unbounded::<SyncEvent>();
        state.set_event_tx(event_tx);
        let in_flight = state.event_tx.clone().expect("the sender a task holds");
        let doc = Id::<Document>::mint();
        state.user_id = Some(Id::<User>::mint());
        state.murmure_roots.insert(FixedBytes::from_array([1; 32]));
        state
            .pinned_inviters
            .insert(doc, FixedBytes::from_array([2; 32]));
        state.listed_keys.insert(
            Id::<Channel>::mint(),
            (Id::<Meta>::mint(), DocumentKey::generate()),
        );
        state.typed_opens.insert(doc);
        state
            .pending_pre_session_updates
            .push((doc, Payload::new(b"plaintext".to_vec()), None));

        state.handle_logout();

        assert!(state.user_id.is_none());
        assert!(state.murmure_roots.is_empty());
        assert!(state.pinned_inviters.is_empty());
        assert!(state.listed_keys.is_empty());
        assert!(state.typed_opens.is_empty());
        assert!(state.pending_pre_session_updates.is_empty());
        let next = state
            .next_events
            .take()
            .expect("the queue of the next session");
        let _ = in_flight.send(SyncEvent::CallTick);
        assert!(
            next.try_recv().is_err(),
            "a task of the ended session reaches nothing"
        );
        let current = state
            .event_tx
            .clone()
            .expect("the sender of the next session");
        assert!(current.send(SyncEvent::CallTick).is_ok());
        assert!(matches!(next.try_recv(), Ok(SyncEvent::CallTick)));
    }

    /// `SYNC-058`: the loop waits without holding the thread. A task beside
    /// it, as a socket callback would be, runs while it waits. Closing the
    /// commands from that task wakes the loop, which then returns.
    #[tokio::test(flavor = "current_thread")]
    async fn the_loop_yields_to_the_tasks_beside_it() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (sink, _logic_rx) = LogicSink::channel();
                let (cmd_tx, cmd_rx) = ambre_bus::conduit::unbounded::<SyncCommand>();
                let looped = tokio::task::spawn_local(run(
                    cmd_rx,
                    SyncState::new(
                        sink,
                        crate::http::Url::<crate::RelayHost>::new(String::new()),
                    ),
                ));
                tokio::task::yield_now().await;

                let ran = Rc::new(Cell::new(false));
                let beside = Rc::clone(&ran);
                tokio::task::spawn_local(async move {
                    beside.set(true);
                    drop(cmd_tx);
                })
                .await
                .unwrap();
                assert!(ran.get(), "the task beside the loop ran");
                looped.await.unwrap();
            })
            .await;
    }

    /// `SYNC-058`: the loop hands each command it waits on to the worker. A
    /// sign-out sent before the commands close is answered once the loop
    /// returns.
    #[tokio::test(flavor = "current_thread")]
    async fn the_loop_hands_each_command_to_the_worker() {
        let (sink, logic_rx) = LogicSink::channel();
        let (cmd_tx, cmd_rx) = ambre_bus::conduit::unbounded::<SyncCommand>();
        cmd_tx
            .send(SyncCommand::Rest {
                id: Seq::<ambre_bus::Request>::new(7),
                cmd: ambre_bus::RestCommand::Logout,
            })
            .expect("send");
        drop(cmd_tx);
        run(
            cmd_rx,
            SyncState::new(
                sink,
                crate::http::Url::<crate::RelayHost>::new(String::new()),
            ),
        )
        .await;
        let answered = std::iter::from_fn(|| logic_rx.try_recv().ok()).any(|event| {
            matches!(
                event,
                LogicEvent::Rest {
                    evt: ambre_bus::RestEvent::LoggedOut,
                    ..
                }
            )
        });
        assert!(answered, "the sign-out reached the worker");
    }
}

#[cfg(test)]
mod setup_tests {
    use super::*;

    /// A request refused for a key setup still owed reaches Logic as a setup
    /// to complete, whichever error variant its handler raised it under
    /// (`SYNC-134`). The refusal is read from its value: a text that ends
    /// with the code, or a bare 403, passes through unchanged.
    #[test]
    fn a_request_refused_for_a_setup_owed_reaches_logic_as_a_setup() {
        let (sink, rx) = LogicSink::channel();
        let url = "https://relay.example/api/documents";
        let refused = http::HttpError::refused(
            http::Method::Get,
            &crate::http::Target::relay(url),
            ambre_core::HttpStatus::new(403),
            Some(ambre_core::RefusalCode::SetupRequired),
        );
        let _ = sink.send(LogicEvent::ApiError {
            id: None,
            request: ApiRequest::ListDocuments,
            failure: refused.failure(),
        });
        let _ = sink.send(auth_failed(refused.clone()));
        let _ = sink.send(LogicEvent::ApiError {
            id: None,
            request: ApiRequest::ListDocuments,
            failure: ApiFailure::NoAnswer,
        });
        let bare = http::HttpError::refused(
            http::Method::Get,
            &crate::http::Target::relay(url),
            ambre_core::HttpStatus::new(403),
            None,
        );
        let _ = sink.send(LogicEvent::ApiError {
            id: None,
            request: ApiRequest::ListDocuments,
            failure: bare.failure(),
        });
        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(matches!(told[0], LogicEvent::SetupRequired));
        assert!(matches!(told[1], LogicEvent::SetupRequired));
        assert!(matches!(
            told[2],
            LogicEvent::ApiError {
                failure: ApiFailure::NoAnswer,
                ..
            }
        ));
        assert!(matches!(
            told[3],
            LogicEvent::ApiError {
                failure: ApiFailure::Relay {
                    status,
                    code: None
                },
                ..
            } if status.get() == 403
        ));
    }

    /// Holds `SYNC-152`: an answer naming a word this build does not know
    /// reaches Logic as an update to install, under either error variant.
    /// Any other parse error passes through unchanged.
    #[test]
    fn an_answer_with_an_unknown_word_reaches_logic_as_an_update() {
        let (sink, rx) = LogicSink::channel();
        use ambre_core::UnreadableCause;
        let unreadable = |cause| http::HttpError::Unreadable {
            method: http::Method::Get,
            url: crate::http::Target::relay("https://relay.example/api/documents/d/members")
                .shown(),
            reason: "unknown variant `auditor`".into(),
            cause,
        };
        let unknown = unreadable(UnreadableCause::UnknownWord);
        let _ = sink.send(LogicEvent::ApiError {
            id: None,
            request: ApiRequest::ListDocumentMembers,
            failure: unknown.failure(),
        });
        let _ = sink.send(auth_failed(unknown));
        let other = unreadable(UnreadableCause::Malformed);
        let _ = sink.send(LogicEvent::ApiError {
            id: None,
            request: ApiRequest::ListDocumentMembers,
            failure: other.failure(),
        });
        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(matches!(told[0], LogicEvent::UpdateRequired));
        assert!(matches!(told[1], LogicEvent::UpdateRequired));
        assert!(matches!(
            told[2],
            LogicEvent::ApiError {
                failure: ApiFailure::Unreadable {
                    cause: UnreadableCause::Malformed
                },
                ..
            }
        ));
    }
}

#[cfg(test)]
mod ephemeral_tests {
    use super::*;

    /// The session nonce of the one open these tests read.
    const SESSION: FixedBytes<SessionNonce> = FixedBytes::from_array([7; 16]);

    fn doc_state() -> SyncDocumentState {
        SyncDocumentState::new(
            Id::<Document>::mint(),
            DocumentKey::generate(),
            DocType::Prose,
            crate::Existence::LocalOnly,
        )
    }

    #[test]
    fn monotonic_counter_accepted_replay_dropped() {
        let mut d = doc_state();
        let author = FixedBytes::from_array([1u8; 32]);
        assert!(d.accept_ephemeral_counter(&author, &SESSION, Seq::<Ephemerals>::new(1)));
        assert!(d.accept_ephemeral_counter(&author, &SESSION, Seq::<Ephemerals>::new(2)));
        // Replay / stale counters are dropped.
        assert!(!d.accept_ephemeral_counter(&author, &SESSION, Seq::<Ephemerals>::new(2)));
        assert!(!d.accept_ephemeral_counter(&author, &SESSION, Seq::<Ephemerals>::new(1)));
        // Forward progress resumes.
        assert!(d.accept_ephemeral_counter(&author, &SESSION, Seq::<Ephemerals>::new(3)));
    }

    /// `SYNC-287`, audit `P22`: Alice reopens the document, and her new
    /// open counts from 1 again. A peer that stayed open takes it, and still
    /// refuses a replay of her former open. Before the fix, the guard read
    /// her key alone and dropped her samples until the new counter passed
    /// the old one.
    #[test]
    fn a_reopened_author_counts_from_one_again() {
        let mut d = doc_state();
        let alice = FixedBytes::from_array([1u8; 32]);
        let reopened = FixedBytes::<SessionNonce>::from_array([8; 16]);
        assert!(d.accept_ephemeral_counter(&alice, &SESSION, Seq::<Ephemerals>::new(40)));
        assert!(d.accept_ephemeral_counter(&alice, &reopened, Seq::<Ephemerals>::new(1)));
        assert!(!d.accept_ephemeral_counter(&alice, &reopened, Seq::<Ephemerals>::new(1)));
        assert!(!d.accept_ephemeral_counter(&alice, &SESSION, Seq::<Ephemerals>::new(40)));
        assert!(d.accept_ephemeral_counter(&alice, &reopened, Seq::<Ephemerals>::new(2)));
    }

    #[test]
    fn author_map_is_bounded() {
        let mut d = doc_state();
        // Insert well past the cap, each a distinct author.
        for i in 0..(u64::from(MAX_EPHEMERAL_REMOTE_AUTHORS.get()) + 100) {
            let mut author = [0u8; 32];
            author[..8].copy_from_slice(&i.to_le_bytes());
            let author = FixedBytes::from_array(author);
            assert!(d.accept_ephemeral_counter(&author, &SESSION, Seq::<Ephemerals>::new(1)));
        }
        assert!(d.ephemeral_remote_counters.len() <= MAX_EPHEMERAL_REMOTE_AUTHORS.get() as usize);
    }

    /// The signing key of author `i`, distinct for each `i`.
    fn author(i: u64) -> FixedBytes<PublicKey> {
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&i.to_le_bytes());
        FixedBytes::from_array(bytes)
    }

    /// Below the cap, a new author evicts nobody: each author keeps the
    /// baseline its replay is refused against.
    #[test]
    fn below_the_cap_each_author_keeps_its_counter() {
        let mut d = doc_state();
        assert!(d.accept_ephemeral_counter(&author(0), &SESSION, Seq::<Ephemerals>::new(1)));
        assert!(d.accept_ephemeral_counter(&author(1), &SESSION, Seq::<Ephemerals>::new(1)));
        assert!(!d.accept_ephemeral_counter(&author(0), &SESSION, Seq::<Ephemerals>::new(1)));
        assert!(!d.accept_ephemeral_counter(&author(1), &SESSION, Seq::<Ephemerals>::new(1)));
    }

    /// At the cap, a known author's next counter evicts nobody: only a new
    /// author makes room.
    #[test]
    fn at_the_cap_a_known_author_evicts_nobody() {
        let mut d = doc_state();
        let cap = u64::from(MAX_EPHEMERAL_REMOTE_AUTHORS.get());
        for i in 0..cap {
            assert!(d.accept_ephemeral_counter(
                &author(i),
                &SESSION,
                Seq::<Ephemerals>::new(i + 1)
            ));
        }
        assert!(d.accept_ephemeral_counter(
            &author(cap - 1),
            &SESSION,
            Seq::<Ephemerals>::new(cap + 1)
        ));
        assert!(
            !d.accept_ephemeral_counter(&author(0), &SESSION, Seq::<Ephemerals>::new(1)),
            "the least active author is still known"
        );
    }

    // -- Presence identity resolution (`sync`, "Presence identity") ------------

    /// The signing key whose 32 bytes all read `byte`.
    fn key(byte: u8) -> FixedBytes<PublicKey> {
        FixedBytes::from_array([byte; 32])
    }

    #[test]
    fn author_resolves_to_the_member_behind_the_signing_key() {
        let mut d = doc_state();
        let alice = Id::<User>::mint();
        d.set_signing_keys([(key(7), alice)]);

        assert_eq!(d.resolve_ephemeral_author(&key(7)), Some(alice));
    }

    #[test]
    fn unknown_author_key_is_not_resolved() {
        let mut d = doc_state();
        d.set_signing_keys([(key(7), Id::<User>::mint())]);

        // A non-member (or a document whose directory is not filled yet) must not
        // be attributed to anyone. The caller drops the sample rather than showing
        // a cursor under a guessed identity.
        assert_eq!(d.resolve_ephemeral_author(&key(9)), None);
        assert_eq!(doc_state().resolve_ephemeral_author(&key(7)), None);
    }

    #[test]
    fn refreshing_the_directory_drops_revoked_members() {
        let mut d = doc_state();
        let revoked = Id::<User>::mint();
        let kept = Id::<User>::mint();
        d.set_signing_keys([(key(1), revoked), (key(2), kept)]);
        assert_eq!(d.resolve_ephemeral_author(&key(1)), Some(revoked));

        // A membership change replaces the directory wholesale, so a revoked
        // member stops resolving instead of lingering.
        d.set_signing_keys([(key(2), kept)]);
        assert_eq!(d.resolve_ephemeral_author(&key(1)), None);
        assert_eq!(d.resolve_ephemeral_author(&key(2)), Some(kept));
    }
}

#[cfg(test)]
mod seal_tests {
    use super::*;
    use ambre_bus::SyncCommand;
    use ambre_core::KeyId;
    use ambre_protocol::messages::{
        Ciphertext, CommitmentTag, DeltaAuthor, Ephemerals, Nonce, Seq, Signature,
    };
    use ambre_protocol::public_data::{EphemeralPublicData, UpdatePublicData};
    use ambre_protocol::ws::decode_client_msg;
    use ambre_protocol::{Ephemeral, Update};

    /// A worker with one open document and a socket that records what it is
    /// handed.
    fn with_socket() -> (
        SyncState,
        Id<Document>,
        ambre_bus::conduit::Receiver<Payload<ambre_protocol::WireFrame>>,
    ) {
        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.signing_key = Some(MasterSeed::generate().derive_signing_key());
        let id = Id::<Document>::mint();
        let doc = SyncDocumentState::new(
            id,
            DocumentKey::generate(),
            DocType::Prose,
            crate::Existence::OnRelay,
        );
        state.documents.insert(id, doc);
        state
            .session_nonces
            .insert(id, FixedBytes::from_array([1; 16]));
        let (tx, wire) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(tx);
        (state, id, wire)
    }

    /// The frames the socket was handed, decoded.
    fn sent(
        wire: &ambre_bus::conduit::Receiver<Payload<ambre_protocol::WireFrame>>,
    ) -> Vec<WsClientMsg> {
        wire.try_iter()
            .map(|bytes| decode_client_msg(&bytes).expect("decode"))
            .collect()
    }

    /// A summary the Logic thread hands with a delta leaves sealed beside it
    /// (`SYNC-113`). One handed before the session holds survives the wait.
    #[test]
    fn a_summary_handed_with_a_delta_leaves_sealed_beside_it() {
        use ambre_bus::ActivityKind;
        let (mut state, id, wire) = with_socket();
        let summary = ActivitySummary {
            kind: ActivityKind::Content,
            mentions: vec![Id::<User>::from_bytes([5; 16])],
            mentions_all: ambre_bus::MentionScope::Named,
            anchor: None,
            call: None,
        };
        let signing_key = state.signing_key.take();
        state.handle_command(SyncCommand::SendUpdate {
            document_id: id,
            payload: b"a message".to_vec().into(),
            summary: Some(summary.clone()),
        });
        assert!(sent(&wire).is_empty(), "sent before the session held");
        state.signing_key = signing_key;
        state.flush_pre_session_updates(id);
        let frames = sent(&wire);
        let [WsClientMsg::Update(update)] = frames.as_slice() else {
            panic!("expected one update, got {frames:?}");
        };
        let body = pipeline::open_update(update, &state.documents[&id].doc_key).expect("opens");
        let body = pipeline::split_update_body(body).expect("splits");
        assert_eq!(body.summary, Some(summary.to_sealed()));
        assert_eq!(body.delta, b"a message");
    }

    /// A payload frame built by hand, its plaintext in the ciphertext field,
    /// never reaches the socket. The same edit through the seal does, and the
    /// socket never sees its plaintext (`SYNC-001`, `ZK-001`).
    #[test]
    fn a_payload_frame_built_outside_the_seal_never_reaches_the_socket() {
        let (state, id, wire) = with_socket();
        let typed = b"a sentence typed in the clear".to_vec();
        let pub_key = FixedBytes::<PublicKey>::from_array([2u8; 32]);
        let key_id = FixedBytes::<KeyId>::from_array([4u8; 16]);
        state.ws_send(&WsClientMsg::Update(Update {
            ciphertext: Payload::<Ciphertext>::new(typed.clone()),
            nonce: FixedBytes::<Nonce>::from_array([0u8; 24]),
            public_data: UpdatePublicData {
                document_id: id,
                pub_key,
                key_id,
                snapshot_id: Id::<Snapshot>::from_bytes([3u8; 16]),
                clock: Seq::<DeltaAuthor>::new(1),
                log_head: ambre_protocol::LogHead::EMPTY,
                log_entry: None,
            },
            commitment_tag: FixedBytes::<CommitmentTag>::from_array([0u8; 32]),
            signature: FixedBytes::<Signature>::from_array([0u8; 64]),
        }));
        state.ws_send(&WsClientMsg::Ephemeral(Ephemeral {
            ciphertext: Payload::<Ciphertext>::new(typed.clone()),
            nonce: FixedBytes::<Nonce>::from_array([0u8; 24]),
            public_data: EphemeralPublicData {
                document_id: id,
                pub_key,
                key_id,
                counter: Seq::<Ephemerals>::new(1),
                log_head: ambre_protocol::LogHead::EMPTY,
                log_entry: None,
            },
            commitment_tag: FixedBytes::<CommitmentTag>::from_array([0u8; 32]),
            signature: FixedBytes::<Signature>::from_array([0u8; 64]),
        }));
        assert!(
            sent(&wire).is_empty(),
            "a hand-built payload reached the socket"
        );

        let mut state = state;
        state.handle_command(SyncCommand::SendUpdate {
            document_id: id,
            payload: typed.clone().into(),
            summary: None,
        });
        state.handle_command(SyncCommand::SendEphemeral {
            document_id: id,
            payload: typed.clone().into(),
        });
        let frames = sent(&wire);
        assert_eq!(frames.len(), 2);
        let key = &state.documents[&id].doc_key;
        let WsClientMsg::Update(update) = &frames[0] else {
            panic!("the first frame is not the update: {:?}", frames[0]);
        };
        assert_ne!(update.ciphertext.as_bytes(), typed);
        let body = pipeline::open_update(update, key).expect("opens");
        assert_eq!(
            pipeline::split_update_body(body).expect("splits").delta,
            typed
        );
        let WsClientMsg::Ephemeral(ephemeral) = &frames[1] else {
            panic!("the second frame is not the ephemeral: {:?}", frames[1]);
        };
        let opened = pipeline::open_ephemeral(ephemeral, key).expect("opens");
        let body = pipeline::split_presence_body(&opened).expect("splits");
        assert_eq!(body.session, FixedBytes::from_array([1; 16]));
        assert_eq!(body.sample, typed);
    }
}
