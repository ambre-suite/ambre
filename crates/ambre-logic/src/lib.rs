// ambre-logic/src/lib.rs
// ============================================================================
// Logic thread entry point — runs in a dedicated wasm_thread, shares the heap
// with the UI Thread and the Sync thread.
//
// Responsibilities:
//   - Own all LoroDoc instances via the AppController/ControllerRegistry.
//   - Compute viewport display lists (P1 < 15 ms, P2 < 50 ms).
//   - Dispatch UiEnvelope commands from the UI Thread.
//   - Forward SyncCommands to the Sync thread and dispatch LogicEvents back.
//
// Out of scope:
//   - Cryptography, compression, network — handled exclusively by ambre-sync.
//   - Rendering — handled by the UI Thread (wgpu + QuadRenderer + GlyphAtlas).
//
// Transport (post-Phase-3): native Rust channels carry typed Rust values.
// No postcard, no ArrayBuffer transfers, no MessagePort. The inter-thread
// payload schema for application commands (Vec<u8>) is still postcard-shaped
// internally to the application crates (ambre-trame, ambre-prose, ...) — that
// remains a per-app schema concern, not a transport one.
// ============================================================================

use std::collections::HashMap;

mod sink;

use ambre_bus::{
    AppController, DocType, IPC_TEST_DOC_ID, LogicEnvelope, LogicEvent, SyncCommand, UiEnvelope,
    Uuid,
};
use ambre_trame::TrameController;
use sink::SendCommandSink;

// -----------------------------------------------------------------------------
// LogicState
// -----------------------------------------------------------------------------

/// All mutable state owned by the Logic thread.
struct LogicState {
    /// Outbound channel: Logic → UI Thread.
    ui_tx: flume::Sender<LogicEnvelope>,
    /// Outbound channel: Logic → Sync thread. Wrapped in `Arc` and given to
    /// every controller as a `SendCommandSink` so they can emit `SyncCommand`s
    /// without holding a reference to this state.
    sync_tx: flume::Sender<SyncCommand>,
    /// Active document controllers, keyed by `document_id`.
    ///
    /// Inserted on the first [`LogicEvent::RemoteSnapshot`] for a document
    /// (or, for the IPC smoke-test, pre-seeded at bootstrap). Removed on
    /// `CloseDocument` / `DeleteDocument` / `Logout`.
    controllers: HashMap<Uuid, Box<dyn AppController>>,
    /// `DocType` carried by `DocumentCreated`, kept until the first snapshot
    /// arrives so the controller factory can pick the right impl.
    pending_doc_types: HashMap<Uuid, DocType>,
}

impl LogicState {
    fn new(
        ui_tx: flume::Sender<LogicEnvelope>,
        sync_tx: flume::Sender<SyncCommand>,
    ) -> Self {
        // Pre-seed a TrameController for the IPC smoke-test document so that
        // key events from the UI Thread are dispatched without going through
        // the full auth + document-open flow. Remove once that flow is wired.
        let mut controllers: HashMap<Uuid, Box<dyn AppController>> = HashMap::new();
        controllers.insert(
            IPC_TEST_DOC_ID,
            Box::new(TrameController::new(Box::new(SendCommandSink::new(sync_tx.clone())))),
        );

        Self {
            ui_tx,
            sync_tx,
            controllers,
            pending_doc_types: HashMap::new(),
        }
    }

    // -- Outbound: Logic → UI Thread ------------------------------------------

    fn send_to_ui(&self, envelope: LogicEnvelope) {
        if let Err(flume::SendError(env)) = self.ui_tx.send(envelope) {
            tracing::warn!(
                "Logic: UI receiver dropped, discarding {:?}",
                std::mem::discriminant(&env),
            );
        }
    }

    fn send_to_sync(&self, cmd: SyncCommand) {
        if let Err(flume::SendError(cmd)) = self.sync_tx.send(cmd) {
            tracing::warn!(
                "Logic: Sync receiver dropped, discarding {:?}",
                std::mem::discriminant(&cmd),
            );
        }
    }

