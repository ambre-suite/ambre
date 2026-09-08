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
                    && let Some(mut state) = DocState::new(doc_type)
                {
                    if let Some(user_id) = self.my_user_id {
                        state.set_user_id(user_id);
                    }
                    if let Some(name) = &self.my_display_name {
                        state.set_display_name(name);
                    }
                    slot.insert(LiveDoc::new(state));
                }
                let Some(doc) = self.docs.get_mut(&document_id).map(|d| &mut d.state) else {
                    tracing::warn!(
                        "Logic: no live document {document_id} to restore snapshot into"
                    );
                    return;
                };
                // A snapshot import is the same operation whichever engine owns
                // the document; each returns its own display-event family, mapped
                // to the matching `AppEvent` variant.
                let events: Vec<AppEvent> = match doc {
                    DocState::Trame(state) => state
                        .import_snapshot(&snapshot)
                        .into_iter()
                        .map(AppEvent::Trame)
                        .collect(),
                    DocState::Toile(state) => state
                        .apply_remote(&snapshot)
                        .into_iter()
                        .map(AppEvent::Toile)
                        .collect(),
                };
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
                    RestCommand::Logout => {
                        self.docs.clear();
                        self.rest_routes.clear();
                    }
                    RestCommand::DeleteDocument { document_id } => {
                        self.docs.remove(document_id);
                        self.rest_routes.retain(|_, d| d != document_id);
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
                    && let Some(mut state) = DocState::new(doc_type)
                {
                    // Seed the identity if the session is already authenticated;
                    // otherwise the later `SessionReady` sweep covers this slot.
                    if let Some(user_id) = self.my_user_id {
                        state.set_user_id(user_id);
                    }
                    if let Some(name) = &self.my_display_name {
                        state.set_display_name(name);
                    }
                    slot.insert(LiveDoc::new(state));
                }
                self.send_to_sync(SyncCommand::OpenDocument {
                    document_id,
                    doc_type,
                });
            }
            LogicCommand::CloseDocument { document_id } => {
                self.docs.remove(&document_id);
                // Drop any in-flight service routes for this document: a response
                // arriving after close has no engine to feed and must not forward
                // to the UI as a stray API event (`toile-media`).
                self.rest_routes.retain(|_, d| *d != document_id);
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
            LogicCommand::LoadFonts { faces } => {
                // Seed the PDF-export FontContext once with the base faces the
                // Main thread fetched (`toile-pdf-export`). Local-only: nothing
                // is sent to Sync. A second seeding is ignored -- the faces are
                // already registered and export needs them exactly once.
                if self.export_cx.is_none() {
                    tracing::info!(
                        "Logic: seeding export FontContext with {} faces",
                        faces.len()
                    );
                    let mut font_cx = parley::FontContext::new();
                    for bytes in faces {
                        font_cx.collection.register_fonts(bytes.into(), None);
                    }
                    self.export_cx = Some(font_cx);
                }
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
                // A response to a request this Logic thread issued on behalf of a
                // document (`toile-media`) is fed back into the owning engine
                // instead of forwarded to the UI. UI-issued requests are absent
                // from `rest_routes` and fall through to the verbatim forward.
                if let Some(document_id) = self.rest_routes.remove(&id) {
                    self.route_service_event(document_id, id, evt);
                    return;
                }
                // `SessionReady` also seeds the canonical `UserId`: capture
                // it for later opens and push it into every already-open document
                // before forwarding the event verbatim to the UI.
                if let RestEvent::SessionReady {
                    user_id,
                    display_name,
                    ..
                } = &evt
                {
                    self.my_user_id = Some(*user_id);
                    self.my_display_name = Some(display_name.clone());
                    for doc in self.docs.values_mut() {
                        doc.state.set_user_id(*user_id);
                        doc.state.set_display_name(display_name);
                    }
                }
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
                from_user_id,
                payload,
            } => {
                self.apply_ephemeral(document_id, from_user_id, &payload);
            }

            LogicEvent::PeerLeft {
                document_id,
                user_id,
            } => {
                self.peer_left(document_id, user_id);
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
            LogicEvent::DocumentReady { document_id, role } => {
                // Sync has now resolved our real role from the members response;
                // it supersedes the optimistic `Owner` the slot was opened with
                // (architecture). A ready for a document already closed is
                // simply nothing to record.
                if let Some(doc) = self.docs.get_mut(&document_id) {
                    doc.role = role;
                }
                self.send_to_ui(UiEvent::DocumentReady { document_id, role });
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
                id,
                request,
                code,
                message,
            } => {
                // A failure correlated to a document's own request
                // (`toile-media`) aborts its pending import rather than
                // surfacing a raw API error to the UI. Uncorrelated errors (id
                // absent, or not routed) forward verbatim, exactly as before.
                if let Some(id) = id
                    && let Some(document_id) = self.rest_routes.remove(&id)
                {
                    self.route_service_error(document_id, id, message);
                    return;
                }
                self.send_to_ui(UiEvent::ApiError {
                    id,
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
// service plane of ambre-spec-message-bus.md).
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::ToileCommand;
    use ambre_bus::{DocumentId, RequestId, RestCommand, RestEvent, TrameCommand};
    use std::io::Cursor;

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
        state.docs.insert(
            doc_id,
            LiveDoc::new(DocState::Trame(Box::new(TrameState::new()))),
        );
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
    fn session_ready_seeds_user_id_into_open_and_future_toile_docs() {
        // #1: SessionReady seeds `my_user_id` into every already-open
        // document and into any opened afterwards.
        let (mut state, _ui, _sync) = harness();
        let already_open = DocumentId::new();
        state.docs.insert(
            already_open,
            LiveDoc::new(DocState::Toile(Box::new(ToileState::new()))),
        );

        let me = ambre_bus::UserId::new();
        state.handle_logic_event(LogicEvent::Rest {
            id: RequestId::next(),
            evt: RestEvent::SessionReady {
                user_id: me,
                display_name: "Ada".into(),
                email: "ada@example.com".into(),
            },
        });

        let toile_user_id = |state: &LogicState, id: &DocumentId| match state.docs.get(id) {
            Some(LiveDoc {
                state: DocState::Toile(s),
                ..
            }) => s.user_id(),
            other => panic!("expected an open Toile doc, got {}", other.is_some()),
        };
        assert_eq!(toile_user_id(&state, &already_open), Some(me));

        // A document opened after authentication is seeded immediately.
        let opened_later = DocumentId::new();
        state.handle_logic_command(LogicCommand::OpenDocument {
            document_id: opened_later,
            doc_type: DocType::Toile,
        });
        assert_eq!(toile_user_id(&state, &opened_later), Some(me));
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
        state.docs.insert(
            doc_id,
            LiveDoc::new(DocState::Trame(Box::new(TrameState::new()))),
        );
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

    /// A mutating Trame command (writes a cell, hence a CRDT delta).
    fn set_cell(text: &str) -> AppCommand {
        AppCommand::Trame(TrameCommand::SetCellText {
            row: 0,
            col: 0,
            text: text.into(),
        })
    }

    /// Seed a live document already carrying `role`, skipping the open handshake.
    fn live_trame(state: &mut LogicState, role: Role) -> DocumentId {
        let doc_id = DocumentId::new();
        let mut doc = LiveDoc::new(DocState::Trame(Box::new(TrameState::new())));
        doc.role = role;
        state.docs.insert(doc_id, doc);
        doc_id
    }

    #[test]
    fn viewer_edit_is_dropped_and_emits_no_update() {
        let (mut state, ui_rx, sync_rx) = harness();
        let doc_id = live_trame(&mut state, Role::Viewer);

        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: set_cell("nope"),
        });

        // The engine is never reached: no display event, and above all no CRDT
        // delta on its way to the server (architecture).
        assert!(ui_rx.try_recv().is_err(), "viewer edit produced a UI event");
        assert!(
            sync_rx.try_recv().is_err(),
            "viewer edit produced an outbound update"
        );
    }

    #[test]
    fn viewer_read_only_command_still_passes() {
        let (mut state, ui_rx, _sync) = harness();
        let doc_id = live_trame(&mut state, Role::Viewer);

        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: scroll(1),
        });

        // Scrolling is local view state: the gate must not touch it, or a viewer
        // could not read the document it is allowed to read.
        assert!(matches!(ui_rx.try_recv(), Ok(UiEvent::Viewport { .. })));
    }

    #[test]
    fn editor_edit_passes_through() {
        let (mut state, _ui, sync_rx) = harness();
        let doc_id = live_trame(&mut state, Role::Editor);

        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: set_cell("42"),
        });

        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::SendUpdate { .. })
        ));
    }

    #[test]
    fn document_ready_supersedes_the_optimistic_owner_role() {
        let (mut state, ui_rx, sync_rx) = harness();
        let doc_id = DocumentId::from_bytes([3; 16]);

        // A freshly opened slot is optimistically writable, before Sync has
        // resolved our real role.
        state.handle_logic_command(LogicCommand::OpenDocument {
            document_id: doc_id,
            doc_type: DocType::Trame,
        });
        assert_eq!(state.docs[&doc_id].role, Role::Owner);

        // The members response says otherwise: the gate closes.
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: doc_id,
            role: Role::Viewer,
        });
        assert_eq!(state.docs[&doc_id].role, Role::Viewer);

        let _ = sync_rx.drain();
        let _ = ui_rx.drain();
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: set_cell("nope"),
        });
        assert!(
            sync_rx.try_recv().is_err(),
            "edit passed after the role was downgraded to viewer"
        );
    }

    // -- Composition: the registry is flat ------------------------------
    //
    // A `doc_embed` child is an ordinary entry in `docs`, keyed by its own id, so
    // a command for it is addressed straight to it and never travels through the
    // parent board. What that buys is the property below: role is resolved per
    // document. These tests seed a parent and a child side by side and pin that
    // the parent's role has no say over the child's -- both directions.

    /// Seed a live Toile already carrying `role`, skipping the open handshake.
    fn live_toile(state: &mut LogicState, role: Role) -> DocumentId {
        let doc_id = DocumentId::new();
        let mut toile = ToileState::new();
        toile.set_user_id(UserId::new());
        let mut doc = LiveDoc::new(DocState::Toile(Box::new(toile)));
        doc.role = role;
        state.docs.insert(doc_id, doc);
        doc_id
    }

    /// Drain every `SendUpdate` and report which documents produced one -- which
    /// document a delta belongs to is exactly what these tests are about.
    fn updated_docs(sync_rx: &flume::Receiver<SyncCommand>) -> Vec<DocumentId> {
        sync_rx
            .drain()
            .filter_map(|cmd| match cmd {
                SyncCommand::SendUpdate { document_id, .. } => Some(document_id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_embedded_child_is_gated_by_its_own_role_not_the_boards() {
        let (mut state, _ui, sync_rx) = harness();
        // We may only read the board, but we own the document sitting on it.
        let _board = live_toile(&mut state, Role::Viewer);
        let child = live_trame(&mut state, Role::Owner);

        state.handle_logic_command(LogicCommand::App {
            document_id: child,
            command: set_cell("42"),
        });

        // Exactly one delta, keyed on the child. Gating this on the board's role
        // would deny a viewer an edit to a document they own inside it.
        assert_eq!(updated_docs(&sync_rx), vec![child]);
    }

    #[test]
    fn owning_the_board_grants_nothing_on_a_document_embedded_in_it() {
        let (mut state, _ui, sync_rx) = harness();
        let _board = live_toile(&mut state, Role::Owner);
        let child = live_trame(&mut state, Role::Viewer);

        state.handle_logic_command(LogicCommand::App {
            document_id: child,
            command: set_cell("nope"),
        });

        // The converse of the test above.
        assert!(updated_docs(&sync_rx).is_empty());
    }

    #[test]
    fn a_command_for_an_unopened_document_is_dropped() {
        let (mut state, ui_rx, sync_rx) = harness();

        state.handle_logic_command(LogicCommand::App {
            document_id: DocumentId::new(),
            command: set_cell("42"),
        });

        // Never synthesise the document: an embed may point at one this client
        // cannot open at all, and an empty state would paint as if it were the
        // real one.
        assert!(updated_docs(&sync_rx).is_empty());
        assert!(ui_rx.try_recv().is_err());
    }

    #[test]
    fn snapshot_required_triggers_send_snapshot() {
        let (mut state, _ui, sync_rx) = harness();
        let doc_id = DocumentId::new();
        state.docs.insert(
            doc_id,
            LiveDoc::new(DocState::Trame(Box::new(TrameState::new()))),
        );
        state.handle_logic_event(LogicEvent::SnapshotNeeded {
            document_id: doc_id,
        });
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::SendSnapshot { .. })
        ));
    }

    // -- Service-plane routing: interop media uploads (`toile-media`) ------
    //
    // A Toile media import issues `UploadMedia` requests the dispatcher lowers to
    // `RestCommand`s and whose answering `RestEvent`s it feeds back into the
    // owning engine instead of forwarding to the UI. These pin that round trip.

    /// A minimal solid-color PNG, decoded by the Toile image converter into one
    /// media blob (hence one `UploadMedia` request).
    fn solid_png(w: u32, h: u32) -> Vec<u8> {
        let buf = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(buf)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .expect("encode png");
        bytes
    }

    /// Import a solid PNG into `doc_id` and return the single `RequestId` the
    /// lowered `UploadMedia` was correlated with.
    fn import_one_image(state: &mut LogicState, doc_id: DocumentId) -> RequestId {
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::ImportImage {
                bytes: solid_png(8, 8),
                mime: "image/png".into(),
                at: kurbo::Point::ZERO,
            }),
        });
        assert_eq!(
            state.rest_routes.len(),
            1,
            "the import should record exactly one service route"
        );
        *state.rest_routes.keys().next().unwrap()
    }

    #[test]
    fn toile_upload_request_lowers_with_doc_id_and_records_route() {
        let (mut state, _ui, sync_rx) = harness();
        let doc_id = live_toile(&mut state, Role::Owner);
        let id = import_one_image(&mut state, doc_id);

        // The route points the correlated response back at the owning document.
        assert_eq!(state.rest_routes.get(&id), Some(&doc_id));

        // The request was lowered to a `RestCommand::UploadMedia` carrying the
        // document id the engine never holds, echoing Toile's own id.
        let uploaded = sync_rx.drain().find_map(|cmd| match cmd {
            SyncCommand::Rest {
                id: got,
                cmd:
                    RestCommand::UploadMedia {
                        document_id,
                        mime_type,
                        ..
                    },
            } => Some((got, document_id, mime_type)),
            _ => None,
        });
        let (got_id, got_doc, mime) = uploaded.expect("an UploadMedia was forwarded to Sync");
        assert_eq!(got_id, id);
        assert_eq!(got_doc, doc_id);
        assert_eq!(mime, "image/png");
    }

    #[test]
    fn routed_media_uploaded_lands_in_engine_not_the_ui() {
        let (mut state, ui_rx, _sync) = harness();
        let doc_id = live_toile(&mut state, Role::Owner);
        let id = import_one_image(&mut state, doc_id);
        let _ = ui_rx.drain();

        // The confirmation is routed into the engine, which applies the import.
        state.handle_logic_event(LogicEvent::Rest {
            id,
            evt: RestEvent::MediaUploaded {
                document_id: doc_id,
                content_hash: "blake2b-real".into(),
            },
        });

        // The route is consumed and the outcome surfaces as a viewport repaint,
        // never as a raw `Rest` event to the UI.
        assert!(state.rest_routes.is_empty());
        let events: Vec<UiEvent> = ui_rx.drain().collect();
        assert!(
            events.iter().any(|e| matches!(e, UiEvent::Viewport { .. })),
            "the applied import should repaint the board"
        );
        assert!(
            !events.iter().any(|e| matches!(e, UiEvent::Rest { .. })),
            "a routed response must not reach the UI as a Rest event"
        );
    }

    #[test]
    fn an_unrouted_rest_event_still_forwards_to_the_ui() {
        // Regression guard: a UI-issued request's response is absent from
        // `rest_routes` and must forward verbatim exactly as before.
        let (mut state, ui_rx, _sync) = harness();
        state.handle_logic_event(LogicEvent::Rest {
            id: RequestId::next(),
            evt: RestEvent::DocumentList { documents: vec![] },
        });
        assert!(matches!(ui_rx.try_recv(), Ok(UiEvent::Rest { .. })));
    }

    #[test]
    fn a_correlated_upload_failure_aborts_the_import_without_a_raw_api_error() {
        let (mut state, ui_rx, _sync) = harness();
        let doc_id = live_toile(&mut state, Role::Owner);
        let id = import_one_image(&mut state, doc_id);
        let _ = ui_rx.drain();

        state.handle_logic_event(LogicEvent::ApiError {
            id: Some(id),
            request: "UploadMedia".into(),
            code: 500,
            message: "boom".into(),
        });

        // The route is dropped and the failure surfaces as an import failure on
        // the board, not as a stray `ApiError` to the UI.
        assert!(state.rest_routes.is_empty());
        let events: Vec<UiEvent> = ui_rx.drain().collect();
        assert!(
            !events.iter().any(|e| matches!(e, UiEvent::ApiError { .. })),
            "a correlated failure must not reach the UI as a raw ApiError"
        );
        assert!(
            events.iter().any(|e| matches!(e, UiEvent::Viewport { .. })),
            "the abort should emit an ImportFailed viewport event"
        );
    }

    #[test]
    fn an_uncorrelated_api_error_still_forwards_to_the_ui() {
        let (mut state, ui_rx, _sync) = harness();
        state.handle_logic_event(LogicEvent::ApiError {
            id: None,
            request: "ListDocuments".into(),
            code: 500,
            message: "boom".into(),
        });
        assert!(matches!(ui_rx.try_recv(), Ok(UiEvent::ApiError { .. })));
    }

    #[test]
    fn a_late_response_after_close_is_dropped_without_panic() {
        let (mut state, ui_rx, _sync) = harness();
        let doc_id = live_toile(&mut state, Role::Owner);
        let id = import_one_image(&mut state, doc_id);
        let _ = ui_rx.drain();

        // Closing the document drops its in-flight route so the response can no
        // longer reach a (now absent) engine, and cannot repaint the board.
        state.handle_logic_command(LogicCommand::CloseDocument {
            document_id: doc_id,
        });
        assert!(state.rest_routes.is_empty());
        assert!(!state.docs.contains_key(&doc_id));

        state.handle_logic_event(LogicEvent::Rest {
            id,
            evt: RestEvent::MediaUploaded {
                document_id: doc_id,
                content_hash: "blake2b-real".into(),
            },
        });
        // No panic, and above all no viewport for a document that no longer exists.
        assert!(
            !ui_rx.drain().any(|e| matches!(e, UiEvent::Viewport { .. })),
            "a closed document must not repaint"
        );
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

    /// `LoadFonts` seeds the export `FontContext` once and is local-only: it
    /// touches neither the UI nor the Sync plane, and a second seeding is a
    /// no-op (`toile-media`).
    #[test]
    fn load_fonts_seeds_export_context_once() {
        let (mut state, ui_rx, sync_rx) = harness();
        assert!(state.export_cx.is_none());

        // Empty faces still build a context: the seed happens even when the
        // fetch delivered nothing (a missing face degrades text, not boot).
        state.handle_logic_command(LogicCommand::LoadFonts { faces: Vec::new() });
        assert!(state.export_cx.is_some(), "export context is seeded");
        assert!(ui_rx.drain().next().is_none(), "no UI event");
        assert!(sync_rx.try_recv().is_err(), "nothing forwarded to Sync");

        // A second delivery is ignored -- the guard keeps the first context.
        state.handle_logic_command(LogicCommand::LoadFonts { faces: Vec::new() });
        assert!(state.export_cx.is_some());
    }
}
