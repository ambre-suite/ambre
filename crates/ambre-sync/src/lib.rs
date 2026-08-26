// ambre-sync/src/lib.rs
// ============================================================================
// Sync logic — runs in its OWN wasm instance (a dedicated Web Worker with its
// own linear memory), separate from the Main instance (UI + Logic). The worker
// shell lives in `ambre-sync-worker`; this crate owns `SyncState` and the
// select loop ([`run_wasm`]).
//
// Responsibilities:
//   - Own ALL cryptographic material (MasterSeed, DocumentKeys) — never leaves
//     this thread.
//   - Own the single session-level WebSocket (one WS, N document subscriptions).
//   - Perform HTTP REST calls (fetch via gloo-net).
//   - Encrypt/decrypt/compress/decompress every outbound and inbound payload.
//
// Out of scope:
//   - CRDT mutations — exclusively in ambre-logic.
//   - Rendering, input — exclusively in the UI Thread.
//
// Transport: inbound `SyncCommand`s arrive over `postMessage` from the Main
// instance, decoded by `ambre-sync-worker` and fed into [`run_wasm`] via an
// in-worker `flume` channel. Outbound `LogicEvent`s are emitted through a
// [`LogicSink`] (the worker posts them back to Main via `postMessage`).
//
// Because this is now a SINGLE worker, the internal event bus and the command
// channel both have their producer and consumer on the same worker, so flume's
// async waker fires correctly (the old cross-worker freeze that motivated
// `sab_channel` cannot happen here). The select loop awaits both channels'
// `recv_async`; the worker's JS event loop stays free for WebSocket / fetch.
//
// WebSocket topology (when implemented):
//   ONE WebSocket per session, opened at Unlock, closed at Logout.
//   OpenDocument / CloseDocument -> Subscribe / Unsubscribe on the shared WS.
//   All documents are demultiplexed by document_id on incoming WsServerMsg.
// ============================================================================

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::WebSocket;
#[cfg(target_arch = "wasm32")]
use web_sys::{CloseEvent, MessageEvent};

use ambre_bus::sync_warning::SyncWarningCode;
use ambre_bus::{
    ChannelId, DocType, DocumentId, DocumentSummary, LogicEvent, MemberInfo, RequestId,
    RestCommand, RestEvent, Role, SecretBytes, SecretString, SnapshotId, SyncCommand, UserId,
    UserInfo,
};
use ambre_core::PublicKey;
use ambre_crypto::{DocumentKey, MasterSeed};
use ambre_protocol::messages::{Clock, Counter, Seq, Update};
use ambre_protocol::public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
#[cfg(target_arch = "wasm32")]
use ambre_protocol::ws::decode_server_msg;
use ambre_protocol::ws::{WsClientMsg, WsServerMsg, encode_client_msg};
use ed25519_dalek::SigningKey;
use x25519_dalek::StaticSecret;

use base64::Engine;
use blake2::Digest;
use hex::encode;

/// Sink for events flowing Sync -> Logic. Sync no longer shares memory with the
/// Logic thread, so it cannot hold a channel `Sender<LogicEvent>` directly:
/// instead it calls this type-erased closure. In the wasm worker the closure
/// postcard-encodes the event and `postMessage`s it to the Main instance; in
/// native tests it pushes into a `flume` channel (see [`LogicSink::channel`]).
///
/// `Rc`-backed (clonable, `'static`) so it can be captured by the `spawn_local`
/// HTTP tasks and JS callbacks. Single-threaded by construction (one worker).
#[derive(Clone)]
pub struct LogicSink(std::rc::Rc<dyn Fn(LogicEvent)>);

/// Returned by [`LogicSink::send`]. The send is infallible in practice (the
/// closure always accepts the event); the type mirrors a channel `SendError` so
/// existing call sites are unchanged.
/// The failed event is boxed so the `Err` variant stays small (the `LogicEvent`
/// payload is large): `send` is in practice infallible, so this never allocates
/// on the hot path.
pub struct SendError(pub Box<LogicEvent>);