    /// Build a fresh `SendCommandSink` for a new controller.
    fn new_sink(&self) -> Box<dyn ambre_bus::CommandSink> {
        Box::new(SendCommandSink::new(self.sync_tx.clone()))
    }

    // -- Controller factory ----------------------------------------------------

    /// Instantiate the right [`AppController`] for `doc_type`, wired to the
    /// Sync thread sink. Returns `None` for doc types not yet implemented.
    fn make_controller(&self, doc_type: DocType) -> Option<Box<dyn AppController>> {
        match doc_type {
            DocType::Trame => Some(Box::new(TrameController::new(self.new_sink()))),
            DocType::Prose | DocType::Toile | DocType::Murmure => {
                tracing::warn!(
                    "Logic: no controller implemented for {:?}, document will not render",
                    doc_type
                );
                None
            }
        }
    }

    // -- Inbound: UI Thread → Logic --------------------------------------------

    fn handle_ui_envelope(&mut self, envelope: UiEnvelope) {
        match envelope {
            // -- P1 — Viewport -------------------------------------------------
            UiEnvelope::ScrollTo {
                document_id,
                top_row,
                left_col,
                viewport_rows,
                viewport_cols,
            } => {
                if let Some(ctrl) = self.controllers.get_mut(&document_id) {
                    let scroll_cmd =
                        postcard::to_allocvec(&(top_row, left_col, viewport_rows, viewport_cols))
                            .expect("postcard serialize scroll params");
                    for payload in ctrl.handle_ui_command(scroll_cmd) {
                        self.send_to_ui(LogicEnvelope::ViewportUpdate {
                            document_id,
                            payload,
                        });
                    }
                }
            }

            // -- P2 — Application command --------------------------------------
            UiEnvelope::AppCommand { document_id, payload } => {
                if let Some(ctrl) = self.controllers.get_mut(&document_id) {
                    for resp in ctrl.handle_ui_command(payload) {
                        self.send_to_ui(LogicEnvelope::ViewportUpdate {
                            document_id,
                            payload: resp,
                        });
                    }
                }
            }

            // -- P3 — Session --------------------------------------------------
            UiEnvelope::Unlock { passphrase } => {
                self.send_to_sync(SyncCommand::Unlock { passphrase });
            }
            UiEnvelope::SetupAccount { passphrase } => {
                self.send_to_sync(SyncCommand::SetupAccount { passphrase });
            }
            UiEnvelope::ChangePassphrase { current_passphrase, new_passphrase } => {
                self.send_to_sync(SyncCommand::ChangePassphrase {
                    current_passphrase,
                    new_passphrase,
                });
            }
            UiEnvelope::RecoverAccount { recovery_words, new_passphrase } => {
                self.send_to_sync(SyncCommand::RecoverAccount { recovery_words, new_passphrase });
            }
            UiEnvelope::Logout => {
                // Drop all controllers before telling the Sync thread to log out,
                // so no in-flight controller can emit further SyncCommands.
                self.controllers.clear();
                self.send_to_sync(SyncCommand::Logout);
            }

            // -- P3 — Document CRUD --------------------------------------------
            UiEnvelope::ListDocuments => self.send_to_sync(SyncCommand::ListDocuments),
            UiEnvelope::CreateDocument { doc_type, name } => {
                self.send_to_sync(SyncCommand::CreateDocument { doc_type, name });
            }
            UiEnvelope::DeleteDocument { document_id } => {
                self.controllers.remove(&document_id);
                self.send_to_sync(SyncCommand::DeleteDocument { document_id });
            }
            UiEnvelope::RenameDocument { document_id, name } => {
                self.send_to_sync(SyncCommand::RenameDocument { document_id, name });
            }

            // -- P3 — Document sync session ------------------------------------
            UiEnvelope::OpenDocument { document_id } => {
                self.send_to_sync(SyncCommand::OpenDocument { document_id });
            }
            UiEnvelope::CloseDocument { document_id } => {
                self.controllers.remove(&document_id);
                self.send_to_sync(SyncCommand::CloseDocument { document_id });
            }

            // -- P3 — Sharing --------------------------------------------------
            UiEnvelope::InviteToDocument { document_id, invitee_email, role } => {
                self.send_to_sync(SyncCommand::InviteToDocument {
                    document_id,
                    invitee_email,
                    role,
                });
            }
            UiEnvelope::RevokeFromDocument { document_id, user_id } => {
                self.send_to_sync(SyncCommand::RevokeFromDocument { document_id, user_id });
            }
            UiEnvelope::ListDocumentMembers { document_id } => {
                self.send_to_sync(SyncCommand::ListDocumentMembers { document_id });
            }
            UiEnvelope::UpdateDocumentMemberRole { document_id, user_id, role } => {
                self.send_to_sync(SyncCommand::UpdateDocumentMemberRole {
                    document_id,
                    user_id,
                    role,
                });
            }

            // -- P3 — Users ----------------------------------------------------
            UiEnvelope::SearchUsers { query } => {
                self.send_to_sync(SyncCommand::SearchUsers { query });
            }

            // -- P3 — Media ----------------------------------------------------
            UiEnvelope::UploadMedia { document_id, data, mime_type } => {
                self.send_to_sync(SyncCommand::UploadMedia { document_id, data, mime_type });
            }
            UiEnvelope::DownloadMedia { document_id, content_hash } => {
                self.send_to_sync(SyncCommand::DownloadMedia { document_id, content_hash });
            }

            // -- P3 — Channels -------------------------------------------------
            UiEnvelope::ListChannels => self.send_to_sync(SyncCommand::ListChannels),
            UiEnvelope::CreateChannel { name } => {
                self.send_to_sync(SyncCommand::CreateChannel { name });
            }
            UiEnvelope::DeleteChannel { channel_id } => {
                self.send_to_sync(SyncCommand::DeleteChannel { channel_id });
            }
            UiEnvelope::OpenChannel { channel_id } => {
                self.send_to_sync(SyncCommand::OpenChannel { channel_id });
            }
            UiEnvelope::CloseChannel { channel_id } => {
                self.send_to_sync(SyncCommand::CloseChannel { channel_id });
            }
            UiEnvelope::InviteToChannel { channel_id, invitee_email, role } => {
                self.send_to_sync(SyncCommand::InviteToChannel {
                    channel_id,
                    invitee_email,
                    role,
                });
            }
            UiEnvelope::RevokeFromChannel { channel_id, user_id } => {
                self.send_to_sync(SyncCommand::RevokeFromChannel { channel_id, user_id });
            }
            UiEnvelope::ListChannelMembers { channel_id } => {
                self.send_to_sync(SyncCommand::ListChannelMembers { channel_id });
            }
            UiEnvelope::UpdateChannelMemberRole { channel_id, user_id, role } => {
                self.send_to_sync(SyncCommand::UpdateChannelMemberRole {
                    channel_id,
                    user_id,
                    role,
                });
            }
            UiEnvelope::SendChannelMessage { channel_id, payload } => {
                self.send_to_sync(SyncCommand::SendChannelUpdate { channel_id, payload });
            }
            UiEnvelope::UpdateReadCursor { channel_id, last_seq } => {
                self.send_to_sync(SyncCommand::UpdateReadCursor { channel_id, last_seq });
            }
            UiEnvelope::GetUnreadCounts => self.send_to_sync(SyncCommand::GetUnreadCounts),

            // -- P3 — Meetings -------------------------------------------------
            UiEnvelope::CreateMeeting { name } => {
                self.send_to_sync(SyncCommand::CreateMeeting { name });
            }
            UiEnvelope::JoinMeeting { meeting_id } => {
                self.send_to_sync(SyncCommand::JoinMeeting { meeting_id });
            }
            UiEnvelope::LeaveMeeting { meeting_id } => {
                self.send_to_sync(SyncCommand::LeaveMeeting { meeting_id });
            }
            UiEnvelope::EndMeeting { meeting_id } => {
                self.send_to_sync(SyncCommand::EndMeeting { meeting_id });
            }
        }
    }

