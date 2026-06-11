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
    RestCommand, RestEvent, Role, SnapshotId, SyncCommand, UserId, UserInfo,
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
pub struct SendError(pub LogicEvent);

impl LogicSink {
    /// Wrap an arbitrary emit closure.
    pub fn new(f: impl Fn(LogicEvent) + 'static) -> Self {
        Self(std::rc::Rc::new(f))
    }

    /// A sink backed by a fresh `flume` channel, returning the receiver so a
    /// (native) test can drain what Sync emits.
    pub fn channel() -> (Self, flume::Receiver<LogicEvent>) {
        let (tx, rx) = flume::unbounded();
        (Self::new(move |ev| { let _ = tx.send(ev); }), rx)
    }

    /// Emit one event to the Logic thread.
    pub fn send(&self, event: LogicEvent) -> Result<(), SendError> {
        (self.0)(event);
        Ok(())
    }
}

mod http;
pub mod mock;
pub mod mock_relay;
mod pipeline;
/// Web Worker entry point (`#[wasm_bindgen(start)]`): the postMessage bridge to
/// the Main instance. wasm-only.
#[cfg(target_arch = "wasm32")]
mod worker;

// -----------------------------------------------------------------------------
// Quarantine bounds (ambre-spec-sync.md §E)
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
        /// Document kind, carried so a not-yet-on-server doc can be lazily created
        /// with the right type on its first outbound update.
        doc_type: DocType,
        /// Whether the document already exists server-side. `false` for a
        /// local-only doc (server returned 404 for its members): no WebSocket
        /// subscription yet, and the first update triggers lazy creation.
        server_created: bool,
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
        revoked_user_id: UserId,
    },
    /// Media upload completed: the encrypted blob was PUT to S3 and the server
    /// confirmed the insertion in the `media` table.
    MediaUploaded {
        id: RequestId,
        document_id: DocumentId,
        content_hash: String,
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
}

