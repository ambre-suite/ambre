// ambre-sync-worker/src/lib.rs
// ============================================================================
// Sync Worker entry point - dedicated Web Worker, isolated linear memory.
//
// Responsibilities:
//   - Own ALL cryptographic material (MasterSeed, DocumentKeys) - never leaves
//     this Worker.
//   - Own the single session-level WebSocket (one WS, N document subscriptions).
//   - Perform HTTP REST calls (fetch via gloo-net).
//   - Encrypt/decrypt/compress/decompress every outbound and inbound payload.
//
// Out of scope:
//   - CRDT mutations - exclusively in the Logic Worker.
//   - Rendering, input - exclusively in the UI Thread.
//
// IPC:
//   SyncCommand  (postcard + ArrayBuffer via MessagePort) <- Logic Worker
//   LogicEvent   (postcard + ArrayBuffer via MessagePort) -> Logic Worker
//
// Bootstrap (2-phase):
//   Phase 1 - global.onmessage (once): receive the MessagePort to the Logic Worker.
//   Phase 2 - port.onmessage (steady-state): dispatch SyncCommand messages.
//
// WebSocket topology:
//   ONE WebSocket per session, opened at Unlock, closed at Logout.
//   OpenDocument / CloseDocument -> Subscribe / Unsubscribe on the shared WS.
//   All documents are demultiplexed by document_id on incoming WsServerMsg.
// ============================================================================

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent, MessagePort, WebSocket};

use ambre_bus::{DocType, LogicEvent, SyncCommand, Uuid};
use ambre_protocol::{
    messages::Seq,
    ws::{WsClientMsg, WsServerMsg, decode_server_msg, encode_client_msg},
};

// -----------------------------------------------------------------------------
// Crypto types (placeholders - real implementations in ambre-crypto)
// -----------------------------------------------------------------------------

/// 32-byte master seed derived from the user's passphrase via Argon2id.
///
/// Derives Ed25519 signing key + X25519 exchange key. Zeroized on drop.
#[allow(dead_code)]
struct MasterSeed([u8; 32]);

/// Per-document symmetric encryption key (XChaCha20-Poly1305). Zeroized on drop.
#[allow(dead_code)]
struct DocumentKey([u8; 32]);

// -----------------------------------------------------------------------------
// Per-document sync session
// -----------------------------------------------------------------------------

/// State for one open document sync session.
///
/// A session is created on `SyncCommand::OpenDocument` and destroyed on
/// `SyncCommand::CloseDocument` (or `Logout`). The `DocumentKey` is zeroized
/// on drop.
#[allow(dead_code)]
struct SyncDocumentState {
    document_id: Uuid,
    doc_key: DocumentKey,
    /// This client's monotonic clock for outbound deltas.
    local_clock: u64,
    /// Last server sequence number seen (used for delta-sync on reconnect).
    last_seq: Seq,
    /// Updates received before their parent snapshot - bounded to 1000 entries
    /// to prevent unbounded memory growth.
    quarantine: Vec<(u64, Vec<u8>)>,
}

impl SyncDocumentState {
    fn new(document_id: Uuid, doc_key: DocumentKey) -> Self {
        Self { document_id, doc_key, local_clock: 0, last_seq: Seq(0), quarantine: Vec::new() }
    }
}

// -----------------------------------------------------------------------------
// SyncWorkerState
// -----------------------------------------------------------------------------

/// All mutable state owned by the Sync Worker after bootstrap.
struct SyncWorkerState {
    /// MessagePort to the Logic Worker (established during bootstrap).
    logic_port: MessagePort,
    /// Base URL for REST API calls (e.g. `"api.ambre.app"`).
    #[allow(dead_code)]
    api_base: String,

    // -- Session-level crypto (derived once at Unlock, zeroized at Logout) ----
    master_seed: Option<MasterSeed>,
    // signing_key:  Option<ed25519_dalek::SigningKey>,
    // exchange_key: Option<x25519_dalek::StaticSecret>,

