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

use std::collections::HashMap;

use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, WebSocket};

use ambre_bus::{
    ChannelId, DocType, DocumentId, LogicEvent, RequestId, RestCommand, RestEvent, SnapshotId,
    SyncCommand, UserId,
};
use ambre_core::PublicKey;
use ambre_crypto::{DocumentKey, MasterSeed};
use ambre_protocol::messages::{Clock, Counter, Seq};
use ambre_protocol::public_data::{EphemeralPublicData, SnapshotPublicData, UpdatePublicData};
use ambre_protocol::ws::{decode_server_msg, encode_client_msg, WsClientMsg, WsServerMsg};
use ed25519_dalek::SigningKey;
use x25519_dalek::StaticSecret;

mod pipeline;

// -----------------------------------------------------------------------------
// Per-document sync session
// -----------------------------------------------------------------------------

#[allow(dead_code)]
struct SyncDocumentState {
    document_id: DocumentId,
    doc_key: DocumentKey,
    /// Snapshot this document's updates are currently based on. Placeholder
    /// until the snapshot flow lands; carried in every `UpdatePublicData`.
    current_snapshot_id: SnapshotId,
    local_clock: u64,
    /// Monotonic counter for outbound ephemerals (anti-replay at the receiver).
    ephemeral_counter: u64,
    last_seq: Seq,
    quarantine: Vec<(u64, Vec<u8>)>,
}

impl SyncDocumentState {
    fn new(document_id: DocumentId, doc_key: DocumentKey) -> Self {
        Self {
            document_id,
            doc_key,
            current_snapshot_id: SnapshotId::new(),
            local_clock: 0,
            ephemeral_counter: 0,
            last_seq: Seq(0),
            quarantine: Vec::new(),
        }
    }
}

// -----------------------------------------------------------------------------
// SyncState
// -----------------------------------------------------------------------------

struct SyncState {
    /// Outbound channel: Sync → Logic thread.
    logic_tx: flume::Sender<LogicEvent>,
    /// Base URL for REST API calls (e.g. `"api.ambre.app"`).
    #[allow(dead_code)]
    api_base: String,

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
    /// JS closures attached to the WebSocket. Kept alive to prevent GC.
    _ws_closures: Vec<Closure<dyn FnMut(MessageEvent)>>,

    // -- Per-document sync sessions --------------------------------------------
    documents: HashMap<DocumentId, SyncDocumentState>,
}

