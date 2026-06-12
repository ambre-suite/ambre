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
// Transport: UI <-> Logic uses `flume` channels in the shared memory of the
// Main instance, carrying typed Rust values by move (no postcard, no `Any`).
// Logic-bound `SyncCommand`s are sent to a `flume` channel whose receiver is on
// the Main thread, which relays them to the separate Sync worker via
// `postMessage`; `LogicEvent`s arrive the same way in reverse.
// ============================================================================

use std::collections::HashMap;

/// IndexedDB-backed local snapshot store. Compiled only for wasm: its callbacks
/// need a live event loop, so the store itself is driven by the Main thread (see
/// `ambre-app`), not by this blocking Logic loop.
#[cfg(target_arch = "wasm32")]
pub mod persistence;

use ambre_bus::{
    AppCommand, AppEvent, DocType, DocumentId, LogicCommand, LogicEvent, RestCommand, RestEvent,
    SMOKE_DOCUMENT_ID, SyncCommand, UiEvent,
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
    /// Outbound channel: Logic → Sync (drained by the Main thread, relayed to
    /// the Sync worker via postMessage).
    to_sync: flume::Sender<SyncCommand>,
    /// Outbound channel: Logic → Main, carrying full document snapshots for
    /// local persistence. The Main thread owns the IndexedDB store (its
    /// callbacks need a live event loop, which this blocking loop cannot offer);
    /// here we only export the bytes and hand them over.
    to_persist: flume::Sender<(DocumentId, Vec<u8>)>,
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
    fn new(
        to_ui: flume::Sender<UiEvent>,
        to_sync: flume::Sender<SyncCommand>,
        to_persist: flume::Sender<(DocumentId, Vec<u8>)>,
    ) -> Self {
        let mut docs = HashMap::new();
        docs.insert(SMOKE_DOCUMENT_ID, DocState::Trame(TrameState::new()));
        Self {
            to_ui,
            to_sync,
            to_persist,
            docs,
            pending_doc_types: HashMap::new(),
        }
    }

    // -- Outbound --------------------------------------------------------------

    fn send_to_ui(&self, event: UiEvent) {
        if let Err(flume::SendError(event)) = self.to_ui.send(event) {
            tracing::warn!(
                "[Logic→UI] UI receiver dropped, discarding {:?}",
                std::mem::discriminant(&event),
            );
        }
    }

    fn send_to_sync(&self, cmd: SyncCommand) {
        if let Err(flume::SendError(cmd)) = self.to_sync.send(cmd) {
            tracing::warn!(
                "[Logic→Sync] receiver dropped, discarding {:?}",
                std::mem::discriminant(&cmd),
            );
        }
    }

    /// Hand a full document snapshot to the Main thread for local persistence.
    /// Best-effort: a dropped receiver only means no local save (the snapshot is
    /// regenerated on the next edit), so it is non-fatal.
    fn send_to_persist(&self, document_id: DocumentId, snapshot: Vec<u8>) {
        if self.to_persist.send((document_id, snapshot)).is_err() {
            tracing::warn!("[Logic→Persist] receiver dropped, snapshot not saved");
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
                // Capture a full snapshot for local persistence whenever the doc
                // changed (an `update` was produced). Exported here, while the
                // `state` borrow is live; persisted by the Main thread.
                let snapshot = outcome.update.is_some().then(|| state.export_snapshot());
                for event in outcome.display {
                    self.send_to_ui(UiEvent::Viewport {
                        document_id,
                        event: AppEvent::Trame(event),
                    });
                }
                if let Some(ref payload) = outcome.update {
                    tracing::info!(
                        "[Logic→Sync] pushing SendUpdate doc={document_id} delta_size={}",
                        payload.len(),
                    );
                    self.send_to_sync(SyncCommand::SendUpdate {
                        document_id,
                        payload: payload.clone(),
                    });
                }
                if let Some(snapshot) = snapshot {
                    self.send_to_persist(document_id, snapshot);
                }
                if let Some(payload) = outcome.ephemeral {
                    self.send_to_sync(SyncCommand::SendEphemeral {
                        document_id,
                        payload,
                    });
                }
            } // Once a second document type exists, add its `(DocState::X,
              // AppCommand::X)` arm plus a `_ => { warn; }` catch-all for
              // mismatched (state, command) pairs (impossible by routing).
        }
    }

    /// Apply a remote CRDT delta to a document and forward viewport events.
    fn apply_remote(&mut self, document_id: DocumentId, payload: &[u8]) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            tracing::warn!(
                "Logic: apply_remote for unopened doc {document_id} ({} bytes), dropping",
                payload.len()
            );
            return;
        };
        let (events, snapshot): (Vec<AppEvent>, Option<Vec<u8>>) = match doc {
            DocState::Trame(state) => {
                let events = state.apply_remote(payload);
                // Persist the merged state locally whenever the remote delta
                // actually changed the document (it emitted display events), so a
                // refresh restores it even for a client that only ever receives
                // (never edits) -- otherwise such a client has no local snapshot
                // and depends entirely on a fresh server replay.
                let snapshot = (!events.is_empty()).then(|| state.export_snapshot());
                (events.into_iter().map(AppEvent::Trame).collect(), snapshot)
            }
        };
        for event in events {
            self.send_to_ui(UiEvent::Viewport { document_id, event });
        }
        if let Some(snapshot) = snapshot {
            self.send_to_persist(document_id, snapshot);
        }
    }

    /// Apply a remote ephemeral payload to a document and forward presence events.
    fn apply_ephemeral(&mut self, document_id: DocumentId, payload: &[u8]) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        let events: Vec<AppEvent> = match doc {
            DocState::Trame(state) => state
                .apply_ephemeral(payload)
                .into_iter()
                .map(AppEvent::Trame)
                .collect(),
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
            LogicCommand::App {
                document_id,
                command,
            } => {
                self.handle_app_command(document_id, command);
            }

            // -- Local restore (Main thread, startup) --------------------------
            // Import a locally persisted snapshot into the document's state and
            // forward the resulting viewport events to the UI. Local-only: never
            // forwarded to Sync.
            LogicCommand::RestoreLocalSnapshot {
                document_id,
                doc_type,
                snapshot,
            } => {
                // Order-independent: the boot-time local restore and the route
                // poll's `OpenDocument` race on startup. If the restore arrives
                // first, materialise the live document here so the snapshot is
                // never dropped; a later `OpenDocument` finds the slot occupied
                // and reuses it.
                if let std::collections::hash_map::Entry::Vacant(slot) =
                    self.docs.entry(document_id)
                    && let Some(state) = DocState::new(doc_type)
                {
                    slot.insert(state);
                }
                let Some(DocState::Trame(state)) = self.docs.get_mut(&document_id) else {
                    tracing::warn!(
                        "Logic: no live document {document_id} to restore snapshot into"
                    );
                    return;
                };
                let events: Vec<AppEvent> = state
                    .import_snapshot(&snapshot)
                    .into_iter()
                    .map(AppEvent::Trame)
                    .collect();
                for event in events {
                    self.send_to_ui(UiEvent::Viewport { document_id, event });
                }
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
            LogicCommand::OpenDocument {
                document_id,
                doc_type,
            } => {
                // Pre-seed the in-memory state so UI edits are routed immediately,
                // before any snapshot/delta arrives (same rationale as the smoke
                // doc). A later RemoteSnapshot reuses the existing state.
                if let std::collections::hash_map::Entry::Vacant(slot) =
                    self.docs.entry(document_id)
                    && let Some(state) = DocState::new(doc_type)
                {
                    slot.insert(state);
                }
                self.send_to_sync(SyncCommand::OpenDocument {
                    document_id,
                    doc_type,
                });
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
            LogicCommand::SendChannelMessage {
                channel_id,
                payload,
            } => {
                self.send_to_sync(SyncCommand::SendChannelUpdate {
                    channel_id,
                    payload,
                });
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
                if let RestEvent::DocumentCreated {
                    document_id,
                    doc_type,
                } = &evt
                {
                    self.pending_doc_types.insert(*document_id, *doc_type);
                }
                self.send_to_ui(UiEvent::Rest { id, evt });
            }

            // -- Real-time sync (server push) ----------------------------------
            LogicEvent::RemoteSnapshot {
                document_id,
                payload,
            } => {
                if !self.docs.contains_key(&document_id) {
                    match self
                        .pending_doc_types
                        .remove(&document_id)
                        .and_then(DocState::new)
                    {
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

            LogicEvent::RemoteUpdate {
                document_id,
                payload,
            } => {
                self.apply_remote(document_id, &payload);
            }

            LogicEvent::RemoteEphemeral {
                document_id,
                payload,
            } => {
                self.apply_ephemeral(document_id, &payload);
            }

            LogicEvent::SnapshotNeeded { document_id } => {
                if let Some(payload) = self.export_snapshot(document_id) {
                    self.send_to_sync(SyncCommand::SendSnapshot {
                        document_id,
                        payload,
                    });
                }
            }

            LogicEvent::ConnectionStatus {
                document_id,
                connected,
            } => {
                self.send_to_ui(UiEvent::ConnectionStatus {
                    document_id,
                    connected,
                });
            }

            LogicEvent::SyncWarning {
                document_id,
                code,
                message,
            } => {
                self.send_to_ui(UiEvent::SyncWarning {
                    document_id,
                    code,
                    message,
                });
            }

            // -- Sharing (server push) -----------------------------------------
            LogicEvent::MemberRevoked {
                document_id,
                revoked_user_id,
            } => {
                self.send_to_ui(UiEvent::MemberRevoked {
                    document_id,
                    revoked_user_id,
                });
            }

            // Server push: a document was shared with us. Pure forward; the UI
            // refreshes its list and notifies. No Logic-side state to mutate (the
            // document materializes lazily when the user opens it).
            LogicEvent::DocumentShared {
                document_id,
                doc_type,
            } => {
                self.send_to_ui(UiEvent::DocumentShared {
                    document_id,
                    doc_type,
                });
            }

            // -- Channels (server push) ----------------------------------------
            LogicEvent::ChannelRemoteUpdate {
                channel_id,
                payload,
            } => {
                self.send_to_ui(UiEvent::ChannelMessage {
                    channel_id,
                    payload,
                });
            }
            LogicEvent::ChannelRemoteEphemeral {
                channel_id,
                payload,
            } => {
                self.send_to_ui(UiEvent::ChannelPresenceUpdate {
                    channel_id,
                    payload,
                });
            }
            LogicEvent::ChannelConnectionStatus {
                channel_id,
                connected,
            } => {
                self.send_to_ui(UiEvent::ChannelConnectionStatus {
                    channel_id,
                    connected,
                });
            }
            LogicEvent::ChannelSnapshotNeeded { channel_id: _ } => {
                tracing::warn!("Logic: ChannelSnapshotNeeded is not yet implemented");
            }

            // -- Meetings (participant push) -----------------------------------
            LogicEvent::MeetingParticipantJoined {
                meeting_id,
                user_id,
            } => {
                self.send_to_ui(UiEvent::MeetingParticipantJoined {
                    meeting_id,
                    user_id,
                });
            }
            LogicEvent::MeetingParticipantLeft {
                meeting_id,
                user_id,
            } => {
                self.send_to_ui(UiEvent::MeetingParticipantLeft {
                    meeting_id,
                    user_id,
                });
            }

            // -- Errors --------------------------------------------------------
            LogicEvent::AuthError { message } => {
                self.send_to_ui(UiEvent::AuthError { message });
            }
            LogicEvent::ApiError {
                request,
                code,
                message,
            } => {
                self.send_to_ui(UiEvent::ApiError {
                    request,
                    code,
                    message,
                });
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Entry point: drain both inbound channels, then park on the shared Waker.
// -----------------------------------------------------------------------------

/// Run the Logic thread to completion.
///
/// Blocks on `flume::Selector` waiting on both inbound channels at once,
/// dispatching each message through [`LogicState`]. Exits when **both** inbound
/// channels disconnect (the UI Thread and the Sync relay both gone).
///
/// `Selector::wait` parks the worker via flume's Atomics-backed blocking path,
/// which wakes correctly across the UI/Logic worker boundary (only flume's
/// *async* waker failed to cross workers — that path is gone, the Sync worker is
/// now a separate wasm instance reached over postMessage). Legal off the main
/// thread; the Logic worker has no browser callbacks to keep alive.
pub fn run(
    from_ui: flume::Receiver<LogicCommand>,
    to_ui: flume::Sender<UiEvent>,
    to_sync: flume::Sender<SyncCommand>,
    from_sync: flume::Receiver<LogicEvent>,
    to_persist: flume::Sender<(DocumentId, Vec<u8>)>,
) {
    use flume::Selector;

    tracing::info!("Ambre Logic thread starting");
    let mut state = LogicState::new(to_ui, to_sync, to_persist);

    /// What the selector picked: a command from the UI, or an event from Sync.
    enum Inbound {
        Command(Result<LogicCommand, flume::RecvError>),
        Event(Result<LogicEvent, flume::RecvError>),
    }

    let mut ui_open = true;
    let mut sync_open = true;
    while ui_open || sync_open {
        // Build a selector over only the still-open channels and block until one
        // yields. A closed channel resolves immediately with `Err`, which we map
        // to "mark closed" -- so a single open channel keeps the loop running.
        let mut selector = Selector::new();
        if ui_open {
            selector = selector.recv(&from_ui, Inbound::Command);
        }
        if sync_open {
            selector = selector.recv(&from_sync, Inbound::Event);
        }
        match selector.wait() {
            Inbound::Command(Ok(command)) => state.handle_logic_command(command),
            Inbound::Command(Err(_)) => ui_open = false,
            Inbound::Event(Ok(event)) => state.handle_logic_event(event),
            Inbound::Event(Err(_)) => sync_open = false,
        }
    }

    tracing::info!("Ambre Logic thread exiting (both inbound channels disconnected)");
    // Drop `state` here: document state is released, the outbound Senders are
    // closed, which the UI thread and the Sync relay observe as `Disconnected`.
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
    fn harness() -> (
        LogicState,
        flume::Receiver<UiEvent>,
        flume::Receiver<SyncCommand>,
    ) {
        let (to_ui, ui_rx) = flume::unbounded();
        let (to_sync, sync_rx) = flume::unbounded();
        // The persist channel is drained by the Main thread in production; tests
        // keep the receiver alive so sends never fail.
        let (to_persist, persist_rx) = flume::unbounded();
        std::mem::forget(persist_rx);
        (LogicState::new(to_ui, to_sync, to_persist), ui_rx, sync_rx)
    }

    /// Wrap a `RestCommand` in a `LogicCommand::Rest` with a fresh id.
    fn rest(cmd: RestCommand) -> LogicCommand {
        LogicCommand::Rest {
            id: RequestId::next(),
            cmd,
        }
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
        state.handle_logic_command(LogicCommand::Rest {
            id,
            cmd: RestCommand::ListDocuments,
        });
        match sync_rx.try_recv() {
            // The envelope forwards verbatim, echoing the same id.
            Ok(SyncCommand::Rest {
                id: got,
                cmd: RestCommand::ListDocuments,
            }) => {
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
            Ok(SyncCommand::Rest {
                cmd: RestCommand::CreateDocument { doc_type, name },
                ..
            }) => {
                assert_eq!(doc_type, DocType::Trame);
                assert_eq!(name, "Budget");
            }
            other => panic!("unexpected SyncCommand: {other:?}"),
        }
    }

    #[test]
    fn close_document_drops_state_and_forwards() {
        let (mut state, _ui, sync_rx) = harness();
        let doc_id = DocumentId::new();
        state
            .docs
            .insert(doc_id, DocState::Trame(TrameState::new()));
        state.handle_logic_command(LogicCommand::CloseDocument {
            document_id: doc_id,
        });
        assert!(!state.docs.contains_key(&doc_id));
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::CloseDocument { .. })
        ));
    }

    #[test]
    fn restore_local_snapshot_materialises_doc_when_open_lost_the_race() {
        // The boot-time local restore can arrive before the route poll's
        // OpenDocument. The handler must materialise the live document itself so
        // the snapshot is applied instead of dropped.
        let (mut state, _ui, _sync) = harness();
        let doc_id = DocumentId::new();
        // An empty Trame snapshot is a valid Loro export; import must not panic
        // and the doc must exist afterwards.
        let snapshot = TrameState::new().export_snapshot();
        assert!(!state.docs.contains_key(&doc_id));
        state.handle_logic_command(LogicCommand::RestoreLocalSnapshot {
            document_id: doc_id,
            doc_type: DocType::Trame,
            snapshot,
        });
        assert!(state.docs.contains_key(&doc_id));
    }

    #[test]
    fn rest_event_forwards_to_ui_with_id() {
        let (mut state, ui_rx, _sync) = harness();
        let id = RequestId::next();
        state.handle_logic_event(LogicEvent::Rest {
            id,
            evt: RestEvent::DocumentList { documents: vec![] },
        });
        match ui_rx.try_recv() {
            Ok(UiEvent::Rest {
                id: got,
                evt: RestEvent::DocumentList { .. },
            }) => {
                assert_eq!(got, id);
            }
            other => panic!("unexpected UiEvent: {other:?}"),
        }
    }

    #[test]
    fn app_command_on_live_doc_yields_viewport() {
        let (mut state, ui_rx, _sync) = harness();
        let doc_id = DocumentId::new();
        state
            .docs
            .insert(doc_id, DocState::Trame(TrameState::new()));
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: scroll(1),
        });
        match ui_rx.try_recv() {
            Ok(UiEvent::Viewport {
                document_id,
                event: AppEvent::Trame(_),
            }) => {
                assert_eq!(document_id, doc_id);
            }
            other => panic!("expected Viewport, got {other:?}"),
        }
    }

    #[test]
    fn app_command_on_unknown_doc_is_dropped() {
        let (mut state, ui_rx, _sync) = harness();
        let unknown = DocumentId::from_bytes([9; 16]);
        state.handle_logic_command(LogicCommand::App {
            document_id: unknown,
            command: scroll(1),
        });
        assert!(
            ui_rx.try_recv().is_err(),
            "no event should be emitted for an unknown doc"
        );
    }

    #[test]
    fn snapshot_creates_state_for_pending_doc_type() {
        let (mut state, ui_rx, _sync) = harness();
        let id = DocumentId::from_bytes([7; 16]);

        // No live state, and no app command can route yet.
        state.handle_logic_command(LogicCommand::App {
            document_id: id,
            command: scroll(1),
        });
        assert!(ui_rx.try_recv().is_err());

        // DocumentCreated registers the pending doc_type (hook in the Rest arm)…
        state.handle_logic_event(LogicEvent::Rest {
            id: RequestId::next(),
            evt: RestEvent::DocumentCreated {
                document_id: id,
                doc_type: DocType::Trame,
            },
        });
        let _ = ui_rx.try_recv(); // drains the forwarded UiEvent::Rest(DocumentCreated)

        // …and the first snapshot promotes it to live state.
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: id,
            payload: vec![],
        });
        state.handle_logic_command(LogicCommand::App {
            document_id: id,
            command: scroll(2),
        });
        assert!(matches!(ui_rx.try_recv(), Ok(UiEvent::Viewport { .. })));
    }

    #[test]
    fn snapshot_required_triggers_send_snapshot() {
        let (mut state, _ui, sync_rx) = harness();
        let doc_id = DocumentId::new();
        state
            .docs
            .insert(doc_id, DocState::Trame(TrameState::new()));
        state.handle_logic_event(LogicEvent::SnapshotNeeded {
            document_id: doc_id,
        });
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
            Ok(SyncCommand::Rest {
                cmd: RestCommand::Logout,
                ..
            })
        ));
    }
}