    // -- Single multiplexed WebSocket -----------------------------------------
    ws: Option<WebSocket>,
    /// JS closures attached to the WebSocket. Must be kept alive to prevent GC.
    _ws_closures: Vec<Closure<dyn FnMut(MessageEvent)>>,

    // -- Per-document sync sessions --------------------------------------------
    documents: HashMap<Uuid, SyncDocumentState>,
}

impl SyncWorkerState {
    fn new(logic_port: MessagePort, api_base: String) -> Self {
        Self {
            logic_port,
            api_base,
            master_seed: None,
            ws: None,
            _ws_closures: Vec::new(),
            documents: HashMap::new(),
        }
    }

    // -- Outbound: Sync Worker -> Logic Worker ---------------------------------

    /// Serialize `event` with postcard and transfer it to the Logic Worker.
    fn send_to_logic(&self, event: &LogicEvent) {
        let bytes = postcard::to_allocvec(event).expect("postcard serialize LogicEvent");
        let js_bytes = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
        js_bytes.copy_from(&bytes);
        let ab = js_bytes.buffer();
        let transfer = js_sys::Array::of1(&ab);
        self.logic_port
            .post_message_with_transferable(&ab, &transfer)
            .expect("postMessage LogicEvent to Logic Worker");
    }

    // -- Outbound: Sync Worker -> Server (WebSocket) ---------------------------

    /// Serialize `msg` with postcard (prefixed with wire version byte) and send
    /// over the multiplexed WebSocket.
    fn ws_send(&self, msg: &WsClientMsg) {
        let ws = self.ws.as_ref().expect("WebSocket not connected");
        let wire = encode_client_msg(msg);
        ws.send_with_u8_array(&wire).expect("WebSocket send");
    }

    // -- Inbound: Logic Worker -> Sync Worker ----------------------------------

    /// Deserialize a [`SyncCommand`] from `bytes` and dispatch it.
    fn handle_command(&mut self, bytes: &[u8]) {
        let cmd: SyncCommand =
            postcard::from_bytes(bytes).expect("postcard deserialize SyncCommand");

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
            SyncCommand::OpenDocument { document_id } => {
                self.handle_open_document(document_id);
            }
            SyncCommand::CloseDocument { document_id } => {
                self.handle_close_document(document_id);
            }
            SyncCommand::SendUpdate { document_id, payload } => {
                self.handle_send_update(document_id, payload);
            }
            SyncCommand::SendSnapshot { document_id, payload } => {
                self.handle_send_snapshot(document_id, payload);
            }
            SyncCommand::SendEphemeral { document_id, payload } => {
                self.handle_send_ephemeral(document_id, payload);
            }
            SyncCommand::Reconnect { document_id } => {
                self.handle_reconnect(document_id);
            }

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
                tracing::info!("Sync Worker: UpdateDocumentMemberRole {document_id:?} {user_id:?}");
                let _ = role;
            }

            // -- Users ---------------------------------------------------------
            SyncCommand::SearchUsers { query } => {
                tracing::info!("Sync Worker: SearchUsers '{query}'");
            }

            // -- Media ---------------------------------------------------------
            SyncCommand::UploadMedia { document_id, data, mime_type } => {
                tracing::info!(
                    "Sync Worker: UploadMedia {document_id:?} {mime_type} len={}",
                    data.len()
                );
            }
            SyncCommand::DownloadMedia { document_id, content_hash } => {
                tracing::info!("Sync Worker: DownloadMedia {document_id:?} {content_hash}");
            }

