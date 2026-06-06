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
    RestCommand, RestEvent, SyncCommand, UiEvent,
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

    /// Route a typed application command to its document's state, send display
    /// events to the UI Thread, and forward CRDT/ephemeral payloads to Sync.
    fn handle_app_command(&mut self, document_id: DocumentId, command: AppCommand) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            tracing::warn!("Logic: no live document {document_id} for app command");
            return;
        };

        match (doc, command) {
            (DocState::Trame(state), AppCommand::Trame(cmd)) => {
                let outcome = state.handle_command(cmd);
                for event in outcome.display {
                    self.send_to_ui(UiEvent::Viewport {
                        document_id,
                        event: AppEvent::Trame(event),
                    });
                }
                if let Some(payload) = outcome.update {
                    self.send_to_sync(SyncCommand::SendUpdate { document_id, payload });
                }
                if let Some(payload) = outcome.ephemeral {
                    self.send_to_sync(SyncCommand::SendEphemeral { document_id, payload });
                }
            }
            // Once a second document type exists, add its `(DocState::X,
            // AppCommand::X)` arm plus a `_ => { warn; }` catch-all for
            // mismatched (state, command) pairs (impossible by routing).
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
            // -- P2 — Content plane --------------------------------------------
            LogicCommand::App { document_id, command } => {
                self.handle_app_command(document_id, command);
            }

            // -- P3 — Service plane: forward verbatim, with state hooks --------
            //
            // The whole relayed request/response family collapses to one arm.
            // A few ops also mutate Logic state before forwarding; those are the
            // only special cases (everything else is a pure relay).
            LogicCommand::Rest { id, cmd } => {
                match &cmd {
                    // Drop all document state before logging out so no in-flight
                    // handler can emit further commands against it.
                    RestCommand::Logout => self.docs.clear(),
                    RestCommand::DeleteDocument { document_id } => {
                        self.docs.remove(document_id);
                    }
                    _ => {}
                }
                self.send_to_sync(SyncCommand::Rest { id, cmd });
            }

            // -- P3 — Control plane: explicit (no correlatable response) -------
            LogicCommand::OpenDocument { document_id } => {
                self.send_to_sync(SyncCommand::OpenDocument { document_id });
            }
            LogicCommand::CloseDocument { document_id } => {
                self.docs.remove(&document_id);
                self.send_to_sync(SyncCommand::CloseDocument { document_id });
            }
            LogicCommand::OpenChannel { channel_id } => {
                self.send_to_sync(SyncCommand::OpenChannel { channel_id });
            }
            LogicCommand::CloseChannel { channel_id } => {
                self.send_to_sync(SyncCommand::CloseChannel { channel_id });
            }
            LogicCommand::SendChannelMessage { channel_id, payload } => {
                self.send_to_sync(SyncCommand::SendChannelUpdate { channel_id, payload });
            }
        }
    }

    // -- Inbound: Sync → Logic -------------------------------------------------

    fn handle_logic_event(&mut self, event: LogicEvent) {
        match event {
            // -- Service plane: forward verbatim, with state hooks -------------
            //
            // The whole relayed response family collapses to one arm. Only
            // `DocumentCreated` also mutates Logic state (it records the new
            // document's type until its first snapshot arrives).
            LogicEvent::Rest { id, evt } => {
                if let RestEvent::DocumentCreated { document_id, doc_type } = &evt {
                    self.pending_doc_types.insert(*document_id, *doc_type);
                }
                self.send_to_ui(UiEvent::Rest { id, evt });
            }

            // -- Real-time sync (server push) ----------------------------------
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

            // -- Sharing (server push) -----------------------------------------
            LogicEvent::MemberRevoked { document_id, revoked_user_id } => {
                self.send_to_ui(UiEvent::MemberRevoked { document_id, revoked_user_id });
            }

            // -- Channels (server push) ----------------------------------------
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

            // -- Meetings (participant push) -----------------------------------
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

// -----------------------------------------------------------------------------
// Tests — the dispatcher is 100% pure Rust (no wasm glue), so the whole routing
// layer is exercisable on the host with mock flume channels. These pin the
// command/event mapping that is otherwise hand-written boilerplate (see the
// "P3 pass-through" open question in ambre-spec-architecture).
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{DocumentId, RequestId, RestCommand, TrameCommand};

    /// A `LogicState` wired to two drainable receivers (UI-bound, Sync-bound).
    fn harness() -> (LogicState, flume::Receiver<UiEvent>, flume::Receiver<SyncCommand>) {
        let (to_ui, ui_rx) = flume::unbounded();
        let (to_sync, sync_rx) = flume::unbounded();
        (LogicState::new(to_ui, to_sync), ui_rx, sync_rx)
    }

    /// Wrap a `RestCommand` in a `LogicCommand::Rest` with a fresh id.
    fn rest(cmd: RestCommand) -> LogicCommand {
        LogicCommand::Rest { id: RequestId::next(), cmd }
    }

    fn scroll(version: u64) -> AppCommand {
        AppCommand::Trame(TrameCommand::ScrollViewport {
            version,
            row_start: 0,
            row_end: 4,
            col_start: 0,
            col_end: 4,
        })
    }

    #[test]
    fn rest_command_forwards_to_sync_with_id() {
        let (mut state, _ui, sync_rx) = harness();
        let id = RequestId::next();
        state.handle_logic_command(LogicCommand::Rest { id, cmd: RestCommand::ListDocuments });
        match sync_rx.try_recv() {
            // The envelope forwards verbatim, echoing the same id.
            Ok(SyncCommand::Rest { id: got, cmd: RestCommand::ListDocuments }) => {
                assert_eq!(got, id);
            }
            other => panic!("unexpected SyncCommand: {other:?}"),
        }
    }

    #[test]
    fn create_document_preserves_fields() {
        let (mut state, _ui, sync_rx) = harness();
        state.handle_logic_command(rest(RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: "Budget".into(),
        }));
        match sync_rx.try_recv() {
            Ok(SyncCommand::Rest { cmd: RestCommand::CreateDocument { doc_type, name }, .. }) => {
                assert_eq!(doc_type, DocType::Trame);
                assert_eq!(name, "Budget");
            }
            other => panic!("unexpected SyncCommand: {other:?}"),
        }
    }

    #[test]
    fn close_document_drops_state_and_forwards() {
        let (mut state, _ui, sync_rx) = harness();
        // IPC_TEST_DOC_ID is pre-seeded by LogicState::new.
        assert!(state.docs.contains_key(&IPC_TEST_DOC_ID));
        state.handle_logic_command(LogicCommand::CloseDocument { document_id: IPC_TEST_DOC_ID });
        assert!(!state.docs.contains_key(&IPC_TEST_DOC_ID));
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::CloseDocument { .. })
        ));
    }

    #[test]
    fn rest_event_forwards_to_ui_with_id() {
        let (mut state, ui_rx, _sync) = harness();
        let id = RequestId::next();
        state.handle_logic_event(LogicEvent::Rest { id, evt: RestEvent::DocumentList { documents: vec![] } });
        match ui_rx.try_recv() {
            Ok(UiEvent::Rest { id: got, evt: RestEvent::DocumentList { .. } }) => {
                assert_eq!(got, id);
            }
            other => panic!("unexpected UiEvent: {other:?}"),
        }
    }

    #[test]
    fn app_command_on_live_doc_yields_viewport() {
        let (mut state, ui_rx, _sync) = harness();
        state.handle_logic_command(LogicCommand::App {
            document_id: IPC_TEST_DOC_ID,
            command: scroll(1),
        });
        match ui_rx.try_recv() {
            Ok(UiEvent::Viewport { document_id, event: AppEvent::Trame(_) }) => {
                assert_eq!(document_id, IPC_TEST_DOC_ID);
            }
            other => panic!("expected Viewport, got {other:?}"),
        }
    }

    #[test]
    fn app_command_on_unknown_doc_is_dropped() {
        let (mut state, ui_rx, _sync) = harness();
        let unknown = DocumentId::from_bytes([9; 16]);
        state.handle_logic_command(LogicCommand::App { document_id: unknown, command: scroll(1) });
        assert!(ui_rx.try_recv().is_err(), "no event should be emitted for an unknown doc");
    }

    #[test]
    fn snapshot_creates_state_for_pending_doc_type() {
        let (mut state, ui_rx, _sync) = harness();
        let id = DocumentId::from_bytes([7; 16]);

        // No live state, and no app command can route yet.
        state.handle_logic_command(LogicCommand::App { document_id: id, command: scroll(1) });
        assert!(ui_rx.try_recv().is_err());

        // DocumentCreated registers the pending doc_type (hook in the Rest arm)…
        state.handle_logic_event(LogicEvent::Rest {
            id: RequestId::next(),
            evt: RestEvent::DocumentCreated { document_id: id, doc_type: DocType::Trame },
        });
        let _ = ui_rx.try_recv(); // drains the forwarded UiEvent::Rest(DocumentCreated)

        // …and the first snapshot promotes it to live state.
        state.handle_logic_event(LogicEvent::RemoteSnapshot { document_id: id, payload: vec![] });
        state.handle_logic_command(LogicCommand::App { document_id: id, command: scroll(2) });
        assert!(matches!(ui_rx.try_recv(), Ok(UiEvent::Viewport { .. })));
    }

    #[test]
    fn snapshot_required_triggers_send_snapshot() {
        let (mut state, _ui, sync_rx) = harness();
        // IPC_TEST_DOC_ID is live, so export_snapshot returns Some.
        state.handle_logic_event(LogicEvent::SnapshotNeeded { document_id: IPC_TEST_DOC_ID });
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::SendSnapshot { .. })
        ));
    }

    #[test]
    fn logout_clears_docs_and_forwards() {
        let (mut state, _ui, sync_rx) = harness();
        state.handle_logic_command(rest(RestCommand::Logout));
        assert!(state.docs.is_empty());
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::Rest { cmd: RestCommand::Logout, .. })
        ));
    }
}
