// ambre-logic/src/lib.rs
// ============================================================================
// Logic thread entry point — runs in a dedicated wasm_thread, shares the heap
// with the UI Thread and the Sync thread.
//
// Responsibilities:
//   - Own all per-document state via the typed `DocState` registry.
//   - Dispatch typed `LogicCommand`s from the UI Thread: `App` commands route
//     to the owning document's state; lifecycle commands forward to Sync.
//   - Dispatch `LogicEvent`s from the Sync thread back to the UI as `UiEvent`s.
//   - Compute viewport display events (P1 < 15 ms, P2 < 50 ms).
//
// Out of scope:
//   - Cryptography, compression, network — handled exclusively by ambre-sync.
//   - Rendering — handled by the UI Thread (wgpu + QuadRenderer + GlyphAtlas).
//
// Transport: native `flume` channels carry typed Rust values by move. No
// postcard, no `Any`, no `Vec<u8>` on the application data plane — `App`
// commands carry a typed `AppCommand`, viewport replies a typed `AppEvent`.
// CRDT deltas / snapshots stay `Vec<u8>` (genuine network bytes).
// ============================================================================

use std::collections::HashMap;

use ambre_bus::{
    AppCommand, AppEvent, DocType, DocumentId, IPC_TEST_DOC_ID, LogicCommand, LogicEvent,
    SyncCommand, UiEvent,
};
use ambre_trame::TrameState;

// -----------------------------------------------------------------------------
// DocState — typed per-document state
// -----------------------------------------------------------------------------

/// Per-document state, one variant per application document type.
///
/// A closed sum mirroring [`AppCommand`] / [`AppEvent`]: the dispatcher matches
/// the `(DocState, AppCommand)` pair to route a typed command to the right
/// document model — no trait object, no downcast.
enum DocState {
    Trame(TrameState),
    // Future: Prose(ProseState), Toile(ToileState), Murmure(MurmureState), …
}

impl DocState {
    /// Build the state for a freshly-opened document of `doc_type`.
    ///
    /// Returns `None` for document types whose logic is not yet implemented.
    fn new(doc_type: DocType) -> Option<Self> {
        match doc_type {
            DocType::Trame => Some(DocState::Trame(TrameState::new())),
            DocType::Prose | DocType::Toile | DocType::Murmure => {
                tracing::warn!(
                    "Logic: no state implemented for {:?}, document will not render",
                    doc_type
                );
                None
            }
        }
    }
}

// -----------------------------------------------------------------------------
// LogicState
// -----------------------------------------------------------------------------

/// All mutable state owned by the Logic thread.
struct LogicState {
    /// Outbound channel: Logic → UI Thread.
    to_ui: flume::Sender<UiEvent>,
    /// Outbound channel: Logic → Sync thread.
    to_sync: flume::Sender<SyncCommand>,
    /// Live document state, keyed by `document_id`.
    ///
    /// Inserted on the first [`LogicEvent::RemoteSnapshot`] for a document
    /// (or, for the IPC smoke-test, pre-seeded at bootstrap). Removed on
    /// `CloseDocument` / `DeleteDocument` / `Logout`.
    docs: HashMap<DocumentId, DocState>,
    /// `DocType` carried by `DocumentCreated`, kept until the first snapshot
    /// arrives so [`DocState::new`] can build the right state.
    pending_doc_types: HashMap<DocumentId, DocType>,
}

impl LogicState {
    fn new(to_ui: flume::Sender<UiEvent>, to_sync: flume::Sender<SyncCommand>) -> Self {
        // Pre-seed a Trame document for the IPC smoke-test so key events from
        // the UI Thread are dispatched without going through the full auth +
        // document-open flow. Remove once that flow is wired.
        let mut docs: HashMap<DocumentId, DocState> = HashMap::new();
        docs.insert(IPC_TEST_DOC_ID, DocState::Trame(TrameState::new()));

        Self { to_ui, to_sync, docs, pending_doc_types: HashMap::new() }
    }

    // -- Outbound --------------------------------------------------------------

    fn send_to_ui(&self, event: UiEvent) {
        if let Err(flume::SendError(event)) = self.to_ui.send(event) {
            tracing::warn!(
                "Logic: UI receiver dropped, discarding {:?}",
                std::mem::discriminant(&event),
            );
        }
    }

    fn send_to_sync(&self, cmd: SyncCommand) {
        if let Err(flume::SendError(cmd)) = self.to_sync.send(cmd) {
            tracing::warn!(
                "Logic: Sync receiver dropped, discarding {:?}",
                std::mem::discriminant(&cmd),
            );
        }
    }

