// ambre-logic/src/residency.rs
// ============================================================================
// Which documents the Logic thread holds (`ambre-spec-residency.md`).
//
// A document stays in memory while a view shows it. When the view closes, the
// document reaches the local store, leaves memory, and Sync keeps a light
// subscription for it. A reopen reads the local copy back and resumes after
// the horizon it holds.
// ============================================================================

use super::*;

use ambre_core::activity::ActivityCounters;

impl LogicState {
    /// The view of a document closed: the document reaches the local store,
    /// then leaves memory (`RESID-001`, `RESID-006`).
    ///
    /// A document with no session identity yet holds a restore that was only
    /// shown, which is never saved (`AUTHOR-027`). It closes the way it did
    /// before residency: Sync drops it whole, and the next open fetches it.
    pub(crate) fn evict(&mut self, document_id: DocumentId) {
        // A running import lands whole first, so the save never holds half of
        // it (`ARCH-031`).
        self.settle_import(document_id);
        let Some(doc) = self.docs.get(&document_id) else {
            return;
        };
        let restoring = doc.restoring;
        let keeps_identity = doc.identity.is_some();
        let horizon = doc.ledger.contiguous_end();
        // The local copy is the whole document, whatever the log held: the
        // reopen starts from it (`RESID-006`). A reopen still reading the
        // store back holds nothing worth saving over it.
        if keeps_identity && restoring.is_none() {
            self.owe_snapshot(document_id);
        }
        self.answer_document(document_id, true);
        self.docs.remove(&document_id);
        // A response arriving after close has no engine to feed, and must not
        // forward to the UI as a stray API event (`toile-media`).
        self.rest_routes.retain(|_, d| *d != document_id);
        self.forget_pending_seals(Some(document_id));
        match (restoring, keeps_identity) {
            // Sync still holds the light subscription the reopen never took.
            (Some(horizon), _) => {
                self.evicted.insert(document_id, horizon);
            }
            (None, true) => {
                self.evicted.insert(document_id, horizon);
                self.send_to_sync(SyncCommand::ReleaseDocument {
                    document_id,
                    applied_seq: horizon,
                });
            }
            (None, false) => self.send_to_sync(SyncCommand::CloseDocument { document_id }),
        }
    }

    /// Reopens a document that left memory: the Main thread reads its local
    /// copy back, and Sync opens once it lands (`RESID-010`). Returns `false`
    /// for a document that never left.
    pub(crate) fn reopen_evicted(&mut self, document_id: DocumentId, doc_type: DocType) -> bool {
        if self.docs.contains_key(&document_id) {
            return false;
        }
        let Some(horizon) = self.evicted.remove(&document_id) else {
            return false;
        };
        let Some(state) = self.fresh_state(doc_type) else {
            self.evicted.insert(document_id, horizon);
            return false;
        };
        let mut doc = LiveDoc::new(state);
        doc.restoring = Some(horizon);
        self.docs.insert(document_id, doc);
        self.send_to_persist(document_id, PersistWrite::ReadBack { doc_type });
        self.owe_layout(document_id);
        true
    }

