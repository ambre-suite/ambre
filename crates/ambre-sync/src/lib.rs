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

use ambre_bus::{DocType, LogicEvent, SyncCommand, Uuid};
use ambre_protocol::{
    messages::Seq,
    ws::{WsClientMsg, WsServerMsg, decode_server_msg, encode_client_msg},
};

// -----------------------------------------------------------------------------
// Crypto types (placeholders — real implementations land in ambre-crypto)
// -----------------------------------------------------------------------------

#[allow(dead_code)]
struct MasterSeed([u8; 32]);

#[allow(dead_code)]
struct DocumentKey([u8; 32]);

// -----------------------------------------------------------------------------
// Per-document sync session
// -----------------------------------------------------------------------------

#[allow(dead_code)]
struct SyncDocumentState {
    document_id: Uuid,
    doc_key: DocumentKey,
    local_clock: u64,
    last_seq: Seq,
    quarantine: Vec<(u64, Vec<u8>)>,
}

impl SyncDocumentState {
    fn new(document_id: Uuid, doc_key: DocumentKey) -> Self {
        Self {
            document_id,
            doc_key,
            local_clock: 0,
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

    // -- Single multiplexed WebSocket -----------------------------------------
    ws: Option<WebSocket>,
    /// JS closures attached to the WebSocket. Kept alive to prevent GC.
    _ws_closures: Vec<Closure<dyn FnMut(MessageEvent)>>,

    // -- Per-document sync sessions --------------------------------------------
    documents: HashMap<Uuid, SyncDocumentState>,
}

impl SyncState {
    fn new(logic_tx: flume::Sender<LogicEvent>, api_base: String) -> Self {
        Self {
            logic_tx,
            api_base,
            master_seed: None,
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

    // -- Outbound: Sync → Server (WebSocket) -----------------------------------

    fn ws_send(&self, msg: &WsClientMsg) {
        let ws = self.ws.as_ref().expect("WebSocket not connected");
        let wire = encode_client_msg(msg);
        ws.send_with_u8_array(&wire).expect("WebSocket send");
    }

    // -- Inbound: Logic → Sync -------------------------------------------------

    fn handle_command(&mut self, cmd: SyncCommand) {
        match cmd {
            // -- Session -------------------------------------------------------
            SyncCommand::Unlock { passphrase } => self.handle_unlock(passphrase),
            SyncCommand::SetupAccount { passphrase } => self.handle_setup_account(passphrase),
            SyncCommand::ChangePassphrase { current_passphrase, new_passphrase } => {
                self.handle_change_passphrase(current_passphrase, new_passphrase);
            }
            SyncCommand::RecoverAccount { recovery_words, new_passphrase } => {
                self.handle_recover_account(recovery_words, new_passphrase);
            }
            SyncCommand::Logout => self.handle_logout(),

            // -- Document CRUD -------------------------------------------------
            SyncCommand::ListDocuments => self.handle_list_documents(),
            SyncCommand::CreateDocument { doc_type, name } => {
                self.handle_create_document(doc_type, name);
            }
            SyncCommand::DeleteDocument { document_id } => {
                self.handle_delete_document(document_id);
            }
            SyncCommand::RenameDocument { document_id, name } => {
                self.handle_rename_document(document_id, name);
            }

            // -- Document sync session -----------------------------------------
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

            // -- Sharing -------------------------------------------------------
            SyncCommand::InviteToDocument { document_id, invitee_email, role } => {
                self.handle_invite_to_document(document_id, invitee_email, role);
            }
            SyncCommand::RevokeFromDocument { document_id, user_id } => {
                self.handle_revoke_from_document(document_id, user_id);
            }
            SyncCommand::ListDocumentMembers { document_id } => {
                self.handle_list_document_members(document_id);
            }
            SyncCommand::UpdateDocumentMemberRole { document_id, user_id, role } => {
                tracing::info!(
                    "Sync: UpdateDocumentMemberRole {document_id:?} {user_id:?}"
                );
                let _ = role;
            }

            // -- Users ---------------------------------------------------------
            SyncCommand::SearchUsers { query } => {
                tracing::info!("Sync: SearchUsers '{query}'");
            }

            // -- Media ---------------------------------------------------------
            SyncCommand::UploadMedia { document_id, data, mime_type } => {
                tracing::info!(
                    "Sync: UploadMedia {document_id:?} {mime_type} len={}",
                    data.len()
                );
            }
            SyncCommand::DownloadMedia { document_id, content_hash } => {
                tracing::info!("Sync: DownloadMedia {document_id:?} {content_hash}");
            }

            // -- Channels ------------------------------------------------------
            SyncCommand::ListChannels => tracing::info!("Sync: ListChannels"),
            SyncCommand::CreateChannel { name } => {
                tracing::info!("Sync: CreateChannel '{name}'");
            }
            SyncCommand::DeleteChannel { channel_id } => {
                tracing::info!("Sync: DeleteChannel {channel_id:?}");
            }
            SyncCommand::OpenChannel { channel_id } => self.handle_open_channel(channel_id),
            SyncCommand::CloseChannel { channel_id } => self.handle_close_channel(channel_id),
            SyncCommand::InviteToChannel { channel_id, invitee_email, role } => {
                tracing::info!(
                    "Sync: InviteToChannel {channel_id:?} {invitee_email} {role:?}"
                );
            }
            SyncCommand::RevokeFromChannel { channel_id, user_id } => {
                tracing::info!("Sync: RevokeFromChannel {channel_id:?} {user_id:?}");
            }
            SyncCommand::ListChannelMembers { channel_id } => {
                tracing::info!("Sync: ListChannelMembers {channel_id:?}");
            }
            SyncCommand::UpdateChannelMemberRole { channel_id, user_id, role } => {
                tracing::info!(
                    "Sync: UpdateChannelMemberRole {channel_id:?} {user_id:?} {role:?}"
                );
            }
            SyncCommand::SendChannelUpdate { channel_id, payload } => {
                self.handle_send_channel_update(channel_id, payload);
            }
            SyncCommand::SendChannelEphemeral { channel_id, payload } => {
                self.handle_send_channel_ephemeral(channel_id, payload);
            }
            SyncCommand::UpdateReadCursor { channel_id, last_seq } => {
                tracing::debug!("Sync: UpdateReadCursor {channel_id:?} seq={last_seq}");
            }
            SyncCommand::GetUnreadCounts => tracing::info!("Sync: GetUnreadCounts"),

            // -- Meetings ------------------------------------------------------
            SyncCommand::CreateMeeting { name } => {
                tracing::info!("Sync: CreateMeeting '{name}'");
            }
            SyncCommand::JoinMeeting { meeting_id } => {
                tracing::info!("Sync: JoinMeeting {meeting_id:?}");
            }
            SyncCommand::LeaveMeeting { meeting_id } => {
                tracing::info!("Sync: LeaveMeeting {meeting_id:?}");
            }
            SyncCommand::EndMeeting { meeting_id } => {
                tracing::info!("Sync: EndMeeting {meeting_id:?}");
            }

            // -- Lifecycle -----------------------------------------------------
            SyncCommand::Shutdown => self.handle_logout(),
        }
    }

    // -- Session handlers ------------------------------------------------------

    fn handle_unlock(&mut self, passphrase: Vec<u8>) {
        tracing::info!("Sync: Unlock (passphrase len={})", passphrase.len());
        self.open_websocket();

        self.send_to_logic(LogicEvent::SessionReady {
            user_id: Uuid::new_v7(),
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

    fn handle_logout(&mut self) {
        if let Some(ws) = self.ws.take() {
            let _ = ws.close();
        }
        self._ws_closures.clear();
        self.documents.clear();
        self.master_seed = None;
        self.send_to_logic(LogicEvent::LoggedOut);
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
                if let Some(doc_state) = self.documents.get_mut(&doc_id) {
                    doc_state.last_seq = seq;
                    let _ = update;
                    // Pipeline: verify signature/HMAC → decrypt → decompress →
                    // send LogicEvent::RemoteUpdate { document_id, payload }.
                }
            }

            WsServerMsg::Ephemeral(eph) => {
                let doc_id = eph.public_data.document_id;
                if let Some(doc_state) = self.documents.get(&doc_id) {
                    let _ = doc_state;
                }
            }

            WsServerMsg::MissingUpdates { document_id, updates } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    for seq_update in &updates {
                        doc_state.last_seq = seq_update.seq;
                        let _ = &seq_update.update;
                    }
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

    fn handle_open_document(&mut self, document_id: Uuid) {
        let doc_key = DocumentKey([0; 32]); // placeholder
        self.documents
            .insert(document_id, SyncDocumentState::new(document_id, doc_key));
        if self.ws.is_some() {
            self.ws_send(&WsClientMsg::Subscribe { document_id });
        }
        tracing::info!("Sync: OpenDocument {document_id:?}");
    }

    fn handle_close_document(&mut self, document_id: Uuid) {
        if self.ws.is_some() {
            self.ws_send(&WsClientMsg::Unsubscribe { document_id });
        }
        self.documents.remove(&document_id);
        tracing::info!("Sync: CloseDocument {document_id:?}");
    }

    fn handle_reconnect(&mut self, document_id: Uuid) {
        if self.ws.is_some() {
            let last_seq =
                self.documents.get(&document_id).map(|d| d.last_seq).unwrap_or(Seq(0));
            self.ws_send(&WsClientMsg::Subscribe { document_id });
            self.ws_send(&WsClientMsg::RequestMissing { document_id, after_seq: last_seq });
        }
    }

    // -- Outbound sync ---------------------------------------------------------

    fn handle_send_update(&mut self, document_id: Uuid, payload: Vec<u8>) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            tracing::warn!("Sync: SendUpdate for unknown doc {document_id:?}");
            return;
        };
        doc_state.local_clock += 1;
        let clock = doc_state.local_clock;
        tracing::debug!(
            "Sync: SendUpdate {document_id:?} clock={clock} len={}",
            payload.len()
        );
    }

    fn handle_send_snapshot(&mut self, document_id: Uuid, payload: Vec<u8>) {
        if !self.documents.contains_key(&document_id) {
            return;
        }
        tracing::info!("Sync: SendSnapshot {document_id:?} len={}", payload.len());
    }

    fn handle_send_ephemeral(&mut self, document_id: Uuid, payload: Vec<u8>) {
        if !self.documents.contains_key(&document_id) {
            return;
        }
        tracing::debug!("Sync: SendEphemeral {document_id:?} len={}", payload.len());
    }

    // -- Document CRUD (HTTP REST) ---------------------------------------------

    fn handle_list_documents(&self) {
        self.send_to_logic(LogicEvent::DocumentList { documents: vec![] });
        tracing::info!("Sync: ListDocuments");
    }

    fn handle_create_document(&self, doc_type: DocType, name: String) {
        tracing::info!("Sync: CreateDocument {doc_type:?} '{name}'");
    }

    fn handle_delete_document(&self, document_id: Uuid) {
        tracing::info!("Sync: DeleteDocument {document_id:?}");
    }

    fn handle_rename_document(&self, document_id: Uuid, name: String) {
        tracing::info!("Sync: RenameDocument {document_id:?} '{name}'");
    }

    // -- Sharing (HTTP REST + crypto) ------------------------------------------

    fn handle_invite_to_document(
        &self,
        document_id: Uuid,
        invitee_email: String,
        role: ambre_bus::Role,
    ) {
        tracing::info!("Sync: InviteToDocument {document_id:?} {invitee_email} {role:?}");
    }

    fn handle_revoke_from_document(&self, document_id: Uuid, user_id: Uuid) {
        tracing::info!("Sync: RevokeFromDocument {document_id:?} {user_id:?}");
    }

    fn handle_list_document_members(&self, document_id: Uuid) {
        tracing::info!("Sync: ListDocumentMembers {document_id:?}");
    }

    // -- Channels --------------------------------------------------------------

    fn handle_open_channel(&mut self, channel_id: Uuid) {
        if self.ws.is_some() {
            self.ws_send(&WsClientMsg::Subscribe { document_id: channel_id });
        }
        tracing::info!("Sync: OpenChannel {channel_id:?}");
    }

    fn handle_close_channel(&mut self, channel_id: Uuid) {
        if self.ws.is_some() {
            self.ws_send(&WsClientMsg::Unsubscribe { document_id: channel_id });
        }
        tracing::info!("Sync: CloseChannel {channel_id:?}");
    }

    fn handle_send_channel_update(&self, channel_id: Uuid, payload: Vec<u8>) {
        tracing::debug!("Sync: SendChannelUpdate {channel_id:?} len={}", payload.len());
    }

    fn handle_send_channel_ephemeral(&self, channel_id: Uuid, payload: Vec<u8>) {
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