    // -- Inbound: Sync → Logic -------------------------------------------------

    fn handle_logic_event(&mut self, event: LogicEvent) {
        match event {
            // -- Session -------------------------------------------------------
            LogicEvent::SessionReady { user_id, display_name, email } => {
                self.send_to_ui(LogicEnvelope::SessionReady { user_id, display_name, email });
            }
            LogicEvent::AccountSetupRequired => {
                self.send_to_ui(LogicEnvelope::AccountSetupRequired);
            }
            LogicEvent::AccountCreated { recovery_words } => {
                self.send_to_ui(LogicEnvelope::AccountCreated { recovery_words });
            }
            LogicEvent::PassphraseChanged => self.send_to_ui(LogicEnvelope::PassphraseChanged),
            LogicEvent::AccountRecovered => self.send_to_ui(LogicEnvelope::AccountRecovered),
            LogicEvent::LoggedOut => self.send_to_ui(LogicEnvelope::LoggedOut),

            // -- Document CRUD -------------------------------------------------
            LogicEvent::DocumentList { documents } => {
                self.send_to_ui(LogicEnvelope::DocumentList { documents });
            }
            LogicEvent::DocumentCreated { document_id, doc_type } => {
                self.pending_doc_types.insert(document_id, doc_type);
                self.send_to_ui(LogicEnvelope::DocumentCreated { document_id, doc_type });
            }
            LogicEvent::DocumentDeleted { document_id } => {
                self.send_to_ui(LogicEnvelope::DocumentDeleted { document_id });
            }
            LogicEvent::DocumentRenamed { document_id } => {
                self.send_to_ui(LogicEnvelope::DocumentRenamed { document_id });
            }

            // -- Real-time sync ------------------------------------------------
            LogicEvent::RemoteSnapshot { document_id, payload } => {
                if !self.controllers.contains_key(&document_id) {
                    if let Some(doc_type) = self.pending_doc_types.remove(&document_id) {
                        if let Some(ctrl) = self.make_controller(doc_type) {
                            self.controllers.insert(document_id, ctrl);
                        }
                    } else {
                        tracing::warn!(
                            "Logic: no pending doc_type for {document_id}, dropping snapshot"
                        );
                        return;
                    }
                }
                let Some(ctrl) = self.controllers.get_mut(&document_id) else {
                    return;
                };
                for resp in
                    ctrl.on_logic_event(LogicEvent::RemoteSnapshot { document_id, payload })
                {
                    self.send_to_ui(LogicEnvelope::ViewportUpdate { document_id, payload: resp });
                }
            }

            LogicEvent::RemoteUpdate { document_id, payload } => {
                if let Some(ctrl) = self.controllers.get_mut(&document_id) {
                    for resp in
                        ctrl.on_logic_event(LogicEvent::RemoteUpdate { document_id, payload })
                    {
                        self.send_to_ui(LogicEnvelope::ViewportUpdate {
                            document_id,
                            payload: resp,
                        });
                    }
                }
            }

            LogicEvent::RemoteEphemeral { document_id, payload } => {
                if let Some(ctrl) = self.controllers.get_mut(&document_id) {
                    for resp in
                        ctrl.on_logic_event(LogicEvent::RemoteEphemeral { document_id, payload })
                    {
                        self.send_to_ui(LogicEnvelope::PresenceUpdate {
                            document_id,
                            payload: resp,
                        });
                    }
                }
            }

            LogicEvent::SnapshotNeeded { document_id } => {
                if let Some(ctrl) = self.controllers.get(&document_id) {
                    let snapshot = ctrl.export_snapshot();
                    self.send_to_sync(SyncCommand::SendSnapshot {
                        document_id,
                        payload: snapshot,
                    });
                }
            }

            LogicEvent::ConnectionStatus { document_id, connected } => {
                self.send_to_ui(LogicEnvelope::ConnectionStatus { document_id, connected });
            }

            LogicEvent::SyncWarning { document_id, code, message } => {
                self.send_to_ui(LogicEnvelope::SyncWarning { document_id, code, message });
            }

            // -- Sharing -------------------------------------------------------
            LogicEvent::InviteSent { document_id, user_id, display_name } => {
                self.send_to_ui(LogicEnvelope::InviteSent {
                    document_id,
                    user_id,
                    display_name,
                });
            }
            LogicEvent::MemberRevoked { document_id, revoked_user_id } => {
                self.send_to_ui(LogicEnvelope::MemberRevoked {
                    document_id,
                    revoked_user_id,
                });
            }
            LogicEvent::RevocationComplete { document_id } => {
                self.send_to_ui(LogicEnvelope::RevocationComplete { document_id });
            }
            LogicEvent::DocumentMembers { document_id, members } => {
                self.send_to_ui(LogicEnvelope::DocumentMembers { document_id, members });
            }
            LogicEvent::MemberRoleUpdated { document_id, user_id, role } => {
                self.send_to_ui(LogicEnvelope::MemberRoleUpdated {
                    document_id,
                    user_id,
                    role,
                });
            }

            // -- Users ---------------------------------------------------------
            LogicEvent::UserSearchResults { users } => {
                self.send_to_ui(LogicEnvelope::UserSearchResults { users });
            }

            // -- Media ---------------------------------------------------------
            LogicEvent::MediaUploaded { document_id, content_hash } => {
                self.send_to_ui(LogicEnvelope::MediaUploaded { document_id, content_hash });
            }
            LogicEvent::MediaDownloaded { document_id, content_hash, data } => {
                self.send_to_ui(LogicEnvelope::MediaDownloaded {
                    document_id,
                    content_hash,
                    data,
                });
            }

            // -- Channels ------------------------------------------------------
            LogicEvent::ChannelList { channels } => {
                self.send_to_ui(LogicEnvelope::ChannelList { channels });
            }
            LogicEvent::ChannelCreated { channel_id } => {
                self.send_to_ui(LogicEnvelope::ChannelCreated { channel_id });
            }
            LogicEvent::ChannelDeleted { channel_id } => {
                self.send_to_ui(LogicEnvelope::ChannelDeleted { channel_id });
            }
            LogicEvent::ChannelInviteSent { channel_id, user_id } => {
                self.send_to_ui(LogicEnvelope::ChannelInviteSent { channel_id, user_id });
            }
            LogicEvent::ChannelMemberRevoked { channel_id, revoked_user_id } => {
                self.send_to_ui(LogicEnvelope::ChannelMemberRevoked {
                    channel_id,
                    revoked_user_id,
                });
            }
            LogicEvent::ChannelRevocationComplete { channel_id } => {
                self.send_to_ui(LogicEnvelope::ChannelRevocationComplete { channel_id });
            }
            LogicEvent::ChannelMembers { channel_id, members } => {
                self.send_to_ui(LogicEnvelope::ChannelMembers { channel_id, members });
            }
            LogicEvent::ChannelMemberRoleUpdated { channel_id, user_id, role } => {
                self.send_to_ui(LogicEnvelope::ChannelMemberRoleUpdated {
                    channel_id,
                    user_id,
                    role,
                });
            }
            LogicEvent::ChannelRemoteUpdate { channel_id, payload } => {
                self.send_to_ui(LogicEnvelope::ChannelMessage { channel_id, payload });
            }
            LogicEvent::ChannelRemoteEphemeral { channel_id, payload } => {
                self.send_to_ui(LogicEnvelope::ChannelPresenceUpdate { channel_id, payload });
            }
            LogicEvent::ChannelConnectionStatus { channel_id, connected } => {
                self.send_to_ui(LogicEnvelope::ChannelConnectionStatus {
                    channel_id,
                    connected,
                });
            }
            LogicEvent::ChannelSnapshotNeeded { channel_id: _ } => {
                tracing::warn!("Logic: ChannelSnapshotNeeded is not yet implemented");
            }
            LogicEvent::UnreadCounts { counts } => {
                self.send_to_ui(LogicEnvelope::UnreadCounts { counts });
            }

            // -- Meetings ------------------------------------------------------
            LogicEvent::MeetingCreated { meeting_id } => {
                self.send_to_ui(LogicEnvelope::MeetingCreated { meeting_id });
            }
            LogicEvent::MeetingJoined { meeting_id } => {
                self.send_to_ui(LogicEnvelope::MeetingJoined { meeting_id });
            }
            LogicEvent::MeetingLeft { meeting_id } => {
                self.send_to_ui(LogicEnvelope::MeetingLeft { meeting_id });
            }
            LogicEvent::MeetingEnded { meeting_id } => {
                self.send_to_ui(LogicEnvelope::MeetingEnded { meeting_id });
            }
            LogicEvent::MeetingParticipantJoined { meeting_id, user_id } => {
                self.send_to_ui(LogicEnvelope::MeetingParticipantJoined {
                    meeting_id,
                    user_id,
                });
            }
            LogicEvent::MeetingParticipantLeft { meeting_id, user_id } => {
                self.send_to_ui(LogicEnvelope::MeetingParticipantLeft { meeting_id, user_id });
            }

            // -- Errors --------------------------------------------------------
            LogicEvent::AuthError { message } => {
                self.send_to_ui(LogicEnvelope::AuthError { message });
            }
            LogicEvent::ApiError { request, code, message } => {
                self.send_to_ui(LogicEnvelope::ApiError { request, code, message });
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Entry point: blocking select on the two inbound channels.
// -----------------------------------------------------------------------------

/// Run the Logic thread to completion.
///
/// Blocks on whichever inbound channel has a message available, dispatches
/// it through [`LogicState`], and loops. Exits when **both** inbound channels
/// disconnect (i.e. the UI Thread and the Sync thread are both gone),
/// signalling app teardown.
pub fn run(
    ui_rx: flume::Receiver<UiEnvelope>,
    ui_tx: flume::Sender<LogicEnvelope>,
    sync_tx: flume::Sender<SyncCommand>,
    sync_rx: flume::Receiver<LogicEvent>,
) {
    tracing::info!("Ambre Logic thread starting");
    let mut state = LogicState::new(ui_tx, sync_tx);

    /// Result of one `Selector::wait` call. Returned by the per-channel
    /// closures so the outer `match` can dispatch with a single `&mut state`
    /// borrow — flume's `Selector` doesn't return the active branch, and the
    /// borrow checker rejects two closures both capturing `&mut state`.
    enum Inbound {
        Ui(UiEnvelope),
        Sync(LogicEvent),
        UiClosed,
        SyncClosed,
    }

    let mut ui_open = true;
    let mut sync_open = true;
    while ui_open || sync_open {
        // Block on whichever side has a message first. `Selector::wait`
        // uses `Atomics.wait` under the hood, which is legal off the
        // main thread.
        let inbound = flume::Selector::new()
            .recv(&ui_rx, |msg| match msg {
                Ok(env) => Inbound::Ui(env),
                Err(flume::RecvError::Disconnected) => Inbound::UiClosed,
            })
            .recv(&sync_rx, |msg| match msg {
                Ok(event) => Inbound::Sync(event),
                Err(flume::RecvError::Disconnected) => Inbound::SyncClosed,
            })
            .wait();

        match inbound {
            Inbound::Ui(env) => state.handle_ui_envelope(env),
            Inbound::Sync(event) => state.handle_logic_event(event),
            Inbound::UiClosed => ui_open = false,
            Inbound::SyncClosed => sync_open = false,
        }
    }

    tracing::info!("Ambre Logic thread exiting (both inbound channels disconnected)");
    // Drop `state` here: controllers are released, the outbound Senders are
    // closed, which the UI and Sync threads observe as `Disconnected` too.
    let _ = state;
}
