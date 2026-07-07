// ambre-logic/src/dispatch.rs
// ============================================================================
// The two inbound dispatchers:
//   - `handle_logic_command`: UI Thread -> Logic (content, control, service).
//   - `handle_logic_event`:  Sync -> Logic (real-time push, service responses).
// Both are pure Rust (no wasm glue), so the whole routing layer is exercised on
// the host by the `tests` module at the bottom with mock flume channels.
// ============================================================================

use super::*;

impl LogicState {
    // -- Inbound: UI Thread → Logic --------------------------------------------

    pub(crate) fn handle_logic_command(&mut self, command: LogicCommand) {
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

    pub(crate) fn handle_logic_event(&mut self, event: LogicEvent) {
        match event {
            // -- Service plane: forward verbatim -------------------------------
            //
            // The whole relayed response family collapses to one arm: a pure
            // forward to the UI. Documents are materialised on `OpenDocument`
            // (which carries the `doc_type`), so no event here mutates state.
            LogicEvent::Rest { id, evt } => {
                self.send_to_ui(UiEvent::Rest { id, evt });
            }

            // -- Real-time sync (server push) ----------------------------------
            // The document is already live (pre-seeded on `OpenDocument`); a
            // snapshot for an unopened doc is dropped by `apply_remote`.
            LogicEvent::RemoteSnapshot {
                document_id,
                payload,
            } => {
                self.apply_remote(document_id, &payload);
            }

            LogicEvent::RemoteUpdate {
                document_id,
                payload,
            } => {
                self.apply_remote(document_id, &payload);
            }

            LogicEvent::RemoteUpdateBatch {
                document_id,
                payloads,
            } => {
                self.apply_remote_batch(document_id, &payloads);
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

            // Initial hydration complete: any snapshot was applied by the
            // preceding RemoteSnapshot arm (channel order is preserved), so the
            // UI can dismiss its loading overlay.
            LogicEvent::DocumentReady { document_id } => {
                self.send_to_ui(UiEvent::DocumentReady { document_id });
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
// Tests — the dispatcher is 100% pure Rust (no wasm glue), so the whole routing
// layer is exercisable on the host with mock flume channels. These pin the
// command/event mapping that is otherwise hand-written boilerplate (see the
// "P3 pass-through" open question in ambre-spec-architecture).
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{DocumentId, RequestId, RestCommand, RestEvent, TrameCommand};

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
    fn open_document_materialises_state_before_any_payload() {
        let (mut state, ui_rx, sync_rx) = harness();
        let id = DocumentId::from_bytes([7; 16]);

        // No live state, and no app command can route yet.
        state.handle_logic_command(LogicCommand::App {
            document_id: id,
            command: scroll(1),
        });
        assert!(ui_rx.try_recv().is_err());

        // OpenDocument pre-seeds the live state (carrying its doc_type) and
        // forwards the control command to Sync.
        state.handle_logic_command(LogicCommand::OpenDocument {
            document_id: id,
            doc_type: DocType::Trame,
        });
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::OpenDocument { .. })
        ));

        // An app command now routes to the freshly-seeded document.
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