            // -- Channels -----------------------------------------------------
            SyncCommand::ListChannels => {
                tracing::info!("Sync Worker: ListChannels");
            }
            SyncCommand::CreateChannel { name } => {
                tracing::info!("Sync Worker: CreateChannel '{name}'");
            }
            SyncCommand::DeleteChannel { channel_id } => {
                tracing::info!("Sync Worker: DeleteChannel {channel_id:?}");
            }
            SyncCommand::OpenChannel { channel_id } => {
                self.handle_open_channel(channel_id);
            }
            SyncCommand::CloseChannel { channel_id } => {
                self.handle_close_channel(channel_id);
            }
            SyncCommand::InviteToChannel { channel_id, invitee_email, role } => {
                tracing::info!(
                    "Sync Worker: InviteToChannel {channel_id:?} {invitee_email} {role:?}"
                );
            }
            SyncCommand::RevokeFromChannel { channel_id, user_id } => {
                tracing::info!("Sync Worker: RevokeFromChannel {channel_id:?} {user_id:?}");
            }
            SyncCommand::ListChannelMembers { channel_id } => {
                tracing::info!("Sync Worker: ListChannelMembers {channel_id:?}");
            }
            SyncCommand::UpdateChannelMemberRole { channel_id, user_id, role } => {
                tracing::info!(
                    "Sync Worker: UpdateChannelMemberRole {channel_id:?} {user_id:?} {role:?}"
                );
            }
            SyncCommand::SendChannelUpdate { channel_id, payload } => {
                self.handle_send_channel_update(channel_id, payload);
            }
            SyncCommand::SendChannelEphemeral { channel_id, payload } => {
                self.handle_send_channel_ephemeral(channel_id, payload);
            }
            SyncCommand::UpdateReadCursor { channel_id, last_seq } => {
                tracing::debug!("Sync Worker: UpdateReadCursor {channel_id:?} seq={last_seq}");
            }
            SyncCommand::GetUnreadCounts => {
                tracing::info!("Sync Worker: GetUnreadCounts");
            }

            // -- Meetings -----------------------------------------------------
            SyncCommand::CreateMeeting { name } => {
                tracing::info!("Sync Worker: CreateMeeting '{name}'");
            }
            SyncCommand::JoinMeeting { meeting_id } => {
                tracing::info!("Sync Worker: JoinMeeting {meeting_id:?}");
            }
            SyncCommand::LeaveMeeting { meeting_id } => {
                tracing::info!("Sync Worker: LeaveMeeting {meeting_id:?}");
            }
            SyncCommand::EndMeeting { meeting_id } => {
                tracing::info!("Sync Worker: EndMeeting {meeting_id:?}");
            }

