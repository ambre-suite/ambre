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

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::WebSocket;
#[cfg(target_arch = "wasm32")]
use web_sys::{CloseEvent, MessageEvent};

use ambre_bus::sync_warning::SyncWarningCode;
use ambre_bus::{
    ChannelId, DocType, DocumentId, DocumentSummary, LogicEvent, MemberInfo, RequestId,
    RestCommand, RestEvent, Role, SecretBytes, SecretString, SignedDelta, SnapshotId,
    SnapshotPurpose, SnapshotVerdict, SyncCommand, UserId, UserInfo,
};
use ambre_core::{KeyId, PublicKey, SessionNonce};
use ambre_crypto::{DocumentKey, MasterSeed};
use ambre_protocol::messages::{Clock, Counter, Seq, Update};
use ambre_protocol::public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
#[cfg(target_arch = "wasm32")]
use ambre_protocol::ws::decode_server_msg;
use ambre_protocol::ws::{WsClientMsg, WsServerMsg, encode_client_msg};
use ed25519_dalek::SigningKey;
use x25519_dalek::StaticSecret;

use base64::Engine;
use hex::encode;

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
pub struct SendError(pub Box<LogicEvent>);

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
        (self.0)(as_setup_owed(event));
        Ok(())
    }
}

/// Tells a failure the relay raised for a key setup still owed as the setup it
/// is, never as a generic error (`ambre-spec-sync.md`, `SYNC-134`). Every error
/// leaves the worker through [`LogicSink::send`], so this one mapping covers
/// every request, whichever handler raised it.
fn as_setup_owed(event: LogicEvent) -> LogicEvent {
    match &event {
        LogicEvent::ApiError { message, .. } | LogicEvent::AuthError { message }
            if http::is_setup_refusal(message) =>
        {
            LogicEvent::SetupRequired
        }
        _ => event,
    }
}

mod auth;
mod chain;
mod channels;
mod docfile;
mod documents;
#[cfg(feature = "fuzz")]
pub mod fuzz;
mod http;
mod lifecycle;
mod light;
mod media;
mod members;
mod membership_log;
pub mod mock;
pub mod mock_relay;
mod pipeline;
mod rekey;
mod sharing;
mod updates;
mod websocket;
/// Web Worker entry point (`#[wasm_bindgen(start)]`): the postMessage bridge to
/// the Main instance. wasm-only.
#[cfg(target_arch = "wasm32")]
mod worker;

// -----------------------------------------------------------------------------
// Quarantine bounds (`ambre-spec-delta-order.md`)
// -----------------------------------------------------------------------------

/// Soft threshold: once this many updates are quarantined for a document, we ask
/// the server to resend the missing history (`RequestMissing`). The parent
/// snapshot can then arrive and release them.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const QUARANTINE_ALERT: usize = 900;
/// Hard cap: beyond this, the oldest update (by `clock`) is dropped to bound the
/// shared heap, and a `SyncWarning` is raised.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const QUARANTINE_LIMIT: usize = 1000;

/// How many of a document's latest sequence numbers the duplicate guard
/// remembers ([`SyncDocumentState::ingested`]).
const APPLIED_WINDOW: usize = 4096;

/// WebSocket close code the server sends when it refuses the upgrade for auth
/// reasons (expired/rotated JWT). In the application-reserved 4000-4999 range;
/// mirrors HTTP 401. The client reacts by refreshing its token and reconnecting.
const WS_CLOSE_UNAUTHORIZED: u16 = 4401;

/// WebSocket close code the server sends when it refuses the upgrade because
/// the account still owes its key setup (`ambre-spec-broadcast.md`, `SRV-164`).
/// A renewal cannot cure it, so the client neither renews nor reconnects until
/// the keys are stored (`ambre-spec-sync.md`, `SYNC-133`).
const WS_CLOSE_SETUP_REQUIRED: u16 = 4403;

/// Hard cap on the per-document ephemeral anti-replay map
/// ([`SyncDocumentState::ephemeral_remote_counters`]). In practice the author set
/// is bounded by document membership, but a malicious peer can mint arbitrary
/// signing keys. The map is therefore capped to bound heap. At the cap a new
/// author evicts the least-active (lowest-counter) entry.
const MAX_EPHEMERAL_REMOTE_AUTHORS: usize = 512;