    // -- Application data plane -------------------------------------------------

    /// Route a typed application command to its document's state and forward the
    /// resulting display events to the UI Thread.
    fn handle_app_command(&mut self, document_id: DocumentId, command: AppCommand) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            tracing::warn!("Logic: no live document {document_id} for app command");
            return;
        };

        let events: Vec<AppEvent> = match (doc, command) {
            (DocState::Trame(state), AppCommand::Trame(cmd)) => {
                state.handle_command(cmd).into_iter().map(AppEvent::Trame).collect()
            }
            // Once a second document type exists, add its `(DocState::X,
            // AppCommand::X)` arm plus a `_ => { warn; vec![] }` catch-all for
            // mismatched (state, command) pairs (impossible by routing).
        };

        for event in events {
            self.send_to_ui(UiEvent::Viewport { document_id, event });
        }
    }

    /// Apply a remote CRDT delta to a document and forward viewport events.
    fn apply_remote(&mut self, document_id: DocumentId, payload: &[u8]) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        let events: Vec<AppEvent> = match doc {
            DocState::Trame(state) => {
                state.apply_remote(payload).into_iter().map(AppEvent::Trame).collect()
            }
        };
        for event in events {
            self.send_to_ui(UiEvent::Viewport { document_id, event });
        }
    }

    /// Apply a remote ephemeral payload to a document and forward presence events.
    fn apply_ephemeral(&mut self, document_id: DocumentId, payload: &[u8]) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        let events: Vec<AppEvent> = match doc {
            DocState::Trame(state) => {
                state.apply_ephemeral(payload).into_iter().map(AppEvent::Trame).collect()
            }
        };
        for event in events {
            self.send_to_ui(UiEvent::Presence { document_id, event });
        }
    }

    /// Export a document's CRDT snapshot for compaction.
    fn export_snapshot(&self, document_id: DocumentId) -> Option<Vec<u8>> {
        match self.docs.get(&document_id)? {
            DocState::Trame(state) => Some(state.export_snapshot()),
        }
    }

    // -- Inbound: UI Thread → Logic --------------------------------------------

    fn handle_logic_command(&mut self, command: LogicCommand) {
        match command {
            // -- P2 — Application command --------------------------------------
            LogicCommand::App { document_id, command } => {
                self.handle_app_command(document_id, command);
            }

            // -- P3 — Session --------------------------------------------------
            LogicCommand::Unlock { passphrase } => {
                self.send_to_sync(SyncCommand::Unlock { passphrase });
            }
            LogicCommand::SetupAccount { passphrase } => {
                self.send_to_sync(SyncCommand::SetupAccount { passphrase });
            }
            LogicCommand::ChangePassphrase { current_passphrase, new_passphrase } => {
                self.send_to_sync(SyncCommand::ChangePassphrase {
                    current_passphrase,
                    new_passphrase,
                });
            }
            LogicCommand::RecoverAccount { recovery_words, new_passphrase } => {
                self.send_to_sync(SyncCommand::RecoverAccount { recovery_words, new_passphrase });
            }
            LogicCommand::Logout => {
                // Drop all document state before telling the Sync thread to log
                // out, so no in-flight handler can emit further SyncCommands.
                self.docs.clear();
                self.send_to_sync(SyncCommand::Logout);
            }

            // -- P3 — Document CRUD --------------------------------------------
            LogicCommand::ListDocuments => self.send_to_sync(SyncCommand::ListDocuments),
            LogicCommand::CreateDocument { doc_type, name } => {
                self.send_to_sync(SyncCommand::CreateDocument { doc_type, name });
            }
            LogicCommand::DeleteDocument { document_id } => {
                self.docs.remove(&document_id);
                self.send_to_sync(SyncCommand::DeleteDocument { document_id });
            }
            LogicCommand::RenameDocument { document_id, name } => {
                self.send_to_sync(SyncCommand::RenameDocument { document_id, name });
            }

            // -- P3 — Document sync session ------------------------------------
            LogicCommand::OpenDocument { document_id } => {
                self.send_to_sync(SyncCommand::OpenDocument { document_id });
            }
            LogicCommand::CloseDocument { document_id } => {
                self.docs.remove(&document_id);
                self.send_to_sync(SyncCommand::CloseDocument { document_id });
            }

            // -- P3 — Sharing --------------------------------------------------
            LogicCommand::InviteToDocument { document_id, invitee_email, role } => {
                self.send_to_sync(SyncCommand::InviteToDocument {
                    document_id,
                    invitee_email,
                    role,
                });
            }
            LogicCommand::RevokeFromDocument { document_id, user_id } => {
                self.send_to_sync(SyncCommand::RevokeFromDocument { document_id, user_id });
            }
            LogicCommand::ListDocumentMembers { document_id } => {
                self.send_to_sync(SyncCommand::ListDocumentMembers { document_id });
            }
            LogicCommand::UpdateDocumentMemberRole { document_id, user_id, role } => {
                self.send_to_sync(SyncCommand::UpdateDocumentMemberRole {
                    document_id,
                    user_id,
                    role,
                });
            }

            // -- P3 — Users ----------------------------------------------------
            LogicCommand::SearchUsers { query } => {
                self.send_to_sync(SyncCommand::SearchUsers { query });
            }

            // -- P3 — Media ----------------------------------------------------
            LogicCommand::UploadMedia { document_id, data, mime_type } => {
                self.send_to_sync(SyncCommand::UploadMedia { document_id, data, mime_type });
            }
            LogicCommand::DownloadMedia { document_id, content_hash } => {
                self.send_to_sync(SyncCommand::DownloadMedia { document_id, content_hash });
            }

            // -- P3 — Channels -------------------------------------------------
            LogicCommand::ListChannels => self.send_to_sync(SyncCommand::ListChannels),
            LogicCommand::CreateChannel { name } => {
                self.send_to_sync(SyncCommand::CreateChannel { name });
            }
            LogicCommand::DeleteChannel { channel_id } => {
                self.send_to_sync(SyncCommand::DeleteChannel { channel_id });
            }
            LogicCommand::OpenChannel { channel_id } => {
                self.send_to_sync(SyncCommand::OpenChannel { channel_id });
            }
            LogicCommand::CloseChannel { channel_id } => {
                self.send_to_sync(SyncCommand::CloseChannel { channel_id });
            }
            LogicCommand::InviteToChannel { channel_id, invitee_email, role } => {
                self.send_to_sync(SyncCommand::InviteToChannel {
                    channel_id,
                    invitee_email,
                    role,
                });
            }
            LogicCommand::RevokeFromChannel { channel_id, user_id } => {
                self.send_to_sync(SyncCommand::RevokeFromChannel { channel_id, user_id });
            }
            LogicCommand::ListChannelMembers { channel_id } => {
                self.send_to_sync(SyncCommand::ListChannelMembers { channel_id });
            }
            LogicCommand::UpdateChannelMemberRole { channel_id, user_id, role } => {
                self.send_to_sync(SyncCommand::UpdateChannelMemberRole {
                    channel_id,
                    user_id,
                    role,
                });
            }
            LogicCommand::SendChannelMessage { channel_id, payload } => {
                self.send_to_sync(SyncCommand::SendChannelUpdate { channel_id, payload });
            }
            LogicCommand::UpdateReadCursor { channel_id, last_seq } => {
                self.send_to_sync(SyncCommand::UpdateReadCursor { channel_id, last_seq });
            }
            LogicCommand::GetUnreadCounts => self.send_to_sync(SyncCommand::GetUnreadCounts),

            // -- P3 — Meetings -------------------------------------------------
            LogicCommand::CreateMeeting { name } => {
                self.send_to_sync(SyncCommand::CreateMeeting { name });
            }
            LogicCommand::JoinMeeting { meeting_id } => {
                self.send_to_sync(SyncCommand::JoinMeeting { meeting_id });
            }
            LogicCommand::LeaveMeeting { meeting_id } => {
                self.send_to_sync(SyncCommand::LeaveMeeting { meeting_id });
            }
            LogicCommand::EndMeeting { meeting_id } => {
                self.send_to_sync(SyncCommand::EndMeeting { meeting_id });
            }
        }
    }

    // -- Inbound: Sync → Logic -------------------------------------------------

    fn handle_logic_event(&mut self, event: LogicEvent) {
        match event {
            // -- Session -------------------------------------------------------
            LogicEvent::SessionReady { user_id, display_name, email } => {
                self.send_to_ui(UiEvent::SessionReady { user_id, display_name, email });
            }
            LogicEvent::AccountSetupRequired => {
                self.send_to_ui(UiEvent::AccountSetupRequired);
            }
            LogicEvent::AccountCreated { recovery_words } => {
                self.send_to_ui(UiEvent::AccountCreated { recovery_words });
            }
            LogicEvent::PassphraseChanged => self.send_to_ui(UiEvent::PassphraseChanged),
            LogicEvent::AccountRecovered => self.send_to_ui(UiEvent::AccountRecovered),
            LogicEvent::LoggedOut => self.send_to_ui(UiEvent::LoggedOut),

            // -- Document CRUD -------------------------------------------------
            LogicEvent::DocumentList { documents } => {
                self.send_to_ui(UiEvent::DocumentList { documents });
            }
            LogicEvent::DocumentCreated { document_id, doc_type } => {
                self.pending_doc_types.insert(document_id, doc_type);
                self.send_to_ui(UiEvent::DocumentCreated { document_id, doc_type });
            }
            LogicEvent::DocumentDeleted { document_id } => {
                self.send_to_ui(UiEvent::DocumentDeleted { document_id });
            }
            LogicEvent::DocumentRenamed { document_id } => {
                self.send_to_ui(UiEvent::DocumentRenamed { document_id });
            }

            // -- Real-time sync ------------------------------------------------
            LogicEvent::RemoteSnapshot { document_id, payload } => {
                if !self.docs.contains_key(&document_id) {
                    match self.pending_doc_types.remove(&document_id).and_then(DocState::new) {
                        Some(state) => {
                            self.docs.insert(document_id, state);
                        }
                        None => {
                            tracing::warn!(
                                "Logic: no pending doc_type for {document_id}, dropping snapshot"
                            );
                            return;
                        }
                    }
                }
                self.apply_remote(document_id, &payload);
            }

            LogicEvent::RemoteUpdate { document_id, payload } => {
                self.apply_remote(document_id, &payload);
            }

            LogicEvent::RemoteEphemeral { document_id, payload } => {
                self.apply_ephemeral(document_id, &payload);
            }

            LogicEvent::SnapshotNeeded { document_id } => {
                if let Some(payload) = self.export_snapshot(document_id) {
                    self.send_to_sync(SyncCommand::SendSnapshot { document_id, payload });
                }
            }

            LogicEvent::ConnectionStatus { document_id, connected } => {
                self.send_to_ui(UiEvent::ConnectionStatus { document_id, connected });
            }

            LogicEvent::SyncWarning { document_id, code, message } => {
                self.send_to_ui(UiEvent::SyncWarning { document_id, code, message });
            }

            // -- Sharing -------------------------------------------------------
            LogicEvent::InviteSent { document_id, user_id, display_name } => {
                self.send_to_ui(UiEvent::InviteSent { document_id, user_id, display_name });
            }
            LogicEvent::MemberRevoked { document_id, revoked_user_id } => {
                self.send_to_ui(UiEvent::MemberRevoked { document_id, revoked_user_id });
            }
            LogicEvent::RevocationComplete { document_id } => {
                self.send_to_ui(UiEvent::RevocationComplete { document_id });
            }
            LogicEvent::DocumentMembers { document_id, members } => {
                self.send_to_ui(UiEvent::DocumentMembers { document_id, members });
            }
            LogicEvent::MemberRoleUpdated { document_id, user_id, role } => {
                self.send_to_ui(UiEvent::MemberRoleUpdated { document_id, user_id, role });
            }

            // -- Users ---------------------------------------------------------
            LogicEvent::UserSearchResults { users } => {
                self.send_to_ui(UiEvent::UserSearchResults { users });
            }

            // -- Media ---------------------------------------------------------
            LogicEvent::MediaUploaded { document_id, content_hash } => {
                self.send_to_ui(UiEvent::MediaUploaded { document_id, content_hash });
            }
            LogicEvent::MediaDownloaded { document_id, content_hash, data } => {
                self.send_to_ui(UiEvent::MediaDownloaded { document_id, content_hash, data });
            }

            // -- Channels ------------------------------------------------------
            LogicEvent::ChannelList { channels } => {
                self.send_to_ui(UiEvent::ChannelList { channels });
            }
            LogicEvent::ChannelCreated { channel_id } => {
                self.send_to_ui(UiEvent::ChannelCreated { channel_id });
            }
            LogicEvent::ChannelDeleted { channel_id } => {
                self.send_to_ui(UiEvent::ChannelDeleted { channel_id });
            }
            LogicEvent::ChannelInviteSent { channel_id, user_id } => {
                self.send_to_ui(UiEvent::ChannelInviteSent { channel_id, user_id });
            }
            LogicEvent::ChannelMemberRevoked { channel_id, revoked_user_id } => {
                self.send_to_ui(UiEvent::ChannelMemberRevoked { channel_id, revoked_user_id });
            }
            LogicEvent::ChannelRevocationComplete { channel_id } => {
                self.send_to_ui(UiEvent::ChannelRevocationComplete { channel_id });
            }
            LogicEvent::ChannelMembers { channel_id, members } => {
                self.send_to_ui(UiEvent::ChannelMembers { channel_id, members });
            }
            LogicEvent::ChannelMemberRoleUpdated { channel_id, user_id, role } => {
                self.send_to_ui(UiEvent::ChannelMemberRoleUpdated { channel_id, user_id, role });
            }
            LogicEvent::ChannelRemoteUpdate { channel_id, payload } => {
                self.send_to_ui(UiEvent::ChannelMessage { channel_id, payload });
            }
            LogicEvent::ChannelRemoteEphemeral { channel_id, payload } => {
                self.send_to_ui(UiEvent::ChannelPresenceUpdate { channel_id, payload });
            }
            LogicEvent::ChannelConnectionStatus { channel_id, connected } => {
                self.send_to_ui(UiEvent::ChannelConnectionStatus { channel_id, connected });
            }
            LogicEvent::ChannelSnapshotNeeded { channel_id: _ } => {
                tracing::warn!("Logic: ChannelSnapshotNeeded is not yet implemented");
            }
            LogicEvent::UnreadCounts { counts } => {
                self.send_to_ui(UiEvent::UnreadCounts { counts });
            }

            // -- Meetings ------------------------------------------------------
            LogicEvent::MeetingCreated { meeting_id } => {
                self.send_to_ui(UiEvent::MeetingCreated { meeting_id });
            }
            LogicEvent::MeetingJoined { meeting_id } => {
                self.send_to_ui(UiEvent::MeetingJoined { meeting_id });
            }
            LogicEvent::MeetingLeft { meeting_id } => {
                self.send_to_ui(UiEvent::MeetingLeft { meeting_id });
            }
            LogicEvent::MeetingEnded { meeting_id } => {
                self.send_to_ui(UiEvent::MeetingEnded { meeting_id });
            }
            LogicEvent::MeetingParticipantJoined { meeting_id, user_id } => {
                self.send_to_ui(UiEvent::MeetingParticipantJoined { meeting_id, user_id });
            }
            LogicEvent::MeetingParticipantLeft { meeting_id, user_id } => {
                self.send_to_ui(UiEvent::MeetingParticipantLeft { meeting_id, user_id });
            }

            // -- Errors --------------------------------------------------------
            LogicEvent::AuthError { message } => {
                self.send_to_ui(UiEvent::AuthError { message });
            }
            LogicEvent::ApiError { request, code, message } => {
                self.send_to_ui(UiEvent::ApiError { request, code, message });
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Entry point: blocking select on the two inbound channels.
// -----------------------------------------------------------------------------

/// Run the Logic thread to completion.
///
/// Blocks on whichever inbound channel has a message available, dispatches it
/// through [`LogicState`], and loops. Exits when **both** inbound channels
/// disconnect (i.e. the UI Thread and the Sync thread are both gone),
/// signalling app teardown.
pub fn run(
    from_ui: flume::Receiver<LogicCommand>,
    to_ui: flume::Sender<UiEvent>,
    to_sync: flume::Sender<SyncCommand>,
    from_sync: flume::Receiver<LogicEvent>,
) {
    tracing::info!("Ambre Logic thread starting");
    let mut state = LogicState::new(to_ui, to_sync);

    /// Result of one `Selector::wait` call. Returned by the per-channel
    /// closures so the outer `match` can dispatch with a single `&mut state`
    /// borrow — flume's `Selector` doesn't return the active branch, and the
    /// borrow checker rejects two closures both capturing `&mut state`.
    enum Inbound {
        Ui(LogicCommand),
        Sync(LogicEvent),
        UiClosed,
        SyncClosed,
    }

    let mut ui_open = true;
    let mut sync_open = true;
    while ui_open || sync_open {
        // Block on whichever side has a message first. `Selector::wait` uses
        // `Atomics.wait` under the hood, which is legal off the main thread.
        let inbound = flume::Selector::new()
            .recv(&from_ui, |msg| match msg {
                Ok(command) => Inbound::Ui(command),
                Err(flume::RecvError::Disconnected) => Inbound::UiClosed,
            })
            .recv(&from_sync, |msg| match msg {
                Ok(event) => Inbound::Sync(event),
                Err(flume::RecvError::Disconnected) => Inbound::SyncClosed,
            })
            .wait();

        match inbound {
            Inbound::Ui(command) => state.handle_logic_command(command),
            Inbound::Sync(event) => state.handle_logic_event(event),
            Inbound::UiClosed => ui_open = false,
            Inbound::SyncClosed => sync_open = false,
        }
    }

    tracing::info!("Ambre Logic thread exiting (both inbound channels disconnected)");
    // Drop `state` here: document state is released, the outbound Senders are
    // closed, which the UI and Sync threads observe as `Disconnected` too.
    let _ = state;
}
