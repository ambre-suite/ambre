// ambre-logic-worker/src/lib.rs
// ============================================================================
// Logic Worker entry point — dedicated Web Worker, isolated linear memory.
//
// Responsibilities:
//   - Own all LoroDoc instances via the AppController/ControllerRegistry.
//   - Compute viewport display lists (P1 < 15 ms, P2 < 50 ms).
//   - Dispatch UiEnvelope commands from the UI Thread.
//   - Forward SyncCommands to the Sync Worker and dispatch LogicEvents back.
//
// Out of scope:
//   - Cryptography, compression, network — handled exclusively by the Sync Worker.
//   - Rendering — handled by the UI Thread (Vello + wgpu).
//
// IPC topology:
//   UI Thread    ---UiEnvelope (postMessage)---->  Logic Worker
//   UI Thread    <---LogicEnvelope (postMessage)-- Logic Worker
//   Logic Worker ---SyncCommand (MessagePort)---->  Sync Worker
//   Logic Worker <---LogicEvent (MessagePort)----  Sync Worker
//
// Bootstrap (2-phase):
//   Phase 1 — global.onmessage (once): receive the MessagePort to the Sync Worker.
//   Phase 2 — global.onmessage (steady-state): dispatch UiEnvelope messages.
//   The Sync Worker MessagePort is wired immediately after Phase 1.
// ============================================================================

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent, MessagePort};

use ambre_bus::{
    AppController, CommandSink, DocType, LogicEnvelope, LogicEvent, SyncCommand, UiEnvelope, Uuid,
};

// ─────────────────────────────────────────────────────────────────────────────
// PortCommandSink — concrete CommandSink backed by a Web Worker MessagePort
// ─────────────────────────────────────────────────────────────────────────────

/// Sends [`SyncCommand`]s to the Sync Worker via the `MessagePort` established
/// during the bootstrap handshake.
///
/// Each call serializes `cmd` with postcard, copies the bytes into a JS
/// `ArrayBuffer`, and transfers it (zero-copy on the JS side) via
/// `port.postMessage(..., [arrayBuffer])`.
struct PortCommandSink {
    port: MessagePort,
}

