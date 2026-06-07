// ambre-sync/src/lib.rs
// ============================================================================
// Sync thread entry point — runs in a dedicated wasm_thread, shares the heap
// with the UI Thread and the Logic thread.
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
// Transport (post-Phase-3): native Rust channels. The Sync thread blocks on
// `flume::Selector::wait` to consume `SyncCommand`s and emits `LogicEvent`s.
//
// ⚠️ Known limitation: blocking on flume inside a wasm Worker uses
// `Atomics.wait` under the hood, which prevents the Worker's JS event loop
// from delivering WebSocket / setTimeout callbacks. Today this is fine —
// the WS pipeline is still placeholder — but when the real WebSocket is
// wired (after Phase 4), the Sync thread must switch to a non-blocking
// loop (recv_async + JsFuture-driven WS reads, or yield via
// `wasm_bindgen_futures::spawn_local`). See
// specs/ambre-multithreaded-wasm-migration.md.
//
// WebSocket topology (when implemented):
//   ONE WebSocket per session, opened at Unlock, closed at Logout.
//   OpenDocument / CloseDocument -> Subscribe / Unsubscribe on the shared WS.
//   All documents are demultiplexed by document_id on incoming WsServerMsg.
// ============================================================================

use std::collections::{HashMap, HashSet};

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
use ambre_protocol::ws::{encode_client_msg, WsClientMsg, WsServerMsg};
#[cfg(target_arch = "wasm32")]
use ambre_protocol::ws::decode_server_msg;
use ed25519_dalek::SigningKey;
use x25519_dalek::StaticSecret;

use base64::Engine;
use hex::encode;

mod pipeline;
pub mod mock;
mod http;
pub mod mock_relay;

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
    WsDisconnected { code: u16, reason: String },
    WsError,
    SessionReady {
        id: RequestId,
        user_id: UserId,
        display_name: String,
        email: String,
        master_seed: MasterSeed,
        signing_key: SigningKey,
        exchange_key: StaticSecret,
    },
    AccountCreated {
        id: RequestId,
        recovery_words: String,
        master_seed: MasterSeed,
        signing_key: SigningKey,
        exchange_key: StaticSecret,
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
}