impl LogicSink {
    /// Wrap an arbitrary emit closure.
    pub fn new(f: impl Fn(LogicEvent) + 'static) -> Self {
        Self(std::rc::Rc::new(f))
    }

    /// A sink backed by a fresh `flume` channel, returning the receiver so a
    /// (native) test can drain what Sync emits.
    pub fn channel() -> (Self, flume::Receiver<LogicEvent>) {
        let (tx, rx) = flume::unbounded();
        (
            Self::new(move |ev| {
                let _ = tx.send(ev);
            }),
            rx,
        )
    }

    /// Emit one event to the Logic thread.
    pub fn send(&self, event: LogicEvent) -> Result<(), SendError> {
        (self.0)(event);
        Ok(())
    }
}

mod auth;
mod channels;
mod documents;
mod http;
mod media;
mod members;
pub mod mock;
pub mod mock_relay;
mod pipeline;
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
/// the server to resend the missing history (`RequestMissing`) so the parent
/// snapshot can arrive and release them.
/// Soft threshold: once this many updates are quarantined for a document, we ask
/// the server to resend the missing history (`RequestMissing`) so the parent
/// snapshot can arrive and release them.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const QUARANTINE_ALERT: usize = 900;
/// Hard cap: beyond this, the oldest update (by `clock`) is dropped to bound the
/// shared heap, and a `SyncWarning` is raised.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const QUARANTINE_LIMIT: usize = 1000;

/// WebSocket close code the server sends when it refuses the upgrade for auth
/// reasons (expired/rotated JWT). In the application-reserved 4000-4999 range;
/// mirrors HTTP 401. The client reacts by refreshing its token and reconnecting.
const WS_CLOSE_UNAUTHORIZED: u16 = 4401;

/// Hard cap on the per-document ephemeral anti-replay map
/// ([`SyncDocumentState::ephemeral_remote_counters`]). In practice the author set
/// is bounded by document membership, but a malicious peer can mint arbitrary
/// signing keys, so the map is capped to bound heap; at the cap a new author
/// evicts the least-active (lowest-counter) entry.
const MAX_EPHEMERAL_REMOTE_AUTHORS: usize = 512;

