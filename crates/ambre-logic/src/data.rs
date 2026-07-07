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
        };
        for event in events {
            self.send_to_ui(UiEvent::Presence { document_id, event });
        }
    }

    /// Export a document's CRDT snapshot for compaction.
    pub(crate) fn export_snapshot(&self, document_id: DocumentId) -> Option<Vec<u8>> {
        match self.docs.get(&document_id)? {
            DocState::Trame(state) => Some(state.export_snapshot()),
        }
    }
}