// -----------------------------------------------------------------------------
// Internal event bus -- carries WebSocket frames and spawned-task results
// into the non-blocking select loop. Sent by JS callbacks (onmessage /
// onclose / onerror) and by HTTP tasks spawned via spawn_local.
// -----------------------------------------------------------------------------

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
enum SyncEvent {
    WsMessage(WsServerMsg),
    WsDisconnected {
        code: u16,
        reason: String,
    },
    WsError,
    /// A frame arrived under a newer wire version than this build's
    /// (`ambre-spec-sync.md`, `SYNC-137`).
    WireOutdated,
    /// Fired by a backoff timer scheduled after a disconnect: time to attempt a
    /// fresh WebSocket connection (see [`schedule_reconnect`](SyncState::schedule_reconnect)).
    ReconnectNow,
    /// The WebSocket `onopen` fired: the socket is now OPEN, so (re)subscriptions
    /// can be sent (they would be dropped if sent while still CONNECTING).
    WsOpened,
    /// Result of a silent token refresh triggered by a WS auth rejection (close
    /// 4401). `ok` true means the access token was renewed and the socket can be
    /// reopened; false means the refresh failed and a fallback re-login is needed.
    Reauthenticated {
        ok: bool,
    },
    SessionReady {
        id: RequestId,
        user_id: UserId,
        display_name: String,
        email: String,
        master_seed: MasterSeed,
        signing_key: SigningKey,
        exchange_key: StaticSecret,
        tokens: http::TokenPair,
    },
    /// A provider return was exchanged: the session's tokens, and whether the
    /// account holds its keys. No key is held yet, so no socket opens.
    ProviderSignedIn {
        id: RequestId,
        user_id: UserId,
        display_name: String,
        email: String,
        keys_stored: bool,
        tokens: http::TokenPair,
    },
    AccountCreated {
        id: RequestId,
        recovery_words: String,
        master_seed: MasterSeed,
        signing_key: SigningKey,
        exchange_key: StaticSecret,
        tokens: http::TokenPair,
    },
    DocumentList {
        id: RequestId,
        documents: Vec<DocumentSummary>,
    },
    DocumentCreated {
        id: RequestId,
        document_id: DocumentId,
        doc_type: DocType,
    },
    DocumentDeleted {
        id: RequestId,
        document_id: DocumentId,
    },
    DocumentRenamed {
        id: RequestId,
        document_id: DocumentId,
    },
    OpenDocumentReady {
        document_id: DocumentId,
        doc_key: DocumentKey,
        snapshot_payload: Option<Vec<u8>>,
        snapshot_id: SnapshotId,
        /// Highest delta `seq` folded into `snapshot_payload`, 0 when no snapshot
        /// was downloaded. The post-open `RequestMissing` starts after this seq,
        /// so the history already in the snapshot is not replayed.
        covered_seq: Seq,
        /// The latest snapshot, when only its author confirmed it and that
        /// author is another user. `snapshot_payload` is then the reference,
        /// and this one is checked once the replay is done (`AUTHOR-022`).
        held_snapshot: Option<HeldSnapshot>,
        /// Document kind, carried so a not-yet-on-server doc can be lazily created
        /// with the right type on its first outbound update.
        doc_type: DocType,
        /// Whether the document already exists server-side. `false` for a
        /// local-only doc (server returned 404 for its members): no WebSocket
        /// subscription yet, and the first update triggers lazy creation.
        server_created: bool,
        /// Members' Ed25519 signing keys -> canonical `UserId`, harvested from
        /// the same members response that carried the wrapped DocumentKey
        /// (`ambre-spec-membership.md`). Seeds `SyncDocumentState::signing_keys` so
        /// inbound ephemerals resolve to a spoof-proof author. Empty for a
        /// local-only open (no members response, hence no peers to resolve).
        signing_keys: Vec<([u8; 32], UserId)>,
        /// This client's own role on this document, read from its entry in the
        /// same members response (architecture). `Role::Owner` for a
        /// local-only open. Gates mutating commands in Logic as a UX measure;
        /// the server remains the sole authority.
        role: Role,
        /// The members answer says the document owes a rotation (`SYNC-128`).
        rotation_owed: bool,
        /// The document's key chain, as the relay serves it (`SYNC-117`).
        /// Empty for a local-only open.
        links: Vec<ambre_protocol::KeyLink>,
    },
    /// The answer to a channel listing (`SyncCommand::OpenChannel`): its
    /// documents, or why the relay refused it.
    ChannelListed {
        channel_id: ChannelId,
        result: Result<Vec<ambre_bus::ChannelDocument>, String>,
    },
    /// The relay's activity answer (`SyncCommand::FetchActivity`,
    /// `SyncCommand::MarkAllRead`).
    ActivityFetched {
        result: Result<Vec<ambre_bus::DocumentActivity>, String>,
    },
    /// Opening a server-side document failed because its wrapped DocumentKey
    /// could not be unwrapped (wrong exchange key, tampered server row, or a
    /// genuine authorization failure). Surfaced as an `AuthError` instead of
    /// silently opening with a random key (H3).
    OpenDocumentFailed {
        document_id: DocumentId,
        message: String,
    },
    /// A local-only document was lazily materialized server-side. Flushes the
    /// buffered updates and opens the WebSocket subscription.
    DocumentMaterialized {
        document_id: DocumentId,
    },
    /// Lazy creation failed. The `creating` guard is cleared so a later update
    /// retries; the buffered updates are kept.
    DocumentCreateFailed {
        document_id: DocumentId,
        message: String,
    },
    HttpError {
        id: RequestId,
        message: String,
    },
    /// TOFU phase 1 (`ambre-spec-key-verification.md`): the invitee was resolved and
    /// their exchange key fetched, with a formatted fingerprint. No key wrapped
    /// yet -- the Main thread decides trust before phase 2 wraps.
    InviteeKeyReady {
        id: RequestId,
        document_id: DocumentId,
        user_id: UserId,
        display_name: String,
        email: String,
        exchange_pub_key: [u8; 32],
        fingerprint: String,
    },
    /// Invite completed: the invitee's DocumentKey was wrapped via ECDH and the
    /// server accepted the share request.
    InviteSent {
        id: RequestId,
        document_id: DocumentId,
        user_id: UserId,
        display_name: String,
    },
    /// Key rotation initiated by this client is finished: the server has the new
    /// `doc_key_enc` entries and the new key must be swapped locally.
    RevocationComplete {
        id: RequestId,
        document_id: DocumentId,
        new_doc_key: DocumentKey,
        /// The link the revoke left, from `new_doc_key` back to the key it
        /// replaced (`SYNC-116`).
        link: ambre_protocol::KeyLink,
    },
    /// Member list for a document has been fetched from the server (decoded,
    /// plaintext names/emails).
    DocumentMembersReady {
        id: RequestId,
        document_id: DocumentId,
        members: Vec<MemberInfo>,
        /// Refreshed presence directory (`ambre-spec-membership.md`), harvested from
        /// the same response. Membership just changed often enough for someone to
        /// ask for the list. This is where a new member becomes resolvable and a
        /// revoked one stops. Ignored when the document is not open locally.
        signing_keys: Vec<([u8; 32], UserId)>,
    },
    /// User search results from the server (decoded, plaintext).
    UserSearchReady {
        id: RequestId,
        users: Vec<UserInfo>,
    },
    /// A document member's role was updated server-side.
    MemberRoleUpdatedEvent {
        id: RequestId,
        document_id: DocumentId,
        user_id: UserId,
        role: Role,
    },
    /// A remote key rotation was performed: the new DocumentKey has been fetched
    /// and unwrapped via ECDH; the local key cache must be swapped.
    KeyRotated {
        document_id: DocumentId,
        new_doc_key: DocumentKey,
        /// The document's chain as the relay serves it after the rotation
        /// (`SYNC-117`).
        links: Vec<ambre_protocol::KeyLink>,
    },
    /// This user's departure landed: the relay removed their own share
    /// (`SYNC-127`). The document closes locally.
    DocumentLeft {
        id: RequestId,
        document_id: DocumentId,
    },
    /// The document owes a rotation: a member left and nobody re-keyed since
    /// (`SYNC-128`). Found in a members answer, never guessed.
    DebtFound {
        document_id: DocumentId,
    },
    /// This client paid the owed rotation: the relay stored `new_doc_key` for
    /// every member, and it becomes the current key (`SYNC-128`).
    DebtSettled {
        document_id: DocumentId,
        new_doc_key: DocumentKey,
        /// The link the rotation left (`SYNC-116`).
        link: ambre_protocol::KeyLink,
    },
    /// The debt was paid by someone else before this client could. Their key
    /// is fetched like any other rotation's.
    DebtGone {
        document_id: DocumentId,
    },
    /// Paying the debt failed. The next frame tries again.
    DebtUnsettled {
        document_id: DocumentId,
        message: String,
    },
    /// Media upload completed: the encrypted blob was PUT to S3 and the server
    /// confirmed the insertion in the `media` table.
    MediaUploaded {
        id: RequestId,
        document_id: DocumentId,
        content_hash: String,
    },
    /// Snapshot upload completed: the sealed blob was PUT to S3 and the server
    /// confirmed the insertion (and broadcast `SnapshotAvailable`). The returned
    /// `snapshot_id` becomes this document's current parent for future deltas.
    SnapshotUploaded {
        document_id: DocumentId,
        snapshot_id: SnapshotId,
    },
    /// A snapshot a peer announced was fetched, or could not be (`None`). Only
    /// its contents reaching the Logic thread make it a known parent
    /// (`SYNC-030`).
    SnapshotFetched {
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        /// The last delta its author says it folds in.
        covered_seq: Seq,
        blob: Option<Vec<u8>>,
    },
    /// Media download completed: the encrypted blob was fetched from S3 and
    /// decrypted. `data` is the raw plaintext bytes ready for display.
    MediaDownloaded {
        id: RequestId,
        document_id: DocumentId,
        content_hash: String,
        data: Vec<u8>,
    },
}

