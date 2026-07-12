// ambre-logic/src/data.rs
// ============================================================================
// Application data plane: route typed app commands to a document's state, apply
// remote CRDT deltas / ephemeral payloads, and export snapshots for compaction.
// All methods borrow `LogicState` and emit through its outbound senders.
// ============================================================================

use super::*;

impl LogicState {
    /// Route a typed application command to its document's state, send display
    /// events to the UI Thread, and forward CRDT/ephemeral payloads to Sync.
    pub(crate) fn handle_app_command(&mut self, document_id: DocumentId, command: AppCommand) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            tracing::warn!("Logic: no live document {document_id} for app command");
            return;
        };

        // Each arm produces the same triple -- display events, an optional CRDT
        // `update`, an optional `ephemeral` payload -- so the fan-out to UI /
        // Sync / persist is factored into `dispatch_outcome`, keeping the arms to
        // the single differing line (which `state` and which `AppEvent` wrap).
        match (doc, command) {
            (DocState::Trame(state), AppCommand::Trame(cmd)) => {
                let outcome = state.handle_command(cmd);
                let snapshot = outcome.update.is_some().then(|| state.export_snapshot());
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Trame).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    snapshot,
                );
            }
            (DocState::Toile(state), AppCommand::Toile(cmd)) => {
                let outcome = state.handle_command(cmd);
                let snapshot = outcome.update.is_some().then(|| state.export_snapshot());
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Toile).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    snapshot,
                );
            }
            // A mismatched (state, command) pair is impossible by routing: the
            // `document_id` selects a single `DocState`, and the UI only ever
            // sends the matching `AppCommand` variant for that document.
            (state, command) => tracing::warn!(
                "Logic: mismatched (state, command) pair for doc {document_id}: {:?} / {:?}",
                std::mem::discriminant(state),
                std::mem::discriminant(&command),
            ),
        }
    }

    /// Fan an already-typed outcome out to the UI Thread (display events), to
    /// Sync (`SendUpdate` / `SendEphemeral`), and to local persistence (a full
    /// snapshot, captured while the `state` borrow was live). Shared by every
    /// `AppCommand` arm; the arm only decides which `AppEvent` variant wraps the
    /// display events and whether a snapshot was taken.
    fn dispatch_outcome(
        &self,
        document_id: DocumentId,
        display: Vec<AppEvent>,
        update: Option<Vec<u8>>,
        ephemeral: Option<Vec<u8>>,
        snapshot: Option<Vec<u8>>,
    ) {
        for event in display {
            self.send_to_ui(UiEvent::Viewport { document_id, event });
        }
        if let Some(payload) = update {
            tracing::info!(
                "[Logic→Sync] pushing SendUpdate doc={document_id} delta_size={}",
                payload.len(),
            );
            self.send_to_sync(SyncCommand::SendUpdate {
                document_id,
                payload,
            });
        }
        if let Some(snapshot) = snapshot {
            self.send_to_persist(document_id, snapshot);
        }
        if let Some(payload) = ephemeral {
            self.send_to_sync(SyncCommand::SendEphemeral {
                document_id,
                payload,
            });
        }
    }

    /// Apply a remote CRDT delta to a document and forward viewport events.
    pub(crate) fn apply_remote(&mut self, document_id: DocumentId, payload: &[u8]) {
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
            DocState::Toile(state) => {
                let events = state.apply_remote(payload);
                let snapshot = (!events.is_empty()).then(|| state.export_snapshot());
                (events.into_iter().map(AppEvent::Toile).collect(), snapshot)
            }
        };
        for event in events {
            self.send_to_ui(UiEvent::Viewport { document_id, event });
        }
        if let Some(snapshot) = snapshot {
            self.send_to_persist(document_id, snapshot);
        }
    }

    /// Apply a batch of remote deltas (a server history replay on open) as one
    /// merge+reseed, so the UI repaints once instead of once per delta. Mirrors
    /// [`apply_remote`] for the local-persist and viewport-forward tail.
    pub(crate) fn apply_remote_batch(&mut self, document_id: DocumentId, payloads: &[Vec<u8>]) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            tracing::warn!(
                "Logic: apply_remote_batch for unopened doc {document_id} ({} delta(s)), dropping",
                payloads.len()
            );
            return;
        };
        let (events, snapshot): (Vec<AppEvent>, Option<Vec<u8>>) = match doc {
            DocState::Trame(state) => {
                let events = state.apply_remote_batch(payloads);
                let snapshot = (!events.is_empty()).then(|| state.export_snapshot());
                (events.into_iter().map(AppEvent::Trame).collect(), snapshot)
            }
            DocState::Toile(state) => {
                let events = state.apply_remote_batch(payloads);
                let snapshot = (!events.is_empty()).then(|| state.export_snapshot());
                (events.into_iter().map(AppEvent::Toile).collect(), snapshot)
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
    pub(crate) fn apply_ephemeral(&mut self, document_id: DocumentId, payload: &[u8]) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        let events: Vec<AppEvent> = match doc {
            DocState::Trame(state) => state
                .apply_ephemeral(payload)
                .into_iter()
                .map(AppEvent::Trame)
                .collect(),
            DocState::Toile(state) => state
                .apply_ephemeral(payload)
                .into_iter()
                .map(AppEvent::Toile)
                .collect(),
        };
        for event in events {
            self.send_to_ui(UiEvent::Presence { document_id, event });
        }
    }

    /// Export a document's CRDT snapshot for compaction.
    pub(crate) fn export_snapshot(&self, document_id: DocumentId) -> Option<Vec<u8>> {
        match self.docs.get(&document_id)? {
            DocState::Trame(state) => Some(state.export_snapshot()),
            DocState::Toile(state) => Some(state.export_snapshot()),
        }
    }
}