impl CommandSink for PortCommandSink {
    fn send(&self, cmd: SyncCommand) {
        let bytes = postcard::to_allocvec(&cmd).expect("postcard serialize SyncCommand");
        let js_bytes = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
        js_bytes.copy_from(&bytes);
        let ab = js_bytes.buffer();
        let transfer = js_sys::Array::of1(&ab);
        self.port
            .post_message_with_transferable(&ab, &transfer)
            .expect("postMessage SyncCommand to Sync Worker");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// LogicWorkerState
// ─────────────────────────────────────────────────────────────────────────────

/// All mutable state owned by the Logic Worker after bootstrap.
struct LogicWorkerState {
    /// Handle to the global Worker scope, used to `postMessage` to the UI Thread.
    global: DedicatedWorkerGlobalScope,
    /// Sink to the Sync Worker (via the `MessagePort` received during bootstrap).
    sync_sink: Rc<PortCommandSink>,
    /// Active document controllers, keyed by `document_id`.
    ///
    /// A controller is inserted when the first [`LogicEvent::RemoteSnapshot`]
    /// arrives for a document (via `ControllerRegistry::create`, not yet wired)
    /// and removed on `CloseDocument` or `DeleteDocument`.
    controllers: HashMap<Uuid, Box<dyn AppController>>,
}

impl LogicWorkerState {
    fn new(global: DedicatedWorkerGlobalScope, sync_port: MessagePort) -> Self {
        Self {
            global,
            sync_sink: Rc::new(PortCommandSink { port: sync_port }),
            controllers: HashMap::new(),
        }
    }

    // ── Outbound: Logic Worker → UI Thread ────────────────────────────────────

    /// Serialize `envelope` with postcard and transfer it to the UI Thread.
    fn send_to_ui(&self, envelope: &LogicEnvelope) {
        let bytes = postcard::to_allocvec(envelope).expect("postcard serialize LogicEnvelope");
        let js_bytes = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
        js_bytes.copy_from(&bytes);
        let ab = js_bytes.buffer();
        let transfer = js_sys::Array::of1(&ab);
        self.global
            .post_message_with_transfer(&ab, &transfer)
            .expect("postMessage LogicEnvelope to UI Thread");
    }

    // ── Inbound: UI Thread → Logic Worker ────────────────────────────────────

    /// Deserialize a [`UiEnvelope`] from `bytes` and dispatch it.
    fn handle_ui_envelope(&mut self, bytes: &[u8]) {
        let envelope: UiEnvelope =
            postcard::from_bytes(bytes).expect("postcard deserialize UiEnvelope");

        match envelope {
            // ── P1 — Viewport ─────────────────────────────────────────────────
            UiEnvelope::ScrollTo {
                document_id,
                top_row,
                left_col,
                viewport_rows,
                viewport_cols,
            } => {
                // Serialize scroll parameters for the AppController.
                // The controller receives a (top_row, left_col, rows, cols) tuple
                // and returns postcard-serialized display lists.
                if let Some(ctrl) = self.controllers.get_mut(&document_id) {
                    let scroll_cmd =
                        postcard::to_allocvec(&(top_row, left_col, viewport_rows, viewport_cols))
                            .expect("postcard serialize scroll params");
                    for payload in ctrl.handle_ui_command(scroll_cmd) {
                        self.send_to_ui(&LogicEnvelope::ViewportUpdate {
                            document_id,
                            payload,
                        });
                    }
                }
            }

            // ── P2 — Application command ───────────────────────────────────────
            UiEnvelope::AppCommand { document_id, payload } => {
                // Dispatch the opaque command to the registered AppController.
                // The controller mutates its LoroDoc, sends a SyncCommand::SendUpdate
                // via its CommandSink, and returns display updates for the UI.
                if let Some(ctrl) = self.controllers.get_mut(&document_id) {
                    for resp in ctrl.handle_ui_command(payload) {
                        self.send_to_ui(&LogicEnvelope::ViewportUpdate {
                            document_id,
                            payload: resp,
                        });
                    }
                }
            }

            // ── P3 — Session ───────────────────────────────────────────────────
            UiEnvelope::Unlock { passphrase } => {
                self.sync_sink.send(SyncCommand::Unlock { passphrase });
            }
            UiEnvelope::SetupAccount { passphrase } => {
                self.sync_sink.send(SyncCommand::SetupAccount { passphrase });
            }
            UiEnvelope::ChangePassphrase { current_passphrase, new_passphrase } => {
                self.sync_sink
                    .send(SyncCommand::ChangePassphrase { current_passphrase, new_passphrase });
            }
            UiEnvelope::RecoverAccount { recovery_words, new_passphrase } => {
                self.sync_sink
                    .send(SyncCommand::RecoverAccount { recovery_words, new_passphrase });
            }
            UiEnvelope::Logout => {
                // Drop all controllers before telling the Sync Worker to log out,
                // so no in-flight controller can emit further SyncCommands.
                self.controllers.clear();
                self.sync_sink.send(SyncCommand::Logout);
            }

            // ── P3 — Document CRUD ─────────────────────────────────────────────
            UiEnvelope::ListDocuments => {
                self.sync_sink.send(SyncCommand::ListDocuments);
            }
            UiEnvelope::CreateDocument { doc_type, name } => {
                self.sync_sink.send(SyncCommand::CreateDocument { doc_type, name });
            }
            UiEnvelope::DeleteDocument { document_id } => {
                self.controllers.remove(&document_id);
                self.sync_sink.send(SyncCommand::DeleteDocument { document_id });
            }
            UiEnvelope::RenameDocument { document_id, name } => {
                self.sync_sink.send(SyncCommand::RenameDocument { document_id, name });
            }

            // ── P3 — Document sync session ─────────────────────────────────────
            UiEnvelope::OpenDocument { document_id } => {
                // The controller is created when the first RemoteSnapshot arrives,
                // not here, because we need the doc_type from the snapshot metadata.
                self.sync_sink.send(SyncCommand::OpenDocument { document_id });
            }
            UiEnvelope::CloseDocument { document_id } => {
                self.controllers.remove(&document_id);
                self.sync_sink.send(SyncCommand::CloseDocument { document_id });
            }

            // ── P3 — Sharing ───────────────────────────────────────────────────
            UiEnvelope::InviteToDocument { document_id, invitee_email, role } => {
                self.sync_sink
                    .send(SyncCommand::InviteToDocument { document_id, invitee_email, role });
            }
            UiEnvelope::RevokeFromDocument { document_id, user_id } => {
                self.sync_sink.send(SyncCommand::RevokeFromDocument { document_id, user_id });
            }
            UiEnvelope::ListDocumentMembers { document_id } => {
                self.sync_sink.send(SyncCommand::ListDocumentMembers { document_id });
            }
            UiEnvelope::UpdateDocumentMemberRole { document_id, user_id, role } => {
                self.sync_sink.send(SyncCommand::UpdateDocumentMemberRole {
                    document_id,
                    user_id,
                    role,
                });
            }

            // ── P3 — Users ─────────────────────────────────────────────────────
            UiEnvelope::SearchUsers { query } => {
                self.sync_sink.send(SyncCommand::SearchUsers { query });
            }

            // ── P3 — Media ─────────────────────────────────────────────────────
            UiEnvelope::UploadMedia { document_id, data, mime_type } => {
                self.sync_sink.send(SyncCommand::UploadMedia { document_id, data, mime_type });
            }
            UiEnvelope::DownloadMedia { document_id, content_hash } => {
                self.sync_sink.send(SyncCommand::DownloadMedia { document_id, content_hash });
            }

            // ── P3 — Channels ──────────────────────────────────────────────────
            UiEnvelope::ListChannels => {
                self.sync_sink.send(SyncCommand::ListChannels);
            }
            UiEnvelope::CreateChannel { name } => {
                self.sync_sink.send(SyncCommand::CreateChannel { name });
            }
            UiEnvelope::DeleteChannel { channel_id } => {
                self.sync_sink.send(SyncCommand::DeleteChannel { channel_id });
            }
            UiEnvelope::OpenChannel { channel_id } => {
                self.sync_sink.send(SyncCommand::OpenChannel { channel_id });
            }
            UiEnvelope::CloseChannel { channel_id } => {
                self.sync_sink.send(SyncCommand::CloseChannel { channel_id });
            }
            UiEnvelope::InviteToChannel { channel_id, invitee_email, role } => {
                self.sync_sink
                    .send(SyncCommand::InviteToChannel { channel_id, invitee_email, role });
            }
            UiEnvelope::RevokeFromChannel { channel_id, user_id } => {
                self.sync_sink.send(SyncCommand::RevokeFromChannel { channel_id, user_id });
            }
            UiEnvelope::ListChannelMembers { channel_id } => {
                self.sync_sink.send(SyncCommand::ListChannelMembers { channel_id });
            }
            UiEnvelope::UpdateChannelMemberRole { channel_id, user_id, role } => {
                self.sync_sink.send(SyncCommand::UpdateChannelMemberRole {
                    channel_id,
                    user_id,
                    role,
                });
            }
            UiEnvelope::SendChannelMessage { channel_id, payload } => {
                // "Channel message" maps to SyncCommand::SendChannelUpdate.
                // Segment management (creation, sealing) is entirely internal
                // to ambre-sync; the Logic Worker just forwards the raw payload.
                self.sync_sink.send(SyncCommand::SendChannelUpdate { channel_id, payload });
            }
            UiEnvelope::UpdateReadCursor { channel_id, last_seq } => {
                self.sync_sink.send(SyncCommand::UpdateReadCursor { channel_id, last_seq });
            }
            UiEnvelope::GetUnreadCounts => {
                self.sync_sink.send(SyncCommand::GetUnreadCounts);
            }

            // ── P3 — Meetings ──────────────────────────────────────────────────
            UiEnvelope::CreateMeeting { name } => {
                self.sync_sink.send(SyncCommand::CreateMeeting { name });
            }
            UiEnvelope::JoinMeeting { meeting_id } => {
                self.sync_sink.send(SyncCommand::JoinMeeting { meeting_id });
            }
            UiEnvelope::LeaveMeeting { meeting_id } => {
                self.sync_sink.send(SyncCommand::LeaveMeeting { meeting_id });
            }
            UiEnvelope::EndMeeting { meeting_id } => {
                self.sync_sink.send(SyncCommand::EndMeeting { meeting_id });
            }
        }
    }

    // ── Inbound: Sync Worker → Logic Worker ──────────────────────────────────

    /// Deserialize a [`LogicEvent`] from `bytes` and dispatch it.
    ///
    /// Session and document lifecycle events are forwarded to the UI Thread as
    /// the corresponding [`LogicEnvelope`] variant. Document-scoped sync events
    /// (`RemoteSnapshot`, `RemoteUpdate`, `RemoteEphemeral`) are dispatched to
    /// the registered [`AppController`], which applies the CRDT delta and returns
    /// display updates for the UI Thread.
    fn handle_logic_event(&mut self, bytes: &[u8]) {
        let event: LogicEvent =
            postcard::from_bytes(bytes).expect("postcard deserialize LogicEvent");

        match event {
            // ── Session ───────────────────────────────────────────────────────
            LogicEvent::SessionReady { user_id, display_name, email } => {
                self.send_to_ui(&LogicEnvelope::SessionReady { user_id, display_name, email });
            }
            LogicEvent::AccountSetupRequired => {
                self.send_to_ui(&LogicEnvelope::AccountSetupRequired);
            }
            LogicEvent::AccountCreated { recovery_words } => {
                self.send_to_ui(&LogicEnvelope::AccountCreated { recovery_words });
            }
            LogicEvent::PassphraseChanged => {
                self.send_to_ui(&LogicEnvelope::PassphraseChanged);
            }
            LogicEvent::AccountRecovered => {
                self.send_to_ui(&LogicEnvelope::AccountRecovered);
            }
            LogicEvent::LoggedOut => {
                self.send_to_ui(&LogicEnvelope::LoggedOut);
            }

            // ── Document CRUD ─────────────────────────────────────────────────
            LogicEvent::DocumentList { documents } => {
                self.send_to_ui(&LogicEnvelope::DocumentList { documents });
            }
            LogicEvent::DocumentCreated { document_id, doc_type } => {
                self.send_to_ui(&LogicEnvelope::DocumentCreated { document_id, doc_type });
            }
            LogicEvent::DocumentDeleted { document_id } => {
                self.send_to_ui(&LogicEnvelope::DocumentDeleted { document_id });
            }
            LogicEvent::DocumentRenamed { document_id } => {
                self.send_to_ui(&LogicEnvelope::DocumentRenamed { document_id });
            }

            // ── Real-time sync ────────────────────────────────────────────────

            LogicEvent::RemoteSnapshot { document_id, payload } => {
                // The controller must already exist (created by ControllerRegistry
                // upon OpenDocument + DocumentCreated). If it doesn't, the snapshot
                // is silently dropped — the Sync Worker will re-send on reconnect.
                let Some(ctrl) = self.controllers.get_mut(&document_id) else {
                    log::warn!(
                        "Logic Worker: no controller for {document_id}, \
                         dropping RemoteSnapshot (ControllerRegistry not yet wired)"
                    );
                    return;
                };
                for resp in
                    ctrl.on_logic_event(LogicEvent::RemoteSnapshot { document_id, payload })
                {
                    self.send_to_ui(&LogicEnvelope::ViewportUpdate { document_id, payload: resp });
                }
            }

            LogicEvent::RemoteUpdate { document_id, payload } => {
                if let Some(ctrl) = self.controllers.get_mut(&document_id) {
                    for resp in
                        ctrl.on_logic_event(LogicEvent::RemoteUpdate { document_id, payload })
                    {
                        self.send_to_ui(&LogicEnvelope::ViewportUpdate {
                            document_id,
                            payload: resp,
                        });
                    }
                }
            }

            LogicEvent::RemoteEphemeral { document_id, payload } => {
                // Ephemeral data (cursor, presence) returns PresenceUpdate, not ViewportUpdate.
                if let Some(ctrl) = self.controllers.get_mut(&document_id) {
                    for resp in
                        ctrl.on_logic_event(LogicEvent::RemoteEphemeral { document_id, payload })
                    {
                        self.send_to_ui(&LogicEnvelope::PresenceUpdate {
                            document_id,
                            payload: resp,
                        });
                    }
                }
            }

            LogicEvent::SnapshotNeeded { document_id } => {
                // Export the full CRDT state and send it to the Sync Worker for
                // S3 compaction. The Sync Worker will compress + encrypt it.
                if let Some(ctrl) = self.controllers.get(&document_id) {
                    let snapshot = ctrl.export_snapshot();
                    self.sync_sink
                        .send(SyncCommand::SendSnapshot { document_id, payload: snapshot });
                }
            }

            LogicEvent::ConnectionStatus { document_id, connected } => {
                self.send_to_ui(&LogicEnvelope::ConnectionStatus { document_id, connected });
            }

            LogicEvent::SyncWarning { document_id, code, message } => {
                self.send_to_ui(&LogicEnvelope::SyncWarning { document_id, code, message });
            }

            // ── Sharing ───────────────────────────────────────────────────────
            LogicEvent::InviteSent { document_id, user_id, display_name } => {
                self.send_to_ui(&LogicEnvelope::InviteSent {
                    document_id,
                    user_id,
                    display_name,
                });
            }
            LogicEvent::MemberRevoked { document_id, revoked_user_id } => {
                self.send_to_ui(&LogicEnvelope::MemberRevoked { document_id, revoked_user_id });
            }
            LogicEvent::RevocationComplete { document_id } => {
                self.send_to_ui(&LogicEnvelope::RevocationComplete { document_id });
            }
            LogicEvent::DocumentMembers { document_id, members } => {
                self.send_to_ui(&LogicEnvelope::DocumentMembers { document_id, members });
            }
            LogicEvent::MemberRoleUpdated { document_id, user_id, role } => {
                self.send_to_ui(&LogicEnvelope::MemberRoleUpdated { document_id, user_id, role });
            }

            // ── Users ─────────────────────────────────────────────────────────
            LogicEvent::UserSearchResults { users } => {
                self.send_to_ui(&LogicEnvelope::UserSearchResults { users });
            }

            // ── Media ─────────────────────────────────────────────────────────
            LogicEvent::MediaUploaded { document_id, content_hash } => {
                self.send_to_ui(&LogicEnvelope::MediaUploaded { document_id, content_hash });
            }
            LogicEvent::MediaDownloaded { document_id, content_hash, data } => {
                self.send_to_ui(&LogicEnvelope::MediaDownloaded {
                    document_id,
                    content_hash,
                    data,
                });
            }

            // ── Channels ──────────────────────────────────────────────────────
            LogicEvent::ChannelList { channels } => {
                self.send_to_ui(&LogicEnvelope::ChannelList { channels });
            }
            LogicEvent::ChannelCreated { channel_id } => {
                self.send_to_ui(&LogicEnvelope::ChannelCreated { channel_id });
            }
            LogicEvent::ChannelDeleted { channel_id } => {
                self.send_to_ui(&LogicEnvelope::ChannelDeleted { channel_id });
            }
            LogicEvent::ChannelInviteSent { channel_id, user_id } => {
                self.send_to_ui(&LogicEnvelope::ChannelInviteSent { channel_id, user_id });
            }
            LogicEvent::ChannelMemberRevoked { channel_id, revoked_user_id } => {
                self.send_to_ui(&LogicEnvelope::ChannelMemberRevoked {
                    channel_id,
                    revoked_user_id,
                });
            }
            LogicEvent::ChannelRevocationComplete { channel_id } => {
                self.send_to_ui(&LogicEnvelope::ChannelRevocationComplete { channel_id });
            }
            LogicEvent::ChannelMembers { channel_id, members } => {
                self.send_to_ui(&LogicEnvelope::ChannelMembers { channel_id, members });
            }
            LogicEvent::ChannelMemberRoleUpdated { channel_id, user_id, role } => {
                self.send_to_ui(&LogicEnvelope::ChannelMemberRoleUpdated {
                    channel_id,
                    user_id,
                    role,
                });
            }
            LogicEvent::ChannelRemoteUpdate { channel_id, payload } => {
                self.send_to_ui(&LogicEnvelope::ChannelMessage { channel_id, payload });
            }
            LogicEvent::ChannelRemoteEphemeral { channel_id, payload } => {
                self.send_to_ui(&LogicEnvelope::ChannelPresenceUpdate { channel_id, payload });
            }
            LogicEvent::ChannelConnectionStatus { channel_id, connected } => {
                self.send_to_ui(&LogicEnvelope::ChannelConnectionStatus { channel_id, connected });
            }
            LogicEvent::ChannelSnapshotNeeded { channel_id: _ } => {
                // Channel snapshot compaction is not yet implemented.
                log::warn!("Logic Worker: ChannelSnapshotNeeded is not yet implemented");
            }
            LogicEvent::UnreadCounts { counts } => {
                self.send_to_ui(&LogicEnvelope::UnreadCounts { counts });
            }

            // ── Meetings ──────────────────────────────────────────────────────
            LogicEvent::MeetingCreated { meeting_id } => {
                self.send_to_ui(&LogicEnvelope::MeetingCreated { meeting_id });
            }
            LogicEvent::MeetingJoined { meeting_id } => {
                self.send_to_ui(&LogicEnvelope::MeetingJoined { meeting_id });
            }
            LogicEvent::MeetingLeft { meeting_id } => {
                self.send_to_ui(&LogicEnvelope::MeetingLeft { meeting_id });
            }
            LogicEvent::MeetingEnded { meeting_id } => {
                self.send_to_ui(&LogicEnvelope::MeetingEnded { meeting_id });
            }
            LogicEvent::MeetingParticipantJoined { meeting_id, user_id } => {
                self.send_to_ui(&LogicEnvelope::MeetingParticipantJoined { meeting_id, user_id });
            }
            LogicEvent::MeetingParticipantLeft { meeting_id, user_id } => {
                self.send_to_ui(&LogicEnvelope::MeetingParticipantLeft { meeting_id, user_id });
            }

            // ── Errors ────────────────────────────────────────────────────────
            LogicEvent::AuthError { message } => {
                self.send_to_ui(&LogicEnvelope::AuthError { message });
            }
            LogicEvent::ApiError { request, code, message } => {
                self.send_to_ui(&LogicEnvelope::ApiError { request, code, message });
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Bootstrap — 2-phase init
// ─────────────────────────────────────────────────────────────────────────────

/// Entry point for the Logic Worker.
///
/// Called either:
/// - directly by `#[wasm_bindgen(start)]` in standalone builds (release pipeline), or
/// - by the dispatcher in `ambre-app` when the worker name is "logic" (dev unified build).
pub fn start() {
    console_log::init_with_level(log::Level::Debug).ok();
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    log::info!("Ambre Logic Worker starting");

    let global: DedicatedWorkerGlobalScope =
        js_sys::global().dyn_into().expect("not running in a Worker");

    let state: Rc<RefCell<Option<LogicWorkerState>>> = Rc::new(RefCell::new(None));
    let state_init = Rc::clone(&state);
    let global_clone = global.clone();

    // Phase 1: one-shot init handler — receive the Sync Worker MessagePort.
    let on_init = Closure::<dyn FnMut(MessageEvent)>::once(move |event: MessageEvent| {
        let data = event.data();
        let sync_port: MessagePort = js_sys::Reflect::get(&data, &"syncPort".into())
            .expect("init message must contain syncPort")
            .dyn_into()
            .expect("syncPort must be a MessagePort");

        *state_init.borrow_mut() =
            Some(LogicWorkerState::new(global_clone.clone(), sync_port.clone()));

        // Wire: Sync Worker → Logic Worker (via MessagePort).
        {
            let state = Rc::clone(&state_init);
            let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                if let Ok(ab) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
                    let js_bytes = js_sys::Uint8Array::new(&ab);
                    let mut bytes = vec![0u8; js_bytes.length() as usize];
                    js_bytes.copy_to(&mut bytes);
                    if let Some(ref mut s) = *state.borrow_mut() {
                        s.handle_logic_event(&bytes);
                    }
                }
            });
            sync_port.set_onmessage(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        }

        // Phase 2: replace global.onmessage with the steady-state UI handler.
        {
            let state = Rc::clone(&state_init);
            let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                if let Ok(ab) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
                    let js_bytes = js_sys::Uint8Array::new(&ab);
                    let mut bytes = vec![0u8; js_bytes.length() as usize];
                    js_bytes.copy_to(&mut bytes);
                    if let Some(ref mut s) = *state.borrow_mut() {
                        s.handle_ui_envelope(&bytes);
                    }
                }
            });
            global_clone.set_onmessage(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        }

        log::info!("Ambre Logic Worker initialized");
    });

    global.set_onmessage(Some(on_init.as_ref().unchecked_ref()));
    on_init.forget();

    // Handshake: onmessage is now set — tell the UI Thread it can safely
    // transfer the syncPort. The UI Thread's one-shot ready handler will
    // respond with { syncPort } and then replace itself with the steady-state
    // LogicEnvelope handler.
    global
        .post_message(&JsValue::from_str("ready"))
        .expect("post ready signal to UI Thread");
}

#[cfg(feature = "standalone")]
#[wasm_bindgen(start)]
pub fn main() {
    start();
}