impl SyncDocumentState {
    fn new(
        document_id: DocumentId,
        doc_key: DocumentKey,
        doc_type: DocType,
        server_created: bool,
    ) -> Self {
        let current_snapshot_id = SnapshotId::new();
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
    /// document keys when sharing (not yet wired).
    #[allow(dead_code)]
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

    // -- In-memory WS transport (test-only, off) -------------------------------
    mock_ws_tx: Option<flume::Sender<Vec<u8>>>,

    // -- Internal event channel (carries WS frames + HTTP results into the     --
    // -- non-blocking select loop; producer and consumer are the same worker)  --
    event_tx: Option<flume::Sender<SyncEvent>>,

    // -- Per-document sync sessions --------------------------------------------
    documents: HashMap<DocumentId, SyncDocumentState>,

    // -- Document metadata (in-memory mirror for mock mode) --------------------
    doc_metadata: HashMap<DocumentId, (DocType, String)>,
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
            mock_ws_tx: None,
            event_tx: None,
            documents: HashMap::new(),
            doc_metadata: HashMap::new(),
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
                std::mem::discriminant(&ev),
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
        tracing::debug!("[Sync→Server] ws_send {:?} → {} bytes wire", std::mem::discriminant(msg), wire.len());
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
            SyncCommand::OpenDocument { document_id, doc_type } => {
                self.handle_open_document(document_id, doc_type)
            }
            SyncCommand::CloseDocument { document_id } => self.handle_close_document(document_id),
            SyncCommand::SendUpdate {
                document_id,
                payload,
            } => {
                tracing::info!("[Sync] received SendUpdate doc={document_id} delta_size={}", payload.len());
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
            RestCommand::SetupAccount { passphrase } => self.handle_setup_account(id, passphrase),
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
            RestCommand::InviteToDocument {
                document_id,
                invitee_email,
                role,
            } => {
                self.handle_invite_to_document(id, document_id, invitee_email, role);
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

    // -- Session handlers ------------------------------------------------------

    fn handle_unlock(&mut self, id: RequestId, passphrase: Vec<u8>) {
        tracing::info!("Sync: Unlock (passphrase len={})", passphrase.len());

        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let event_tx = self.event_tx.clone();
        let auth = self.auth_context();

        if let Some(tx) = event_tx {
            // WASM path: spawn an async HTTP task; the result flows back
            // through the SyncEvent channel into handle_sync_event.
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("{url_scheme}://{api_base}/api/keys/master");
                match http::get_json_auth::<http::MasterKeyResponse>(&url, auth.clone()).await {
                    Ok(resp) => {
                        let salt = match base64::engine::general_purpose::URL_SAFE.decode(&resp.master_key_salt) {
                            Ok(s) => s,
                            Err(e) => {
                                let _ = logic_tx.send(LogicEvent::AuthError {
                                    message: format!("Invalid salt: {e}"),
                                });
                                return;
                            }
                        };
                        let master_key_enc = match base64::engine::general_purpose::URL_SAFE.decode(&resp.master_key_enc) {
                            Ok(k) => k,
                            Err(e) => {
                                let _ = logic_tx.send(LogicEvent::AuthError {
                                    message: format!("Invalid master_key_enc: {e}"),
                                });
                                return;
                            }
                        };
                        if master_key_enc.len() < 24 {
                            let _ = logic_tx.send(LogicEvent::AuthError {
                                message: "Master key too short".into(),
                            });
                            return;
                        }
                        let nonce: &[u8; 24] = master_key_enc[..24].try_into().expect("nonce len");
                        let ciphertext = &master_key_enc[24..];
                        let user_key =
                            match ambre_crypto::passphrase::derive_user_key(&passphrase, &salt) {
                                Ok(k) => k,
                                Err(e) => {
                                    let _ = logic_tx.send(LogicEvent::AuthError {
                                        message: format!("Key derivation failed: {e}"),
                                    });
                                    return;
                                }
                            };
                        let user_doc_key = DocumentKey::from_array(user_key);
                        let seed_bytes = match ambre_crypto::aead::open(
                            nonce,
                            ciphertext,
                            b"ambre-master-seed",
                            &user_doc_key,
                        ) {
                            Ok(bytes) => bytes,
                            Err(e) => {
                                let _ = logic_tx.send(LogicEvent::AuthError {
                                    message: format!("Master seed decryption failed: {e}"),
                                });
                                return;
                            }
                        };
                        let seed_arr: [u8; 32] = match seed_bytes.try_into() {
                            Ok(arr) => arr,
                            Err(_) => {
                                let _ = logic_tx.send(LogicEvent::AuthError {
                                    message: "Master seed wrong length".into(),
                                });
                                return;
                            }
                        };
                        let master_seed = MasterSeed::from_array(seed_arr);
                        let signing_key = master_seed.derive_signing_key();
                        let exchange_key = master_seed.derive_exchange_key();

                        let auth_url = format!("{url_scheme}://{api_base}/api/auth/me");
                        match http::get_json_auth::<http::AuthMeResponse>(&auth_url, auth.clone()).await {
                            Ok(me) => {
                                let user_id = UserId::parse_str(&me.user_id)
                                    .unwrap_or_else(|_| UserId::new());
                                let _ = tx.send(SyncEvent::SessionReady {
                                    id,
                                    user_id,
                                    display_name: me.display_name,
                                    email: me.email,
                                    master_seed,
                                    signing_key,
                                    exchange_key,
                                    tokens: auth.tokens.borrow().as_ref().cloned().unwrap_or(http::TokenPair {
                                        access_token: String::new(),
                                        refresh_token: String::new(),
                                        access_expires_in: 0,
                                    }),
                                });
                            }
                            Err(e) => {
                                let _ = logic_tx.send(LogicEvent::AuthError { message: e });
                            }
                        }
                    }
                    Err(e) => {
                        let _ = logic_tx.send(LogicEvent::AuthError { message: e });
                    }
                }
            });
        } else {
            // Host-test / offline path: generate keys locally.
            let master_seed = MasterSeed::generate();
            self.signing_key = Some(master_seed.derive_signing_key());
            self.exchange_key = Some(master_seed.derive_exchange_key());
            self.master_seed = Some(master_seed);
            self.open_websocket();
            self.reply(
                id,
                RestEvent::SessionReady {
                    user_id: UserId::new(),
                    display_name: "offline".into(),
                    email: "offline@localhost".into(),
                },
            );
        }
    }

    /// Dev-only login: authenticate as `username` with keys derived
    /// deterministically from the username (no passphrase, no master-key
    /// round-trip). The server creates the user on the fly and is the sole gate
    /// (it rejects this unless `DEV_MODE` is set). Lets `?user=<name>` test
    /// multi-user behaviour on one machine; an edit survives a refresh because
    /// the same username always derives the same identity.
    fn handle_dev_login(&mut self, id: RequestId, username: String) {
        tracing::info!("Sync: DevLogin (username={username})");

        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let event_tx = self.event_tx.clone();

        // Deterministic seed: domain-separated BLAKE2b of the username. The same
        // username always yields the same keys, so a refresh re-derives the
        // identity with no server-side key storage.
        let master_seed = MasterSeed::from_array(ambre_crypto::hash::chain_hash(
            b"ambre-dev-seed",
            username.as_bytes(),
        ));
        let signing_key = master_seed.derive_signing_key();
        let exchange_key = master_seed.derive_exchange_key();

        if let Some(tx) = event_tx {
            let signing_pub_key = hex::encode(signing_key.verifying_key().to_bytes());
            let exchange_pub_key = {
                let pub_key = x25519_dalek::PublicKey::from(&exchange_key);
                hex::encode(pub_key.as_bytes())
            };
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("{url_scheme}://{api_base}/api/auth/dev/login");
                let body = http::DevLoginRequest {
                    username,
                    signing_pub_key,
                    exchange_pub_key,
                };
                match http::post_json::<_, http::DevLoginResponse>(&url, &body).await {
                    Ok(resp) => {
                        let user_id =
                            UserId::parse_str(&resp.user_id).unwrap_or_else(|_| UserId::new());
                        let _ = tx.send(SyncEvent::SessionReady {
                            id,
                            user_id,
                            display_name: resp.display_name,
                            email: String::new(),
                            master_seed,
                            signing_key,
                            exchange_key,
                            tokens: http::TokenPair {
                                access_token: resp.access_token,
                                refresh_token: resp.refresh_token,
                                access_expires_in: resp.access_expires_in,
                            },
                        });
                    }
                    Err(e) => {
                        let _ = logic_tx.send(LogicEvent::AuthError { message: e });
                    }
                }
            });
        } else {
            // Host-test / offline path: adopt the deterministic keys locally.
            self.signing_key = Some(signing_key);
            self.exchange_key = Some(exchange_key);
            self.master_seed = Some(master_seed);
            self.open_websocket();
            self.reply(
                id,
                RestEvent::SessionReady {
                    user_id: UserId::new(),
                    display_name: username,
                    email: String::new(),
                },
            );
        }
    }

    fn handle_setup_account(&mut self, id: RequestId, passphrase: Vec<u8>) {
        tracing::info!("Sync: SetupAccount (passphrase len={})", passphrase.len());

        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let master_seed = MasterSeed::generate();
                let recovery_words = encode(master_seed.as_bytes());
                let signing_key = master_seed.derive_signing_key();
                let exchange_key = master_seed.derive_exchange_key();
                let signing_pub_key = hex::encode(signing_key.verifying_key().to_bytes());
                let exchange_pub_key = {
                    let pub_key = x25519_dalek::PublicKey::from(&exchange_key);
                    hex::encode(pub_key.as_bytes())
                };
                let salt = {
                    let mut buf = [0u8; 32];
                    getrandom::fill(&mut buf).expect("CSPRNG");
                    buf
                };
                let user_key = ambre_crypto::passphrase::derive_user_key(&passphrase, &salt)
                    .expect("key derivation");
                let user_doc_key = DocumentKey::from_array(user_key);
                let (nonce, master_key_enc) = ambre_crypto::aead::seal(
                    master_seed.as_bytes(),
                    b"ambre-master-seed",
                    &user_doc_key,
                );
                let mut master_key_with_nonce = nonce.to_vec();
                master_key_with_nonce.extend_from_slice(&master_key_enc);

                let url = format!("{url_scheme}://{api_base}/api/auth/setup");
                let body = http::SetupAccountRequest {
                    passphrase: String::from_utf8_lossy(&passphrase).into_owned(),
                    master_key_enc: base64::engine::general_purpose::URL_SAFE.encode(&master_key_with_nonce),
                    master_key_salt: base64::engine::general_purpose::URL_SAFE.encode(salt),
                    signing_pubkey: signing_pub_key,
                    exchange_pubkey: exchange_pub_key,
                };
                match http::post_json::<_, http::SetupAccountResponse>(&url, &body).await {
                    Ok(resp) => {
                        let _ = tx.send(SyncEvent::AccountCreated {
                            id,
                            recovery_words,
                            master_seed,
                            signing_key,
                            exchange_key,
                            tokens: http::TokenPair {
                                access_token: resp.access_token,
                                refresh_token: resp.refresh_token,
                                access_expires_in: resp.access_expires_in,
                            },
                        });
                    }
                    Err(e) => {
                        let _ = logic_tx.send(LogicEvent::AuthError { message: e });
                    }
                }
            });
        } else {
            let _ = passphrase;
            let master_seed = MasterSeed::generate();
            self.signing_key = Some(master_seed.derive_signing_key());
            self.exchange_key = Some(master_seed.derive_exchange_key());
            let recovery_words = encode(master_seed.as_bytes());
            self.master_seed = Some(master_seed);
            self.open_websocket();
            self.reply(id, RestEvent::AccountCreated { recovery_words });
        }
    }

    fn handle_change_passphrase(&mut self, current_passphrase: Vec<u8>, new_passphrase: Vec<u8>) {
        tracing::info!("Sync: ChangePassphrase");
        let _ = (current_passphrase, new_passphrase);
    }

    fn handle_recover_account(&mut self, recovery_words: String, new_passphrase: Vec<u8>) {
        tracing::info!("Sync: RecoverAccount");
        let _ = (recovery_words, new_passphrase);
    }

    /// Tear down the session (WebSocket, document sessions, keys). The
    /// `LoggedOut` response — when there is a request to answer — is sent by the
    /// caller (`Shutdown` performs the teardown without replying).
    fn handle_logout(&mut self) {
        if let Some(ws) = self.ws.take() {
            let _ = ws.close();
        }
        self._onmessage = None;
        self._onclose = None;
        self._onerror = None;
        self.documents.clear();
        self.master_seed = None;
        self.signing_key = None;
        self.exchange_key = None;
        *self.tokens.borrow_mut() = None;
        tracing::info!("Sync: logged out, all keys zeroized");
    }

    // -- WebSocket lifecycle ---------------------------------------------------

    /// Opens the session-level multiplexed WebSocket. Called after Unlock or
    /// SetupAccount. If the socket is already present (reconnection), it is
    /// replaced. All open documents are automatically re-subscribed.
    #[cfg(target_arch = "wasm32")]
    fn open_websocket(&mut self) {
        // Mirror the REST scheme: `ws` on localhost (http), `wss` otherwise.
        // A hardcoded `wss://` fails on a localhost http dev server (TLS error,
        // WS close code 1015).
        let ws_scheme = if self.url_scheme == "http" { "ws" } else { "wss" };
        let Some(access_token) = self.tokens.borrow().as_ref().map(|t| t.access_token.clone()) else {
            tracing::warn!("Sync: WebSocket open requested before access token is ready");
            return;
        };
        let token = js_sys::encode_uri_component(&access_token)
            .as_string()
            .unwrap_or(access_token);
        let url = format!("{ws_scheme}://{}/api/ws?token={token}", self.api_base);
        match WebSocket::new(&url) {
            Ok(ws) => {
                ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
                self.wire_ws_callbacks(&ws);
                self.ws = Some(ws);
                for doc_id in self.documents.keys().copied().collect::<Vec<_>>() {
                    self.ws_send(&WsClientMsg::Subscribe {
                        document_id: doc_id,
                    });
                }
                tracing::info!("Sync: WebSocket opened ({url})");
            }
            Err(e) => {
                tracing::error!("Sync: WebSocket::new({url}) failed: {e:?}");
            }
        }
    }

    /// Native / host-test stub: does nothing (mock transport handles WS).
    #[cfg(not(target_arch = "wasm32"))]
    fn open_websocket(&mut self) {
        tracing::info!("Sync: WebSocket opened (native mock)");
    }

    /// Attaches onmessage / onclose / onerror closures to `ws`. Each closure
    /// pushes the decoded event into `self.event_tx`, so the non-blocking select
    /// loop in [`run_wasm`] processes them without blocking the JS event loop.
    #[cfg(target_arch = "wasm32")]
    fn wire_ws_callbacks(&mut self, ws: &WebSocket) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx.clone() {
            let onmessage = Closure::wrap(Box::new(move |e: JsValue| {
                let msg: MessageEvent = match e.dyn_into() {
                    Ok(m) => m,
                    Err(_) => return,
                };
                let data = msg.data();
                let array_buf: js_sys::ArrayBuffer = match data.dyn_into() {
                    Ok(a) => a,
                    Err(_) => return,
                };
                let bytes = js_sys::Uint8Array::new(&array_buf).to_vec();
                match decode_server_msg(&bytes) {
                    Ok(server_msg) => {
                        let _ = tx.send(SyncEvent::WsMessage(server_msg));
                    }
                    Err(e) => tracing::error!("Sync: WS decode error: {e:?}"),
                }
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            self._onmessage = Some(onmessage);
        }

        if let Some(tx) = event_tx.clone() {
            let onclose = Closure::wrap(Box::new(move |e: JsValue| {
                let ce: CloseEvent = match e.dyn_into() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let _ = tx.send(SyncEvent::WsDisconnected {
                    code: ce.code(),
                    reason: ce.reason(),
                });
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
            self._onclose = Some(onclose);
        }

        if let Some(tx) = event_tx {
            let onerror = Closure::wrap(Box::new(move |_e: JsValue| {
                let _ = tx.send(SyncEvent::WsError);
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
            self._onerror = Some(onerror);
        }
    }

    /// Called by the select loop on a `WsDisconnected` event. Clears the socket
    /// and notifies the Logic thread that every open document lost its
    /// connection. A reconnection is attempted immediately.
    #[allow(dead_code)]
    fn handle_ws_disconnected(&mut self, code: u16, reason: String) {
        tracing::warn!("Sync: WS disconnected (code={code}): {reason}");
        self.ws = None;
        self._onmessage = None;
        self._onclose = None;
        self._onerror = None;
        for doc_id in self.documents.keys().copied().collect::<Vec<_>>() {
            self.send_to_logic(LogicEvent::ConnectionStatus {
                document_id: doc_id,
                connected: false,
            });
        }
    }

    /// Processes one [`SyncEvent`] received from the internal event channel.
    /// Dispatches WebSocket frames to the existing [`handle_ws_message`]
    /// pipeline, applies state mutations from HTTP result events, and handles
    /// disconnection / errors.
    #[allow(dead_code)]
    pub(crate) fn handle_sync_event(&mut self, event: SyncEvent) {
        match event {
            SyncEvent::WsMessage(msg) => self.handle_ws_message(msg),
            SyncEvent::WsDisconnected { code, reason } => {
                self.handle_ws_disconnected(code, reason);
                if !self.documents.is_empty() {
                    self.open_websocket();
                }
            }
            SyncEvent::WsError => {
                tracing::error!("Sync: WebSocket error");
            }
            SyncEvent::SessionReady {
                id,
                user_id,
                display_name,
                email,
                master_seed,
                signing_key,
                exchange_key,
                tokens,
            } => {
                self.user_id = Some(user_id);
                self.master_seed = Some(master_seed);
                self.signing_key = Some(signing_key);
                self.exchange_key = Some(exchange_key);
                *self.tokens.borrow_mut() = Some(tokens);
                self.open_websocket();
                self.reply(
                    id,
                    RestEvent::SessionReady {
                        user_id,
                        display_name,
                        email,
                    },
                );
            }
            SyncEvent::AccountCreated {
                id,
                recovery_words,
                master_seed,
                signing_key,
                exchange_key,
                tokens,
            } => {
                self.master_seed = Some(master_seed);
                self.signing_key = Some(signing_key);
                self.exchange_key = Some(exchange_key);
                *self.tokens.borrow_mut() = Some(tokens);
                self.open_websocket();
                self.reply(id, RestEvent::AccountCreated { recovery_words });
            }
            SyncEvent::DocumentList { id, documents } => {
                self.reply(id, RestEvent::DocumentList { documents });
            }
            SyncEvent::DocumentCreated {
                id,
                document_id,
                doc_type,
            } => {
                self.reply(
                    id,
                    RestEvent::DocumentCreated {
                        document_id,
                        doc_type,
                    },
                );
            }
            SyncEvent::DocumentDeleted { id, document_id } => {
                self.reply(id, RestEvent::DocumentDeleted { document_id });
            }
            SyncEvent::DocumentRenamed { id, document_id } => {
                self.reply(id, RestEvent::DocumentRenamed { document_id });
            }
            SyncEvent::OpenDocumentReady {
                document_id,
                doc_key,
                snapshot_payload,
                snapshot_id,
                doc_type,
                server_created,
            } => {
                self.documents.insert(
                    document_id,
                    SyncDocumentState::new(document_id, doc_key, doc_type, server_created),
                );
                let doc_state = self.documents.get_mut(&document_id).expect("just inserted");
                doc_state.current_snapshot_id = snapshot_id;
                doc_state.known_snapshots.insert(snapshot_id);
                if let Some(payload) = snapshot_payload {
                    self.send_to_logic(LogicEvent::RemoteSnapshot {
                        document_id,
                        payload,
                    });
                }
                // Only subscribe over WS if the document actually exists
                // server-side; a local-only doc subscribes after lazy creation.
                if server_created && self.ws_connected() {
                    self.ws_send(&WsClientMsg::Subscribe { document_id });
                }
                self.send_to_logic(LogicEvent::ConnectionStatus {
                    document_id,
                    connected: self.ws_connected(),
                });
            }
            SyncEvent::DocumentMaterialized { document_id } => {
                let pending = match self.documents.get_mut(&document_id) {
                    Some(doc_state) => {
                        doc_state.server_created = true;
                        doc_state.creating = false;
                        std::mem::take(&mut doc_state.pending_updates)
                    }
                    None => Vec::new(),
                };
                if self.ws_connected() {
                    self.ws_send(&WsClientMsg::Subscribe { document_id });
                }
                tracing::info!(
                    "Sync: document {document_id:?} materialized, flushing {} update(s)",
                    pending.len()
                );
                for payload in pending {
                    self.handle_send_update(document_id, payload);
                }
            }
            SyncEvent::DocumentCreateFailed { document_id, message } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.creating = false;
                }
                tracing::error!("Sync: lazy create failed for {document_id:?}: {message}");
            }
            SyncEvent::HttpError { id: _id, message } => {
                self.send_to_logic(LogicEvent::ApiError {
                    request: "rest".into(),
                    code: 500,
                    message,
                });
            }
            SyncEvent::InviteSent {
                id,
                document_id,
                user_id,
                display_name,
            } => {
                self.reply(
                    id,
                    RestEvent::InviteSent {
                        document_id,
                        user_id,
                        display_name,
                    },
                );
            }
            SyncEvent::RevocationComplete {
                id,
                document_id,
                new_doc_key,
            } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.rotate_key(new_doc_key);
                }
                self.reply(id, RestEvent::RevocationComplete { document_id });
            }
            SyncEvent::DocumentMembersReady {
                id,
                document_id,
                members,
            } => {
                self.reply(
                    id,
                    RestEvent::DocumentMembers {
                        document_id,
                        members,
                    },
                );
            }
            SyncEvent::UserSearchReady { id, users } => {
                self.reply(id, RestEvent::UserSearchResults { users });
            }
            SyncEvent::MemberRoleUpdatedEvent {
                id,
                document_id,
                user_id,
                role,
            } => {
                self.reply(
                    id,
                    RestEvent::MemberRoleUpdated {
                        document_id,
                        user_id,
                        role,
                    },
                );
            }
            SyncEvent::KeyRotated {
                document_id,
                new_doc_key,
                revoked_user_id,
            } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.rotate_key(new_doc_key);
                }
                self.send_to_logic(LogicEvent::MemberRevoked {
                    document_id,
                    revoked_user_id,
                });
            }
            SyncEvent::MediaUploaded {
                id,
                document_id,
                content_hash,
            } => {
                self.reply(
                    id,
                    RestEvent::MediaUploaded {
                        document_id,
                        content_hash,
                    },
                );
            }
            SyncEvent::MediaDownloaded {
                id,
                document_id,
                content_hash,
                data,
            } => {
                self.reply(
                    id,
                    RestEvent::MediaDownloaded {
                        document_id,
                        content_hash,
                        data,
                    },
                );
            }
        }
    }

    // -- Inbound: Server -> Sync (WebSocket demux) -----------------------------

    /// Demultiplex an inbound [`WsServerMsg`] received from the WebSocket.
    /// Decoding from wire bytes is done in the `onmessage` callback;
    /// this method receives the already-decoded message.
    #[allow(dead_code)]
    fn handle_ws_message(&mut self, server_msg: WsServerMsg) {
        match server_msg {
            WsServerMsg::Subscribed { document_id } => {
                self.send_to_logic(LogicEvent::ConnectionStatus {
                    document_id,
                    connected: true,
                });
            }

            WsServerMsg::Update { seq, update } => {
                tracing::debug!(
                    "[Sync←Server] WS Update seq={} doc={} clock={}",
                    seq.0,
                    update.public_data.document_id,
                    update.public_data.clock.0,
                );
                self.receive_update(seq, update);
            }

            WsServerMsg::Ephemeral(eph) => {
                let doc_id = eph.public_data.document_id;
                let Some(doc_state) = self.documents.get_mut(&doc_id) else {
                    return;
                };

                // Anti-replay: reject ephemerals with a non-monotonic counter
                // for the same author.
                let author_pk = *eph.public_data.pub_key.as_array();
                let last_counter = doc_state
                    .ephemeral_remote_counters
                    .get(&author_pk)
                    .copied()
                    .unwrap_or(0);
                if eph.public_data.counter.0 <= last_counter {
                    tracing::debug!(
                        "Sync: dropping replayed ephemeral from {:?} (counter {})",
                        author_pk,
                        eph.public_data.counter.0
                    );
                    return;
                }
                doc_state
                    .ephemeral_remote_counters
                    .insert(author_pk, eph.public_data.counter.0);

                let opened = doc_state.open_ephemeral_fallback(&eph);
                self.deliver_remote(doc_id, Some(opened), |payload| {
                    LogicEvent::RemoteEphemeral {
                        document_id: doc_id,
                        payload,
                    }
                });
            }

            WsServerMsg::MissingUpdates {
                document_id: _,
                updates,
            } => {
                for seq_update in updates {
                    self.receive_update(seq_update.seq, seq_update.update);
                }
            }

            WsServerMsg::SnapshotRequired { document_id } => {
                self.send_to_logic(LogicEvent::SnapshotNeeded { document_id });
            }

            WsServerMsg::SnapshotAvailable {
                document_id,
                snapshot_id,
            } => {
                tracing::info!("Sync: SnapshotAvailable {snapshot_id:?} for {document_id:?}");
                self.snapshot_known(document_id, snapshot_id);
            }

            WsServerMsg::KeyRotationRequired {
                document_id,
                revoked_user_id,
            } => {
                // If WE are the revoked user, close the document and notify.
                if Some(revoked_user_id) == self.user_id {
                    self.documents.remove(&document_id);
                    self.send_to_logic(LogicEvent::MemberRevoked {
                        document_id,
                        revoked_user_id,
                    });
                    return;
                }

                // Otherwise, fetch the new wrapped key from the server,
                // unwrap it, and rotate locally.
                let event_tx = self.event_tx.clone();
                let api_base = self.api_base.clone();
                let url_scheme = self.url_scheme;
                let exchange_key = self.exchange_key.clone();
                let doc_id = document_id;
                let auth = self.auth_context();

                if let Some(tx) = event_tx {
                    let Some(ek) = exchange_key else { return };
                    wasm_bindgen_futures::spawn_local(async move {
                        let members_url =
                            format!("{url_scheme}://{api_base}/api/documents/{doc_id}/members");
                        let members =
                            match http::get_json_auth::<http::MembersResponse>(&members_url, auth).await {
                                Ok(resp) => resp.members,
                                Err(e) => {
                                    tracing::error!(
                                        "Sync: KeyRotationRequired fetch members failed: {e}"
                                    );
                                    return;
                                }
                            };

                        let our_exchange_pub = {
                            let pk = x25519_dalek::PublicKey::from(&ek);
                            hex::encode(pk.as_bytes())
                        };

                        let entry = match members
                            .iter()
                            .find(|m| m.exchange_pub_key == our_exchange_pub)
                        {
                            Some(e) => e,
                            None => {
                                tracing::warn!(
                                    "Sync: KeyRotationRequired: own member entry not found"
                                );
                                return;
                            }
                        };

                        let new_doc_key = match (|| -> Result<DocumentKey, String> {
                            let encrypted = base64::engine::general_purpose::URL_SAFE
                                .decode(&entry.doc_key_enc)
                                .map_err(|e| format!("base64 decode doc_key_enc: {e}"))?;
                            if encrypted.len() < 24 {
                                return Err("wrapped key too short".into());
                            }
                            let nonce: [u8; 24] = encrypted[..24]
                                .try_into()
                                .map_err(|_| "invalid nonce length")?;
                            let ciphertext = encrypted[24..].to_vec();
                            let peer_pub: [u8; 32] = hex::decode(&entry.exchange_pub_key)
                                .map_err(|e| format!("hex decode pubkey: {e}"))?
                                .try_into()
                                .map_err(|_| "invalid pubkey length")?;
                            let wrapped = ambre_crypto::WrappedKey { nonce, ciphertext };
                            ambre_crypto::ecdh::unwrap_document_key(&ek, &peer_pub, &wrapped)
                                .map_err(|e| format!("ECDH unwrap: {e}"))
                        })() {
                            Ok(k) => k,
                            Err(e) => {
                                tracing::error!("Sync: KeyRotationRequired unwrap failed: {e}");
                                return;
                            }
                        };

                        let _ = tx.send(SyncEvent::KeyRotated {
                            document_id: doc_id,
                            new_doc_key,
                            revoked_user_id,
                        });
                    });
                }
            }

            WsServerMsg::ChannelSubscribed { channel_id } => {
                self.send_to_logic(LogicEvent::ChannelConnectionStatus {
                    channel_id,
                    connected: true,
                });
            }

            WsServerMsg::ChannelUpdate {
                channel_id,
                seq: _,
                update,
            } => {
                let _ = (channel_id, update);
            }

            WsServerMsg::ChannelEphemeral {
                channel_id,
                ephemeral,
            } => {
                let _ = (channel_id, ephemeral);
            }

            WsServerMsg::Error { code, message } => {
                tracing::error!("Sync: server error {code}: {message}");
                self.send_to_logic(LogicEvent::ApiError {
                    request: "websocket".into(),
                    code,
                    message,
                });
            }
        }
    }

    // -- Causal quarantine (ambre-spec-sync.md §E) -----------------------------

    /// Ingest one server-sequenced [`Update`]. If its parent snapshot is known,
    /// it is decrypted and delivered immediately; otherwise it is held in
    /// quarantine until the parent snapshot arrives (then released in `clock`
    /// order by [`release_quarantine`]).
    #[allow(dead_code)]
    fn receive_update(&mut self, seq: Seq, update: Update) {
        let document_id = update.public_data.document_id;
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            tracing::warn!("Sync: Update for unopened doc {document_id:?}, dropping");
            return;
        };
        doc_state.last_seq = seq;

        let snapshot_id = update.public_data.snapshot_id;
        if doc_state.known_snapshots.contains(&snapshot_id) {
            let opened = doc_state.open_update_fallback(&update);
            tracing::info!(
                "[Sync→Logic] delivering RemoteUpdate doc={document_id} seq={} clock={} size={}",
                seq.0,
                update.public_data.clock.0,
                opened.as_ref().map(|b| b.len()).unwrap_or(0),
            );
            self.deliver_remote(document_id, Some(opened), |payload| {
                LogicEvent::RemoteUpdate {
                    document_id,
                    payload,
                }
            });
            return;
        }

        doc_state
            .quarantine
            .entry(snapshot_id)
            .or_default()
            .push(update);
        tracing::debug!(
            "[Sync] quarantined update doc={document_id} seq={} snapshot={snapshot_id:?}",
            seq.0,
        );
        self.enforce_quarantine_bounds(document_id);
    }

    /// Keep a document's quarantine within [`QUARANTINE_LIMIT`]: request the
    /// missing history at the soft threshold, and drop the oldest update (by
    /// `clock`) past the hard cap, raising a `SyncWarning`.
    fn enforce_quarantine_bounds(&mut self, document_id: DocumentId) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };

        if doc_state.quarantine_len() >= QUARANTINE_ALERT {
            let after_seq = doc_state.last_seq;
            if self.ws_connected() {
                self.ws_send(&WsClientMsg::RequestMissing {
                    document_id,
                    after_seq,
                });
            }
        }

        let doc_state = self.documents.get_mut(&document_id).expect("doc present");
        if doc_state.quarantine_len() >= QUARANTINE_LIMIT {
            // Drop the globally oldest quarantined update (smallest clock).
            let oldest = doc_state
                .quarantine
                .iter()
                .filter_map(|(snap, updates)| {
                    updates
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, u)| u.public_data.clock.0)
                        .map(|(idx, u)| (*snap, idx, u.public_data.clock.0))
                })
                .min_by_key(|&(_, _, clock)| clock);

            if let Some((snap, idx, clock)) = oldest {
                if let Some(updates) = doc_state.quarantine.get_mut(&snap) {
                    updates.remove(idx);
                    if updates.is_empty() {
                        doc_state.quarantine.remove(&snap);
                    }
                }
                self.send_to_logic(LogicEvent::SyncWarning {
                    document_id,
                    code: SyncWarningCode::QuarantineOverflow,
                    message: format!("quarantine overflow, dropped oldest update (clock {clock})"),
                });
            }
        }
    }

    /// Mark `snapshot_id` as known for `document_id` and release any updates that
    /// were quarantined waiting for it, in ascending `clock` order.
    fn snapshot_known(&mut self, document_id: DocumentId, snapshot_id: SnapshotId) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            tracing::warn!("Sync: SnapshotAvailable for unopened doc {document_id:?}");
            return;
        };
        doc_state.known_snapshots.insert(snapshot_id);
        doc_state.current_snapshot_id = snapshot_id;
        self.release_quarantine(document_id, snapshot_id);
    }

    /// Decrypt and deliver every update quarantined under `snapshot_id`, oldest
    /// `clock` first.
    fn release_quarantine(&mut self, document_id: DocumentId, snapshot_id: SnapshotId) {
        let mut pending = {
            let Some(doc_state) = self.documents.get_mut(&document_id) else {
                return;
            };
            doc_state
                .quarantine
                .remove(&snapshot_id)
                .unwrap_or_default()
        };
        pending.sort_by_key(|u| u.public_data.clock.0);

        let results: Vec<Result<Vec<u8>, pipeline::PipelineError>> = {
            let Some(doc_state) = self.documents.get(&document_id) else {
                return;
            };
            pending
                .iter()
                .map(|u| doc_state.open_update_fallback(u))
                .collect()
        };
        for opened in results {
            self.deliver_remote(document_id, Some(opened), |payload| {
                LogicEvent::RemoteUpdate {
                    document_id,
                    payload,
                }
            });
        }
    }

    // -- Document lifecycle ----------------------------------------------------

    fn handle_open_document(&mut self, document_id: DocumentId, doc_type: DocType) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let doc_id = document_id;
            let exchange_key = self.exchange_key.clone();
            let auth = self.auth_context();
            wasm_bindgen_futures::spawn_local(async move {
                let members_url = format!("{url_scheme}://{api_base}/api/documents/{doc_id}/members");
                // 404 -> the doc does not exist server-side yet (local-only):
                // generate a fresh key and defer creation to the first update.
                let (doc_key, server_created) =
                    match http::get_json_opt_auth::<http::MembersResponse>(&members_url, auth.clone()).await {
                        Ok(Some(resp)) => {
                            let key = match &exchange_key {
                                Some(ek) => Self::unwrap_document_key(ek, &resp.members)
                                    .unwrap_or_else(|_| DocumentKey::generate()),
                                None => DocumentKey::generate(),
                            };
                            (key, true)
                        }
                        // Not found, or a transient error: treat as local-only.
                        // A transient error simply means no remote history this
                        // open; the doc still works locally and reconciles later.
                        Ok(None) | Err(_) => (DocumentKey::generate(), false),
                    };
                let snapshot_payload = if server_created {
                    let snapshot_url =
                        format!("{url_scheme}://{api_base}/api/documents/{doc_id}/snapshot/latest");
                    match http::get_json_auth::<http::SnapshotPresignedResponse>(&snapshot_url, auth).await {
                        Ok(presigned) => http::fetch_bytes(&presigned.url).await.ok(),
                        Err(_) => None,
                    }
                } else {
                    None
                };
                let snapshot_id = SnapshotId::new();
                let _ = tx.send(SyncEvent::OpenDocumentReady {
                    document_id: doc_id,
                    doc_key,
                    snapshot_payload,
                    snapshot_id,
                    doc_type,
                    server_created,
                });
            });
        } else {
            let doc_key = DocumentKey::generate();
            self.documents.insert(
                document_id,
                SyncDocumentState::new(document_id, doc_key, doc_type, false),
            );
            if self.ws_connected() {
                self.ws_send(&WsClientMsg::Subscribe { document_id });
            }
            tracing::info!("Sync: OpenDocument {document_id:?}");
        }
    }

    /// Decrypts the DocumentKey from the server's member list using X25519 ECDH.
    ///
    /// 1. Computes the local exchange public key from `exchange_key`.
    /// 2. Finds our own [`MemberEntry`] by matching `exchange_pub_key`.
    /// 3. Hex-decodes the `doc_key_enc` blob (24-byte nonce + AEAD ciphertext
    ///    with Poly1305 tag).
    /// 4. Calls [`ambre_crypto::ecdh::unwrap_document_key`] to perform the
    ///    ECDH shared-secret derivation, KEK derivation, and AEAD decryption.
    fn unwrap_document_key(
        exchange_key: &x25519_dalek::StaticSecret,
        members: &[http::MemberEntry],
    ) -> Result<DocumentKey, String> {
        let our_exchange_pub = {
            let pk = x25519_dalek::PublicKey::from(exchange_key);
            hex::encode(pk.as_bytes())
        };

        let entry = members
            .iter()
            .find(|m| m.exchange_pub_key == our_exchange_pub)
            .ok_or("own member entry not found")?;

        let encrypted = base64::engine::general_purpose::URL_SAFE
            .decode(&entry.doc_key_enc)
            .map_err(|e| format!("base64 decode doc_key_enc: {e}"))?;
        if encrypted.len() < 24 {
            return Err("wrapped key too short".into());
        }
        let nonce: [u8; 24] = encrypted[..24]
            .try_into()
            .map_err(|_| "invalid nonce length")?;
        let ciphertext = encrypted[24..].to_vec();

        let peer_pub_bytes: [u8; 32] = hex::decode(&entry.exchange_pub_key)
            .map_err(|e| format!("hex decode exchange_pub_key: {e}"))?
            .try_into()
            .map_err(|_| "invalid exchange pubkey length")?;

        let wrapped = ambre_crypto::WrappedKey { nonce, ciphertext };
        ambre_crypto::ecdh::unwrap_document_key(exchange_key, &peer_pub_bytes, &wrapped)
            .map_err(|e| format!("ECDH unwrap: {e}"))
    }

    fn handle_close_document(&mut self, document_id: DocumentId) {
        if self.ws_connected() {
            self.ws_send(&WsClientMsg::Unsubscribe { document_id });
        }
        self.documents.remove(&document_id);
        tracing::info!("Sync: CloseDocument {document_id:?}");
    }

    fn handle_reconnect(&mut self, document_id: DocumentId) {
        if self.ws_connected() {
            let last_seq = self
                .documents
                .get(&document_id)
                .map(|d| d.last_seq)
                .unwrap_or(Seq(0));
            self.ws_send(&WsClientMsg::Subscribe { document_id });
            self.ws_send(&WsClientMsg::RequestMissing {
                document_id,
                after_seq: last_seq,
            });
        }
    }

    // -- Outbound sync ---------------------------------------------------------

    /// The author identity (Ed25519 verifying key) for outbound messages, or
    /// `None` before Unlock has seeded the session keys.
    fn author_pub_key(&self) -> Option<PublicKey> {
        self.signing_key
            .as_ref()
            .map(|sk| PublicKey::from_array(sk.verifying_key().to_bytes()))
    }

    fn handle_send_update(&mut self, document_id: DocumentId, payload: Vec<u8>) {
        let Some(signing_key) = self.signing_key.clone() else {
            tracing::warn!("[Sync] SendUpdate before session keys ready, dropping");
            return;
        };
        let Some(pub_key) = self.author_pub_key() else {
            return;
        };
        // Lazy server creation: a local-only doc materializes on its first update.
        // Buffer the payload and fire the create POST once; the buffer is flushed
        // in order from the `DocumentMaterialized` handler.
        if self
            .documents
            .get(&document_id)
            .is_some_and(|d| !d.server_created)
        {
            if let Some(doc_state) = self.documents.get_mut(&document_id) {
                doc_state.pending_updates.push(payload);
            }
            self.trigger_lazy_create(document_id);
            return;
        }
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            tracing::warn!("[Sync] SendUpdate for unknown doc {document_id:?}");
            return;
        };
        doc_state.local_clock += 1;
        let public_data = UpdatePublicData {
            document_id,
            pub_key,
            snapshot_id: doc_state.current_snapshot_id,
            clock: Clock(doc_state.local_clock),
        };
        match pipeline::seal_update(&payload, public_data, &doc_state.doc_key, &signing_key) {
            Ok(update) => {
                tracing::info!(
                    "[Sync] seal_update ok doc={document_id} clock={} → {} bytes ciphertext",
                    doc_state.local_clock,
                    update.ciphertext.0.len(),
                );
                self.ws_send(&WsClientMsg::Update(update));
            }
            Err(e) => tracing::error!("[Sync] SendUpdate seal failed for {document_id:?}: {e}"),
        }
    }

    /// Lazily materialize a local-only document server-side (first-modification
    /// path): wrap its DocumentKey for the owner and POST `/api/documents` with
    /// the client-generated id. Idempotent server-side and guarded by the
    /// per-doc `creating` flag, so concurrent updates issue exactly one POST.
    /// On success a [`SyncEvent::DocumentMaterialized`] flushes the buffer.
    fn trigger_lazy_create(&mut self, document_id: DocumentId) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        if doc_state.creating {
            return;
        }
        let doc_type = doc_state.doc_type;
        let doc_key = doc_state.doc_key.clone();
        let Some(tx) = self.event_tx.clone() else {
            return;
        };
        let Some(exchange_key) = self.exchange_key.clone() else {
            tracing::warn!("[Sync] lazy create for {document_id:?} without exchange key");
            return;
        };
        doc_state.creating = true;
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let own_pub = x25519_dalek::PublicKey::from(&exchange_key);
            let wrapped =
                ambre_crypto::ecdh::wrap_document_key(&exchange_key, own_pub.as_bytes(), &doc_key);
            let mut enc = wrapped.nonce.to_vec();
            enc.extend_from_slice(&wrapped.ciphertext);
            let doc_key_enc = base64::engine::general_purpose::URL_SAFE.encode(&enc);

            let url = format!("{url_scheme}://{api_base}/api/documents");
            let body = http::CreateDocumentRequest {
                doc_type: doc_type_to_api_string(doc_type),
                name_enc: None,
                doc_key_enc,
                document_id: Some(document_id.to_string()),
            };
            match http::post_json_auth::<_, http::CreateDocumentResponse>(&url, &body, auth)
                .await
            {
                Ok(_) => {
                    let _ = tx.send(SyncEvent::DocumentMaterialized { document_id });
                }
                Err(e) => {
                    let _ = tx.send(SyncEvent::DocumentCreateFailed {
                        document_id,
                        message: e,
                    });
                }
            }
        });
    }

    fn handle_send_snapshot(&mut self, document_id: DocumentId, payload: Vec<u8>) {
        let Some(signing_key) = self.signing_key.clone() else {
            tracing::warn!("Sync: SendSnapshot before session keys ready, dropping");
            return;
        };
        let Some(pub_key) = self.author_pub_key() else {
            return;
        };
        let Some(doc_state) = self.documents.get(&document_id) else {
            tracing::warn!("Sync: SendSnapshot for unknown doc {document_id:?}");
            return;
        };
        let public_data = SnapshotPublicData {
            document_id,
            pub_key,
            snapshot_id: doc_state.current_snapshot_id,
            parent_snapshot_proof: Vec::new(),
            parent_snapshot_update_clocks: Vec::new(),
        };
        match pipeline::seal_snapshot(&payload, public_data, &doc_state.doc_key, &signing_key) {
            Ok(snapshot) => tracing::info!(
                "Sync: SendSnapshot {document_id:?} sealed ({} bytes ciphertext), S3 upload pending",
                snapshot.ciphertext.0.len()
            ),
            Err(e) => tracing::error!("Sync: SendSnapshot seal failed for {document_id:?}: {e}"),
        }
    }

    fn handle_send_ephemeral(&mut self, document_id: DocumentId, payload: Vec<u8>) {
        let Some(signing_key) = self.signing_key.clone() else {
            tracing::warn!("Sync: SendEphemeral before session keys ready, dropping");
            return;
        };
        let Some(pub_key) = self.author_pub_key() else {
            return;
        };
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            tracing::warn!("Sync: SendEphemeral for unknown doc {document_id:?}");
            return;
        };
        doc_state.ephemeral_counter += 1;
        let public_data = EphemeralPublicData {
            document_id,
            pub_key,
            counter: Counter(doc_state.ephemeral_counter),
        };
        match pipeline::seal_ephemeral(&payload, public_data, &doc_state.doc_key, &signing_key) {
            Ok(ephemeral) => self.ws_send(&WsClientMsg::Ephemeral(ephemeral)),
            Err(e) => tracing::error!("Sync: SendEphemeral seal failed for {document_id:?}: {e}"),
        }
    }

    // -- Document CRUD (HTTP REST) ---------------------------------------------

    fn handle_list_documents(&self, id: RequestId) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let auth = self.auth_context();
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("{url_scheme}://{api_base}/api/documents");
                match http::get_json_auth::<http::DocumentListResponse>(&url, auth).await {
                    Ok(resp) => {
                        let docs: Vec<DocumentSummary> = resp
                            .documents
                            .into_iter()
                            .map(|e| DocumentSummary {
                                document_id: DocumentId::parse_str(&e.id)
                                    .unwrap_or_else(|_| DocumentId::new()),
                                doc_type: match e.doc_type.as_str() {
                                    "prose" => DocType::Prose,
                                    "toile" => DocType::Toile,
                                    "murmure" => DocType::Murmure,
                                    _ => DocType::Trame,
                                },
                                name: e.name_enc,
                                role: match e.role.as_str() {
                                    "editor" => Role::Editor,
                                    "viewer" => Role::Viewer,
                                    _ => Role::Owner,
                                },
                                updated_at: 0,
                            })
                            .collect();
                        let _ = tx.send(SyncEvent::DocumentList {
                            id,
                            documents: docs,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::HttpError { id, message: e });
                    }
                }
            });
        } else {
            let documents: Vec<DocumentSummary> = self
                .doc_metadata
                .iter()
                .map(|(&document_id, (doc_type, name))| DocumentSummary {
                    document_id,
                    doc_type: *doc_type,
                    name: Some(name.clone()),
                    role: Role::Owner,
                    updated_at: 0,
                })
                .collect();
            let count = documents.len();
            self.reply(id, RestEvent::DocumentList { documents });
            tracing::info!("Sync: ListDocuments ({count} docs)");
        }
    }

    fn handle_create_document(&mut self, id: RequestId, doc_type: DocType, name: String) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let auth = self.auth_context();
            let exchange_key = self.exchange_key.clone();
            wasm_bindgen_futures::spawn_local(async move {
                // E2EE: the server only ever stores ciphertext. Generate a fresh
                // DocumentKey, seal the name with it, and wrap the key for the
                // owner (ourselves) via X25519 so we can recover it on open.
                let Some(exchange_key) = exchange_key else {
                    let _ = tx.send(SyncEvent::HttpError {
                        id,
                        message: "CreateDocument: session locked (no exchange key)".into(),
                    });
                    return;
                };
                let doc_key = DocumentKey::generate();

                let name_enc = if name.is_empty() {
                    None
                } else {
                    let (nonce, ct) =
                        ambre_crypto::aead::seal(name.as_bytes(), b"ambre-doc-name", &doc_key);
                    let mut buf = nonce.to_vec();
                    buf.extend_from_slice(&ct);
                    Some(base64::engine::general_purpose::URL_SAFE.encode(&buf))
                };

                let own_pub = x25519_dalek::PublicKey::from(&exchange_key);
                let wrapped = ambre_crypto::ecdh::wrap_document_key(
                    &exchange_key,
                    own_pub.as_bytes(),
                    &doc_key,
                );
                let mut enc = wrapped.nonce.to_vec();
                enc.extend_from_slice(&wrapped.ciphertext);
                let doc_key_enc = base64::engine::general_purpose::URL_SAFE.encode(&enc);

                let url = format!("{url_scheme}://{api_base}/api/documents");
                let body = http::CreateDocumentRequest {
                    doc_type: doc_type_to_api_string(doc_type),
                    name_enc,
                    doc_key_enc,
                    document_id: None,
                };
                match http::post_json_auth::<_, http::CreateDocumentResponse>(&url, &body, auth)
                    .await
                {
                    Ok(resp) => {
                        let document_id = DocumentId::parse_str(&resp.document_id)
                            .unwrap_or_else(|_| DocumentId::new());
                        let _ = tx.send(SyncEvent::DocumentCreated {
                            id,
                            document_id,
                            doc_type,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::HttpError { id, message: e });
                    }
                }
            });
        } else {
            tracing::info!("Sync: CreateDocument {doc_type:?} '{name}'");
            let document_id = DocumentId::new();
            self.doc_metadata.insert(document_id, (doc_type, name));
            self.reply(
                id,
                RestEvent::DocumentCreated {
                    document_id,
                    doc_type,
                },
            );
        }
    }

    fn handle_delete_document(&mut self, id: RequestId, document_id: DocumentId) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let auth = self.auth_context();
            let doc_id = document_id;
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("{url_scheme}://{api_base}/api/documents/{doc_id}");
                match http::delete_auth(&url, auth).await {
                    Ok(()) => {
                        let _ = tx.send(SyncEvent::DocumentDeleted {
                            id,
                            document_id: doc_id,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::HttpError { id, message: e });
                    }
                }
            });
        } else {
            tracing::info!("Sync: DeleteDocument {document_id:?}");
            self.doc_metadata.remove(&document_id);
            self.reply(id, RestEvent::DocumentDeleted { document_id });
        }
    }

    fn handle_rename_document(&mut self, id: RequestId, document_id: DocumentId, name: String) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let url_scheme = self.url_scheme;
            let auth = self.auth_context();
            let doc_id = document_id;
            let doc_key = self.documents.get(&document_id).map(|ds| ds.doc_key.clone());
            wasm_bindgen_futures::spawn_local(async move {
                // E2EE: the new name is sealed with the document key before it
                // leaves the client. The server only ever stores ciphertext.
                let Some(doc_key) = doc_key else {
                    let _ = tx.send(SyncEvent::HttpError {
                        id,
                        message: "RenameDocument: document not open (no key)".into(),
                    });
                    return;
                };
                let (nonce, ct) = ambre_crypto::aead::seal(name.as_bytes(), b"ambre-doc-name", &doc_key);
                let mut buf = nonce.to_vec();
                buf.extend_from_slice(&ct);
                let name_enc = base64::engine::general_purpose::URL_SAFE.encode(&buf);

                // PATCH /api/documents/{id} (matches the server route).
                let url = format!("{url_scheme}://{api_base}/api/documents/{doc_id}");
                let body = http::RenameDocumentRequest { name_enc };
                match http::patch_json_auth(&url, &body, auth).await {
                    Ok(()) => {
                        let _ = tx.send(SyncEvent::DocumentRenamed {
                            id,
                            document_id: doc_id,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::HttpError { id, message: e });
                    }
                }
            });
        } else {
            tracing::info!("Sync: RenameDocument {document_id:?} '{name}'");
            if let Some((_doc_type, old_name)) = self.doc_metadata.get_mut(&document_id) {
                *old_name = name;
            }
            self.reply(id, RestEvent::DocumentRenamed { document_id });
        }
    }

    // -- Sharing (HTTP REST + crypto) ------------------------------------------

    /// Invites a user to a document:
    /// 1. Searches for the invitee by email via `GET /api/users/search`.
    /// 2. Wraps the document key for the invitee using X25519 ECDH.
    /// 3. POSTs the wrapped key to `/api/documents/{id}/share`.
    ///
    /// On the WASM path, the async HTTP work is spawned via `spawn_local` and
    /// the result flows back as [`SyncEvent::InviteSent`].
    fn handle_invite_to_document(
        &self,
        id: RequestId,
        document_id: DocumentId,
        invitee_email: String,
        role: Role,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let exchange_key = self.exchange_key.clone();
        let doc_key = self
            .documents
            .get(&document_id)
            .map(|ds| ds.doc_key.clone());

        let Some(tx) = event_tx else {
            self.reply(
                id,
                RestEvent::InviteSent {
                    document_id,
                    user_id: UserId::new(),
                    display_name: invitee_email.clone(),
                },
            );
            return;
        };
        let Some(exchange_key) = exchange_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                request: "InviteToDocument".into(),
                code: 401,
                message: "Not authenticated".into(),
            });
            return;
        };
        let Some(doc_key) = doc_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                request: "InviteToDocument".into(),
                code: 404,
                message: "Document not open".into(),
            });
            return;
        };

        wasm_bindgen_futures::spawn_local(async move {
            let search_url = format!(
                "{url_scheme}://{api_base}/api/users/search?query={}",
                invitee_email.replace('@', "%40")
            );
            let users = match http::get_json_auth::<http::UserSearchResult>(&search_url, auth.clone()).await {
                Ok(resp) => resp.users,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        request: "InviteToDocument".into(),
                        code: 500,
                        message: format!("User search failed: {e}"),
                    });
                    return;
                }
            };
            let invitee = match users.iter().find(|u| u.email == invitee_email) {
                Some(u) => u,
                None => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        request: "InviteToDocument".into(),
                        code: 404,
                        message: "User not found".into(),
                    });
                    return;
                }
            };
            let peer_pub_bytes: [u8; 32] = match hex::decode(&invitee.exchange_pubkey) {
                Ok(b) => match b.try_into() {
                    Ok(arr) => arr,
                    Err(_) => {
                        let _ = logic_tx.send(LogicEvent::ApiError {
                            request: "InviteToDocument".into(),
                            code: 500,
                            message: "Invalid exchange pubkey".into(),
                        });
                        return;
                    }
                },
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        request: "InviteToDocument".into(),
                        code: 500,
                        message: format!("Hex decode pubkey: {e}"),
                    });
                    return;
                }
            };

            let wrapped =
                ambre_crypto::ecdh::wrap_document_key(&exchange_key, &peer_pub_bytes, &doc_key);
            let mut enc = wrapped.nonce.to_vec();
            enc.extend_from_slice(&wrapped.ciphertext);
            let doc_key_enc = base64::engine::general_purpose::URL_SAFE.encode(&enc);

            let invite_url = format!("{url_scheme}://{api_base}/api/documents/{document_id}/share");
            let body = http::InviteRequest {
                invitee_email: invitee_email.clone(),
                role: role.to_string(),
                doc_key_enc,
            };
            match http::post_json_auth::<_, http::InviteResponse>(&invite_url, &body, auth)
                .await
            {
                Ok(resp) => {
                    let user_id =
                        UserId::parse_str(&resp.user_id).unwrap_or_else(|_| UserId::new());
                    let _ = tx.send(SyncEvent::InviteSent {
                        id,
                        document_id,
                        user_id,
                        display_name: resp.display_name,
                    });
                }
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        request: "InviteToDocument".into(),
                        code: 500,
                        message: e,
                    });
                }
            }
        });
    }

    /// Revokes a member from a document and rotates the document key:
    /// 1. DELETEs the share at `/api/documents/{id}/share/{uid}`.
    /// 2. Generates a fresh [`DocumentKey`] via CSPRNG.
    /// 3. GETs the remaining members from the server.
    /// 4. Wraps the new key for each remaining member via X25519 ECDH.
    /// 5. POSTs the wrapped keys to `/api/documents/{id}/rotate-key` (the
    ///    server atomically replaces the stored `doc_key_enc` entries and
    ///    broadcasts `WsServerMsg::KeyRotationRequired` to every client).
    /// 6. Emits [`SyncEvent::RevocationComplete`], which causes
    ///    [`handle_sync_event`] to swap the local key and push the old key
    ///    into the [`SyncDocumentState::old_keys`] LRU cache.
    fn handle_revoke_from_document(&self, id: RequestId, document_id: DocumentId, user_id: UserId) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let exchange_key = self.exchange_key.clone();
        let doc_state = self.documents.get(&document_id);
        let old_doc_key = doc_state.map(|ds| ds.doc_key.clone());

        let Some(tx) = event_tx else {
            self.reply(id, RestEvent::RevocationComplete { document_id });
            return;
        };
        let Some(exchange_key) = exchange_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                request: "RevokeFromDocument".into(),
                code: 401,
                message: "Not authenticated".into(),
            });
            return;
        };

        wasm_bindgen_futures::spawn_local(async move {
            // 1. DELETE /api/documents/{id}/share/{uid}
            let revoke_url =
                format!("{url_scheme}://{api_base}/api/documents/{document_id}/share/{user_id}");
            if let Err(e) = http::delete_auth(&revoke_url, auth.clone()).await {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    request: "RevokeFromDocument".into(),
                    code: 500,
                    message: format!("Revoke failed: {e}"),
                });
                return;
            }

            // 2. Generate new DocumentKey
            let new_doc_key = DocumentKey::generate();

            // 3. GET remaining members
            let members_url = format!("{url_scheme}://{api_base}/api/documents/{document_id}/members");
            let members = match http::get_json_auth::<http::DocumentMembersResponse>(&members_url, auth.clone()).await
            {
                Ok(resp) => resp.members,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        request: "RevokeFromDocument".into(),
                        code: 500,
                        message: format!("Get members failed: {e}"),
                    });
                    return;
                }
            };

            // 4. Wrap new key for each remaining member
            let mut keys = Vec::with_capacity(members.len());
            for member in &members {
                let peer_pub_bytes: [u8; 32] = match hex::decode(&member.exchange_pub_key) {
                    Ok(b) => match b.try_into() {
                        Ok(arr) => arr,
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                };
                let wrapped = ambre_crypto::ecdh::wrap_document_key(
                    &exchange_key,
                    &peer_pub_bytes,
                    &new_doc_key,
                );
                let mut enc = wrapped.nonce.to_vec();
                enc.extend_from_slice(&wrapped.ciphertext);
                let doc_key_enc = base64::engine::general_purpose::URL_SAFE.encode(&enc);
                keys.push(http::RotateKeyEntry {
                    user_id: member.user_id.clone(),
                    doc_key_enc,
                });
            }

            // 5. POST /api/documents/{id}/rotate-key
            let rotate_url = format!("{url_scheme}://{api_base}/api/documents/{document_id}/rotate-key");
            let body = http::RotateKeyRequest { keys };
            if let Err(e) =
                http::post_json_auth::<_, serde_json::Value>(&rotate_url, &body, auth).await
            {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    request: "RevokeFromDocument".into(),
                    code: 500,
                    message: format!("Rotate key failed: {e}"),
                });
                return;
            }

            // 6. Send back new key for swap
            let _ = tx.send(SyncEvent::RevocationComplete {
                id,
                document_id,
                new_doc_key,
            });
            // old_doc_key is dropped (zeroized) here — old_keys cache is
            // populated by handle_sync_event when it swaps the keys.
            let _ = old_doc_key;
        });
    }

    /// Fetches the member list for a document via `GET /api/documents/{id}/members`,
    /// maps the server response into [`MemberInfo`] values (plaintext names,
    /// decoded roles), and replies with [`RestEvent::DocumentMembers`].
    fn handle_list_document_members(&self, id: RequestId, document_id: DocumentId) {
        let event_tx = self.event_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("{url_scheme}://{api_base}/api/documents/{document_id}/members");
                match http::get_json_auth::<http::DocumentMembersResponse>(&url, auth).await {
                    Ok(resp) => {
                        let members: Vec<MemberInfo> = resp
                            .members
                            .into_iter()
                            .map(|m| MemberInfo {
                                user_id: UserId::parse_str(&m.user_id)
                                    .unwrap_or_else(|_| UserId::new()),
                                display_name: m.display_name,
                                email: m.email,
                                role: m.role.parse().unwrap_or(Role::Viewer),
                            })
                            .collect();
                        let _ = tx.send(SyncEvent::DocumentMembersReady {
                            id,
                            document_id,
                            members,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::HttpError { id, message: e });
                    }
                }
            });
        } else {
            self.reply(
                id,
                RestEvent::DocumentMembers {
                    document_id,
                    members: Vec::new(),
                },
            );
        }
    }

    /// Updates a member's role via `PATCH /api/documents/{id}/members/{uid}`.
    /// Sends the new role as a JSON body and replies with
    /// [`RestEvent::MemberRoleUpdated`] on success.
    fn handle_update_document_member_role(
        &self,
        id: RequestId,
        document_id: DocumentId,
        user_id: UserId,
        role: Role,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let url =
                    format!("{url_scheme}://{api_base}/api/documents/{document_id}/members/{user_id}");
                let body = serde_json::json!({ "role": role.to_string() });
                match http::patch_json_auth(&url, &body, auth).await {
                    Ok(()) => {
                        let _ = tx.send(SyncEvent::MemberRoleUpdatedEvent {
                            id,
                            document_id,
                            user_id,
                            role,
                        });
                    }
                    Err(e) => {
                        let _ = logic_tx.send(LogicEvent::ApiError {
                            request: "UpdateDocumentMemberRole".into(),
                            code: 500,
                            message: e,
                        });
                    }
                }
            });
        } else {
            tracing::info!("Sync: UpdateDocumentMemberRole {document_id:?} {user_id:?} {role:?}");
            self.reply(
                id,
                RestEvent::MemberRoleUpdated {
                    document_id,
                    user_id,
                    role,
                },
            );
        }
    }

    /// Searches users via `GET /api/users/search?query=...`, maps the server
    /// response into [`UserInfo`] values, and replies with
    /// [`RestEvent::UserSearchResults`]. The query is percent-encoded for the
    /// `@` character before being placed in the URL.
    fn handle_search_users(&self, id: RequestId, query: String) {
        let event_tx = self.event_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!(
                    "{url_scheme}://{api_base}/api/users/search?query={}",
                    query.replace('@', "%40")
                );
                match http::get_json_auth::<http::UserSearchResult>(&url, auth).await {
                    Ok(resp) => {
                        let users: Vec<UserInfo> = resp
                            .users
                            .into_iter()
                            .map(|u| UserInfo {
                                user_id: UserId::parse_str(&u.id).unwrap_or_else(|_| UserId::new()),
                                display_name: u.display_name,
                                email: u.email,
                            })
                            .collect();
                        let _ = tx.send(SyncEvent::UserSearchReady { id, users });
                    }
                    Err(e) => {
                        let _ = tx.send(SyncEvent::HttpError { id, message: e });
                    }
                }
            });
        } else {
            tracing::info!("Sync: SearchUsers '{query}'");
            self.reply(id, RestEvent::UserSearchResults { users: Vec::new() });
        }
    }

    // -- Media (upload / download via presigned S3 URLs) -----------------------

    /// Uploads a media blob to S3 via the presigned-URL flow:
    /// 1. Hashes the plaintext data (BLAKE2b-256 -> content_hash).
    /// 2. Encrypts the data with the document key (AEAD, "ambre-media" aad).
    /// 3. Requests a presigned S3 PUT URL from the server.
    /// 4. PUTs the encrypted blob (nonce || ciphertext) directly to S3.
    /// 5. Confirms the upload via `POST /api/documents/{id}/media`.
    /// 6. Emits [`SyncEvent::MediaUploaded`].
    fn handle_upload_media(
        &self,
        id: RequestId,
        document_id: DocumentId,
        data: Vec<u8>,
        mime_type: String,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let doc_key = self
            .documents
            .get(&document_id)
            .map(|ds| ds.doc_key.clone());

        let Some(tx) = event_tx else {
            let content_hash = {
                let mut hasher = blake2::Blake2b512::new();
                hasher.update(&data);
                hex::encode(&hasher.finalize()[..32])
            };
            self.reply(
                id,
                RestEvent::MediaUploaded {
                    document_id,
                    content_hash,
                },
            );
            return;
        };
        let Some(doc_key) = doc_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                request: "UploadMedia".into(),
                code: 404,
                message: "Document not open".into(),
            });
            return;
        };

        wasm_bindgen_futures::spawn_local(async move {
            let (nonce, ciphertext) = ambre_crypto::aead::seal(&data, b"ambre-media", &doc_key);
            let encrypted_size = (ambre_crypto::aead::NONCE_LEN + ciphertext.len()) as u64;

            let content_hash = {
                let mut hasher = blake2::Blake2b512::new();
                hasher.update(&data);
                hex::encode(&hasher.finalize()[..32])
            };

            let upload_url_req =
                format!("{url_scheme}://{api_base}/api/documents/{document_id}/media/upload-url");
            let body = http::UploadUrlRequest {
                content_hash: content_hash.clone(),
                mime_type: mime_type.clone(),
                size_bytes: encrypted_size,
            };
            let upload_url = match http::post_json_auth::<_, http::UploadUrlResponse>(
                &upload_url_req,
                &body,
                auth.clone(),
            )
            .await
            {
                Ok(resp) => resp.upload_url,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        request: "UploadMedia".into(),
                        code: 500,
                        message: format!("upload-url: {e}"),
                    });
                    return;
                }
            };

            let mut blob = nonce.to_vec();
            blob.extend_from_slice(&ciphertext);
            // S3 presigned PUT goes straight to object storage, not our API:
            // no session, no CSRF header.
            if let Err(e) =
                http::put_bytes(&upload_url, &blob, "application/octet-stream").await
            {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    request: "UploadMedia".into(),
                    code: 500,
                    message: format!("S3 PUT: {e}"),
                });
                return;
            }

            let confirm_url = format!("{url_scheme}://{api_base}/api/documents/{document_id}/media");
            let confirm_body = http::ConfirmUploadRequest {
                content_hash: content_hash.clone(),
                mime_type,
                size_bytes: encrypted_size as i64,
            };
            if let Err(e) =
                http::post_json_auth::<_, serde_json::Value>(&confirm_url, &confirm_body, auth)
                    .await
            {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    request: "UploadMedia".into(),
                    code: 500,
                    message: format!("confirm: {e}"),
                });
                return;
            }

            let _ = tx.send(SyncEvent::MediaUploaded {
                id,
                document_id,
                content_hash,
            });
        });
    }

    /// Downloads a media blob from S3 via the presigned-URL flow:
    /// 1. Requests a presigned S3 GET URL from the server by content_hash.
    /// 2. GETs the encrypted blob directly from S3.
    /// 3. Splits the nonce (first [`ambre_crypto::aead::NONCE_LEN`] bytes) from
    ///    the ciphertext and decrypts with the document key.
    /// 4. Emits [`SyncEvent::MediaDownloaded`] with the plaintext data.
    fn handle_download_media(&self, id: RequestId, document_id: DocumentId, content_hash: String) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let auth = self.auth_context();
        let doc_key = self
            .documents
            .get(&document_id)
            .map(|ds| ds.doc_key.clone());

        let Some(tx) = event_tx else {
            self.reply(
                id,
                RestEvent::MediaDownloaded {
                    document_id,
                    content_hash,
                    data: Vec::new(),
                },
            );
            return;
        };
        let Some(doc_key) = doc_key else {
            let _ = logic_tx.send(LogicEvent::ApiError {
                request: "DownloadMedia".into(),
                code: 404,
                message: "Document not open".into(),
            });
            return;
        };

        wasm_bindgen_futures::spawn_local(async move {
            let download_url_req =
                format!("{url_scheme}://{api_base}/api/documents/{document_id}/media/download-url");
            let body = http::DownloadUrlRequest {
                content_hash: content_hash.clone(),
            };
            let download_url = match http::post_json_auth::<_, http::DownloadUrlResponse>(
                &download_url_req,
                &body,
                auth,
            )
            .await
            {
                Ok(resp) => resp.download_url,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        request: "DownloadMedia".into(),
                        code: 500,
                        message: format!("download-url: {e}"),
                    });
                    return;
                }
            };

            let blob = match http::fetch_bytes(&download_url).await {
                Ok(b) => b,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        request: "DownloadMedia".into(),
                        code: 500,
                        message: format!("S3 GET: {e}"),
                    });
                    return;
                }
            };

            if blob.len() < ambre_crypto::aead::NONCE_LEN {
                let _ = logic_tx.send(LogicEvent::ApiError {
                    request: "DownloadMedia".into(),
                    code: 500,
                    message: "Blob too short".into(),
                });
                return;
            }
            let (nonce_bytes, ciphertext) = blob.split_at(ambre_crypto::aead::NONCE_LEN);
            let nonce: [u8; ambre_crypto::aead::NONCE_LEN] =
                nonce_bytes.try_into().expect("nonce len");

            let data = match ambre_crypto::aead::open(&nonce, ciphertext, b"ambre-media", &doc_key)
            {
                Ok(d) => d,
                Err(e) => {
                    let _ = logic_tx.send(LogicEvent::ApiError {
                        request: "DownloadMedia".into(),
                        code: 500,
                        message: format!("Decrypt: {e}"),
                    });
                    return;
                }
            };

            let _ = tx.send(SyncEvent::MediaDownloaded {
                id,
                document_id,
                content_hash,
                data,
            });
        });
    }

    // -- Channels --------------------------------------------------------------

    fn handle_open_channel(&mut self, channel_id: ChannelId) {
        if self.ws_connected() {
            self.ws_send(&WsClientMsg::SubscribeChannel { channel_id });
        }
        tracing::info!("Sync: OpenChannel {channel_id:?}");
    }

    fn handle_close_channel(&mut self, channel_id: ChannelId) {
        if self.ws_connected() {
            self.ws_send(&WsClientMsg::UnsubscribeChannel { channel_id });
        }
        tracing::info!("Sync: CloseChannel {channel_id:?}");
    }

    fn handle_send_channel_update(&self, channel_id: ChannelId, payload: Vec<u8>) {
        tracing::debug!(
            "Sync: SendChannelUpdate {channel_id:?} len={}",
            payload.len()
        );
    }

    fn handle_send_channel_ephemeral(&self, channel_id: ChannelId, payload: Vec<u8>) {
        tracing::debug!(
            "Sync: SendChannelEphemeral {channel_id:?} len={}",
            payload.len()
        );
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

// -----------------------------------------------------------------------------
// Tests — causal quarantine (host, no network / no WASM).
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A `SyncState` with one opened document, a controlled `doc_key` and a known
    /// signing key, plus the drainable Logic receiver and the document's known
    /// snapshot id.
    fn setup() -> (
        SyncState,
        flume::Receiver<LogicEvent>,
        DocumentId,
        DocumentKey,
        SigningKey,
        SnapshotId,
    ) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        let document_id = DocumentId::new();
        let doc_key = DocumentKey::generate();
        let signing_key = MasterSeed::generate().derive_signing_key();
        let doc_state = SyncDocumentState::new(document_id, doc_key.clone(), DocType::Trame, true);
        let known_snapshot = doc_state.current_snapshot_id;
        state.documents.insert(document_id, doc_state);
        (state, rx, document_id, doc_key, signing_key, known_snapshot)
    }

    fn make_update(
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        clock: u64,
        payload: &[u8],
        doc_key: &DocumentKey,
        signing_key: &SigningKey,
    ) -> Update {
        let pub_key = PublicKey::from_array(signing_key.verifying_key().to_bytes());
        let public_data = UpdatePublicData {
            document_id,
            pub_key,
            snapshot_id,
            clock: Clock(clock),
        };
        pipeline::seal_update(payload, public_data, doc_key, signing_key).expect("seal")
    }

    fn drain(rx: &flume::Receiver<LogicEvent>) -> Vec<LogicEvent> {
        rx.try_iter().collect()
    }

    #[test]
    fn update_with_known_snapshot_delivered_immediately() {
        let (mut state, rx, doc, key, sk, snap) = setup();
        let update = make_update(doc, snap, 1, b"hello", &key, &sk);

        state.receive_update(Seq(1), update);

        let events = drain(&rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            LogicEvent::RemoteUpdate {
                document_id,
                payload,
            } => {
                assert_eq!(*document_id, doc);
                assert_eq!(payload, b"hello");
            }
            other => panic!("expected RemoteUpdate, got {other:?}"),
        }
        assert_eq!(state.documents[&doc].quarantine_len(), 0);
    }

    #[test]
    fn update_with_unknown_snapshot_quarantined() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let unknown = SnapshotId::new();
        let update = make_update(doc, unknown, 1, b"later", &key, &sk);

        state.receive_update(Seq(1), update);

        assert!(drain(&rx).is_empty(), "nothing should be delivered yet");
        assert_eq!(state.documents[&doc].quarantine_len(), 1);
    }

    #[test]
    fn quarantine_released_in_clock_order() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let parent = SnapshotId::new();
        // Arrive out of clock order: clock 5 then clock 2.
        state.receive_update(Seq(1), make_update(doc, parent, 5, b"five", &key, &sk));
        state.receive_update(Seq(2), make_update(doc, parent, 2, b"two", &key, &sk));
        assert!(drain(&rx).is_empty());
        assert_eq!(state.documents[&doc].quarantine_len(), 2);

        state.snapshot_known(doc, parent);

        let payloads: Vec<Vec<u8>> = drain(&rx)
            .into_iter()
            .map(|e| match e {
                LogicEvent::RemoteUpdate { payload, .. } => payload,
                other => panic!("expected RemoteUpdate, got {other:?}"),
            })
            .collect();
        assert_eq!(payloads, vec![b"two".to_vec(), b"five".to_vec()]);
        assert_eq!(state.documents[&doc].quarantine_len(), 0);
    }

    #[test]
    fn quarantine_overflow_drops_oldest_and_warns() {
        let (mut state, rx, doc, key, sk, _snap) = setup();
        let parent = SnapshotId::new();
        for clock in 1..=(QUARANTINE_LIMIT as u64) {
            state.receive_update(Seq(clock), make_update(doc, parent, clock, b"x", &key, &sk));
        }

        let warnings = drain(&rx)
            .into_iter()
            .filter(|e| matches!(e, LogicEvent::SyncWarning { code, .. } if *code == SyncWarningCode::QuarantineOverflow))
            .count();
        assert_eq!(warnings, 1, "exactly one overflow warning at the hard cap");
        assert_eq!(state.documents[&doc].quarantine_len(), QUARANTINE_LIMIT - 1);
    }
}