// -----------------------------------------------------------------------------
// Internal event bus — carries WebSocket frames and spawned-task results
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
        /// Highest delta `seq` folded into `snapshot_payload` (0 when no snapshot
        /// was downloaded): the post-open `RequestMissing` starts after this seq
        /// so the history already in the snapshot is not replayed.
        covered_seq: Seq,
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
    },
    /// Member list for a document has been fetched from the server (decoded,
    /// plaintext names/emails).
    DocumentMembersReady {
        id: RequestId,
        document_id: DocumentId,
        members: Vec<MemberInfo>,
        /// Refreshed presence directory (`ambre-spec-membership.md`), harvested from
        /// the same response. Membership just changed often enough for someone to
        /// ask for the list, so this is where a new member becomes resolvable and
        /// a revoked one stops. Ignored when the document is not open locally.
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
    /// Outbound CRDT update payloads buffered while the document is being lazily
    /// created server-side. Flushed in order once it is materialized.
    pending_updates: Vec<Vec<u8>>,
    /// Snapshot this document's updates are currently based on. Placeholder
    /// until the snapshot flow lands; carried in every `UpdatePublicData`.
    current_snapshot_id: SnapshotId,
    local_clock: u64,
    /// Monotonic counter for outbound ephemerals (anti-replay at the receiver).
    ephemeral_counter: u64,
    /// Highest server-assigned `seq` seen for this document. Used as
    /// `after_seq` when requesting the missing history.
    last_seq: Seq,
    /// Snapshots whose contents the Logic thread already holds. An inbound
    /// `Update` whose parent `snapshot_id` is here can be applied immediately;
    /// otherwise it is quarantined under that id.
    known_snapshots: HashSet<SnapshotId>,
    /// Updates held back because their parent snapshot is not yet known, keyed by
    /// that parent `snapshot_id`. Stored sealed; decrypted only on release.
    /// Bounded by [`QUARANTINE_LIMIT`] (oldest-by-`clock` dropped past the cap).
    quarantine: HashMap<SnapshotId, Vec<Update>>,
    /// LRU cache of previous DocumentKeys (max 10). Used by
    /// [`open_update_fallback`] and [`open_ephemeral_fallback`] to decrypt
    /// messages sealed with a key that was rotated before the rotation
    /// notification propagated to this client.
    old_keys: Vec<DocumentKey>,
    /// Per-author ephemeral anti-replay counter. Keyed by the author's Ed25519
    /// verifying key bytes. An inbound ephemeral whose counter is ≤ the stored
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
    /// because `DocumentReady` fires from two places (immediately for a
    /// local-only / offline open, or after the initial replay), and both must
    /// report the same role.
    ///
    /// Defaults to `Role::Owner`: a document with no members response is
    /// local-only, and its creator owns it. The server stays authoritative
    /// either way (architecture) -- this drives UX only.
    role: Role,
    /// True between sending the post-open `RequestMissing` and receiving its
    /// `MissingUpdates` reply. The initial catch-up replay is the point at which
    /// the document is fully hydrated (snapshot + every delta the server already
    /// holds), so `DocumentReady` is emitted when this clears -- not at open,
    /// which would dismiss the UI loading overlay before delta-only documents
    /// have any content. Stays `false` for local-only / offline opens, which
    /// emit `DocumentReady` immediately (no replay to wait for).
    awaiting_initial_hydration: bool,
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
        // deltas reference this id, so a fresh open (or a peer) recognises their
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
            current_snapshot_id,
            local_clock: 0,
            ephemeral_counter: 0,
            last_seq: Seq(0),
            known_snapshots: HashSet::from([current_snapshot_id]),
            quarantine: HashMap::new(),
            old_keys: Vec::new(),
            ephemeral_remote_counters: HashMap::new(),
            signing_keys: HashMap::new(),
            role: Role::Owner,
            awaiting_initial_hydration: false,
        }
    }

    /// Total number of quarantined updates across all parent snapshots.
    #[allow(dead_code)]
    fn quarantine_len(&self) -> usize {
        self.quarantine.values().map(Vec::len).sum()
    }

    /// Decrypts an ephemeral message with the active key, falling back to every
    /// key in the [`old_keys`] LRU cache if the active key fails. Returns the
    /// result of the last attempt (the active key) if all keys fail.
    fn open_ephemeral_fallback(
        &self,
        eph: &ambre_protocol::Ephemeral,
    ) -> Result<Vec<u8>, pipeline::PipelineError> {
        match pipeline::open_ephemeral(eph, &self.doc_key) {
            Ok(payload) => Ok(payload),
            Err(_) => {
                for old_key in &self.old_keys {
                    if let Ok(payload) = pipeline::open_ephemeral(eph, old_key) {
                        return Ok(payload);
                    }
                }
                pipeline::open_ephemeral(eph, &self.doc_key)
            }
        }
    }

    /// Anti-replay gate for an inbound ephemeral. Returns `true` if `counter` is
    /// strictly greater than the last value seen for `author_pk` (accept and
    /// record it); `false` if it is a replay (drop).
    ///
    /// The counter map is bounded by [`MAX_EPHEMERAL_REMOTE_AUTHORS`]: when a new
    /// author would exceed the cap, the least-active entry (lowest counter) is
    /// evicted first. Eviction resets that author's baseline, so a later *stale*
    /// ephemeral from an evicted author could replay once -- harmless for
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
    /// Entries whose key is not valid hex or not 32 bytes are skipped: a member
    /// we cannot key on simply never resolves, which costs that peer's cursor
    /// rather than the whole document's presence.
    fn set_signing_keys(&mut self, entries: impl IntoIterator<Item = ([u8; 32], UserId)>) {
        self.signing_keys = entries.into_iter().collect();
    }

    /// Resolve the canonical `UserId` behind an ephemeral's authoring Ed25519
    /// key (`ambre-spec-membership.md`). Presence must be keyed by a per-user
    /// identity, not by the per-device signing key, and the sender-supplied
    /// payload can never be trusted for identity -- so the key is matched
    /// against the members directory, on this trusted side.
    ///
    /// `None` means the author is not a known member of this document (or the
    /// directory has not been filled yet). The caller drops the frame: presence
    /// is best-effort, so showing nothing is preferable to attributing a cursor
    /// to a guessed identity.
    fn resolve_ephemeral_author(&self, author_pk: [u8; 32]) -> Option<UserId> {
        self.signing_keys.get(&author_pk).copied()
    }

    /// Decrypts an update with the active key, falling back to every key in the
    /// [`old_keys`] LRU cache.
    fn open_update_fallback(
        &self,
        update: &ambre_protocol::Update,
    ) -> Result<Vec<u8>, pipeline::PipelineError> {
        match pipeline::open_update(update, &self.doc_key) {
            Ok(payload) => Ok(payload),
            Err(_) => {
                for old_key in &self.old_keys {
                    if let Ok(payload) = pipeline::open_update(update, old_key) {
                        return Ok(payload);
                    }
                }
                pipeline::open_update(update, &self.doc_key)
            }
        }
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

pub struct SyncState {
    /// Outbound sink: Sync → Logic thread (postMessage in the worker).
    logic_tx: LogicSink,
    /// Base URL for REST API calls (e.g. `"api.ambre.app"`).
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
    /// Consecutive failed/closed connection count, reset to 0 once a socket
    /// actually opens. Drives the exponential reconnect backoff so a server that
    /// rejects the upgrade (e.g. 429 rate limit) is not hammered into a storm.
    reconnect_attempts: u32,

    // -- In-memory WS transport (test-only, off) -------------------------------
    mock_ws_tx: Option<flume::Sender<Vec<u8>>>,

    // -- Internal event channel (carries WS frames + HTTP results into the     --
    // -- non-blocking select loop; producer and consumer are the same worker)  --
    event_tx: Option<flume::Sender<SyncEvent>>,

    // -- Per-document sync sessions --------------------------------------------
    documents: HashMap<DocumentId, SyncDocumentState>,

    // -- Document metadata (in-memory mirror for mock mode) --------------------
    doc_metadata: HashMap<DocumentId, (DocType, String)>,

    /// Opens received before the session keys were ready (e.g. a deep-link
    /// refresh dispatches `OpenDocument` at frame 0, before `DevLogin` lands).
    /// Held here and replayed by [`flush_pending_opens`](Self::flush_pending_opens)
    /// once the session is established, so the document fetches its real key and
    /// hydrates instead of opening local-only with a throwaway key.
    pending_opens: Vec<(DocumentId, DocType)>,

    /// Outbound updates produced before the session keys / document were ready
    /// (e.g. the user edits a restored cell in the milliseconds after F5, while
    /// `DevLogin` is still in flight). Held here instead of dropped, and flushed
    /// once the document is open: for a server-side doc after its `RequestMissing`
    /// replay (so `local_clock` has caught up to the server and no CLOCK_MISMATCH
    /// occurs), for a local-only doc right when it opens.
    pending_pre_session_updates: Vec<(DocumentId, Vec<u8>)>,

    /// Last dev username seen on `DevLogin`, kept so the WS reauth path can
    /// silently re-login if the refresh token is also dead (e.g. the server was
    /// restarted with a fresh ephemeral JWT key and an empty Redis). `None` in a
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
                    access_token,
                    refresh_token,
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
            reconnect_attempts: 0,
            mock_ws_tx: None,
            event_tx: None,
            documents: HashMap::new(),
            doc_metadata: HashMap::new(),
            pending_opens: Vec::new(),
            pending_pre_session_updates: Vec::new(),
            dev_username: None,
        }
    }

    /// Stores the sender half of the internal event channel. Called once by
    /// [`run_wasm`] before the select loop, so JS closures and HTTP tasks can
    /// push events into the main loop without blocking.
    #[allow(dead_code)]
    pub(crate) fn set_event_tx(&mut self, tx: flume::Sender<SyncEvent>) {
        self.event_tx = Some(tx);
    }

    fn auth_context(&self) -> http::AuthContext {
        http::AuthContext {
            api_base: self.api_base.clone(),
            url_scheme: self.url_scheme,
            tokens: self.tokens.clone(),
        }
    }

    // -- Outbound: Sync → Logic ------------------------------------------------

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
    fn deliver_remote(
        &self,
        document_id: DocumentId,
        opened: Option<Result<Vec<u8>, pipeline::PipelineError>>,
        make_event: impl FnOnce(Vec<u8>) -> LogicEvent,
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

    // -- Outbound: Sync → Server (WebSocket) -----------------------------------

    /// Hook the in-memory mock transport onto this SyncState. In test mode the
    /// mock captures every WS message that would be sent, without touching
    /// JS / WASM APIs.
    pub fn set_mock_ws(&mut self, tx: flume::Sender<Vec<u8>>) {
        self.mock_ws_tx = Some(tx);
    }

    /// Test seam: mark an already-open document as existing server-side, so the
    /// outbound path sends updates directly instead of buffering them behind a
    /// lazy-creation POST. In production this flag is set by the async
    /// `OpenDocumentReady` (server returned members) or `DocumentMaterialized`
    /// flows, neither of which can complete in a synchronous native test.
    pub fn mark_document_server_created(&mut self, document_id: DocumentId) {
        if let Some(doc_state) = self.documents.get_mut(&document_id) {
            doc_state.server_created = true;
        }
    }

    /// Number of currently-open document sync sessions.
    pub fn documents_len(&self) -> usize {
        self.documents.len()
    }

    /// Encode and push a client message onto the WebSocket.
    ///
    /// A missing socket or a failed `send` is a transient network condition, not
    /// a programmer error: it must **not** panic (a panic is `panic=abort` and
    /// would tear the Sync thread down silently). Callers already gate on
    /// `self.ws.is_some()`; this only logs if that race is lost. The reconnect
    /// path (onclose → backoff) re-establishes the socket.
    fn ws_send(&self, msg: &WsClientMsg) {
        let wire = encode_client_msg(msg);
        let Some(ws) = self.ws.as_ref() else {
            if let Some(tx) = &self.mock_ws_tx {
                let _ = tx.send(wire);
                return;
            }
            tracing::warn!("[Sync→Server] ws_send called with no socket, dropping message");
            return;
        };
        tracing::debug!(
            "[Sync→Server] ws_send {:?} → {} bytes wire",
            std::mem::discriminant(msg),
            wire.len()
        );
        // `wire` lives in the wasm heap, which is a SharedArrayBuffer under the
        // shared-memory threading model. Browsers reject `WebSocket.send` on a
        // view backed by a SharedArrayBuffer, so copy into a fresh (non-shared)
        // ArrayBuffer first.
        let buf = js_sys::Uint8Array::new_with_length(wire.len() as u32);
        buf.copy_from(&wire);
        if let Err(e) = ws.send_with_array_buffer(&buf.buffer()) {
            tracing::warn!("[Sync→Server] WebSocket send failed ({e:?}); reconnect will retry");
        }
        if let Some(tx) = &self.mock_ws_tx {
            let _ = tx.send(wire);
        }
    }

    // -- Inbound: Logic → Sync -------------------------------------------------

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
            SyncCommand::SendUpdate {
                document_id,
                payload,
            } => {
                tracing::info!(
                    "[Sync] received SendUpdate doc={document_id} delta_size={}",
                    payload.len()
                );
                self.handle_send_update(document_id, payload);
            }
            SyncCommand::SendSnapshot {
                document_id,
                payload,
            } => {
                self.handle_send_snapshot(document_id, payload);
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
            SyncCommand::CloseChannel { channel_id } => self.handle_close_channel(channel_id),
            SyncCommand::SendChannelUpdate {
                channel_id,
                payload,
            } => {
                self.handle_send_channel_update(channel_id, payload);
            }
            SyncCommand::SendChannelEphemeral {
                channel_id,
                payload,
            } => {
                self.handle_send_channel_ephemeral(channel_id, payload);
            }

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
            RestCommand::UploadMedia {
                document_id,
                data,
                mime_type,
            } => {
                self.handle_upload_media(id, document_id, data, mime_type);
            }
            RestCommand::DownloadMedia {
                document_id,
                content_hash,
            } => {
                self.handle_download_media(id, document_id, content_hash);
            }

            // -- Channels (CRUD & sharing) -------------------------------------
            RestCommand::ListChannels => tracing::info!("Sync: ListChannels"),
            RestCommand::CreateChannel { name } => tracing::info!("Sync: CreateChannel '{name}'"),
            RestCommand::DeleteChannel { channel_id } => {
                tracing::info!("Sync: DeleteChannel {channel_id:?}");
            }
            RestCommand::InviteToChannel {
                channel_id,
                invitee_email,
                role,
            } => {
                tracing::info!("Sync: InviteToChannel {channel_id:?} {invitee_email} {role:?}");
            }
            RestCommand::RevokeFromChannel {
                channel_id,
                user_id,
            } => {
                tracing::info!("Sync: RevokeFromChannel {channel_id:?} {user_id:?}");
            }
            RestCommand::ListChannelMembers { channel_id } => {
                tracing::info!("Sync: ListChannelMembers {channel_id:?}");
            }
            RestCommand::UpdateChannelMemberRole {
                channel_id,
                user_id,
                role,
            } => {
                tracing::info!("Sync: UpdateChannelMemberRole {channel_id:?} {user_id:?} {role:?}");
            }
            RestCommand::UpdateReadCursor {
                channel_id,
                last_seq,
            } => {
                tracing::debug!("Sync: UpdateReadCursor {channel_id:?} seq={last_seq}");
            }
            RestCommand::GetUnreadCounts => tracing::info!("Sync: GetUnreadCounts"),

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
/// - `cmd_rx`: in-worker `flume` channel fed by the worker's `onmessage` handler
///   (decoded `SyncCommand`s relayed from the Main instance via `postMessage`).
/// - `logic_sink`: emits `LogicEvent`s back to Main (postMessage).
///
/// Internally it creates the event bus (`event_tx`/`event_rx`) into which WS /
/// HTTP callbacks push `SyncEvent`s, then `select!`s over both `recv_async`
/// futures. Producer and consumer share this worker, so the wakers fire; the
/// worker's JS event loop stays free for WebSocket / fetch callbacks.
#[cfg(target_arch = "wasm32")]
pub async fn run_wasm(
    cmd_rx: flume::Receiver<SyncCommand>,
    logic_sink: LogicSink,
    api_base: String,
) {
    use futures_util::future::FutureExt;

    tracing::info!("Ambre Sync worker starting (wasm async)");

    let mut state = SyncState::new(logic_sink, api_base, None);
    let (event_tx, event_rx) = flume::unbounded::<SyncEvent>();
    state.set_event_tx(event_tx);

    loop {
        futures_util::select! {
            cmd = cmd_rx.recv_async().fuse() => match cmd {
                Ok(cmd) => state.handle_command(cmd),
                // Command channel closed: the Main instance is gone (teardown).
                Err(_) => break,
            },
            ev = event_rx.recv_async().fuse() => match ev {
                Ok(ev) => state.handle_sync_event(ev),
                // event_tx is held by `state`, so this only errors at teardown.
                Err(_) => {}
            },
        }
    }

    tracing::info!("Ambre Sync worker exiting (command channel closed)");
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
        // be attributed to anyone: the caller drops the frame rather than showing
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