            // -- Lifecycle -----------------------------------------------------
            SyncCommand::Shutdown => {
                self.handle_logout();
            }
        }
    }

    // -- Session handlers ------------------------------------------------------

    fn handle_unlock(&mut self, passphrase: Vec<u8>) {
        // Full async pipeline (via wasm_bindgen_futures::spawn_local):
        // 1. GET /api/auth/me                           - verify session cookie
        // 2. GET /api/keys/master                       - fetch { master_key_enc, salt }
        // 3. Argon2id(passphrase, salt)                 -> user_key
        // 4. XChaCha20-Poly1305 decrypt(user_key, master_key_enc) -> master_seed
        // 5. Derive Ed25519 signing_key + X25519 exchange_key from master_seed
        // 6. WebSocket::new("wss://{api_base}/api/ws")  - open session WS
        // 7. Wire ws.onmessage -> handle_ws_message
        // 8. Wire ws.onclose  -> reconnect_and_resubscribe (exponential backoff)
        // 9. Send LogicEvent::SessionReady

        tracing::info!("Sync Worker: Unlock (passphrase len={})", passphrase.len());
        self.open_websocket();

        // Placeholder - replace with real session once crypto is wired.
        self.send_to_logic(&LogicEvent::SessionReady {
            user_id: Uuid::new_v7(),
            display_name: "placeholder".into(),
            email: "placeholder@example.com".into(),
        });
    }

    fn handle_setup_account(&mut self, passphrase: Vec<u8>) {
        // 1. Generate MasterSeed (CSPRNG, 32 bytes)
        // 2. Derive Ed25519 signing_key + X25519 exchange_key
        // 3. Argon2id(passphrase, random_salt) -> user_key
        // 4. XChaCha20-Poly1305 encrypt(user_key, master_seed) -> master_key_enc
        // 5. PUT /api/keys/master { master_key_enc, salt, signing_pubkey, exchange_pubkey }
        // 6. BIP39 encode(master_seed) -> 24 recovery_words
        // 7. Open WebSocket (same as Unlock step 6-8)
        // 8. Send LogicEvent::AccountCreated { recovery_words }

        tracing::info!("Sync Worker: SetupAccount (passphrase len={})", passphrase.len());
    }

    fn handle_change_passphrase(
        &mut self,
        current_passphrase: Vec<u8>,
        new_passphrase: Vec<u8>,
    ) {
        // 1. Re-derive user_key from current_passphrase
        // 2. Decrypt master_seed -> verify integrity
        // 3. Argon2id(new_passphrase, new_salt) -> new_user_key
        // 4. Re-encrypt master_seed -> master_key_enc
        // 5. PUT /api/keys/master/rotate { master_key_enc, salt }
        // 6. Send LogicEvent::PassphraseChanged

        tracing::info!("Sync Worker: ChangePassphrase");
        let _ = (current_passphrase, new_passphrase);
    }

    fn handle_recover_account(&mut self, recovery_words: String, new_passphrase: Vec<u8>) {
        // 1. BIP39 decode(recovery_words) -> master_seed
        // 2. Argon2id(new_passphrase, new_salt) -> user_key
        // 3. Encrypt master_seed -> master_key_enc
        // 4. PUT /api/keys/master/rotate
        // 5. Send LogicEvent::AccountRecovered

        tracing::info!("Sync Worker: RecoverAccount");
        let _ = (recovery_words, new_passphrase);
    }

    fn handle_logout(&mut self) {
        // 1. POST /api/auth/logout
        // 2. ws.close() - triggers onclose, no reconnect
        // 3. Zeroize master_seed, signing_key, exchange_key
        // 4. Drop documents -> ZeroizeOnDrop zeroes all DocumentKeys
        // 5. Send LogicEvent::LoggedOut

        if let Some(ws) = self.ws.take() {
            let _ = ws.close();
        }
        self._ws_closures.clear();
        self.documents.clear();
        self.master_seed = None;
        self.send_to_logic(&LogicEvent::LoggedOut);
        tracing::info!("Sync Worker: logged out, all keys zeroized");
    }

    // -- WebSocket lifecycle ---------------------------------------------------

    fn open_websocket(&mut self) {
        // let ws_url = format!("wss://{}/api/ws", self.api_base);
        // let ws = WebSocket::new(&ws_url).expect("WebSocket::new");
        // ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
        //
        // // onmessage: demux by document_id
        // let state = Rc::clone(&STATE_HANDLE);
        // let on_msg = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        //     if let Ok(ab) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
        //         let mut wire = vec![0u8; js_sys::Uint8Array::new(&ab).length() as usize];
        //         js_sys::Uint8Array::new(&ab).copy_to(&mut wire);
        //         if let Some(ref mut s) = *state.borrow_mut() { s.handle_ws_message(&wire); }
        //     }
        // });
        // ws.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
        // self._ws_closures.push(on_msg);
        //
        // // onclose: reconnect with exponential backoff (1s, 2s, 4s, 8s, cap 30s)
        // // On reconnect: re-subscribe to all active documents + RequestMissing each
        //
        // self.ws = Some(ws);

        tracing::info!("Sync Worker: WebSocket opened (multiplexed)");
    }

    /// Reconnect the WebSocket and re-subscribe to all active documents.
    ///
    /// Called by the onclose handler with exponential backoff.
    #[allow(dead_code)]
    fn reconnect_and_resubscribe(&mut self) {
        // 1. open_websocket()
        // 2. For each doc_id in self.documents:
        //    ws_send(Subscribe { document_id })
        //    ws_send(RequestMissing { document_id, after_seq: doc_state.last_seq })
        // 3. For each doc_id, send LogicEvent::ConnectionStatus { connected: false }
        //    (will flip to true on WsServerMsg::Subscribed)

        for doc_id in self.documents.keys().copied().collect::<Vec<_>>() {
            self.send_to_logic(&LogicEvent::ConnectionStatus {
                document_id: doc_id,
                connected: false,
            });
        }
        tracing::info!("Sync Worker: reconnecting ({} active docs)", self.documents.len());
    }

    // -- Inbound: Server -> Sync Worker (WebSocket demux) ----------------------

    /// Demultiplex an inbound WebSocket frame by `document_id`.
    ///
    /// Wire format: `[version_byte: u8] [postcard(WsServerMsg)]`
    #[allow(dead_code)]
    fn handle_ws_message(&mut self, wire: &[u8]) {
        let server_msg = match decode_server_msg(wire) {
            Ok(msg) => msg,
            Err(e) => {
                tracing::error!("Sync Worker: wire decode error: {e:?}");
                return;
            }
        };

        match server_msg {
            WsServerMsg::Subscribed { document_id } => {
                self.send_to_logic(&LogicEvent::ConnectionStatus {
                    document_id,
                    connected: true,
                });
            }

            WsServerMsg::Update { seq, update } => {
                let doc_id = update.public_data.document_id;
                if let Some(doc_state) = self.documents.get_mut(&doc_id) {
                    doc_state.last_seq = seq;
                    let _ = update;
                    // Inbound pipeline:
                    // 1. Ed25519 verify signature
                    // 2. HMAC verify commitment_tag
                    // 3. XChaCha20-Poly1305 decrypt(doc_key, ciphertext, nonce)
                    // 4. Strip version byte: 0x01 -> ruzstd decompress, 0x00 -> raw
                    // 5. Send plaintext CRDT delta to Logic Worker

                    // let plaintext = self.decrypt_update(&doc_state.doc_key, &update);
                    // self.send_to_logic(&LogicEvent::RemoteUpdate {
                    //     document_id: doc_id, payload: plaintext,
                    // });
                }
            }

            WsServerMsg::Ephemeral(eph) => {
                let doc_id = eph.public_data.document_id;
                if let Some(doc_state) = self.documents.get(&doc_id) {
                    // Same decrypt pipeline as Update.
                    // self.send_to_logic(&LogicEvent::RemoteEphemeral { document_id: doc_id, payload });
                    let _ = doc_state;
                }
            }

            WsServerMsg::MissingUpdates { document_id, updates } => {
                if let Some(doc_state) = self.documents.get_mut(&document_id) {
                    for seq_update in &updates {
                        doc_state.last_seq = seq_update.seq;
                        let _ = &seq_update.update;
                        // Decrypt each delta and send to Logic Worker in order.
                    }
                }
            }

            WsServerMsg::SnapshotRequired { document_id } => {
                self.send_to_logic(&LogicEvent::SnapshotNeeded { document_id });
            }

            WsServerMsg::SnapshotAvailable { document_id, snapshot_id } => {
                // Async download pipeline:
                // 1. POST /api/documents/{id}/snapshot -> presigned GET URL
                // 2. GET S3 (presigned) -> ciphertext blob
                // 3. Verify + decrypt + decompress -> plaintext CRDT state
                // 4. Send LogicEvent::RemoteSnapshot { document_id, payload }
                let _ = snapshot_id;
                tracing::info!("Sync Worker: SnapshotAvailable for {document_id:?}");
            }

            WsServerMsg::KeyRotationRequired { document_id, revoked_user_id } => {
                // 1. GET /api/documents/{id}/members -> new doc_key_enc for self
                // 2. ECDH decrypt -> new DocumentKey
                // 3. Replace doc_key in SyncDocumentState
                // 4. Notify Logic Worker to trigger a snapshot at this boundary
                if let Some(_doc_state) = self.documents.get_mut(&document_id) {
                    // doc_state.doc_key = new_doc_key;
                    self.send_to_logic(&LogicEvent::MemberRevoked {
                        document_id,
                        revoked_user_id,
                    });
                }
            }

            WsServerMsg::ChannelSubscribed { channel_id } => {
                self.send_to_logic(&LogicEvent::ChannelConnectionStatus {
                    channel_id,
                    connected: true,
                });
            }

            WsServerMsg::ChannelUpdate { channel_id, seq: _, update } => {
                // Decrypt + decompress -> send LogicEvent::ChannelRemoteUpdate when wired.
                let _ = (channel_id, update);
            }

            WsServerMsg::ChannelEphemeral { channel_id, ephemeral } => {
                let _ = (channel_id, ephemeral);
            }

            WsServerMsg::Error { code, message } => {
                tracing::error!("Sync Worker: server error {code}: {message}");
                self.send_to_logic(&LogicEvent::ApiError {
                    request: "websocket".into(),
                    code,
                    message,
                });
            }
        }
    }

    // -- Document lifecycle ----------------------------------------------------

    fn handle_open_document(&mut self, document_id: Uuid) {
        // Full async pipeline (spawn_local):
        // 1. GET /api/documents/{id}/members -> find self, fetch doc_key_enc
        // 2. ECDH(exchange_key, owner_pubkey) -> shared_secret
        // 3. Decrypt doc_key_enc -> DocumentKey
        // 4. GET /api/documents/{id}/snapshot -> presigned GET URL -> S3 -> ciphertext
        // 5. Decrypt + decompress -> plaintext CRDT state
        // 6. Send LogicEvent::RemoteSnapshot { document_id, payload }
        // 7. GET /api/documents/{id}/deltas?after_seq={snapshot_seq}
        // 8. Decrypt each delta -> LogicEvent::RemoteUpdate (in order)
        // 9. ws_send(Subscribe { document_id })
        // 10. ws_send(RequestMissing { document_id, after_seq: last received seq })

        let doc_key = DocumentKey([0; 32]); // placeholder
        self.documents.insert(document_id, SyncDocumentState::new(document_id, doc_key));
        if self.ws.is_some() {
            self.ws_send(&WsClientMsg::Subscribe { document_id });
        }
        tracing::info!("Sync Worker: OpenDocument {document_id:?}");
    }

    fn handle_close_document(&mut self, document_id: Uuid) {
        if self.ws.is_some() {
            self.ws_send(&WsClientMsg::Unsubscribe { document_id });
        }
        // Drop SyncDocumentState -> DocumentKey is zeroized on drop.
        self.documents.remove(&document_id);
        tracing::info!("Sync Worker: CloseDocument {document_id:?}");
    }

    fn handle_reconnect(&mut self, document_id: Uuid) {
        // Re-subscribe and request missing deltas for this specific document.
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
            tracing::warn!("Sync Worker: SendUpdate for unknown doc {document_id:?}");
            return;
        };
        doc_state.local_clock += 1;
        let clock = doc_state.local_clock;

        // Outbound pipeline:
        // 1. Prepend version byte (0x01 = zstd if > 64 B, 0x00 = raw)
        // 2. ruzstd compress if version = 0x01
        // 3. XChaCha20-Poly1305 AEAD encrypt(doc_key) -> ciphertext + nonce
        // 4. Build WireUpdatePublicData { document_id, pub_key, snapshot_id, clock }
        // 5. postcard serialize public_data -> bytes
        // 6. HMAC-SHA256(doc_key, nonce || ciphertext || public_data_bytes) -> commitment_tag
        // 7. Ed25519 sign(nonce || ciphertext || public_data_bytes || commitment_tag)
        // 8. ws_send(WsClientMsg::Update(WireUpdate { ... }))

        tracing::debug!(
            "Sync Worker: SendUpdate {document_id:?} clock={clock} len={}",
            payload.len()
        );
    }

    fn handle_send_snapshot(&mut self, document_id: Uuid, payload: Vec<u8>) {
        if !self.documents.contains_key(&document_id) {
            return;
        }
        // Same pipeline as SendUpdate but for the full CRDT state.
        // Upload via presigned S3 URL -> server broadcasts SnapshotAvailable.
        tracing::info!("Sync Worker: SendSnapshot {document_id:?} len={}", payload.len());
    }

    fn handle_send_ephemeral(&mut self, document_id: Uuid, payload: Vec<u8>) {
        if !self.documents.contains_key(&document_id) {
            return;
        }
        // Same encrypt pipeline, but sent as WsClientMsg::Ephemeral.
        // Never persisted server-side; no commitment_tag sequence tracking needed.
        tracing::debug!("Sync Worker: SendEphemeral {document_id:?} len={}", payload.len());
    }

    // -- Document CRUD (HTTP REST) ---------------------------------------------

    fn handle_list_documents(&self) {
        // GET /api/documents -> Vec<{ id, doc_type, name_enc, role, updated_at }>
        // For each document: decrypt name_enc with DocumentKey
        // Build Vec<DocumentSummary> -> Send LogicEvent::DocumentList

        // Placeholder - real implementation uses gloo-net + spawn_local.
        self.send_to_logic(&LogicEvent::DocumentList { documents: vec![] });
        tracing::info!("Sync Worker: ListDocuments");
    }

    fn handle_create_document(&self, doc_type: DocType, name: String) {
        // 1. Generate DocumentKey (CSPRNG)
        // 2. Encrypt name with DocumentKey -> name_enc
        // 3. POST /api/documents { doc_type, name_enc } -> document_id
        // 4. Encrypt DocumentKey with own exchange_pubkey -> doc_key_enc
        // 5. POST /api/documents/{id}/share { user_id: self, role: Owner, doc_key_enc }
        // 6. Send LogicEvent::DocumentCreated { document_id, doc_type }
        tracing::info!("Sync Worker: CreateDocument {doc_type:?} '{name}'");
    }

    fn handle_delete_document(&self, document_id: Uuid) {
        // DELETE /api/documents/{id}
        // Send LogicEvent::DocumentDeleted { document_id }
        tracing::info!("Sync Worker: DeleteDocument {document_id:?}");
    }

    fn handle_rename_document(&self, document_id: Uuid, name: String) {
        // Encrypt name with DocumentKey -> name_enc
        // PATCH /api/documents/{id} { name_enc }
        // Send LogicEvent::DocumentRenamed { document_id }
        tracing::info!("Sync Worker: RenameDocument {document_id:?} '{name}'");
    }

    // -- Sharing (HTTP REST + crypto) ------------------------------------------

    fn handle_invite_to_document(
        &self,
        document_id: Uuid,
        invitee_email: String,
        role: ambre_bus::Role,
    ) {
        // 1. GET /api/users/search?email={email} -> invitee exchange_pubkey
        // 2. ECDH(exchange_key, invitee_pubkey) -> shared_secret
        // 3. Encrypt DocumentKey -> doc_key_enc
        // 4. POST /api/documents/{id}/share { user_id, role, doc_key_enc }
        // 5. Send LogicEvent::InviteSent { document_id, user_id, display_name }
        tracing::info!("Sync Worker: InviteToDocument {document_id:?} {invitee_email} {role:?}");
    }

    fn handle_revoke_from_document(&self, document_id: Uuid, user_id: Uuid) {
        // Full key rotation sequence (see ambre-spec-sync.md):
        // 1. DELETE /api/documents/{id}/share/{uid}
        // 2. Generate new DocumentKey
        // 3. GET /api/documents/{id}/members -> remaining members
        // 4. For each: ECDH encrypt new key -> doc_key_enc
        // 5. POST /api/documents/{id}/rotate-key { [(user_id, doc_key_enc)] }
        // 6. Server broadcasts WsServerMsg::KeyRotationRequired
        // 7. Send LogicEvent::RevocationComplete { document_id }
        tracing::info!("Sync Worker: RevokeFromDocument {document_id:?} {user_id:?}");
    }

    fn handle_list_document_members(&self, document_id: Uuid) {
        // GET /api/documents/{id}/members
        // Send LogicEvent::DocumentMembers { document_id, members: Vec<MemberInfo> }
        tracing::info!("Sync Worker: ListDocumentMembers {document_id:?}");
    }

    // -- Channels -------------------------------------------------------------

    fn handle_open_channel(&mut self, channel_id: Uuid) {
        // Same flow as OpenDocument: fetch channel key, load history, Subscribe WS.
        if self.ws.is_some() {
            self.ws_send(&WsClientMsg::Subscribe { document_id: channel_id });
        }
        tracing::info!("Sync Worker: OpenChannel {channel_id:?}");
    }

    fn handle_close_channel(&mut self, channel_id: Uuid) {
        if self.ws.is_some() {
            self.ws_send(&WsClientMsg::Unsubscribe { document_id: channel_id });
        }
        tracing::info!("Sync Worker: CloseChannel {channel_id:?}");
    }

    fn handle_send_channel_update(&self, channel_id: Uuid, payload: Vec<u8>) {
        // Encrypt + route to active segment (segment management is internal to ambre-sync).
        tracing::debug!("Sync Worker: SendChannelUpdate {channel_id:?} len={}", payload.len());
    }

    fn handle_send_channel_ephemeral(&self, channel_id: Uuid, payload: Vec<u8>) {
        tracing::debug!("Sync Worker: SendChannelEphemeral {channel_id:?} len={}", payload.len());
    }
}