// -----------------------------------------------------------------------------
// Per-document sync session
// -----------------------------------------------------------------------------

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct SyncDocumentState {
    #[allow(dead_code)]
    document_id: DocumentId,
    doc_key: DocumentKey,
    /// Document kind, used for lazy server creation on the first outbound update.
    doc_type: DocType,
    /// Whether the document row exists server-side. Starts `false` for a
    /// local-only doc; flipped to `true` once lazily created so the POST and the
    /// WebSocket subscription happen exactly once.
    server_created: bool,
    /// A lazy-creation POST is in flight. Guards against firing several POSTs
    /// when updates arrive back-to-back before the first one completes.
    creating: bool,
    /// Outbound update bodies, each carrying its session nonce, buffered while
    /// the document is being lazily created server-side. Flushed in order once
    /// it is materialized.
    pending_updates: Vec<Vec<u8>>,
    /// Uploads asked for while the document is local-only. The relay signs no
    /// link for a document it does not hold, so each one waits for the lazy
    /// creation (`SYNC-145`).
    pending_uploads: Vec<(RequestId, Vec<u8>)>,
    /// Snapshot this document's updates are currently based on. Placeholder
    /// until the snapshot flow lands; carried in every `UpdatePublicData`.
    current_snapshot_id: SnapshotId,
    local_clock: u64,
    /// Monotonic counter for outbound ephemerals (anti-replay at the receiver).
    ephemeral_counter: u64,
    /// Highest server-assigned `seq` seen for this document. Used as
    /// `after_seq` when requesting the missing history.
    last_seq: Seq,
    /// The latest sequence numbers this client ingested, applied or held in
    /// quarantine. A reconnect replay can overlap a broadcast received during
    /// the same round trip, and the second copy is dropped here (`SYNC-087`).
    /// Bounded by [`APPLIED_WINDOW`].
    ingested: BTreeSet<u64>,
    /// Snapshots whose contents the Logic thread already holds. An inbound
    /// `Update` whose parent `snapshot_id` is here can be applied immediately;
    /// otherwise it is quarantined under that id.
    known_snapshots: HashSet<SnapshotId>,
    /// Updates held back because their parent snapshot is not yet known, keyed by
    /// that parent `snapshot_id`. Stored sealed; decrypted only on release.
    /// Bounded by [`QUARANTINE_LIMIT`] (oldest-by-`clock` dropped past the cap).
    /// Each keeps the number the relay gave it, which the Logic thread's
    /// snapshot check reads.
    quarantine: HashMap<SnapshotId, Vec<(Seq, Update)>>,
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
    /// Per-author ephemeral anti-replay counter. Keyed by the author's Ed25519
    /// verifying key bytes. An inbound ephemeral whose counter is <= the stored
    /// value is silently dropped.
    ephemeral_remote_counters: HashMap<[u8; 32], u64>,
    /// Directory of this document's members' Ed25519 signing keys, mapping an
    /// ephemeral author's verifying key to their canonical `UserId`
    /// (`ambre-spec-membership.md`). Built from the members response already fetched at open and
    /// rebuilt when membership changes. A key absent here is not resolved, so the
    /// frame is dropped rather than attributed to a guessed identity.
    ///
    /// This is a *membership* directory, not a *trust* store: it trusts the
    /// server's key mapping. Detecting a server that substitutes a member's keys
    /// is the TOFU trusted-keys store's job (architecture) and is orthogonal.
    signing_keys: HashMap<[u8; 32], UserId>,
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
    checks: HashMap<SnapshotId, PendingCheck>,
    /// A snapshot only its author confirmed, held until the open has replayed
    /// every delta the relay kept. It is checked then (`AUTHOR-022`).
    held_snapshot: Option<HeldSnapshot>,
    /// The Logic thread evicted the document at this horizon. The session
    /// becomes a light subscription once every frame sent is acknowledged
    /// (`RESID-009`).
    released: Option<Seq>,
}

/// A snapshot the Logic thread is checking.
struct PendingCheck {
    /// The signing key its sealed blob verified under: who built it.
    author: [u8; 32],
    covered_seq: Seq,
    /// The opened snapshot, kept on open only: a match adopts it.
    payload: Option<Vec<u8>>,
}

/// A snapshot only its author confirmed, still sealed (`SRV-128`).
#[derive(Debug, Clone)]
pub(crate) struct HeldSnapshot {
    pub snapshot_id: SnapshotId,
    pub covered_seq: Seq,
    pub blob: Vec<u8>,
}