// -----------------------------------------------------------------------------
// Per-document sync session
// -----------------------------------------------------------------------------

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct SyncDocumentState {
    #[allow(dead_code)]
    document_id: DocumentId,
    doc_key: DocumentKey,
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
    fn new(document_id: DocumentId, doc_key: DocumentKey) -> Self {
        let current_snapshot_id = SnapshotId::new();
        Self {
            document_id,
            doc_key,
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

    pub struct SyncState {
    /// Outbound channel: Sync → Logic thread.
    logic_tx: flume::Sender<LogicEvent>,
    /// Base URL for REST API calls (e.g. `"api.ambre.app"`).
    #[allow(dead_code)]
    api_base: String,
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

    // -- Internal event channel (WASM: carries WS frames + HTTP results into   --
    // -- the non-blocking select loop) --
    event_tx: Option<flume::Sender<SyncEvent>>,

    // -- Per-document sync sessions --------------------------------------------
    documents: HashMap<DocumentId, SyncDocumentState>,

    // -- Document metadata (in-memory mirror for mock mode) --------------------
    doc_metadata: HashMap<DocumentId, (DocType, String)>,
}

impl SyncState {
    #[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
    pub fn new(logic_tx: flume::Sender<LogicEvent>, api_base: String) -> Self {
        Self {
            logic_tx,
            api_base,
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

    // -- Outbound: Sync → Logic ------------------------------------------------

    fn send_to_logic(&self, event: LogicEvent) {
        if let Err(flume::SendError(ev)) = self.logic_tx.send(event) {
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
            None => tracing::warn!("Sync: inbound message for unopened doc {document_id:?}, dropping"),
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
            tracing::warn!("Sync: ws_send called with no socket connected, dropping message");
            return;
        };
        if let Err(e) = ws.send_with_u8_array(&wire) {
            tracing::warn!("Sync: WebSocket send failed ({e:?}); reconnect will retry");
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
            SyncCommand::OpenDocument { document_id } => self.handle_open_document(document_id),
            SyncCommand::CloseDocument { document_id } => self.handle_close_document(document_id),
            SyncCommand::SendUpdate { document_id, payload } => {
                self.handle_send_update(document_id, payload);
            }
            SyncCommand::SendSnapshot { document_id, payload } => {
                self.handle_send_snapshot(document_id, payload);
            }
            SyncCommand::SendEphemeral { document_id, payload } => {
                self.handle_send_ephemeral(document_id, payload);
            }
            SyncCommand::Reconnect { document_id } => self.handle_reconnect(document_id),

            // -- Channel sync session (control) --------------------------------
            SyncCommand::OpenChannel { channel_id } => self.handle_open_channel(channel_id),
            SyncCommand::CloseChannel { channel_id } => self.handle_close_channel(channel_id),
            SyncCommand::SendChannelUpdate { channel_id, payload } => {
                self.handle_send_channel_update(channel_id, payload);
            }
            SyncCommand::SendChannelEphemeral { channel_id, payload } => {
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
            RestCommand::ChangePassphrase { current_passphrase, new_passphrase } => {
                self.handle_change_passphrase(current_passphrase, new_passphrase);
            }
            RestCommand::RecoverAccount { recovery_words, new_passphrase } => {
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
            RestCommand::InviteToDocument { document_id, invitee_email, role } => {
                self.handle_invite_to_document(id, document_id, invitee_email, role);
            }
            RestCommand::RevokeFromDocument { document_id, user_id } => {
                self.handle_revoke_from_document(id, document_id, user_id);
            }
            RestCommand::ListDocumentMembers { document_id } => {
                self.handle_list_document_members(id, document_id);
            }
            RestCommand::UpdateDocumentMemberRole { document_id, user_id, role } => {
                self.handle_update_document_member_role(id, document_id, user_id, role);
            }

            // -- Users ---------------------------------------------------------
            RestCommand::SearchUsers { query } => self.handle_search_users(id, query),

            // -- Media ---------------------------------------------------------
            RestCommand::UploadMedia { document_id, data, mime_type } => {
                tracing::info!("Sync: UploadMedia {document_id:?} {mime_type} len={}", data.len());
            }
            RestCommand::DownloadMedia { document_id, content_hash } => {
                tracing::info!("Sync: DownloadMedia {document_id:?} {content_hash}");
            }

            // -- Channels (CRUD & sharing) -------------------------------------
            RestCommand::ListChannels => tracing::info!("Sync: ListChannels"),
            RestCommand::CreateChannel { name } => tracing::info!("Sync: CreateChannel '{name}'"),
            RestCommand::DeleteChannel { channel_id } => {
                tracing::info!("Sync: DeleteChannel {channel_id:?}");
            }
            RestCommand::InviteToChannel { channel_id, invitee_email, role } => {
                tracing::info!("Sync: InviteToChannel {channel_id:?} {invitee_email} {role:?}");
            }
            RestCommand::RevokeFromChannel { channel_id, user_id } => {
                tracing::info!("Sync: RevokeFromChannel {channel_id:?} {user_id:?}");
            }
            RestCommand::ListChannelMembers { channel_id } => {
                tracing::info!("Sync: ListChannelMembers {channel_id:?}");
            }
            RestCommand::UpdateChannelMemberRole { channel_id, user_id, role } => {
                tracing::info!("Sync: UpdateChannelMemberRole {channel_id:?} {user_id:?} {role:?}");
            }
            RestCommand::UpdateReadCursor { channel_id, last_seq } => {
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
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            // WASM path: spawn an async HTTP task; the result flows back
            // through the SyncEvent channel into handle_sync_event.
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("https://{api_base}/api/keys/master");
                match http::get_json::<http::MasterKeyResponse>(&url).await {
                    Ok(resp) => {
                        let salt = match hex::decode(&resp.master_key_salt) {
                            Ok(s) => s,
                            Err(e) => {
                                let _ = logic_tx.send(LogicEvent::AuthError {
                                    message: format!("Invalid salt: {e}"),
                                });
                                return;
                            }
                        };
                        let master_key_enc = match hex::decode(&resp.master_key_enc) {
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
                        let user_key = match ambre_crypto::passphrase::derive_user_key(&passphrase, &salt) {
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

                        let auth_url = format!("https://{api_base}/api/auth/me");
                        match http::get_json::<http::AuthMeResponse>(&auth_url).await {
                            Ok(me) => {
                                let user_id = UserId::parse_str(&me.user_id).unwrap_or_else(|_| UserId::new());
                                let _ = tx.send(SyncEvent::SessionReady {
                                    id,
                                    user_id,
                                    display_name: me.display_name,
                                    email: me.email,
                                    master_seed,
                                    signing_key,
                                    exchange_key,
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
            self.reply(id, RestEvent::SessionReady {
                user_id: UserId::new(),
                display_name: "offline".into(),
                email: "offline@localhost".into(),
            });
        }
    }

    fn handle_setup_account(&mut self, id: RequestId, passphrase: Vec<u8>) {
        tracing::info!("Sync: SetupAccount (passphrase len={})", passphrase.len());

        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
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
                let salt_hex = hex::encode(salt);
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

                let url = format!("https://{api_base}/api/keys/master");
                let body = http::SetupKeyRequest {
                    master_key_enc: hex::encode(&master_key_with_nonce),
                    master_key_salt: salt_hex,
                    signing_pub_key,
                    exchange_pub_key,
                };
                match http::post_json::<_, serde_json::Value>(&url, &body).await {
                    Ok(_) => {
                        let _ = tx.send(SyncEvent::AccountCreated {
                            id,
                            recovery_words,
                            master_seed,
                            signing_key,
                            exchange_key,
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

    fn handle_change_passphrase(
        &mut self,
        current_passphrase: Vec<u8>,
        new_passphrase: Vec<u8>,
    ) {
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
        tracing::info!("Sync: logged out, all keys zeroized");
    }

    // -- WebSocket lifecycle ---------------------------------------------------

    /// Opens the session-level multiplexed WebSocket. Called after Unlock or
    /// SetupAccount. If the socket is already present (reconnection), it is
    /// replaced. All open documents are automatically re-subscribed.
    #[cfg(target_arch = "wasm32")]
    fn open_websocket(&mut self) {
        let url = format!("wss://{}/api/ws", self.api_base);
        match WebSocket::new(&url) {
            Ok(ws) => {
                ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
                self.wire_ws_callbacks(&ws);
                self.ws = Some(ws);
                for doc_id in self.documents.keys().copied().collect::<Vec<_>>() {
                    self.ws_send(&WsClientMsg::Subscribe { document_id: doc_id });
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
            SyncEvent::SessionReady { id, user_id, display_name, email, master_seed, signing_key, exchange_key } => {
                self.user_id = Some(user_id);
                self.master_seed = Some(master_seed);
                self.signing_key = Some(signing_key);
                self.exchange_key = Some(exchange_key);
                self.open_websocket();
                self.reply(id, RestEvent::SessionReady { user_id, display_name, email });
            }
            SyncEvent::AccountCreated { id, recovery_words, master_seed, signing_key, exchange_key } => {
                self.master_seed = Some(master_seed);
                self.signing_key = Some(signing_key);
                self.exchange_key = Some(exchange_key);
                self.open_websocket();
                self.reply(id, RestEvent::AccountCreated { recovery_words });
            }
            SyncEvent::DocumentList { id, documents } => {
                self.reply(id, RestEvent::DocumentList { documents });
            }
            SyncEvent::DocumentCreated { id, document_id, doc_type } => {
                self.reply(id, RestEvent::DocumentCreated { document_id, doc_type });
            }
            SyncEvent::DocumentDeleted { id, document_id } => {
                self.reply(id, RestEvent::DocumentDeleted { document_id });
            }
            SyncEvent::DocumentRenamed { id, document_id } => {
                self.reply(id, RestEvent::DocumentRenamed { document_id });
            }
            SyncEvent::OpenDocumentReady { document_id, doc_key, snapshot_payload, snapshot_id } => {
                self.documents.insert(document_id, SyncDocumentState::new(document_id, doc_key));
                let doc_state = self.documents.get_mut(&document_id).expect("just inserted");
                doc_state.current_snapshot_id = snapshot_id;
                doc_state.known_snapshots.insert(snapshot_id);
                if let Some(payload) = snapshot_payload {
                    self.send_to_logic(LogicEvent::RemoteSnapshot { document_id, payload });
                }
                if self.ws_connected() {
                    self.ws_send(&WsClientMsg::Subscribe { document_id });
                }
                self.send_to_logic(LogicEvent::ConnectionStatus {
                    document_id,
                    connected: self.ws_connected(),
                });
            }
            SyncEvent::HttpError { id: _id, message } => {
                self.send_to_logic(LogicEvent::ApiError {
                    request: "rest".into(),
                    code: 500,
                    message,
                });
            }
            SyncEvent::InviteSent { id, document_id, user_id, display_name } => {
                self.reply(id, RestEvent::InviteSent { document_id, user_id, display_name });
            }
            SyncEvent::RevocationComplete { id, document_id, new_doc_key } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.rotate_key(new_doc_key);
                }
                self.reply(id, RestEvent::RevocationComplete { document_id });
            }
            SyncEvent::DocumentMembersReady { id, document_id, members } => {
                self.reply(id, RestEvent::DocumentMembers { document_id, members });
            }
            SyncEvent::UserSearchReady { id, users } => {
                self.reply(id, RestEvent::UserSearchResults { users });
            }
            SyncEvent::MemberRoleUpdatedEvent { id, document_id, user_id, role } => {
                self.reply(id, RestEvent::MemberRoleUpdated { document_id, user_id, role });
            }
            SyncEvent::KeyRotated { document_id, new_doc_key, revoked_user_id } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    doc_state.rotate_key(new_doc_key);
                }
                self.send_to_logic(LogicEvent::MemberRevoked { document_id, revoked_user_id });
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

            WsServerMsg::Update { seq, update } => self.receive_update(seq, update),

            WsServerMsg::Ephemeral(eph) => {
                let doc_id = eph.public_data.document_id;
                let Some(doc_state) = self.documents.get_mut(&doc_id) else {
                    return;
                };

                // Anti-replay: reject ephemerals with a non-monotonic counter
                // for the same author.
                let author_pk = *eph.public_data.pub_key.as_array();
                let last_counter = doc_state.ephemeral_remote_counters.get(&author_pk).copied().unwrap_or(0);
                if eph.public_data.counter.0 <= last_counter {
                    tracing::debug!("Sync: dropping replayed ephemeral from {:?} (counter {})", author_pk, eph.public_data.counter.0);
                    return;
                }
                doc_state.ephemeral_remote_counters.insert(author_pk, eph.public_data.counter.0);

                let opened =
                    doc_state.open_ephemeral_fallback(&eph);
                self.deliver_remote(doc_id, Some(opened), |payload| LogicEvent::RemoteEphemeral {
                    document_id: doc_id,
                    payload,
                });
            }

            WsServerMsg::MissingUpdates { document_id: _, updates } => {
                for seq_update in updates {
                    self.receive_update(seq_update.seq, seq_update.update);
                }
            }

            WsServerMsg::SnapshotRequired { document_id } => {
                self.send_to_logic(LogicEvent::SnapshotNeeded { document_id });
            }

            WsServerMsg::SnapshotAvailable { document_id, snapshot_id } => {
                tracing::info!("Sync: SnapshotAvailable {snapshot_id:?} for {document_id:?}");
                self.snapshot_known(document_id, snapshot_id);
            }

            WsServerMsg::KeyRotationRequired { document_id, revoked_user_id } => {
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
                let exchange_key = self.exchange_key.clone();
                let doc_id = document_id;

                if let Some(tx) = event_tx {
                    let Some(ek) = exchange_key else { return };
                    wasm_bindgen_futures::spawn_local(async move {
                        let members_url =
                            format!("https://{api_base}/api/documents/{doc_id}/members");
                        let members =
                            match http::get_json::<http::MembersResponse>(&members_url).await {
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
                            let encrypted = hex::decode(&entry.doc_key_enc)
                                .map_err(|e| format!("hex decode: {e}"))?;
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
                            ambre_crypto::ecdh::unwrap_document_key(
                                &ek, &peer_pub, &wrapped,
                            )
                            .map_err(|e| format!("ECDH unwrap: {e}"))
                        })() {
                            Ok(k) => k,
                            Err(e) => {
                                tracing::error!(
                                    "Sync: KeyRotationRequired unwrap failed: {e}"
                                );
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

            WsServerMsg::ChannelUpdate { channel_id, seq: _, update } => {
                let _ = (channel_id, update);
            }

            WsServerMsg::ChannelEphemeral { channel_id, ephemeral } => {
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
            self.deliver_remote(document_id, Some(opened), |payload| LogicEvent::RemoteUpdate {
                document_id,
                payload,
            });
            return;
        }

        doc_state.quarantine.entry(snapshot_id).or_default().push(update);
        self.enforce_quarantine_bounds(document_id);
    }

    /// Keep a document's quarantine within [`QUARANTINE_LIMIT`]: request the
    /// missing history at the soft threshold, and drop the oldest update (by
    /// `clock`) past the hard cap, raising a `SyncWarning`.
    fn enforce_quarantine_bounds(&mut self, document_id: DocumentId) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else { return };

        if doc_state.quarantine_len() >= QUARANTINE_ALERT {
            let after_seq = doc_state.last_seq;
            if self.ws_connected() {
                self.ws_send(&WsClientMsg::RequestMissing { document_id, after_seq });
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
            let Some(doc_state) = self.documents.get_mut(&document_id) else { return };
            doc_state.quarantine.remove(&snapshot_id).unwrap_or_default()
        };
        pending.sort_by_key(|u| u.public_data.clock.0);

        let results: Vec<Result<Vec<u8>, pipeline::PipelineError>> = {
            let Some(doc_state) = self.documents.get(&document_id) else { return };
            pending.iter().map(|u| doc_state.open_update_fallback(u)).collect()
        };
        for opened in results {
            self.deliver_remote(document_id, Some(opened), |payload| LogicEvent::RemoteUpdate {
                document_id,
                payload,
            });
        }
    }

    // -- Document lifecycle ----------------------------------------------------

    fn handle_open_document(&mut self, document_id: DocumentId) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let doc_id = document_id;
            let exchange_key = self.exchange_key.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let members_url = format!("https://{api_base}/api/documents/{doc_id}/members");
                let doc_key = match http::get_json::<http::MembersResponse>(&members_url).await {
                    Ok(resp) => {
                        match &exchange_key {
                            Some(ek) => Self::unwrap_document_key(ek, &resp.members)
                                .unwrap_or_else(|_| DocumentKey::generate()),
                            None => DocumentKey::generate(),
                        }
                    }
                    Err(_) => DocumentKey::generate(),
                };
                let snapshot_url = format!("https://{api_base}/api/documents/{doc_id}/snapshot/latest");
                let snapshot_payload = match http::get_json::<http::SnapshotPresignedResponse>(&snapshot_url).await {
                    Ok(presigned) => http::fetch_bytes(&presigned.url).await.ok(),
                    Err(_) => None,
                };
                let snapshot_id = SnapshotId::new();
                let _ = tx.send(SyncEvent::OpenDocumentReady {
                    document_id: doc_id,
                    doc_key,
                    snapshot_payload,
                    snapshot_id,
                });
            });
        } else {
            let doc_key = DocumentKey::generate();
            self.documents
                .insert(document_id, SyncDocumentState::new(document_id, doc_key));
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

        let encrypted = hex::decode(&entry.doc_key_enc)
            .map_err(|e| format!("hex decode doc_key_enc: {e}"))?;
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
            let last_seq =
                self.documents.get(&document_id).map(|d| d.last_seq).unwrap_or(Seq(0));
            self.ws_send(&WsClientMsg::Subscribe { document_id });
            self.ws_send(&WsClientMsg::RequestMissing { document_id, after_seq: last_seq });
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
            tracing::warn!("Sync: SendUpdate before session keys ready, dropping");
            return;
        };
        let Some(pub_key) = self.author_pub_key() else { return };
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            tracing::warn!("Sync: SendUpdate for unknown doc {document_id:?}");
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
            Ok(update) => self.ws_send(&WsClientMsg::Update(update)),
            Err(e) => tracing::error!("Sync: SendUpdate seal failed for {document_id:?}: {e}"),
        }
    }

    fn handle_send_snapshot(&mut self, document_id: DocumentId, payload: Vec<u8>) {
        let Some(signing_key) = self.signing_key.clone() else {
            tracing::warn!("Sync: SendSnapshot before session keys ready, dropping");
            return;
        };
        let Some(pub_key) = self.author_pub_key() else { return };
        let Some(doc_state) = self.documents.get(&document_id) else {
            tracing::warn!("Sync: SendSnapshot for unknown doc {document_id:?}");
            return;
        };
        let public_data = SnapshotPublicData {
            document_id,
            pub_key,
            snapshot_id: doc_state.current_snapshot_id,
            // TODO (snapshot flow): real BLAKE2b parent proof + per-author clocks.
            parent_snapshot_proof: Vec::new(),
            parent_snapshot_update_clocks: Vec::new(),
        };
        match pipeline::seal_snapshot(&payload, public_data, &doc_state.doc_key, &signing_key) {
            // TODO (snapshot transport): upload via S3 presigned PUT + POST
            // metadata. Snapshots do not transit the WebSocket.
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
        let Some(pub_key) = self.author_pub_key() else { return };
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
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("https://{api_base}/api/documents");
                match http::get_json::<http::DocumentListResponse>(&url).await {
                    Ok(resp) => {
                        let docs: Vec<DocumentSummary> = resp.documents.into_iter().map(|e| {
                            DocumentSummary {
                                document_id: DocumentId::parse_str(&e.document_id).unwrap_or_else(|_| DocumentId::new()),
                                doc_type: DocType::from_u8(e.doc_type).unwrap_or(DocType::Trame),
                                name: Some(e.name_enc),
                                role: match e.role.as_str() { "editor" => Role::Editor, "viewer" => Role::Viewer, _ => Role::Owner },
                                updated_at: e.updated_at,
                            }
                        }).collect();
                        let _ = tx.send(SyncEvent::DocumentList { id, documents: docs });
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
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("https://{api_base}/api/documents");
                let body = http::CreateDocumentRequest { doc_type: doc_type as u8, name_enc: name };
                match http::post_json::<_, http::CreateDocumentResponse>(&url, &body).await {
                    Ok(resp) => {
                        let document_id = DocumentId::parse_str(&resp.document_id).unwrap_or_else(|_| DocumentId::new());
                        let _ = tx.send(SyncEvent::DocumentCreated { id, document_id, doc_type });
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
            self.reply(id, RestEvent::DocumentCreated { document_id, doc_type });
        }
    }

    fn handle_delete_document(&mut self, id: RequestId, document_id: DocumentId) {
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            let api_base = self.api_base.clone();
            let doc_id = document_id;
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("https://{api_base}/api/documents/{doc_id}");
                match http::delete(&url).await {
                    Ok(()) => { let _ = tx.send(SyncEvent::DocumentDeleted { id, document_id: doc_id }); }
                    Err(e) => { let _ = tx.send(SyncEvent::HttpError { id, message: e }); }
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
            let doc_id = document_id;
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("https://{api_base}/api/documents/{doc_id}/rename");
                let body = http::RenameDocumentRequest { name_enc: name };
                match http::put_json(&url, &body).await {
                    Ok(()) => { let _ = tx.send(SyncEvent::DocumentRenamed { id, document_id: doc_id }); }
                    Err(e) => { let _ = tx.send(SyncEvent::HttpError { id, message: e }); }
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
        let exchange_key = self.exchange_key.clone();
        let doc_key = self.documents.get(&document_id).map(|ds| ds.doc_key.clone());

        let Some(tx) = event_tx else {
            self.reply(id, RestEvent::InviteSent {
                document_id,
                user_id: UserId::new(),
                display_name: invitee_email.clone(),
            });
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
                "https://{api_base}/api/users/search?query={}",
                invitee_email.replace('@', "%40")
            );
            let users = match http::get_json::<http::UserSearchResult>(&search_url).await {
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

            let wrapped = ambre_crypto::ecdh::wrap_document_key(&exchange_key, &peer_pub_bytes, &doc_key);
            let mut enc = wrapped.nonce.to_vec();
            enc.extend_from_slice(&wrapped.ciphertext);
            let doc_key_enc = base64::engine::general_purpose::URL_SAFE.encode(&enc);

            let invite_url = format!("https://{api_base}/api/documents/{document_id}/share");
            let body = http::InviteRequest {
                invitee_email: invitee_email.clone(),
                role: role.to_string(),
                doc_key_enc,
            };
            match http::post_json::<_, http::InviteResponse>(&invite_url, &body).await {
                Ok(resp) => {
                    let user_id = UserId::parse_str(&resp.user_id)
                        .unwrap_or_else(|_| UserId::new());
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
    fn handle_revoke_from_document(
        &self,
        id: RequestId,
        document_id: DocumentId,
        user_id: UserId,
    ) {
        let event_tx = self.event_tx.clone();
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
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
            let revoke_url = format!(
                "https://{api_base}/api/documents/{document_id}/share/{user_id}"
            );
            if let Err(e) = http::delete(&revoke_url).await {
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
            let members_url = format!(
                "https://{api_base}/api/documents/{document_id}/members"
            );
            let members = match http::get_json::<http::DocumentMembersResponse>(&members_url).await
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
                let wrapped =
                    ambre_crypto::ecdh::wrap_document_key(&exchange_key, &peer_pub_bytes, &new_doc_key);
                let mut enc = wrapped.nonce.to_vec();
                enc.extend_from_slice(&wrapped.ciphertext);
                let doc_key_enc = base64::engine::general_purpose::URL_SAFE.encode(&enc);
                keys.push(http::RotateKeyEntry {
                    user_id: member.user_id.clone(),
                    doc_key_enc,
                });
            }

            // 5. POST /api/documents/{id}/rotate-key
            let rotate_url = format!(
                "https://{api_base}/api/documents/{document_id}/rotate-key"
            );
            let body = http::RotateKeyRequest { keys };
            if let Err(e) = http::post_json::<_, serde_json::Value>(&rotate_url, &body).await {
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

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!(
                    "https://{api_base}/api/documents/{document_id}/members"
                );
                match http::get_json::<http::DocumentMembersResponse>(&url).await {
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
            self.reply(id, RestEvent::DocumentMembers {
                document_id,
                members: Vec::new(),
            });
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

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!(
                    "https://{api_base}/api/documents/{document_id}/members/{user_id}"
                );
                let body = serde_json::json!({ "role": role.to_string() });
                match http::put_json(&url, &body).await {
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
            tracing::info!(
                "Sync: UpdateDocumentMemberRole {document_id:?} {user_id:?} {role:?}"
            );
            self.reply(id, RestEvent::MemberRoleUpdated {
                document_id,
                user_id,
                role,
            });
        }
    }

    /// Searches users via `GET /api/users/search?query=...`, maps the server
    /// response into [`UserInfo`] values, and replies with
    /// [`RestEvent::UserSearchResults`]. The query is percent-encoded for the
    /// `@` character before being placed in the URL.
    fn handle_search_users(&self, id: RequestId, query: String) {
        let event_tx = self.event_tx.clone();
        let api_base = self.api_base.clone();

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!(
                    "https://{api_base}/api/users/search?query={}",
                    query.replace('@', "%40")
                );
                match http::get_json::<http::UserSearchResult>(&url).await {
                    Ok(resp) => {
                        let users: Vec<UserInfo> = resp
                            .users
                            .into_iter()
                            .map(|u| UserInfo {
                                user_id: UserId::parse_str(&u.id)
                                    .unwrap_or_else(|_| UserId::new()),
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
        tracing::debug!("Sync: SendChannelUpdate {channel_id:?} len={}", payload.len());
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

/// Non-blocking WASM entry point. Spawned inside a `wasm_thread` Web Worker.
///
/// Creates an internal event channel (`event_tx` / `event_rx`) and runs a
/// `futures_util::select!` loop that races between incoming [`SyncCommand`]s
/// from the Logic thread and [`SyncEvent`]s pushed by JS WebSocket callbacks
/// and spawned HTTP tasks. Unlike the synchronous [`run`], this loop does NOT
/// call `Atomics.wait` and therefore does not freeze the Worker's JS event
/// loop.
#[cfg(target_arch = "wasm32")]
pub async fn run_wasm(
    logic_rx: flume::Receiver<SyncCommand>,
    logic_tx: flume::Sender<LogicEvent>,
    api_base: String,
) {
    tracing::info!("Ambre Sync thread starting (wasm async)");

    let mut state = SyncState::new(logic_tx, api_base);
    let (event_tx, event_rx) = flume::unbounded::<SyncEvent>();
    state.set_event_tx(event_tx);

    loop {
        futures_util::select! {
            cmd = logic_rx.recv_async() => {
                match cmd {
                    Ok(cmd) => state.handle_command(cmd),
                    Err(flume::RecvError::Disconnected) => break,
                }
            }
            event = event_rx.recv_async() => {
                match event {
                    Ok(ev) => state.handle_sync_event(ev),
                    Err(flume::RecvError::Disconnected) => break,
                }
            }
        }
    }

    tracing::info!("Ambre Sync thread exiting (channels disconnected)");
}

/// Synchronous, blocking entry point for host (native) tests. Does not touch
/// WASM APIs — uses in-memory mock transport and pre-seeded keys.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(
    logic_rx: flume::Receiver<SyncCommand>,
    logic_tx: flume::Sender<LogicEvent>,
) {
    tracing::info!("Ambre Sync thread starting (native blocking)");

    let mut state = SyncState::new(logic_tx, String::new());

    while let Ok(cmd) = logic_rx.recv() {
        state.handle_command(cmd);
    }

    tracing::info!("Ambre Sync thread exiting (Logic thread disconnected)");
    let _ = state;
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
        let (tx, rx) = flume::unbounded();
        let mut state = SyncState::new(tx, String::new());
        let document_id = DocumentId::new();
        let doc_key = DocumentKey::generate();
        let signing_key = MasterSeed::generate().derive_signing_key();
        let doc_state = SyncDocumentState::new(document_id, doc_key.clone());
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
        let public_data = UpdatePublicData { document_id, pub_key, snapshot_id, clock: Clock(clock) };
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
            LogicEvent::RemoteUpdate { document_id, payload } => {
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
