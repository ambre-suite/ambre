// ambre-logic/src/dispatch.rs
// ============================================================================
// The two inbound dispatchers:
//   - `handle_logic_command`: UI Thread -> Logic (content, control, service).
//   - `handle_logic_event`:  Sync -> Logic (real-time push, service responses).
// Both are pure Rust (no wasm glue). The whole routing layer is therefore
// exercised on the host by the `tests` module at the bottom, with test link
// queues.
// ============================================================================

use super::*;

impl LogicState {
    // -- Inbound: UI Thread -> Logic --------------------------------------------

    pub(crate) fn handle_logic_command(&mut self, command: LogicCommand) {
        match command {
            // -- P2 -- Content plane --------------------------------------------
            LogicCommand::App {
                document_id,
                command,
            } => {
                self.handle_app_command(document_id, command);
            }
            LogicCommand::LocalStoreFull { document_id, full } => {
                self.store_full(document_id, full);
            }

            // -- Local restore (Main thread, startup) --------------------------
            // Import a locally persisted snapshot into the document's state and
            // forward the resulting viewport events to the UI. Local-only: never
            // forwarded to Sync.
            LogicCommand::RestoreLocalSnapshot {
                document_id,
                doc_type,
                snapshot,
                deltas,
                sessions,
            } => {
                // The local copy a reopening channel asked for.
                if doc_type == DocType::Murmure && self.reads_channel_copy(document_id) {
                    self.channel_copy_landed(document_id, snapshot, &deltas);
                    return;
                }
                // The local copy a reopen asked for (`RESID-010`).
                if let Some(horizon) = self
                    .docs
                    .get_mut(&document_id)
                    .and_then(|doc| doc.restoring.take())
                {
                    self.reopen_from_store(
                        document_id,
                        doc_type,
                        horizon,
                        HeldRestore { snapshot, deltas },
                        sessions,
                    );
                    return;
                }
                // Order-independent: the boot-time local restore and the route
                // poll's `OpenDocument` race on startup. If the restore arrives
                // first, materialise the live document here so the snapshot is
                // never dropped. A later `OpenDocument` finds the slot occupied
                // and reuses it.
                if !self.docs.contains_key(&document_id)
                    && let Some(state) = self.fresh_state(doc_type)
                {
                    self.docs.insert(document_id, LiveDoc::new(state));
                }
                let Some(doc) = self.docs.get_mut(&document_id) else {
                    tracing::warn!(
                        "Logic: no live document {document_id} to restore snapshot into"
                    );
                    return;
                };
                // A restore can write under the document's PeerID, and nothing
                // written before the session identity can travel (`AUTHOR-008`).
                // It is shown now and kept, and the identity rebuilds the
                // document from it (`AUTHOR-027`).
                if doc.identity.is_none() {
                    doc.held_restores.push(HeldRestore {
                        snapshot: snapshot.clone(),
                        deltas: deltas.clone(),
                    });
                    let _ = self.restore_local(document_id, snapshot, deltas, Restore::Shown);
                } else {
                    let _ = self.restore_local(document_id, snapshot, deltas, Restore::Kept);
                }
                // The sessions that wrote the restored operations, so what they
                // left unsent travels (`AUTHOR-028`).
                self.note_earlier_sessions(document_id, sessions);
            }

            // -- P3 -- Service plane: forward verbatim, with state hooks --------
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
                        self.evicted.clear();
                        self.store_full.clear();
                        self.channels.clear();
                        self.channel_ledgers.clear();
                        self.opening.clear();
                        self.read_cursors.clear();
                        self.closed_channels.clear();
                        self.channel_watchers.clear();
                        self.spaces.clear();
                        self.space_opening.clear();
                        self.creating.clear();
                        self.joining.clear();
                        self.direct.clear();
                        self.unsent.clear();
                        self.murmure_identities.clear();
                        self.murmure_waiting.clear();
                        self.owed.clear();
                        self.rest_routes.clear();
                        self.forget_pending_seals(None);
                    }
                    RestCommand::DeleteDocument { document_id } => {
                        self.docs.remove(document_id);
                        self.evicted.remove(document_id);
                        self.owed.remove(document_id);
                        self.rest_routes.retain(|_, d| d != document_id);
                        self.forget_pending_seals(Some(*document_id));
                    }
                    _ => {}
                }
                self.send_to_sync(SyncCommand::Rest { id, cmd });
            }

            // -- P3 -- Control plane: explicit (no correlatable response) -------
            LogicCommand::OpenDocument {
                document_id,
                doc_type,
            } => {
                // A view shows it: its local copy stays (`RESID-003`).
                self.send_to_persist(document_id, PersistWrite::Shown);
                // A document that left memory reads its local copy back first
                // (`RESID-010`).
                if self.reopen_evicted(document_id, doc_type) {
                    return;
                }
                // Pre-seed the in-memory state so UI edits are routed immediately,
                // before any snapshot/delta arrives (same rationale as the smoke
                // doc). A later RemoteSnapshot reuses the existing state.
                if !self.docs.contains_key(&document_id)
                    && let Some(state) = self.fresh_state(doc_type)
                {
                    self.docs.insert(document_id, LiveDoc::new(state));
                }
                self.send_to_sync(SyncCommand::OpenDocument {
                    document_id,
                    doc_type,
                });
                // An empty document is one blank page (`PROSE-011`), and the
                // view shows it before anything arrives.
                self.owe_layout(document_id);
            }
            LogicCommand::CloseDocument { document_id } => {
                // The last edits of the pass are saved before the state goes,
                // so closing a document never loses them (`ARCH-017`). It then
                // leaves memory, and Sync keeps it light (`RESID-001`).
                self.evict(document_id);
            }
            LogicCommand::OpenChannel { channel_id } => self.open_channel(channel_id),
            LogicCommand::CloseChannel { channel_id } => self.close_channel(channel_id),
            LogicCommand::OpenDirectMessage { draft, now } => self.open_direct_message(draft, now),
            LogicCommand::FetchActivity => self.send_to_sync(SyncCommand::FetchActivity),
            LogicCommand::ListMemberChannels => {
                self.send_to_sync(SyncCommand::ListMemberChannels);
            }
            LogicCommand::MarkAllRead => self.send_to_sync(SyncCommand::MarkAllRead),
            LogicCommand::SearchMessages { query } => self.search_messages(query),
            LogicCommand::CreateSpace {
                space_id,
                name,
                now,
            } => self.create_space(space_id, &name, now),
            LogicCommand::OpenSpace { space_id } => self.open_space(space_id),
            LogicCommand::CloseSpace { space_id } => self.close_space(space_id),
            LogicCommand::Space { space_id, command } => {
                self.handle_space_command(space_id, command);
            }
            // -- Content plane: a Murmure command, addressed to its channel ---
            LogicCommand::Channel {
                channel_id,
                command,
            } => self.handle_channel_command(channel_id, command),
            LogicCommand::LoadFonts { faces } => {
                // The Logic thread cannot fetch, so this is the only way a face
                // reaches it (`CANVAS-067`). Local-only: nothing is sent to
                // Sync. Two consumers take the same bytes.
                tracing::info!("Logic: {} faces delivered", faces.len());
                // Every open document that measures text. A Prose paginator
                // shapes on a sealed context and holds nothing until now
                // (`PROSE-114`, `PROSE-115`).
                for doc in self.docs.values_mut() {
                    doc.state.load_faces(&faces);
                }
                // A new face re-shapes the runs it covers (`PROSE-111`).
                let open: Vec<DocumentId> = self.docs.keys().copied().collect();
                for document_id in open {
                    self.owe_layout(document_id);
                }
                // The PDF-export context (`toile-pdf-export`), built on the
                // first delivery and extended by each later one, so a script
                // face that arrived after boot exports too. Sealed, so the page
                // is shaped on these faces and on none the machine carries
                // (`PDF-007`).
                let export_cx = self
                    .export_cx
                    .get_or_insert_with(ambre_draw::sealed_font_context);
                for bytes in &faces {
                    export_cx
                        .collection
                        .register_fonts(bytes.clone().into(), None);
                }
                // Kept for the documents opened after this point.
                self.base_faces.extend(faces);
            }
        }
    }

    // -- Inbound: Sync -> Logic -------------------------------------------------

    pub(crate) fn handle_logic_event(&mut self, event: LogicEvent) {
        match event {
            // -- Service plane: forward verbatim -------------------------------
            //
            // The whole relayed response family collapses to one arm: a pure
            // forward to the UI. Documents are materialised on `OpenDocument`
            // (which carries the `doc_type`), so no event here mutates state.
            LogicEvent::Rest { id, evt } => {
                // The answer to a sealed `.ambre` export this thread asked for
                // (`DOCFILE-014`). It becomes a display event of the owning
                // application here, and never reaches the engine, which knows
                // nothing of the seal.
                if self.resolve_pending_seal(id, &evt) {
                    return;
                }
                // A response to a request this Logic thread issued on behalf of a
                // document (`toile-media`) is fed back into the owning engine. It
                // is not forwarded to the UI. UI-issued requests are absent from
                // `rest_routes` and fall through to the verbatim forward.
                if let Some(document_id) = self.rest_routes.remove(&id) {
                    self.route_service_event(document_id, id, evt);
                    return;
                }
                // `SessionReady` also seeds the canonical `UserId`. Capture it
                // for later opens and push it into every already-open document,
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
                    self.spaces_know_me(*user_id);
                }
                self.send_to_ui(UiEvent::Rest { id, evt });
            }

            // -- Real-time sync (server push) ----------------------------------
            // The document is already live (pre-seeded on `OpenDocument`); a
            // snapshot for an unopened doc is dropped by `apply_remote`.
            // The relay's numbers also give each message of a channel segment
            // its place in the relay's order (the Murmure unread spec).
            LogicEvent::RemoteSnapshot {
                document_id,
                payload,
                covered_seq,
            } => {
                self.adopt_snapshot(document_id, &payload, covered_seq);
            }

            // Checked against the deltas held, never imported (`AUTHOR-018`).
            LogicEvent::PendingSnapshot {
                document_id,
                snapshot_id,
                covered_seq,
                payload,
                purpose,
            } => {
                self.check_pending_snapshot(
                    document_id,
                    snapshot_id,
                    covered_seq,
                    &payload,
                    purpose,
                );
            }

            // Sync derived the PeerID from the signer; Logic compares it with
            // every operation the delta carries (`AUTHOR-012`).
            LogicEvent::RemoteUpdate { document_id, delta } => {
                self.apply_signed(document_id, &delta);
            }

            LogicEvent::RemoteUpdateBatch {
                document_id,
                deltas,
            } => {
                self.apply_signed_batch(document_id, deltas);
            }

            LogicEvent::SessionIdentity {
                document_id,
                peer,
                entry,
                nonce,
            } => {
                self.adopt_session(document_id, peer, entry, nonce);
            }

            LogicEvent::RemoteEphemeral {
                document_id,
                from_user_id,
                payload,
            } => {
                if !self.space_presence_heard(document_id, from_user_id, &payload)
                    && !self.channel_sample_heard(document_id, from_user_id, &payload)
                {
                    self.apply_ephemeral(document_id, from_user_id, &payload);
                }
            }

            LogicEvent::PeerLeft {
                document_id,
                user_id,
            } => {
                if !self.space_peer_left(document_id, user_id)
                    && !self.channel_peer_left(document_id, user_id)
                {
                    self.peer_left(document_id, user_id);
                }
            }

            LogicEvent::MembersChanged {
                document_id,
                members,
            } => {
                // Only a space shows its members live today; any other
                // document reads them when its view asks.
                self.space_members_changed(document_id, members);
            }

            LogicEvent::SnapshotNeeded { document_id } => {
                let covered = self
                    .export_covered_snapshot(document_id)
                    .or_else(|| self.channel_snapshot(document_id))
                    .or_else(|| self.space_snapshot(document_id));
                if let Some((payload, covered_seq)) = covered {
                    self.send_to_sync(SyncCommand::SendSnapshot {
                        document_id,
                        payload,
                        covered_seq,
                    });
                }
                // After the export: the snapshot holds the signed deltas alone,
                // and the removal travels as the next one
                // (`ambre-spec-prose-schema.md` section 2).
                self.sweep_hidden(document_id);
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

            // Counted by Sync from the summaries alone, for a document this
            // thread no longer holds (`RESID-005`).
            LogicEvent::Activity {
                document_id,
                counters,
            } => {
                // A channel's document counts for its channel, not alone.
                if !self.count_closed(document_id, counters) {
                    self.send_to_ui(UiEvent::Activity {
                        document_id,
                        counters,
                    });
                }
            }
            LogicEvent::Released { document_id } => {
                self.store_sent(document_id);
            }

            // Initial hydration complete: any snapshot was applied by the
            // preceding RemoteSnapshot arm (queue order is preserved), so the
            // UI can dismiss its loading overlay.
            LogicEvent::DocumentReady { document_id, role } => {
                // Sync has now resolved our real role from the members response;
                // it supersedes the optimistic `Owner` the slot was opened with
                // (architecture). A ready for a document already closed is
                // simply nothing to record.
                if let Some(doc) = self.docs.get_mut(&document_id) {
                    doc.role = role;
                    doc.ready = true;
                } else {
                    // A document this client created goes up whole first.
                    self.flush_unsent(document_id);
                    if self.channel_document_ready(document_id)
                        || self.space_document_ready(document_id, role)
                    {
                        // A channel or a space document is no document of
                        // the view: its channel or space answers for it.
                        return;
                    }
                }
                self.send_to_ui(UiEvent::DocumentReady { document_id, role });
                // The other sessions learn this one is open, though its caret
                // never moves (`ambre-spec-prose-schema.md` section 2).
                self.announce(document_id);
                // The relay sent its whole replay. What reads all of it runs
                // once its last slice is imported (`SYNC-131`).
                self.ready(document_id);
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
                if Some(revoked_user_id) == self.my_user_id {
                    self.channel_revoked(document_id);
                }
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

            // -- Channels (Murmure) --------------------------------------------
            LogicEvent::ChannelDocuments {
                channel_id,
                documents,
            } => self.channel_listed(channel_id, &documents),
            LogicEvent::ChannelOpenFailed {
                channel_id,
                message,
            } => {
                if self.opening.remove(&channel_id).is_some() {
                    self.end_light_channel(channel_id);
                    self.channel_open_failed(channel_id, message);
                }
            }
            LogicEvent::ChannelRefused { channel_id } => self.channel_refused(channel_id),
            LogicEvent::ChannelActivity { documents } => self.channel_activity(&documents),
            LogicEvent::ChannelCreated { channel_id, key } => {
                self.channel_created(channel_id, key);
            }
            LogicEvent::ChannelJoined { channel_id } => self.channel_joined(channel_id),
            LogicEvent::ChannelDeleted { channel_id } => self.channel_deleted(channel_id),
            LogicEvent::ChannelLeft { channel_id } => self.channel_left(channel_id),
            LogicEvent::DirectMessageOpened {
                requested,
                channel_id,
            } => self.direct_message_opened(requested, channel_id),
            LogicEvent::ChannelRequestFailed {
                channel_id,
                message,
            } => self.channel_request_failed(channel_id, message),
            LogicEvent::SpaceChannels { space_id, channels } => {
                self.space_channels(space_id, &channels);
            }
            // The sidebar learns of the channels, and none of them opens
            // (the Murmure channels spec).
            LogicEvent::MemberChannels { channels } => {
                self.send_to_ui(UiEvent::MemberChannels { channels });
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
            LogicEvent::DocumentOpenFailed {
                document_id,
                message,
            } => {
                if !self.channel_document_failed(document_id, &message)
                    && !self.space_document_failed(document_id, &message)
                {
                    self.send_to_ui(UiEvent::DocumentOpenFailed {
                        document_id,
                        message,
                    });
                }
            }
            LogicEvent::AuthError { message } => {
                self.send_to_ui(UiEvent::AuthError { message });
            }
            LogicEvent::SetupRequired => self.send_to_ui(UiEvent::SetupRequired),
            LogicEvent::UpdateRequired => self.send_to_ui(UiEvent::UpdateRequired),
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
// Tests -- the dispatcher is 100% pure Rust (no wasm glue), so the whole routing
// layer is exercisable on the host with test link queues. These pin the
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
    fn harness() -> (LogicState, Receiver<UiEvent>, Receiver<SyncCommand>) {
        let (to_ui, ui_rx) = test_link();
        let (to_sync, sync_rx) = test_link();
        // The persist queue is drained by the Main thread in production; tests
        // keep the receiver alive so sends never fail.
        let (to_persist, persist_rx) = test_link();
        std::mem::forget(persist_rx);
        (LogicState::new(to_ui, to_sync, to_persist), ui_rx, sync_rx)
    }

    /// Drive `ExportAmbreFile` on a live document of `doc_type` and hand back
    /// the bytes the UI would download.
    ///
    /// Goes through the real dispatcher rather than calling the engine, because
    /// the rule under test is exactly what the dispatcher adds. The header's
    /// identity comes from the registry that keyed the document, and an engine
    /// has no way to supply it.
    fn exported_file(doc_type: DocType, document_id: DocumentId) -> Vec<u8> {
        let (mut state, ui, _sync) = harness();
        state.open_with_identity(document_id, doc_type);
        let command = match doc_type {
            DocType::Trame => AppCommand::Trame(TrameCommand::ExportAmbreFile),
            DocType::Toile => AppCommand::Toile(ToileCommand::ExportAmbreFile),
            DocType::Prose => {
                AppCommand::Prose(ambre_bus::app::prose::ProseCommand::ExportDocument {
                    format: ambre_bus::app::prose::ExportFormat::Ambre,
                })
            }
            other => panic!("no engine for {other:?}"),
        };
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command,
        });
        // The answer is the one event carrying bytes; every other event the open
        // produced is a viewport refresh.
        std::iter::from_fn(|| ui.try_recv().ok())
            .find_map(|evt| match evt {
                UiEvent::Viewport {
                    event: AppEvent::Trame(ambre_bus::TrameEvent::ExportComplete { data, .. }),
                    ..
                }
                | UiEvent::Viewport {
                    event:
                        AppEvent::Toile(ambre_bus::app::toile::ToileEvent::ExportComplete {
                            data, ..
                        }),
                    ..
                }
                | UiEvent::Viewport {
                    event:
                        AppEvent::Prose(ambre_bus::app::prose::ProseEvent::ExportReady {
                            data, ..
                        }),
                    ..
                } => Some(data),
                _ => None,
            })
            .expect("the export answered with bytes")
    }

    /// The bytes `ExportAmbreFile` would answer with, for a document already
    /// live in `state`. The command path itself is driven by
    /// [`exported_file`]; this reads the same answer without a second registry.
    fn exported_file_from(state: &LogicState, document_id: DocumentId) -> Vec<u8> {
        let doc = state.docs.get(&document_id).expect("a live document");
        match doc.state.ambre_file_event(document_id) {
            AppEvent::Trame(ambre_bus::TrameEvent::ExportComplete { data, .. })
            | AppEvent::Toile(ambre_bus::app::toile::ToileEvent::ExportComplete { data, .. })
            | AppEvent::Prose(ambre_bus::app::prose::ProseEvent::ExportReady { data, .. }) => data,
            other => panic!("the export answered with {other:?}"),
        }
    }

    /// `PROSE-IO-020`, `PROSE-IO-028`: the save command reaches the
    /// dispatcher, and the file it writes opens in a fresh Prose document.
    #[test]
    fn a_prose_document_saves_to_a_native_file_and_reopens() {
        let document_id = DocumentId::new();
        let bytes = exported_file(DocType::Prose, document_id);
        let mut reopened = ambre_prose::logic::ProseState::with_peer(9).expect("a document");
        let (id, _) = reopened
            .import_ambre_plaintext(&bytes)
            .expect("the file opens");
        assert_eq!(id, document_id.into());
    }

    /// `PROSE-062`: one outcome, three destinations.
    ///
    /// The display events reach the UI thread. The delta reaches the Sync
    /// worker under the document id the registry keyed, and the presence
    /// sample reaches it as an ephemeral. An engine answers a value and sends
    /// nothing, so this fan-out is the dispatcher's whole job and no
    /// engine-side test can reach it.
    #[test]
    fn a_prose_outcome_reaches_the_ui_thread_and_the_sync_worker() {
        let (mut state, ui, sync) = harness();
        let document_id = DocumentId::new();
        state.open_with_identity(document_id, DocType::Prose);
        let block = match &state.docs.get(&document_id).expect("a live document").state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ambre_bus::app::prose::ProseCommand::InsertText {
                block_id: block,
                pos: 0,
                text: "typed".to_owned(),
            }),
        });

        let confirmed = std::iter::from_fn(|| ui.try_recv().ok())
            .find_map(|evt| match evt {
                UiEvent::Viewport {
                    event: AppEvent::Prose(ambre_bus::app::prose::ProseEvent::DocUpdated { delta }),
                    document_id: id,
                } if id == document_id => Some(delta),
                _ => None,
            })
            .expect("the confirmed change reached the UI thread");
        let broadcast = std::iter::from_fn(|| sync.try_recv().ok())
            .find_map(|cmd| match cmd {
                SyncCommand::SendUpdate {
                    document_id: id,
                    payload,
                    ..
                } if id == document_id => Some(payload),
                _ => None,
            })
            .expect("the delta reached the Sync worker");
        // One outcome, and the two halves carry the same bytes: the event is
        // not a re-export (`PROSE-023`).
        assert_eq!(confirmed, broadcast);
    }

    /// `PROSE-UI-006`: the layout reaches the view computed and shaped. The
    /// view adopts it from the event alone, and holds nothing that could lay
    /// text out. A caret move writes nothing, so it sends no layout.
    #[test]
    fn the_view_draws_a_layout_the_logic_thread_computed() {
        use ambre_bus::app::prose::ProseEvent;
        let (mut state, ui, _sync) = harness();
        let document_id = DocumentId::new();
        state.open_with_identity(document_id, DocType::Prose);
        state.answer_pass();
        let block = match &state.docs.get(&document_id).expect("a live document").state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        let prose = |command| LogicCommand::App {
            document_id,
            command: AppCommand::Prose(command),
        };
        state.handle_logic_command(prose(ambre_bus::app::prose::ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "typed".to_owned(),
        }));
        state.answer_pass();
        let drain = |ui: &Receiver<UiEvent>| -> Vec<ProseEvent> {
            std::iter::from_fn(|| ui.try_recv().ok())
                .filter_map(|event| match event {
                    UiEvent::Viewport {
                        event: AppEvent::Prose(event),
                        ..
                    } => Some(event),
                    _ => None,
                })
                .collect()
        };
        let events = drain(&ui);
        let layouts = events
            .iter()
            .filter(|event| matches!(event, ProseEvent::LayoutShaped { .. }))
            .count();
        // One on opening, one for the edit.
        assert_eq!(layouts, 2);
        let mut view = ambre_prose::ui::ProseUiState::default();
        for event in &events {
            view.apply_event(event);
        }
        assert_eq!(
            view.layout_version, 2,
            "the later layout is the one in force"
        );
        assert_eq!(view.shaped.layout.page_count, 1);
        assert!(
            !view.shaped.blocks.is_empty(),
            "the runs arrive shaped with the layout"
        );

        state.handle_logic_command(prose(ambre_bus::app::prose::ProseCommand::UpdateCursor {
            pos: ambre_bus::app::prose::CursorPosition {
                block_id: block,
                offset: 1,
                anchor: None,
            },
        }));
        assert!(
            !drain(&ui)
                .iter()
                .any(|event| matches!(event, ProseEvent::LayoutShaped { .. })),
            "a caret move lays nothing out"
        );
    }

    /// The other half of `PROSE-062`: the presence sample leaves as an
    /// ephemeral, and a caret produces no CRDT delta.
    #[test]
    fn a_prose_caret_leaves_as_an_ephemeral_and_not_as_a_delta() {
        let (mut state, _ui, sync) = harness();
        let document_id = DocumentId::new();
        state.open_with_identity(document_id, DocType::Prose);
        let block = match &state.docs.get(&document_id).expect("a live document").state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ambre_bus::app::prose::ProseCommand::UpdateCursor {
                pos: ambre_bus::app::prose::CursorPosition {
                    block_id: block,
                    offset: 0,
                    anchor: None,
                },
            }),
        });
        let sent: Vec<SyncCommand> = std::iter::from_fn(|| sync.try_recv().ok()).collect();
        assert!(
            sent.iter()
                .any(|cmd| matches!(cmd, SyncCommand::SendEphemeral { document_id: id, .. } if *id == document_id)),
            "the presence sample reached the Sync worker"
        );
        assert!(
            !sent
                .iter()
                .any(|cmd| matches!(cmd, SyncCommand::SendUpdate { .. })),
            "a caret is never written to the document"
        );
    }

    /// `DOCFILE-007`, and the reason this command is served by the dispatcher:
    /// the container's header carries the document's identity, which only the
    /// registry knows. An engine-side implementation could not write this byte
    /// range at all.
    #[test]
    fn a_saved_file_carries_the_document_id_the_registry_keyed_it_under() {
        for doc_type in [DocType::Trame, DocType::Toile, DocType::Prose] {
            let document_id = DocumentId::new();
            let bytes = exported_file(doc_type, document_id);
            let header = ambre_core::docfile::read_ambre_plaintext(&bytes)
                .expect("the command wrote a valid container");
            assert_eq!(header.doc_type, doc_type, "the header names the wrong app");
            assert_eq!(
                header.document_id,
                ambre_bus::Uuid::from(document_id),
                "the file would open as a different document than the one saved"
            );
        }
    }

    /// `DOCFILE-001` over the whole gesture: what the user saves is what opens,
    /// down the path the launcher actually takes. The file's payload is fed
    /// through `RestoreLocalSnapshot`, the same leg the open gesture reaches by
    /// way of the local snapshot store. This covers the hand-off the picker
    /// cannot test in a browser-free tier.
    ///
    /// The document is edited before saving. A round trip that silently
    /// returned an empty document fails here rather than passing on two
    /// identical fresh states.
    #[test]
    fn a_saved_file_opens_back_as_the_document_that_was_saved() {
        let document_id = DocumentId::new();
        let (mut state, _ui, _sync) = harness();
        state.open_with_identity(document_id, DocType::Trame);
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Trame(TrameCommand::SetCellText {
                row: 3,
                col: 1,
                text: "kept".to_owned(),
            }),
        });
        let saved = exported_file_from(&state, document_id);
        let header = ambre_core::docfile::read_ambre_plaintext(&saved).expect("valid container");

        // A second session, as a page load after the open gesture. Nothing is
        // live, and the file's payload arrives the way the store hands it over.
        let (mut reopened, ui, _sync) = harness();
        reopened.handle_logic_command(LogicCommand::RestoreLocalSnapshot {
            document_id: header.document_id.into(),
            doc_type: header.doc_type,
            snapshot: Some(header.payload),
            deltas: Vec::new(),
            sessions: Vec::new(),
        });
        reopened.grant_identity(document_id);
        reopened.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Trame(TrameCommand::ScrollViewport {
                row_start: 0,
                col_start: 0,
                row_end: 16,
                col_end: 8,
                version: 1,
            }),
        });
        let kept = std::iter::from_fn(|| ui.try_recv().ok()).any(|evt| match evt {
            UiEvent::Viewport {
                event: AppEvent::Trame(ambre_bus::TrameEvent::ViewportData { cells, .. }),
                ..
            } => cells
                .iter()
                .any(|c| c.row == 3 && c.col == 1 && c.text == "kept"),
            _ => false,
        });
        assert!(kept, "the saved file reopened without the edit it carried");
    }

    // -- The sealed file (`DOCFILE-014`) ---------------------------------------

    /// Open a document and ask for a sealed `.ambre`, returning the state, the
    /// UI queue, and the service request the seal produced.
    fn sealing(doc_type: DocType) -> (LogicState, Receiver<UiEvent>, RequestId, Vec<u8>) {
        let document_id = DocumentId::new();
        let (mut state, ui, sync_rx) = harness();
        state.open_with_identity(document_id, doc_type);
        let passphrase = ambre_bus::SecretBytes::from(b"a passphrase".to_vec());
        let command = match doc_type {
            DocType::Trame => AppCommand::Trame(TrameCommand::ExportAmbreFileSealed { passphrase }),
            DocType::Toile => AppCommand::Toile(ToileCommand::ExportAmbreFileSealed { passphrase }),
            other => panic!("no engine for {other:?}"),
        };
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command,
        });
        let (id, plaintext) = std::iter::from_fn(|| sync_rx.try_recv().ok())
            .find_map(|cmd| match cmd {
                SyncCommand::Rest {
                    id,
                    cmd: RestCommand::SealDocFile { plaintext, .. },
                } => Some((id, plaintext.to_vec())),
                _ => None,
            })
            .expect("the sealed export asked Sync to seal");
        (state, ui, id, plaintext)
    }

    /// The file bytes of the one export event on `ui`, if there is one.
    fn export_bytes(ui: &Receiver<UiEvent>) -> Option<Vec<u8>> {
        std::iter::from_fn(|| ui.try_recv().ok()).find_map(|evt| match evt {
            UiEvent::Viewport {
                event: AppEvent::Trame(ambre_bus::TrameEvent::ExportComplete { data, .. }),
                ..
            }
            | UiEvent::Viewport {
                event:
                    AppEvent::Toile(ambre_bus::app::toile::ToileEvent::ExportComplete { data, .. }),
                ..
            } => Some(data),
            _ => None,
        })
    }

    /// `ARCH-004`: the sealed export sends a plaintext container to the Sync
    /// worker and hands the UI nothing until the answer lands. No file reaches
    /// a surface unsealed, and this thread derives no key.
    #[test]
    fn a_sealed_export_hands_the_plaintext_to_sync_and_nothing_to_the_ui() {
        for doc_type in [DocType::Trame, DocType::Toile] {
            let (_state, ui, _id, plaintext) = sealing(doc_type);
            let header = ambre_core::docfile::read_ambre_plaintext(&plaintext)
                .expect("Sync was handed a valid plaintext container");
            assert_eq!(header.doc_type, doc_type);
            assert_eq!(
                export_bytes(&ui),
                None,
                "{doc_type:?}: a file reached the surface before it was sealed",
            );
        }
    }

    /// The sealed bytes come back as the application's ordinary export event,
    /// so the surface downloads them exactly as it downloads a plaintext file.
    #[test]
    fn the_sealed_bytes_come_back_as_the_application_s_own_export_event() {
        for doc_type in [DocType::Trame, DocType::Toile] {
            let (mut state, ui, id, _plaintext) = sealing(doc_type);
            state.handle_logic_event(LogicEvent::Rest {
                id,
                evt: RestEvent::DocFileSealed {
                    data: b"sealed bytes".to_vec(),
                },
            });
            assert_eq!(export_bytes(&ui).as_deref(), Some(&b"sealed bytes"[..]));
        }
    }

    /// A seal that failed leaves no file: the application says so through its
    /// own failure event rather than downloading something half-made.
    #[test]
    fn a_failed_seal_answers_a_failure_and_no_file() {
        for doc_type in [DocType::Trame, DocType::Toile] {
            let (mut state, ui, id, _plaintext) = sealing(doc_type);
            state.handle_logic_event(LogicEvent::Rest {
                id,
                evt: RestEvent::DocFileSealFailed,
            });
            let events: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    UiEvent::Viewport {
                        event: AppEvent::Trame(ambre_bus::TrameEvent::Error { .. })
                            | AppEvent::Toile(ambre_bus::app::toile::ToileEvent::ExportFailed(_)),
                        ..
                    }
                )),
                "{doc_type:?}: the failure did not reach the surface",
            );
            assert!(
                !events.iter().any(|e| matches!(
                    e,
                    UiEvent::Viewport {
                        event: AppEvent::Trame(ambre_bus::TrameEvent::ExportComplete { .. })
                            | AppEvent::Toile(
                                ambre_bus::app::toile::ToileEvent::ExportComplete { .. }
                            ),
                        ..
                    }
                )),
                "{doc_type:?}: a failed seal produced a file",
            );
        }
    }

    /// The passphrase never reaches the engine: the dispatcher intercepts the
    /// command by value and moves the secret straight to Sync. A document
    /// closed mid-seal has nothing left to answer.
    #[test]
    fn a_document_closed_mid_seal_drops_the_answer() {
        let (mut state, ui, id, _plaintext) = sealing(DocType::Trame);
        let document_id = *state.docs.keys().next().expect("a live document");
        state.handle_logic_command(LogicCommand::CloseDocument { document_id });
        state.handle_logic_event(LogicEvent::Rest {
            id,
            evt: RestEvent::DocFileSealed {
                data: b"sealed bytes".to_vec(),
            },
        });
        assert_eq!(
            export_bytes(&ui),
            None,
            "a seal answered into a closed document",
        );
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

    /// The history lives in the document's state, so closing drops it with the
    /// rest in each application (`UNDO-004`).
    #[test]
    fn closing_a_document_of_each_kind_drops_its_state() {
        for doc_type in [DocType::Trame, DocType::Toile, DocType::Prose] {
            let (mut state, _ui, sync_rx) = harness();
            let doc_id = DocumentId::new();
            let doc = DocState::new(doc_type).expect("a state for each application");
            state.docs.insert(doc_id, LiveDoc::new(doc));
            state.handle_logic_command(LogicCommand::CloseDocument {
                document_id: doc_id,
            });
            assert!(!state.docs.contains_key(&doc_id), "{doc_type:?} kept");
            assert!(matches!(
                sync_rx.try_recv(),
                Ok(SyncCommand::CloseDocument { .. })
            ));
        }
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
            snapshot: Some(snapshot),
            deltas: Vec::new(),
            sessions: Vec::new(),
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
        assert!(state.docs.is_empty(), "the unknown doc was synthesised");
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
        state.grant_identity(doc_id);
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

    /// An import hands the Sync worker a CRDT delta, never the bytes of the
    /// file the user opened. The worker seals that delta before it leaves.
    #[test]
    fn an_import_hands_sync_a_delta_and_never_the_file() {
        let (mut state, _ui, sync_rx) = harness();
        let doc_id = live_trame(&mut state, Role::Editor);
        let file = b"Quarterly report,Owner\nNorth region,Alice Marchand\n".to_vec();
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Trame(TrameCommand::ImportCsv {
                data: file.clone(),
                delimiter: None,
                has_header: false,
                target_sheet_id: None,
                start_row: 0,
                start_col: 0,
            }),
        });
        let sent: Vec<SyncCommand> = std::iter::from_fn(|| sync_rx.try_recv().ok()).collect();
        assert!(
            sent.iter()
                .any(|c| matches!(c, SyncCommand::SendUpdate { .. })),
            "the import reached no peer",
        );
        // Every byte buffer prints as a list of numbers: the file's run must
        // appear in none of them.
        let run = format!("{file:?}");
        let run = run.trim_start_matches('[').trim_end_matches(']');
        let forwarded = SyncCommand::SendUpdate {
            document_id: doc_id,
            payload: file.clone(),
            summary: None,
        };
        assert!(
            format!("{forwarded:?}").contains(run),
            "the search sees a forwarded file"
        );
        for cmd in &sent {
            assert!(
                !format!("{cmd:?}").contains(run),
                "the file left as it was opened"
            );
        }
    }

    #[test]
    fn document_ready_supersedes_the_optimistic_owner_role() {
        let (mut state, ui_rx, sync_rx) = harness();
        let doc_id = DocumentId::from_bytes([3; 16]);

        // A freshly opened slot is optimistically writable, before Sync has
        // resolved our real role.
        state.open_with_identity(doc_id, DocType::Trame);
        assert_eq!(state.docs[&doc_id].role, Role::Owner);

        // The members response says otherwise: the gate closes.
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: doc_id,
            role: Role::Viewer,
        });
        assert_eq!(state.docs[&doc_id].role, Role::Viewer);

        sync_rx.try_iter().for_each(drop);
        ui_rx.try_iter().for_each(drop);
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
    // A `doc_embed` child is an ordinary entry in `docs`, keyed by its own id. A
    // command for it is addressed straight to it and never travels through the
    // parent board. What that buys is the property below: role is resolved per
    // document. These tests seed a parent and a child side by side. They pin that
    // the parent's role has no say over the child's, in both directions.

    /// Seed a live Toile already carrying `role`, skipping the open handshake.
    fn live_toile(state: &mut LogicState, role: Role) -> DocumentId {
        let doc_id = DocumentId::new();
        let mut toile = ToileState::new();
        toile.set_user_id(UserId::new());
        let mut doc = LiveDoc::new(DocState::Toile(Box::new(toile)));
        doc.role = role;
        state.docs.insert(doc_id, doc);
        state.grant_identity(doc_id);
        doc_id
    }

    /// Drain every `SendUpdate` and report which documents produced one -- which
    /// document a delta belongs to is exactly what these tests are about.
    fn updated_docs(sync_rx: &Receiver<SyncCommand>) -> Vec<DocumentId> {
        sync_rx
            .try_iter()
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
        // would refuse a viewer an edit to a document they own inside it.
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

        // Never synthesise the document. An embed may point at one this client
        // cannot open at all, and an empty state would paint as if it were the
        // real one.
        assert!(updated_docs(&sync_rx).is_empty());
        assert!(ui_rx.try_recv().is_err());
        assert!(state.docs.is_empty(), "the unopened doc was synthesised");
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

    /// The `.ambre` container is a file format, never a wire format
    /// (`DOCFILE-011`). This is the one place a snapshot enters the wire, so it
    /// is the one place the header could be added. The payload is the engine's
    /// own export, and a container would announce itself in the first four
    /// bytes.
    #[test]
    fn a_snapshot_on_the_wire_carries_no_container_header() {
        let (mut state, _ui, sync_rx) = harness();
        let doc_id = DocumentId::new();
        state.docs.insert(
            doc_id,
            LiveDoc::new(DocState::Trame(Box::new(TrameState::new()))),
        );
        state.handle_logic_event(LogicEvent::SnapshotNeeded {
            document_id: doc_id,
        });

        let Ok(SyncCommand::SendSnapshot { payload, .. }) = sync_rx.try_recv() else {
            panic!("a snapshot is sent");
        };
        assert!(
            !payload.starts_with(&ambre_core::AMBRE_MAGIC),
            "the wire payload must be a bare snapshot, not a container"
        );
    }

    /// A Sync request for a board's snapshot answers with the whole board: a
    /// fresh engine that imports the payload holds what the board held.
    #[test]
    fn a_snapshot_request_answers_with_the_board() {
        let (mut state, _ui, sync_rx) = harness();
        let doc_id = board_with_content(&mut state);
        while sync_rx.try_recv().is_ok() {}

        state.handle_logic_event(LogicEvent::SnapshotNeeded {
            document_id: doc_id,
        });

        let Ok(SyncCommand::SendSnapshot {
            document_id,
            payload,
            ..
        }) = sync_rx.try_recv()
        else {
            panic!("a snapshot is sent");
        };
        assert_eq!(document_id, doc_id);
        let mut reopened = ToileState::new();
        let _ = reopened.import_snapshot(&payload);
        assert_eq!(reopened.element_count(), 1);
    }

    // -- Service-plane routing: interop media uploads (`toile-media`) ------
    //
    // A Toile media import issues `UploadMedia` requests, which the dispatcher
    // lowers to `RestCommand`s. The answering `RestEvent`s are fed back into the
    // owning engine instead of forwarded to the UI. These pin that round trip.

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
        let uploaded = sync_rx.try_iter().find_map(|cmd| match cmd {
            SyncCommand::Rest {
                id: got,
                cmd: RestCommand::UploadMedia { document_id, .. },
            } => Some((got, document_id)),
            _ => None,
        });
        let (got_id, got_doc) = uploaded.expect("an UploadMedia was forwarded to Sync");
        assert_eq!(got_id, id);
        assert_eq!(got_doc, doc_id);
    }

    #[test]
    fn routed_media_uploaded_lands_in_engine_not_the_ui() {
        let (mut state, ui_rx, _sync) = harness();
        let doc_id = live_toile(&mut state, Role::Owner);
        let id = import_one_image(&mut state, doc_id);
        ui_rx.try_iter().for_each(drop);

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
        let events: Vec<UiEvent> = ui_rx.try_iter().collect();
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
        ui_rx.try_iter().for_each(drop);

        state.handle_logic_event(LogicEvent::ApiError {
            id: Some(id),
            request: "UploadMedia".into(),
            code: 500,
            message: "boom".into(),
        });

        // The route is dropped and the failure surfaces as an import failure on
        // the board, not as a stray `ApiError` to the UI.
        assert!(state.rest_routes.is_empty());
        let events: Vec<UiEvent> = ui_rx.try_iter().collect();
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
        ui_rx.try_iter().for_each(drop);

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
            !ui_rx
                .try_iter()
                .any(|e| matches!(e, UiEvent::Viewport { .. })),
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

    /// Every service request but the two that remove a document, each naming
    /// `doc` where it names one. The `match` has no wildcard, so a new variant
    /// fails the build here until it is listed.
    fn relayed_requests(doc: DocumentId) -> Vec<RestCommand> {
        use ambre_bus::rest::{SecretBytes, SecretString};
        use ambre_bus::{MeetingId, Role, UserId};
        let secret = || SecretBytes::new(b"pass".to_vec());
        let (user, meeting) = (UserId::new(), MeetingId::new());
        let all = vec![
            RestCommand::Unlock {
                passphrase: secret(),
            },
            RestCommand::SetupAccount {
                passphrase: secret(),
                email: "a@b.c".to_owned(),
                display_name: "A".to_owned(),
            },
            RestCommand::DevLogin {
                username: "a".to_owned(),
            },
            RestCommand::ChangePassphrase {
                current_passphrase: secret(),
                new_passphrase: secret(),
            },
            RestCommand::RecoverAccount {
                recovery_words: SecretString::new("words".to_owned()),
                new_passphrase: secret(),
            },
            RestCommand::ListDocuments,
            RestCommand::CreateDocument {
                doc_type: DocType::Trame,
                name: "n".to_owned(),
            },
            RestCommand::RenameDocument {
                document_id: doc,
                name: "n".to_owned(),
            },
            RestCommand::FetchInviteeKey {
                document_id: doc,
                invitee_email: "a@b.c".to_owned(),
            },
            RestCommand::InviteToDocument {
                document_id: doc,
                invitee_email: "a@b.c".to_owned(),
                role: Role::Editor,
                expect_exchange_pub_key: [0; 32],
            },
            RestCommand::RevokeFromDocument {
                document_id: doc,
                user_id: user,
            },
            RestCommand::LeaveDocument { document_id: doc },
            RestCommand::ListDocumentMembers { document_id: doc },
            RestCommand::UpdateDocumentMemberRole {
                document_id: doc,
                user_id: user,
                role: Role::Viewer,
            },
            RestCommand::SearchUsers {
                query: "a".to_owned(),
            },
            RestCommand::UploadMedia {
                document_id: doc,
                data: vec![1],
            },
            RestCommand::DownloadMedia {
                document_id: doc,
                content_hash: "h".to_owned(),
            },
            RestCommand::SealDocFile {
                plaintext: secret(),
                passphrase: secret(),
            },
            RestCommand::OpenDocFile {
                sealed: vec![1],
                passphrase: secret(),
            },
            RestCommand::CreateMeeting {
                name: "m".to_owned(),
            },
            RestCommand::JoinMeeting {
                meeting_id: meeting,
            },
            RestCommand::LeaveMeeting {
                meeting_id: meeting,
            },
            RestCommand::EndMeeting {
                meeting_id: meeting,
            },
            RestCommand::ProviderSignIn {
                provider: "p".to_owned(),
            },
            RestCommand::ProviderReturn {
                provider: "p".to_owned(),
                code: "c".to_owned(),
                state: "s".to_owned(),
            },
        ];
        for cmd in &all {
            let removes = match cmd {
                RestCommand::Logout | RestCommand::DeleteDocument { .. } => true,
                RestCommand::Unlock { .. }
                | RestCommand::SetupAccount { .. }
                | RestCommand::DevLogin { .. }
                | RestCommand::ChangePassphrase { .. }
                | RestCommand::RecoverAccount { .. }
                | RestCommand::ListDocuments
                | RestCommand::CreateDocument { .. }
                | RestCommand::RenameDocument { .. }
                | RestCommand::FetchInviteeKey { .. }
                | RestCommand::InviteToDocument { .. }
                | RestCommand::RevokeFromDocument { .. }
                | RestCommand::LeaveDocument { .. }
                | RestCommand::ListDocumentMembers { .. }
                | RestCommand::UpdateDocumentMemberRole { .. }
                | RestCommand::SearchUsers { .. }
                | RestCommand::UploadMedia { .. }
                | RestCommand::DownloadMedia { .. }
                | RestCommand::SealDocFile { .. }
                | RestCommand::OpenDocFile { .. }
                | RestCommand::CreateMeeting { .. }
                | RestCommand::JoinMeeting { .. }
                | RestCommand::LeaveMeeting { .. }
                | RestCommand::EndMeeting { .. }
                | RestCommand::ProviderSignIn { .. }
                | RestCommand::ProviderReturn { .. } => false,
            };
            assert!(
                !removes,
                "the two removing commands are pinned on their own"
            );
        }
        all
    }

    /// `BUS-019`: relaying a service request leaves the registry alone. Each
    /// request reaches Sync, and the open document stays open, even when the
    /// request names it.
    #[test]
    fn a_relayed_request_leaves_the_registry_alone() {
        let (mut state, _ui, sync_rx) = harness();
        let doc = DocumentId::new();
        state.open_with_identity(doc, DocType::Trame);
        let requests = relayed_requests(doc);
        let count = requests.len();
        for cmd in requests {
            let name = format!("{cmd:?}");
            state.handle_logic_command(rest(cmd));
            assert!(state.docs.contains_key(&doc), "{name} closed the document");
            assert_eq!(state.docs.len(), 1, "{name} touched the registry");
        }
        let relayed = sync_rx
            .try_iter()
            .filter(|c| matches!(c, SyncCommand::Rest { .. }))
            .count();
        assert_eq!(relayed, count, "every request reaches Sync");
    }

    /// `LoadFonts` seeds the export `FontContext` once and is local-only. It
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
        assert!(ui_rx.try_iter().next().is_none(), "no UI event");
        assert!(sync_rx.try_recv().is_err(), "nothing forwarded to Sync");

        // A second delivery extends the same context rather than replacing it:
        // a script face fetched after boot must export too (`CANVAS-073`).
        state.handle_logic_command(LogicCommand::LoadFonts { faces: Vec::new() });
        assert!(state.export_cx.is_some());
    }

    /// The faces the workspace ships, read off the tree. A running peer gets
    /// the same bytes over the bus; here the test plays the Main thread.
    fn shipped_faces() -> Vec<Vec<u8>> {
        const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
        ["NotoSans-Regular.ttf", "DejaVuSerif.ttf"]
            .iter()
            .filter_map(|file| std::fs::read(format!("{DIR}{file}")).ok())
            .collect()
    }

    /// The export context holds what was delivered and nothing of the machine
    /// (`PDF-007`).
    ///
    /// The families below ship with common desktop distributions and none is
    /// one of Ambre's own faces. A context that discovered the device would
    /// find them, and shape a missing codepoint on one of them.
    #[test]
    fn the_export_context_holds_only_the_faces_delivered_to_it() {
        let (mut state, _ui, _sync) = harness();
        let faces = shipped_faces();
        assert!(!faces.is_empty(), "the workspace ships its faces");
        state.handle_logic_command(LogicCommand::LoadFonts { faces });
        let Some(export_cx) = state.export_cx.as_mut() else {
            panic!("a delivery seeds the export context");
        };
        assert!(
            export_cx.collection.family_id("Noto Sans").is_some(),
            "a delivered face is there"
        );
        for family in ["DejaVu Sans", "Liberation Sans", "FreeSans", "Arial"] {
            assert!(
                export_cx.collection.family_id(family).is_none(),
                "the export context found `{family}` on the machine"
            );
        }
    }

    /// A delivery reaches the documents already open, and a document opened
    /// afterwards takes the same set (`CANVAS-067`, `PROSE-115`).
    ///
    /// A Prose measuring context is sealed, so a document that misses the
    /// delivery shapes every run into tofu and paginates on nothing.
    #[test]
    fn every_prose_document_takes_the_faces_whenever_it_was_opened() {
        let (mut state, _ui, _sync) = harness();
        let faces = shipped_faces();
        assert!(!faces.is_empty(), "the workspace ships its faces");

        let before = DocumentId::new();
        state.open_with_identity(before, DocType::Prose);
        assert_eq!(
            state.docs[&before].state.face_count(),
            0,
            "nothing has been delivered yet"
        );

        state.handle_logic_command(LogicCommand::LoadFonts {
            faces: faces.clone(),
        });
        assert_eq!(
            state.docs[&before].state.face_count(),
            faces.len(),
            "a document open at delivery takes the faces"
        );

        let after = DocumentId::new();
        state.open_with_identity(after, DocType::Prose);
        assert_eq!(
            state.docs[&after].state.face_count(),
            faces.len(),
            "a document opened afterwards takes the same set"
        );

        // A later delivery (a script face) reaches both (`CANVAS-073`).
        state.handle_logic_command(LogicCommand::LoadFonts {
            faces: vec![faces[0].clone()],
        });
        for id in [before, after] {
            assert_eq!(
                state.docs[&id].state.face_count(),
                faces.len() + 1,
                "a face fetched later reaches an open document too"
            );
        }
    }

    // -- The deck's PDF export (`toile-pdf-export`, `TOILE-IO-029`) ------------
    //
    // This is the one app command the Logic thread serves itself rather than
    // handing to the document, because the base font faces live here. These
    // tests hold that routing: the command reaches the handler, and the handler
    // answers -- with the file or with the reason there is none.

    /// Ask a live board for its deck, and report the Toile event that came back.
    fn export_pdf_answer(
        state: &mut LogicState,
        ui_rx: &Receiver<UiEvent>,
        doc_id: DocumentId,
    ) -> ambre_bus::app::toile::ToileEvent {
        // Whatever seeded the board has already spoken; the export's own answer
        // is the only event this reads.
        ui_rx.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::ExportPdf),
        });
        ui_rx
            .try_iter()
            .find_map(|e| match e {
                UiEvent::Viewport {
                    event: AppEvent::Toile(te),
                    ..
                } => Some(te),
                _ => None,
            })
            .expect("the export answers")
    }

    /// A board with a rect on it, so the deck has a page to draw.
    fn board_with_content(state: &mut LogicState) -> DocumentId {
        let doc_id = live_toile(state, Role::Owner);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::CreateElement {
                kind: ambre_toile::logic::model::ElementKind::Shape(
                    ambre_toile::logic::model::ShapeKind::Rect,
                ),
                at: kurbo::Point::new(0.0, 0.0),
                size: kurbo::Size::new(100.0, 60.0),
                content: ambre_bus::app::toile::InitContent::None,
                fill: None,
                correlate: 0,
            }),
        });
        doc_id
    }

    /// Nothing is ever silent. A build without the export compiled in
    /// answers by name, as does a session whose fonts have not landed.
    /// Neither leaves the user waiting on a file.
    #[test]
    fn a_deck_export_without_fonts_answers_with_a_reason() {
        let (mut state, ui_rx, _sync) = harness();
        let doc_id = board_with_content(&mut state);
        let answer = export_pdf_answer(&mut state, &ui_rx, doc_id);
        assert!(
            matches!(answer, ambre_bus::app::toile::ToileEvent::ExportFailed(_)),
            "an export with no font context must say why"
        );
    }

    /// The whole seam, once: the command routes past the document, the render
    /// walk runs against the seeded faces, and the bytes come back named.
    #[cfg(feature = "toile-pdf")]
    #[test]
    fn a_deck_export_answers_with_the_file() {
        let (mut state, ui_rx, _sync) = harness();
        state.handle_logic_command(LogicCommand::LoadFonts { faces: Vec::new() });
        let doc_id = board_with_content(&mut state);
        match export_pdf_answer(&mut state, &ui_rx, doc_id) {
            ambre_bus::app::toile::ToileEvent::ExportComplete { filename, data, .. } => {
                assert_eq!(filename, "deck.pdf");
                assert!(data.starts_with(b"%PDF-"), "a PDF header");
            }
            other => panic!("expected the file, got {other:?}"),
        }
    }

    /// A deck export sends nothing to Sync: no delta leaves for a peer
    /// (`PDF-016`).
    #[cfg(feature = "toile-pdf")]
    #[test]
    fn a_deck_export_sends_no_delta() {
        let (mut state, ui_rx, sync_rx) = harness();
        state.handle_logic_command(LogicCommand::LoadFonts { faces: Vec::new() });
        let doc_id = board_with_content(&mut state);
        sync_rx.try_iter().for_each(drop);
        let answer = export_pdf_answer(&mut state, &ui_rx, doc_id);
        assert!(matches!(
            answer,
            ambre_bus::app::toile::ToileEvent::ExportComplete { .. }
        ));
        assert!(sync_rx.try_recv().is_err(), "the export sent a delta");
    }

    /// An export reads the board and writes nothing, so a read-only viewer keeps
    /// the right to take a copy out (`ARCH-ROLE-GATE`).
    #[test]
    fn a_viewer_may_export_the_deck() {
        let (mut state, ui_rx, _sync) = harness();
        let doc_id = live_toile(&mut state, Role::Viewer);
        // The gate drops a refused command before the handler, so an answer at
        // all is the property under test.
        let _ = export_pdf_answer(&mut state, &ui_rx, doc_id);
    }

    /// A setup still owed reaches the UI as a setup to complete, and as
    /// nothing else (`SYNC-134`).
    #[test]
    fn a_setup_owed_reaches_the_ui_as_a_setup() {
        let (mut state, ui, _sync) = harness();
        state.handle_logic_event(LogicEvent::SetupRequired);
        let told: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        assert!(matches!(told.as_slice(), [UiEvent::SetupRequired]));
    }

    /// A relay on a newer wire version reaches the UI as an update to
    /// install, and as nothing else (`SYNC-137`).
    #[test]
    fn a_newer_relay_reaches_the_ui_as_an_update() {
        let (mut state, ui, _sync) = harness();
        state.handle_logic_event(LogicEvent::UpdateRequired);
        let told: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        assert!(matches!(told.as_slice(), [UiEvent::UpdateRequired]));
    }

    /// A plain document that could not open reaches the UI as a failed open
    /// naming it, and as no authentication error (`BUS-030`).
    #[test]
    fn a_failed_open_reaches_the_ui_naming_its_document() {
        let (mut state, ui, _sync) = harness();
        let failed = DocumentId::new();
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: failed,
            message: "replay stopped".to_owned(),
        });
        let told: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        assert!(
            matches!(
                told.as_slice(),
                [UiEvent::DocumentOpenFailed { document_id, .. }] if *document_id == failed
            ),
            "the failed open was told as {told:?}"
        );
    }
}