impl SyncDocumentState {
    fn new(
        document_id: DocumentId,
        doc_key: DocumentKey,
        doc_type: DocType,
        server_created: bool,
    ) -> Self {
        // Anchor the base to the document's genesis snapshot, which every client
        // derives identically. Until a real snapshot is uploaded, the earliest
        // deltas reference this id. A fresh open (or a peer) then recognises their
        // parent instead of quarantining them forever. A later `OpenDocumentReady`
        // or `SnapshotAvailable` carrying a real snapshot id supersedes it.
        let current_snapshot_id = SnapshotId::genesis(document_id);
        Self {
            document_id,
            doc_key,
            doc_type,
            server_created,
            creating: false,
            pending_updates: Vec::new(),
            pending_uploads: Vec::new(),
            current_snapshot_id,
            local_clock: 0,
            ephemeral_counter: 0,
            last_seq: Seq(0),
            ingested: BTreeSet::new(),
            known_snapshots: HashSet::from([current_snapshot_id]),
            quarantine: HashMap::new(),
            gap_requested: false,
            old_keys: Vec::new(),
            chain: chain::Chain::default(),
            ephemeral_remote_counters: HashMap::new(),
            signing_keys: HashMap::new(),
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
    fn first_arrival(&mut self, seq: Seq) -> bool {
        if !self.ingested.insert(seq.0) {
            return false;
        }
        if self.ingested.len() > APPLIED_WINDOW {
            self.ingested.pop_first();
        }
        true
    }

    /// Total number of quarantined updates across all parent snapshots.
    #[allow(dead_code)]
    fn quarantine_len(&self) -> usize {
        self.quarantine.values().map(Vec::len).sum()
    }

    /// The key a frame names: the active key, one in the [`old_keys`] cache,
    /// or one the chain reaches from the active key. `None` when this client
    /// reaches no such key (`SYNC-045`, `SYNC-119`).
    fn key_named(&self, id: KeyId) -> Option<DocumentKey> {
        chain::find_key(
            self.document_id,
            &self.doc_key,
            &self.old_keys,
            &self.chain,
            id,
        )
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
    ) -> Result<Vec<u8>, pipeline::PipelineError> {
        let key = self
            .key_named(eph.public_data.key_id)
            .ok_or(pipeline::PipelineError::WrongKey)?;
        pipeline::open_ephemeral(eph, &key)
    }

    /// Anti-replay gate for an inbound ephemeral. Returns `true` if `counter` is
    /// strictly greater than the last value seen for `author_pk` (accept and
    /// record it); `false` if it is a replay (drop).
    ///
    /// The counter map is bounded by [`MAX_EPHEMERAL_REMOTE_AUTHORS`]: when a new
    /// author would exceed the cap, the least-active entry (lowest counter) is
    /// evicted first. Eviction resets that author's baseline, so a later *stale*
    /// ephemeral from an evicted author could replay once. That is harmless for
    /// presence, and it cannot affect an active author whose counter keeps rising.
    fn accept_ephemeral_counter(&mut self, author_pk: [u8; 32], counter: u64) -> bool {
        let last = self
            .ephemeral_remote_counters
            .get(&author_pk)
            .copied()
            .unwrap_or(0);
        if counter <= last {
            return false;
        }
        let is_new = !self.ephemeral_remote_counters.contains_key(&author_pk);
        if is_new && self.ephemeral_remote_counters.len() >= MAX_EPHEMERAL_REMOTE_AUTHORS {
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
        self.ephemeral_remote_counters.insert(author_pk, counter);
        true
    }

    /// Replace the members' signing-key directory (`ambre-spec-membership.md`).
    /// Called with the members response fetched at open, and again whenever
    /// membership changes, so a revoked member stops resolving.
    ///
    /// Entries whose key is not valid hex or not 32 bytes are skipped. A member
    /// we cannot key on simply never resolves, which costs that peer's cursor
    /// rather than the whole document's presence.
    fn set_signing_keys(&mut self, entries: impl IntoIterator<Item = ([u8; 32], UserId)>) {
        self.signing_keys = entries.into_iter().collect();
    }

    /// Resolve the canonical `UserId` behind an ephemeral's authoring Ed25519
    /// key (`ambre-spec-membership.md`). Presence must be keyed by a per-user
    /// identity, not by the per-device signing key. The sender-supplied payload
    /// can never be trusted for identity, so the key is matched against the
    /// members directory, on this trusted side.
    ///
    /// `None` means the author is not a known member of this document (or the
    /// directory has not been filled yet). The caller drops the frame: presence
    /// is best-effort, so showing nothing is preferable to attributing a cursor
    /// to a guessed identity.
    fn resolve_ephemeral_author(&self, author_pk: [u8; 32]) -> Option<UserId> {
        self.signing_keys.get(&author_pk).copied()
    }

    /// Opens a snapshot blob with the one key it names: a snapshot sealed
    /// before a rotation opens with a key the chain recovers (`SYNC-119`).
    fn open_snapshot_blob(&self, blob: &[u8]) -> Result<Vec<u8>, pipeline::PipelineError> {
        self.open_signed_snapshot(blob).map(|(payload, _)| payload)
    }

    /// Opens a snapshot blob, and names the signing key it verified under: the
    /// member who built it, which the snapshot check needs (`AUTHOR-021`).
    fn open_signed_snapshot(
        &self,
        blob: &[u8],
    ) -> Result<(Vec<u8>, [u8; 32]), pipeline::PipelineError> {
        let snapshot = pipeline::decode_snapshot_blob(blob, self.document_id)?;
        let key = self
            .key_named(snapshot.public_data.key_id)
            .ok_or(pipeline::PipelineError::WrongKey)?;
        let payload = pipeline::open_snapshot(&snapshot, &key)?;
        Ok((payload, *snapshot.public_data.pub_key.as_array()))
    }

    /// Opens an update with the one key it names, and with no other
    /// (`SYNC-045`).
    fn open_update_named(
        &self,
        update: &ambre_protocol::Update,
    ) -> Result<Vec<u8>, pipeline::PipelineError> {
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
        seq: Seq,
    ) -> Result<SignedDelta, pipeline::PipelineError> {
        let body = pipeline::split_update_body(self.open_update_named(update)?)?;
        let signer = *update.public_data.pub_key.as_array();
        Ok(SignedDelta {
            seq: seq.0,
            peer: ambre_crypto::peer::derive_peer_id(&signer, &body.nonce),
            nonce: body.nonce,
            user: self.signing_keys.get(&signer).copied(),
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

fn doc_type_to_api_string(doc_type: DocType) -> String {
    match doc_type {
        DocType::Prose => "prose",
        DocType::Trame => "trame",
        DocType::Toile => "toile",
        DocType::Murmure => "murmure",
    }
    .into()
}

/// Whether a client frame carries a sealed payload, which only the seal may
/// build (`SYNC-001`).
fn carries_payload(msg: &WsClientMsg) -> bool {
    matches!(
        msg,
        WsClientMsg::Update(_)
            | WsClientMsg::Ephemeral(_)
            | WsClientMsg::ChannelUpdate { .. }
            | WsClientMsg::ChannelEphemeral { .. }
    )
}

pub struct SyncState {
    /// Outbound sink: Sync -> Logic thread (postMessage in the worker).
    logic_tx: LogicSink,
    /// Base URL for REST API calls (for example `"api.ambre.app"`).
    #[allow(dead_code)]
    api_base: String,
    /// URL scheme: `"http"` on localhost, `"https"` otherwise.
    #[allow(dead_code)]
    url_scheme: &'static str,
    /// Access + refresh tokens returned by login/setup and rotated by HTTP 401
    /// retry helpers. Kept in worker memory only.
    tokens: http::TokenStore,
    /// Authenticated user id, set after Unlock/SetupAccount.
    user_id: Option<UserId>,

    // -- Session-level crypto (derived once at Unlock, zeroized at Logout) ----
    master_seed: Option<MasterSeed>,
    /// Ed25519 signing key, derived from the master seed. Signs every outbound
    /// envelope; its public half is the author identity in `public_data`.
    signing_key: Option<SigningKey>,
    /// X25519 exchange key, derived from the master seed. Used to wrap/unwrap
    /// document keys when sharing (invite + key rotation on revoke).
    exchange_key: Option<StaticSecret>,

    // -- Single multiplexed WebSocket -----------------------------------------
    ws: Option<WebSocket>,
    /// JS closures attached to the WebSocket (onmessage / onclose / onerror).
    /// Kept alive to prevent GC. Each is type-erased to `JsValue` because the
    /// callback signatures differ (MessageEvent / CloseEvent / Event).
    #[allow(dead_code)]
    _onmessage: Option<Closure<dyn FnMut(JsValue)>>,
    #[allow(dead_code)]
    _onclose: Option<Closure<dyn FnMut(JsValue)>>,
    #[allow(dead_code)]
    _onerror: Option<Closure<dyn FnMut(JsValue)>>,
    #[allow(dead_code)]
    _onopen: Option<Closure<dyn FnMut(JsValue)>>,
    /// Closure kept alive for the pending reconnect `setTimeout`, if any.
    #[allow(dead_code)]
    _reconnect_timer: Option<Closure<dyn FnMut()>>,
    /// The socket's reconnect state: the backoff and its deadline. A relay
    /// that rejects the upgrade (for example a 429) is not hammered.
    lifecycle: lifecycle::Lifecycle,
    /// The clock the lifecycle reads. A host test advances its own.
    clock: lifecycle::Clock,
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
    held_outbox: RefCell<Vec<Vec<u8>>>,
    /// Sockets opened on the native mock transport, which has no socket to
    /// inspect: a host test reads it to tell a retry from none.
    #[cfg(not(target_arch = "wasm32"))]
    socket_opens: u32,

    // -- In-memory WS transport (test-only, off) -------------------------------
    mock_ws_tx: Option<ambre_bus::conduit::UnboundedSender<Vec<u8>>>,

    // -- Internal event queue (carries WS frames + HTTP results into the     --
    // -- non-blocking select loop; producer and consumer are the same worker)  --
    event_tx: Option<ambre_bus::conduit::UnboundedSender<SyncEvent>>,

    // -- Per-document sync sessions --------------------------------------------
    documents: HashMap<DocumentId, SyncDocumentState>,

    // -- Document metadata (in-memory mirror for mock mode) --------------------
    doc_metadata: HashMap<DocumentId, (DocType, String)>,

    /// Opens received before the session keys were ready (for example a deep-link
    /// refresh dispatches `OpenDocument` at frame 0, before `DevLogin` lands). Held
    /// here and replayed by [`flush_pending_opens`](Self::flush_pending_opens) once
    /// the session is established. The document then fetches its real key and
    /// hydrates instead of opening local-only with a throwaway key.
    pending_opens: Vec<(DocumentId, DocType)>,

    /// Outbound updates produced before the session keys / document were ready.
    /// The user edits a restored cell in the milliseconds after F5, while
    /// `DevLogin` is still in flight. Held here instead of dropped, and flushed
    /// once the document is open. A server-side doc flushes after its
    /// `RequestMissing` replay, so `local_clock` has caught up to the server and
    /// no CLOCK_MISMATCH occurs. A local-only doc flushes right when it opens.
    pending_pre_session_updates: Vec<(DocumentId, Vec<u8>)>,

    /// The session nonce of each open document, drawn when it opens and dropped
    /// when it closes (`AUTHOR-002`). Every update this session seals for the
    /// document carries it (`SYNC-112`).
    session_nonces: HashMap<DocumentId, SessionNonce>,

    /// The documents the Logic thread evicted, each kept as a light
    /// subscription (`RESID-004`).
    light: HashMap<DocumentId, light::LightSubscription>,

    /// Last dev username seen on `DevLogin`, kept so the WS reauth path can
    /// silently re-login if the refresh token is also dead. That happens when
    /// the server restarts with a fresh JWT key and an empty Redis. `None` in a
    /// real (non-dev) session, where reauth relies solely on the refresh token.
    dev_username: Option<String>,
}

impl SyncState {
    #[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
    pub fn new(
        logic_tx: LogicSink,
        api_base: String,
        tokens: Option<(String, String, u64)>,
    ) -> Self {
        let url_scheme = if api_base.starts_with("localhost") || api_base.starts_with("127.0.0.1") {
            "http"
        } else {
            "https"
        };
        Self {
            logic_tx,
            api_base,
            url_scheme,
            tokens: Rc::new(RefCell::new(tokens.map(
                |(access_token, refresh_token, access_expires_in)| http::TokenPair {
                    access_token: http::SecretToken::from(access_token),
                    refresh_token: http::SecretToken::from(refresh_token),
                    access_expires_in,
                },
            ))),
            user_id: None,
            master_seed: None,
            signing_key: None,
            exchange_key: None,
            ws: None,
            _onmessage: None,
            _onclose: None,
            _onerror: None,
            _onopen: None,
            _reconnect_timer: None,
            lifecycle: lifecycle::Lifecycle::default(),
            clock: lifecycle::system_clock(),
            setup_owed: false,
            update_owed: false,
            inbound_held: false,
            held_outbox: RefCell::new(Vec::new()),
            #[cfg(not(target_arch = "wasm32"))]
            socket_opens: 0,
            mock_ws_tx: None,
            event_tx: None,
            documents: HashMap::new(),
            doc_metadata: HashMap::new(),
            pending_opens: Vec::new(),
            pending_pre_session_updates: Vec::new(),
            session_nonces: HashMap::new(),
            light: HashMap::new(),
            dev_username: None,
        }
    }

    /// Stores the sender half of the internal event queue. Called once by
    /// [`run_wasm`] before the select loop, so JS closures and HTTP tasks can
    /// push events into the main loop without blocking.
    #[allow(dead_code)]
    pub(crate) fn set_event_tx(&mut self, tx: ambre_bus::conduit::UnboundedSender<SyncEvent>) {
        self.event_tx = Some(tx);
    }

    fn auth_context(&self) -> http::AuthContext {
        http::AuthContext {
            api_base: self.api_base.clone(),
            url_scheme: self.url_scheme,
            tokens: self.tokens.clone(),
        }
    }

    // -- Outbound: Sync -> Logic ------------------------------------------------

    fn send_to_logic(&self, event: LogicEvent) {
        if let Err(SendError(ev)) = self.logic_tx.send(event) {
            tracing::warn!(
                "Sync: Logic receiver dropped, discarding {:?}",
                std::mem::discriminant(&*ev),
            );
        }
    }

    /// Routes a decrypted inbound payload to the Logic thread, or raises a
    /// non-fatal `SyncWarning` on a decode/decrypt failure (drop semantics).
    /// `opened` is `None` when the document is not open in this session.
    #[allow(dead_code)]
    fn deliver_remote<T>(
        &self,
        document_id: DocumentId,
        opened: Option<Result<T, pipeline::PipelineError>>,
        make_event: impl FnOnce(T) -> LogicEvent,
    ) {
        match opened {
            None => {
                tracing::warn!("Sync: inbound message for unopened doc {document_id:?}, dropping")
            }
            Some(Ok(payload)) => self.send_to_logic(make_event(payload)),
            Some(Err(e)) => {
                tracing::warn!("Sync: dropping inbound for {document_id:?}: {e}");
                self.send_to_logic(LogicEvent::SyncWarning {
                    document_id,
                    code: SyncWarningCode::DecryptFailed,
                    message: e.to_string(),
                });
            }
        }
    }

    // -- Outbound: Sync -> Server (WebSocket) -----------------------------------

    /// Hook the in-memory mock transport onto this SyncState. In test mode the
    /// mock captures every WS message that would be sent, without touching
    /// JS / WASM APIs.
    pub fn set_mock_ws(&mut self, tx: ambre_bus::conduit::UnboundedSender<Vec<u8>>) {
        self.mock_ws_tx = Some(tx);
    }

    /// Test seam: mark an already-open document as existing server-side, so the
    /// outbound path sends updates directly instead of buffering them behind a
    /// lazy-creation POST. In production this flag is set by the async
    /// `OpenDocumentReady` (server returned members) or `DocumentMaterialized`
    /// flows. Neither can complete in a synchronous native test.
    pub fn mark_document_server_created(&mut self, document_id: DocumentId) {
        if let Some(doc_state) = self.documents.get_mut(&document_id) {
            doc_state.server_created = true;
        }
    }

    /// Number of currently-open document sync sessions.
    pub fn documents_len(&self) -> usize {
        self.documents.len()
    }

    /// Sends a control frame: a subscription or a catch-up request.
    ///
    /// A frame that carries a payload is refused and dropped here. Only the
    /// seal builds one, and it reaches the socket through [`Self::ws_send_sealed`]
    /// (`SYNC-001`).
    fn ws_send(&self, msg: &WsClientMsg) {
        if carries_payload(msg) {
            tracing::error!("[Sync->Server] a payload frame that skipped the seal, dropped");
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
            if self.inbound_held
                && matches!(
                    msg,
                    WsClientMsg::Update(_) | WsClientMsg::ChannelUpdate { .. }
                )
            {
                // A hold is ours, not the network's: keep the edit for the
                // reopen rather than lose it (`SYNC-059`).
                self.held_outbox.borrow_mut().push(wire);
                return;
            }
            tracing::warn!("[Sync->Server] ws_send called with no socket, dropping message");
            return;
        };
        tracing::debug!(
            "[Sync->Server] ws_send {:?} -> {} bytes wire",
            std::mem::discriminant(msg),
            wire.len()
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
    fn send_wire(&self, ws: &WebSocket, wire: Vec<u8>) {
        // `wire` lives in the wasm heap, which is a SharedArrayBuffer under the
        // shared-memory threading model. Browsers reject `WebSocket.send` on a
        // view backed by a SharedArrayBuffer, so copy into a fresh (non-shared)
        // ArrayBuffer first.
        let buf = js_sys::Uint8Array::new_with_length(wire.len() as u32);
        buf.copy_from(&wire);
        if let Err(e) = ws.send_with_array_buffer(&buf.buffer()) {
            tracing::warn!("[Sync->Server] WebSocket send failed ({e:?}); reconnect will retry");
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
            } => self.handle_open_document(document_id, doc_type),
            SyncCommand::CloseDocument { document_id } => self.handle_close_document(document_id),
            SyncCommand::ReleaseDocument {
                document_id,
                applied_seq,
            } => self.handle_release_document(document_id, Seq(applied_seq)),
            SyncCommand::SendUpdate {
                document_id,
                payload,
                // The summary rides sealed beside the delta once `SYNC-113`
                // lands. Until then the worker drops it (the Murmure messages spec).
                summary: _,
            } => {
                tracing::info!(
                    "[Sync] received SendUpdate doc={document_id} delta_size={}",
                    payload.len()
                );
                self.handle_send_update(document_id, payload);
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
                self.handle_send_snapshot(document_id, payload, Seq(covered_seq));
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
            SyncCommand::OpenChannel { channel_id } => self.handle_open_channel(channel_id),
            SyncCommand::MarkRead { live, finals } => self.handle_mark_read(&live, finals),
            SyncCommand::MarkAllRead => self.handle_mark_all_read(),
            SyncCommand::FetchActivity => self.handle_fetch_activity(),

            // -- Flow control --------------------------------------------------
            SyncCommand::HoldInbound => self.hold_inbound(),
            SyncCommand::ReleaseInbound => self.release_inbound(),

            // -- Lifecycle -----------------------------------------------------
            SyncCommand::Shutdown => self.handle_logout(),
        }
    }

    /// Reply to a service request, echoing its [`RequestId`].
    fn reply(&self, id: RequestId, evt: RestEvent) {
        self.send_to_logic(LogicEvent::Rest { id, evt });
    }

    /// Dispatch a relayed [`RestCommand`]. Most arms are placeholders today; the
    /// ones that already produce data answer via [`reply`](Self::reply).
    fn handle_rest(&mut self, id: RequestId, cmd: RestCommand) {
        match cmd {
            // -- Session -------------------------------------------------------
            RestCommand::Unlock { passphrase } => self.handle_unlock(id, passphrase),
            RestCommand::SetupAccount {
                passphrase,
                email,
                display_name,
            } => self.handle_setup_account(id, passphrase, email, display_name),
            RestCommand::DevLogin { username } => self.handle_dev_login(id, username),
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
                self.handle_change_passphrase(current_passphrase, new_passphrase);
            }
            RestCommand::RecoverAccount {
                recovery_words,
                new_passphrase,
            } => {
                self.handle_recover_account(recovery_words, new_passphrase);
            }
            RestCommand::Logout => {
                self.handle_logout();
                self.reply(id, RestEvent::LoggedOut);
            }

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
            RestCommand::InviteToDocument {
                document_id,
                invitee_email,
                role,
                expect_exchange_pub_key,
            } => {
                self.handle_invite_to_document(
                    id,
                    document_id,
                    invitee_email,
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
                self.handle_list_document_members(id, document_id);
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

            // -- Meetings ------------------------------------------------------
            RestCommand::CreateMeeting { name } => tracing::info!("Sync: CreateMeeting '{name}'"),
            RestCommand::JoinMeeting { meeting_id } => {
                tracing::info!("Sync: JoinMeeting {meeting_id:?}");
            }
            RestCommand::LeaveMeeting { meeting_id } => {
                tracing::info!("Sync: LeaveMeeting {meeting_id:?}");
            }
            RestCommand::EndMeeting { meeting_id } => {
                tracing::info!("Sync: EndMeeting {meeting_id:?}");
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
pub async fn run_wasm(
    cmd_rx: ambre_bus::conduit::Receiver<SyncCommand>,
    logic_sink: LogicSink,
    api_base: String,
) {
    tracing::info!("Ambre Sync worker starting (wasm async)");
    run(cmd_rx, SyncState::new(logic_sink, api_base, None)).await;
    tracing::info!("Ambre Sync worker exiting (command channel closed)");
}

/// The worker's loop: one wait over the commands relayed in and the events
/// of its own socket and requests. Between them it yields, so the tasks
/// beside it keep running (`SYNC-058`). It returns once the commands close.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
async fn run(cmd_rx: ambre_bus::conduit::Receiver<SyncCommand>, mut state: SyncState) {
    use futures_util::future::FutureExt;

    let (event_tx, event_rx) = ambre_bus::conduit::unbounded::<SyncEvent>();
    state.set_event_tx(event_tx);

    loop {
        futures_util::select! {
            cmd = cmd_rx.recv().fuse() => match cmd {
                Ok(cmd) => state.handle_command(cmd),
                // Command queue closed: the Main instance is gone (teardown).
                Err(_) => break,
            },
            // event_tx is held by `state`, so this only errors at teardown.
            ev = event_rx.recv().fuse() => if let Ok(ev) = ev {
                state.handle_sync_event(ev);
            },
        }
    }
}

#[cfg(test)]
mod run_tests {
    use super::*;
    use std::cell::Cell;

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
                    SyncState::new(sink, String::new(), None),
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
}

#[cfg(test)]
mod setup_tests {
    use super::*;

    /// A request refused for a key setup still owed reaches Logic as a setup
    /// to complete, whichever error variant its handler raised it under
    /// (`SYNC-134`). Any other failure passes through unchanged.
    #[test]
    fn a_request_refused_for_a_setup_owed_reaches_logic_as_a_setup() {
        let (sink, rx) = LogicSink::channel();
        let refused = "GET https://relay.example/api/documents: SETUP_REQUIRED";
        assert!(http::is_setup_refusal(refused));
        let _ = sink.send(LogicEvent::ApiError {
            id: None,
            request: "ListDocuments".into(),
            code: 500,
            message: refused.into(),
        });
        let _ = sink.send(LogicEvent::AuthError {
            message: refused.into(),
        });
        let _ = sink.send(LogicEvent::ApiError {
            id: None,
            request: "ListDocuments".into(),
            code: 403,
            message: "GET https://relay.example/api/documents: HTTP 403".into(),
        });
        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(matches!(told[0], LogicEvent::SetupRequired));
        assert!(matches!(told[1], LogicEvent::SetupRequired));
        assert!(matches!(told[2], LogicEvent::ApiError { code: 403, .. }));
    }
}

#[cfg(test)]
mod ephemeral_tests {
    use super::*;

    fn doc_state() -> SyncDocumentState {
        SyncDocumentState::new(
            DocumentId::new(),
            DocumentKey::generate(),
            DocType::Prose,
            false,
        )
    }

    #[test]
    fn monotonic_counter_accepted_replay_dropped() {
        let mut d = doc_state();
        let author = [1u8; 32];
        assert!(d.accept_ephemeral_counter(author, 1));
        assert!(d.accept_ephemeral_counter(author, 2));
        // Replay / stale counters are dropped.
        assert!(!d.accept_ephemeral_counter(author, 2));
        assert!(!d.accept_ephemeral_counter(author, 1));
        // Forward progress resumes.
        assert!(d.accept_ephemeral_counter(author, 3));
    }

    #[test]
    fn author_map_is_bounded() {
        let mut d = doc_state();
        // Insert well past the cap, each a distinct author.
        for i in 0..(MAX_EPHEMERAL_REMOTE_AUTHORS as u64 + 100) {
            let mut author = [0u8; 32];
            author[..8].copy_from_slice(&i.to_le_bytes());
            assert!(d.accept_ephemeral_counter(author, 1));
        }
        assert!(d.ephemeral_remote_counters.len() <= MAX_EPHEMERAL_REMOTE_AUTHORS);
    }

    // -- Presence identity resolution (`sync`, "Presence identity") ------------

    #[test]
    fn author_resolves_to_the_member_behind_the_signing_key() {
        let mut d = doc_state();
        let alice = UserId::new();
        d.set_signing_keys([([7u8; 32], alice)]);

        assert_eq!(d.resolve_ephemeral_author([7u8; 32]), Some(alice));
    }

    #[test]
    fn unknown_author_key_is_not_resolved() {
        let mut d = doc_state();
        d.set_signing_keys([([7u8; 32], UserId::new())]);

        // A non-member (or a document whose directory is not filled yet) must not
        // be attributed to anyone. The caller drops the frame rather than showing
        // a cursor under a guessed identity.
        assert_eq!(d.resolve_ephemeral_author([9u8; 32]), None);
        assert_eq!(doc_state().resolve_ephemeral_author([7u8; 32]), None);
    }

    #[test]
    fn refreshing_the_directory_drops_revoked_members() {
        let mut d = doc_state();
        let revoked = UserId::new();
        let kept = UserId::new();
        d.set_signing_keys([([1u8; 32], revoked), ([2u8; 32], kept)]);
        assert_eq!(d.resolve_ephemeral_author([1u8; 32]), Some(revoked));

        // A membership change replaces the directory wholesale, so a revoked
        // member stops resolving instead of lingering.
        d.set_signing_keys([([2u8; 32], kept)]);
        assert_eq!(d.resolve_ephemeral_author([1u8; 32]), None);
        assert_eq!(d.resolve_ephemeral_author([2u8; 32]), Some(kept));
    }
}

#[cfg(test)]
mod seal_tests {
    use super::*;
    use ambre_bus::SyncCommand;
    use ambre_core::{KeyId, SnapshotId};
    use ambre_protocol::messages::{Ciphertext, Clock, CommitmentTag, Counter, Nonce, Signature};
    use ambre_protocol::public_data::{EphemeralPublicData, UpdatePublicData};
    use ambre_protocol::ws::decode_client_msg;
    use ambre_protocol::{Ephemeral, Update};

    /// A worker with one open document and a socket that records what it is
    /// handed.
    fn with_socket() -> (SyncState, DocumentId, ambre_bus::conduit::Receiver<Vec<u8>>) {
        let (sink, _rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        state.signing_key = Some(MasterSeed::generate().derive_signing_key());
        let id = DocumentId::new();
        let doc = SyncDocumentState::new(id, DocumentKey::generate(), DocType::Prose, true);
        state.documents.insert(id, doc);
        state.session_nonces.insert(id, SessionNonce([1; 16]));
        let (tx, wire) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(tx);
        (state, id, wire)
    }

    /// The frames the socket was handed, decoded.
    fn sent(wire: &ambre_bus::conduit::Receiver<Vec<u8>>) -> Vec<WsClientMsg> {
        wire.try_iter()
            .map(|bytes| decode_client_msg(&bytes).expect("decode"))
            .collect()
    }

    /// A payload frame built by hand, its plaintext in the ciphertext field,
    /// never reaches the socket. The same edit through the seal does, and the
    /// socket never sees its plaintext (`SYNC-001`, `ZK-001`).
    #[test]
    fn a_payload_frame_built_outside_the_seal_never_reaches_the_socket() {
        let (state, id, wire) = with_socket();
        let typed = b"a sentence typed in the clear".to_vec();
        let pub_key = PublicKey::from_array([2u8; 32]);
        let key_id = KeyId::from_array([4u8; 16]);
        state.ws_send(&WsClientMsg::Update(Update {
            ciphertext: Ciphertext(typed.clone()),
            nonce: Nonce([0u8; 24]),
            public_data: UpdatePublicData {
                document_id: id,
                pub_key: pub_key.clone(),
                key_id,
                snapshot_id: SnapshotId::from_bytes([3u8; 16]),
                clock: Clock(1),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        }));
        state.ws_send(&WsClientMsg::Ephemeral(Ephemeral {
            ciphertext: Ciphertext(typed.clone()),
            nonce: Nonce([0u8; 24]),
            public_data: EphemeralPublicData {
                document_id: id,
                pub_key,
                key_id,
                counter: Counter(1),
            },
            commitment_tag: CommitmentTag([0u8; 32]),
            signature: Signature([0u8; 64]),
        }));
        assert!(
            sent(&wire).is_empty(),
            "a hand-built payload reached the socket"
        );

        let mut state = state;
        state.handle_command(SyncCommand::SendUpdate {
            document_id: id,
            payload: typed.clone(),
            summary: None,
        });
        state.handle_command(SyncCommand::SendEphemeral {
            document_id: id,
            payload: typed.clone(),
        });
        let frames = sent(&wire);
        assert_eq!(frames.len(), 2);
        let key = &state.documents[&id].doc_key;
        let WsClientMsg::Update(update) = &frames[0] else {
            panic!("the first frame is not the update: {:?}", frames[0]);
        };
        assert_ne!(update.ciphertext.0, typed);
        let body = pipeline::open_update(update, key).expect("opens");
        assert_eq!(
            pipeline::split_update_body(body).expect("splits").delta,
            typed
        );
        let WsClientMsg::Ephemeral(ephemeral) = &frames[1] else {
            panic!("the second frame is not the ephemeral: {:?}", frames[1]);
        };
        assert_eq!(
            pipeline::open_ephemeral(ephemeral, key).expect("opens"),
            typed
        );
    }
}