    /// The local copy of a reopened document landed (`RESID-010`). An empty
    /// or refused copy is fetched from the relay instead (`RESID-012`).
    ///
    /// The badge goes back to nothing: the document now holds its own counts
    /// (`RESID-011`).
    pub(crate) fn reopen_from_store(
        &mut self,
        document_id: DocumentId,
        doc_type: DocType,
        horizon: u64,
        restored: HeldRestore,
        sessions: Vec<(u64, SessionNonce)>,
    ) {
        let HeldRestore { snapshot, deltas } = restored;
        let held = snapshot.is_some() || !deltas.is_empty();
        if let Some(doc) = self.docs.get_mut(&document_id) {
            doc.held_restores.push(HeldRestore {
                snapshot: snapshot.clone(),
                deltas: deltas.clone(),
            });
        }
        let opened = held && self.restore_local(document_id, snapshot, deltas, Restore::Shown);
        if opened {
            if let Some(doc) = self.docs.get_mut(&document_id) {
                let base = doc.state.crdt().oplog_vv();
                doc.ledger.reset(horizon, base);
            }
            self.note_earlier_sessions(document_id, sessions);
        } else {
            tracing::warn!("Logic: doc {document_id} has no usable local copy, fetching it");
            if let (Some(state), Some(doc)) =
                (self.fresh_state(doc_type), self.docs.get_mut(&document_id))
            {
                doc.state = state;
                doc.held_restores.clear();
            }
            // Dropping the light subscription makes the open a full one.
            self.send_to_sync(SyncCommand::CloseDocument { document_id });
        }
        self.send_to_sync(SyncCommand::OpenDocument {
            document_id,
            doc_type,
        });
        self.send_to_ui(UiEvent::Activity {
            document_id,
            counters: ActivityCounters::default(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::conduit::Receiver;

    fn drain<T>(rx: &Receiver<T>) -> Vec<T> {
        rx.try_iter().collect()
    }

    /// The state, its one document, and the Sync, persist and UI outboxes.
    type Opened = (
        LogicState,
        DocumentId,
        Receiver<SyncCommand>,
        Receiver<(DocumentId, PersistWrite)>,
        Receiver<UiEvent>,
    );

    /// A state with one Prose document open under its session identity, the
    /// relay's numbers up to 4 applied.
    fn opened() -> Opened {
        let (to_ui, ui) = test_link();
        let (to_sync, sync) = test_link();
        let (to_persist, persist) = test_link();
        let mut state = LogicState::new(to_ui, to_sync, to_persist);
        let document_id = DocumentId::new();
        state.open_with_identity(document_id, DocType::Prose);
        if let Some(doc) = state.docs.get_mut(&document_id) {
            doc.ledger.reset(4, doc.state.crdt().oplog_vv());
        }
        drain(&sync);
        drain(&persist);
        drain(&ui);
        (state, document_id, sync, persist, ui)
    }

    /// Holds `RESID-001` and `RESID-006`. A document whose view closed
    /// reaches the local store whole, then leaves memory. Sync keeps it at
    /// the horizon it holds.
    #[test]
    fn a_closed_document_reaches_the_store_then_leaves_memory() {
        let (mut state, document_id, sync, persist, _ui) = opened();
        state.handle_logic_command(LogicCommand::CloseDocument { document_id });
        assert!(!state.docs.contains_key(&document_id));
        let writes: Vec<PersistWrite> = drain(&persist).into_iter().map(|(_, w)| w).collect();
        assert!(
            matches!(writes.as_slice(), [PersistWrite::Snapshot(_)]),
            "the document left memory unsaved"
        );
        let commands = drain(&sync);
        assert!(
            matches!(
                commands.as_slice(),
                [SyncCommand::ReleaseDocument { applied_seq: 4, .. }]
            ),
            "got {commands:?}"
        );
    }

    /// Holds `RESID-003`: a document a view still shows stays in memory
    /// while another closes.
    #[test]
    fn a_shown_document_never_leaves_memory() {
        let (mut state, shown, _sync, _persist, _ui) = opened();
        let other = DocumentId::new();
        state.handle_logic_command(LogicCommand::OpenDocument {
            document_id: other,
            doc_type: DocType::Prose,
        });
        state.handle_logic_command(LogicCommand::CloseDocument { document_id: other });
        assert!(state.docs.contains_key(&shown));
        assert!(!state.docs.contains_key(&other));
    }

    /// Holds `RESID-010` on the Logic side, and `RESID-011`. A reopen reads
    /// the local copy back before Sync opens. It resumes after the horizon
    /// and clears the badge.
    #[test]
    fn a_reopen_starts_from_the_local_copy() {
        let (mut state, document_id, sync, persist, ui) = opened();
        state.handle_logic_command(LogicCommand::CloseDocument { document_id });
        let saved = drain(&persist).into_iter().find_map(|(_, w)| match w {
            PersistWrite::Snapshot(bytes) => Some(bytes),
            _ => None,
        });
        drain(&sync);
        drain(&ui);
        state.handle_logic_command(LogicCommand::OpenDocument {
            document_id,
            doc_type: DocType::Prose,
        });
        assert!(
            matches!(
                drain(&persist).as_slice(),
                [(_, PersistWrite::ReadBack { .. })]
            ),
            "the reopen did not read the local copy"
        );
        assert!(
            drain(&sync).is_empty(),
            "Sync opened before the copy landed"
        );
        state.handle_logic_command(LogicCommand::RestoreLocalSnapshot {
            document_id,
            doc_type: DocType::Prose,
            snapshot: saved,
            deltas: Vec::new(),
            sessions: Vec::new(),
        });
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::OpenDocument { .. }]
        ));
        let doc = state.docs.get(&document_id).expect("reopened");
        assert_eq!(doc.ledger.contiguous_end(), 4);
        assert!(drain(&ui).iter().any(|event| matches!(
            event,
            UiEvent::Activity { counters, .. } if *counters == ActivityCounters::default()
        )));
    }

    /// Holds `RESID-012`: a reopen whose local copy is missing or refused
    /// drops the light subscription, so Sync fetches the whole document.
    #[test]
    fn a_reopen_with_no_usable_copy_fetches_from_the_relay() {
        for snapshot in [None, Some(b"not a document".to_vec())] {
            let (mut state, document_id, sync, _persist, _ui) = opened();
            state.handle_logic_command(LogicCommand::CloseDocument { document_id });
            state.handle_logic_command(LogicCommand::OpenDocument {
                document_id,
                doc_type: DocType::Prose,
            });
            drain(&sync);
            state.handle_logic_command(LogicCommand::RestoreLocalSnapshot {
                document_id,
                doc_type: DocType::Prose,
                snapshot,
                deltas: Vec::new(),
                sessions: Vec::new(),
            });
            let commands = drain(&sync);
            assert!(
                matches!(
                    commands.as_slice(),
                    [
                        SyncCommand::CloseDocument { .. },
                        SyncCommand::OpenDocument { .. }
                    ]
                ),
                "got {commands:?}"
            );
        }
    }
}