impl SyncState {
    fn new(logic_tx: flume::Sender<LogicEvent>, api_base: String) -> Self {
        Self {
            logic_tx,
            api_base,
            master_seed: None,
            signing_key: None,
            exchange_key: None,
            ws: None,
            _ws_closures: Vec::new(),
            documents: HashMap::new(),
        }
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
                    code: 1,
                    message: e.to_string(),
                });
            }
        }
    }

    // -- Outbound: Sync → Server (WebSocket) -----------------------------------

    /// Encode and push a client message onto the WebSocket.
    ///
    /// A missing socket or a failed `send` is a transient network condition, not
    /// a programmer error: it must **not** panic (a panic is `panic=abort` and
    /// would tear the Sync thread down silently). Callers already gate on
    /// `self.ws.is_some()`; this only logs if that race is lost. The reconnect
    /// path (onclose → backoff) re-establishes the socket.
    fn ws_send(&self, msg: &WsClientMsg) {
        let Some(ws) = self.ws.as_ref() else {
            tracing::warn!("Sync: ws_send called with no socket connected, dropping message");
            return;
        };
        let wire = encode_client_msg(msg);
        if let Err(e) = ws.send_with_u8_array(&wire) {
            tracing::warn!("Sync: WebSocket send failed ({e:?}); reconnect will retry");
        }
    }

    // -- Inbound: Logic → Sync -------------------------------------------------

    fn handle_command(&mut self, cmd: SyncCommand) {
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
            RestCommand::SetupAccount { passphrase } => self.handle_setup_account(passphrase),
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
                self.handle_create_document(doc_type, name);
            }
            RestCommand::DeleteDocument { document_id } => {
                self.handle_delete_document(document_id);
            }
            RestCommand::RenameDocument { document_id, name } => {
                self.handle_rename_document(document_id, name);
            }

            // -- Sharing -------------------------------------------------------
            RestCommand::InviteToDocument { document_id, invitee_email, role } => {
                self.handle_invite_to_document(document_id, invitee_email, role);
            }
            RestCommand::RevokeFromDocument { document_id, user_id } => {
                self.handle_revoke_from_document(document_id, user_id);
            }
            RestCommand::ListDocumentMembers { document_id } => {
                self.handle_list_document_members(document_id);
            }
            RestCommand::UpdateDocumentMemberRole { document_id, user_id, role } => {
                tracing::info!("Sync: UpdateDocumentMemberRole {document_id:?} {user_id:?} {role:?}");
            }

            // -- Users ---------------------------------------------------------
            RestCommand::SearchUsers { query } => tracing::info!("Sync: SearchUsers '{query}'"),

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

        // TODO (Unlock task): fetch the passphrase-wrapped master key
        // (GET /api/keys/master), Argon2id-derive the user key, decrypt it into
        // the real MasterSeed, and return the real user identity. For now we
        // seed the session locally so the crypto pipeline is functional offline.
        let master_seed = MasterSeed::generate();
        self.signing_key = Some(master_seed.derive_signing_key());
        self.exchange_key = Some(master_seed.derive_exchange_key());
        self.master_seed = Some(master_seed);

        self.open_websocket();

        self.reply(id, RestEvent::SessionReady {
            user_id: UserId::new(),
            display_name: "placeholder".into(),
            email: "placeholder@example.com".into(),
        });
    }

    fn handle_setup_account(&mut self, passphrase: Vec<u8>) {
        tracing::info!("Sync: SetupAccount (passphrase len={})", passphrase.len());
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
        self._ws_closures.clear();
        self.documents.clear();
        self.master_seed = None;
        self.signing_key = None;
        self.exchange_key = None;
        tracing::info!("Sync: logged out, all keys zeroized");
    }

    // -- WebSocket lifecycle ---------------------------------------------------

    fn open_websocket(&mut self) {
        // Placeholder — real implementation will:
        //   1. WebSocket::new("wss://{api_base}/api/ws")
        //   2. Wire ws.onmessage -> handle_ws_message via a flume::Sender
        //      delivered into the run() loop (because Atomics.wait blocks the
        //      JS event loop on a wasm Worker, the closure must push into a
        //      channel that the run() loop selects on).
        //   3. Wire ws.onclose -> reconnect_and_resubscribe (exponential backoff)
        tracing::info!("Sync: WebSocket opened (multiplexed)");
    }

    #[allow(dead_code)]
    fn reconnect_and_resubscribe(&mut self) {
        for doc_id in self.documents.keys().copied().collect::<Vec<_>>() {
            self.send_to_logic(LogicEvent::ConnectionStatus {
                document_id: doc_id,
                connected: false,
            });
        }
        tracing::info!("Sync: reconnecting ({} active docs)", self.documents.len());
    }

    // -- Inbound: Server -> Sync (WebSocket demux) -----------------------------

    /// Demultiplex an inbound WebSocket frame by `document_id`.
    ///
    /// Wire format: `[version_byte: u8] [postcard(WsServerMsg)]`
    #[allow(dead_code)]
    fn handle_ws_message(&mut self, wire: &[u8]) {
        let server_msg = match decode_server_msg(wire) {
            Ok(msg) => msg,
            Err(e) => {
                tracing::error!("Sync: wire decode error: {e:?}");
                return;
            }
        };

        match server_msg {
            WsServerMsg::Subscribed { document_id } => {
                self.send_to_logic(LogicEvent::ConnectionStatus {
                    document_id,
                    connected: true,
                });
            }

            WsServerMsg::Update { seq, update } => {
                let doc_id = update.public_data.document_id;
                let opened = match self.documents.get_mut(&doc_id) {
                    Some(doc_state) => {
                        doc_state.last_seq = seq;
                        Some(pipeline::open_update(&update, &doc_state.doc_key))
                    }
                    None => None,
                };
                self.deliver_remote(doc_id, opened, |payload| LogicEvent::RemoteUpdate {
                    document_id: doc_id,
                    payload,
                });
            }

            WsServerMsg::Ephemeral(eph) => {
                let doc_id = eph.public_data.document_id;
                let opened = self
                    .documents
                    .get(&doc_id)
                    .map(|doc_state| pipeline::open_ephemeral(&eph, &doc_state.doc_key));
                self.deliver_remote(doc_id, opened, |payload| LogicEvent::RemoteEphemeral {
                    document_id: doc_id,
                    payload,
                });
            }

            WsServerMsg::MissingUpdates { document_id, updates } => {
                for seq_update in updates {
                    let opened = match self.documents.get_mut(&document_id) {
                        Some(doc_state) => {
                            doc_state.last_seq = seq_update.seq;
                            Some(pipeline::open_update(&seq_update.update, &doc_state.doc_key))
                        }
                        None => None,
                    };
                    self.deliver_remote(document_id, opened, |payload| {
                        LogicEvent::RemoteUpdate { document_id, payload }
                    });
                }
            }

            WsServerMsg::SnapshotRequired { document_id } => {
                self.send_to_logic(LogicEvent::SnapshotNeeded { document_id });
            }

            WsServerMsg::SnapshotAvailable { document_id, snapshot_id } => {
                let _ = snapshot_id;
                tracing::info!("Sync: SnapshotAvailable for {document_id:?}");
            }

            WsServerMsg::KeyRotationRequired { document_id, revoked_user_id } => {
                if let Some(_doc_state) = self.documents.get_mut(&document_id) {
                    self.send_to_logic(LogicEvent::MemberRevoked {
                        document_id,
                        revoked_user_id,
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

    // -- Document lifecycle ----------------------------------------------------

    fn handle_open_document(&mut self, document_id: DocumentId) {
        // TODO (sharing task): derive the real DocumentKey via X25519 unwrap of
        // the server-stored, member-wrapped key. Generated locally for now so
        // the pipeline round-trips within a single client.
        let doc_key = DocumentKey::generate();
        self.documents
            .insert(document_id, SyncDocumentState::new(document_id, doc_key));
        if self.ws.is_some() {
            self.ws_send(&WsClientMsg::Subscribe { document_id });
        }
        tracing::info!("Sync: OpenDocument {document_id:?}");
    }

    fn handle_close_document(&mut self, document_id: DocumentId) {
        if self.ws.is_some() {
            self.ws_send(&WsClientMsg::Unsubscribe { document_id });
        }
        self.documents.remove(&document_id);
        tracing::info!("Sync: CloseDocument {document_id:?}");
    }

    fn handle_reconnect(&mut self, document_id: DocumentId) {
        if self.ws.is_some() {
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
        self.reply(id, RestEvent::DocumentList { documents: vec![] });
        tracing::info!("Sync: ListDocuments");
    }

    fn handle_create_document(&self, doc_type: DocType, name: String) {
        tracing::info!("Sync: CreateDocument {doc_type:?} '{name}'");
    }

    fn handle_delete_document(&self, document_id: DocumentId) {
        tracing::info!("Sync: DeleteDocument {document_id:?}");
    }

    fn handle_rename_document(&self, document_id: DocumentId, name: String) {
        tracing::info!("Sync: RenameDocument {document_id:?} '{name}'");
    }

    // -- Sharing (HTTP REST + crypto) ------------------------------------------

    fn handle_invite_to_document(
        &self,
        document_id: DocumentId,
        invitee_email: String,
        role: ambre_bus::Role,
    ) {
        tracing::info!("Sync: InviteToDocument {document_id:?} {invitee_email} {role:?}");
    }

    fn handle_revoke_from_document(&self, document_id: DocumentId, user_id: UserId) {
        tracing::info!("Sync: RevokeFromDocument {document_id:?} {user_id:?}");
    }

    fn handle_list_document_members(&self, document_id: DocumentId) {
        tracing::info!("Sync: ListDocumentMembers {document_id:?}");
    }

    // -- Channels --------------------------------------------------------------

    fn handle_open_channel(&mut self, channel_id: ChannelId) {
        if self.ws.is_some() {
            self.ws_send(&WsClientMsg::SubscribeChannel { channel_id });
        }
        tracing::info!("Sync: OpenChannel {channel_id:?}");
    }

    fn handle_close_channel(&mut self, channel_id: ChannelId) {
        if self.ws.is_some() {
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
// Entry point: blocking recv on the inbound channel.
// -----------------------------------------------------------------------------

/// Run the Sync thread to completion.
///
/// Blocks on `logic_rx` and dispatches every [`SyncCommand`] through
/// [`SyncState::handle_command`]. Exits when `logic_rx` is disconnected
/// (i.e. the Logic thread has shut down).
pub fn run(
    logic_rx: flume::Receiver<SyncCommand>,
    logic_tx: flume::Sender<LogicEvent>,
) {
    tracing::info!("Ambre Sync thread starting");
    let mut state = SyncState::new(logic_tx, String::new());

    while let Ok(cmd) = logic_rx.recv() {
        state.handle_command(cmd);
    }

    tracing::info!("Ambre Sync thread exiting (Logic thread disconnected)");
    let _ = state;
}