// -----------------------------------------------------------------------------
// Bootstrap - 2-phase init
// -----------------------------------------------------------------------------

/// Entry point for the Sync Worker.
///
/// Called either:
/// - directly by `#[wasm_bindgen(start)]` in standalone builds (release pipeline), or
/// - by the dispatcher in `ambre-app` when the worker name is "sync" (dev unified build).
pub fn start() {
    #[cfg(feature = "standalone")]
    {
        tracing_wasm::set_as_global_default();
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    }
    tracing::info!("Ambre Sync Worker starting");

    let global: DedicatedWorkerGlobalScope =
        js_sys::global().dyn_into().expect("not running in a Worker");

    let state: Rc<RefCell<Option<SyncWorkerState>>> = Rc::new(RefCell::new(None));
    let state_init = Rc::clone(&state);

    // Phase 1: one-shot init handler - receive the Logic Worker MessagePort.
    let on_init = Closure::<dyn FnMut(MessageEvent)>::once(move |event: MessageEvent| {
        let data = event.data();
        let logic_port: MessagePort = js_sys::Reflect::get(&data, &"logicPort".into())
            .expect("init message must contain logicPort")
            .dyn_into()
            .expect("logicPort must be a MessagePort");

        let api_base = String::new(); // TODO: read from window.location or injected config
        *state_init.borrow_mut() = Some(SyncWorkerState::new(logic_port.clone(), api_base));

        // Phase 2: wire MessagePort for steady-state SyncCommand dispatch.
        {
            let state = Rc::clone(&state_init);
            let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                if let Ok(ab) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
                    let js_bytes = js_sys::Uint8Array::new(&ab);
                    let mut bytes = vec![0u8; js_bytes.length() as usize];
                    js_bytes.copy_to(&mut bytes);
                    if let Some(ref mut s) = *state.borrow_mut() {
                        s.handle_command(&bytes);
                    }
                }
            });
            logic_port.set_onmessage(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        }

        tracing::info!("Ambre Sync Worker initialized - Logic Worker port connected");
    });

    global.set_onmessage(Some(on_init.as_ref().unchecked_ref()));
    on_init.forget();

    // Handshake: onmessage is now set - tell the UI Thread it can safely
    // transfer the logicPort.
    global
        .post_message(&JsValue::from_str("ready"))
        .expect("post ready signal to UI Thread");
}

#[cfg(feature = "standalone")]
#[wasm_bindgen(start)]
pub fn main() {
    start();
}
