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
use ambre_murmure::logic::runtime::{Effect, Runtime, StoreWrite};

/// Murmure's runtime, as the Logic runtime holds it. A channel spans several
/// documents, so Murmure keeps its own state in its crate, and this dispatch
/// alone routes to it (`ARCH-121`).
#[derive(Default)]
pub(crate) struct Murmure {
    runtime: Runtime<AppEvent>,
}

impl LogicState {
    /// Run one handler of the Murmure runtime, then fan out what it owes the
    /// other threads, in the order it owes it (`BUS-010`).
    pub(crate) fn route<T>(&mut self, handler: impl FnOnce(&mut Runtime<AppEvent>) -> T) -> T {
        let answer = handler(&mut self.murmure.runtime);
        self.fan_out();
        answer
    }

    /// Hand each effect the Murmure runtime owes to its thread. Kept out of
    /// line: every call site of [`Self::route`] shares one copy, so the
    /// routing adds no code per handler to the Main instance.
    #[inline(never)]
    fn fan_out(&mut self) {
        for effect in self.murmure.runtime.take_outcome() {
            match effect {
                Effect::Ui(event) => self.send_to_ui(event),
                Effect::Sync(command) => self.send_to_sync(command),
                Effect::Store(document_id, write) => {
                    let write = match write {
                        StoreWrite::Snapshot(copy) => PersistWrite::Snapshot(copy),
                        StoreWrite::Hidden => PersistWrite::Hidden,
                        StoreWrite::ReadBack => PersistWrite::ReadBack {
                            doc_type: DocType::Murmure,
                        },
                        StoreWrite::Erase => PersistWrite::Erase,
                    };
                    self.send_to_persist(document_id, write);
                }
                Effect::Preview {
                    channel_id,
                    document_id,
                    media_ref,
                } => self.show_picture(channel_id, document_id, media_ref),
            }
        }
    }

    /// The Murmure runtime, for a test that reads its state.
    #[cfg(test)]
    pub(crate) fn rt(&self) -> &Runtime<AppEvent> {
        &self.murmure.runtime
    }

    /// The Murmure runtime, for a test that plants a state.
    #[cfg(test)]
    pub(crate) fn rt_mut(&mut self) -> &mut Runtime<AppEvent> {
        &mut self.murmure.runtime
    }

    /// Show the view of `channel_id` the preview of `media_ref`, or none.
    pub(crate) fn send_picture(
        &self,
        channel_id: Id<Channel>,
        media_ref: ambre_core::Validated<ambre_bus::ContentHash>,
        preview: Option<pictures::Preview>,
    ) {
        let picture = preview.map(|preview| ambre_murmure::message::Picture {
            size: preview.size,
            frames: preview
                .frames
                .into_iter()
                .map(|frame| ambre_murmure::message::PictureFrame {
                    pixels: frame.pixels,
                    delay: frame.delay,
                })
                .collect(),
        });
        self.send_to_ui(UiEvent::Channel {
            channel_id,
            event: ambre_murmure::message::MurmureEvent::Picture { media_ref, picture },
        });
    }

    /// Note the session identity Sync issued for a document no entry of
    /// `docs` holds: a channel or a space document (`AUTHOR-002`).
    pub(crate) fn murmure_session(
        &mut self,
        document_id: Id<Document>,
        peer: ambre_core::PeerId,
        entry: ambre_core::PeerEntry,
    ) {
        self.route(|m| m.adopt_murmure_session(document_id, peer, entry));
    }

    /// Whether no other session of the space that holds `document_id` holds
    /// `peer` (`AUTHOR-033`). A document no open space holds reads as
    /// unclaimed: no registry of this thread names its PeerIDs.
    pub(crate) fn murmure_peer_unclaimed(
        &mut self,
        document_id: Id<Document>,
        peer: ambre_core::PeerId,
        nonce: &ambre_core::FixedBytes<ambre_core::SessionNonce>,
    ) -> bool {
        self.route(|m| m.space_peer_unclaimed(document_id, peer, nonce))
    }

    /// Apply signed deltas to the channel that holds `document_id`. Returns
    /// `false` when no open or opening channel holds it.
    pub(crate) fn murmure_signed(
        &mut self,
        document_id: Id<Document>,
        deltas: &[SignedDelta],
    ) -> bool {
        self.route(|m| m.apply_channel_signed(document_id, deltas))
    }

    /// Keep signed deltas, with their signers, for the space still opening
    /// that holds `document_id` (`AUTHOR-037`). Returns `false` when no
    /// opening space holds it.
    pub(crate) fn murmure_space_signed(
        &mut self,
        document_id: Id<Document>,
        deltas: &[SignedDelta],
    ) -> bool {
        self.route(|m| m.keep_space_signed(document_id, deltas))
    }

    /// Apply a trusted payload the relay placed at `seq` to the channel or
    /// the space that holds `document_id`. Returns `false` when neither does.
    pub(crate) fn murmure_remote(
        &mut self,
        document_id: Id<Document>,
        seq: Seq<Relay>,
        payload: ambre_core::LoroPayloadRef<'_>,
    ) -> bool {
        let order = match payload {
            ambre_core::LoroPayloadRef::Delta(_) => ambre_murmure::Order::Update(seq),
            ambre_core::LoroPayloadRef::Snapshot(_) => ambre_murmure::Order::Snapshot(seq),
        };
        self.route(|m| {
            m.apply_channel_remote(document_id, &[(Some(order), payload)])
                || m.apply_space_remote(document_id, &[payload])
        })
    }

    /// Apply a replay of deltas to the space that holds `document_id`.
    /// Returns `false` when no open space holds it.
    pub(crate) fn murmure_replay(
        &mut self,
        document_id: Id<Document>,
        payloads: &[ambre_core::LoroPayloadRef<'_>],
    ) -> bool {
        self.route(|m| m.apply_space_remote(document_id, payloads))
    }

    /// Check a snapshot of a channel document, and answer the verdict to
    /// Sync. A document no open channel holds is not answered.
    pub(crate) fn murmure_check_snapshot(
        &mut self,
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        covered: Seq<Relay>,
        payload: &ambre_core::Payload<ambre_core::SnapshotBytes>,
        purpose: SnapshotPurpose,
    ) {
        self.route(|m| {
            m.check_channel_snapshot(document_id, snapshot_id, covered, payload, purpose)
        });
    }

    /// Note a snapshot the relay served for a channel or a space document.
    pub(crate) fn murmure_snapshot_served(
        &mut self,
        document_id: Id<Document>,
        covered: Seq<Relay>,
    ) {
        self.route(|m| m.note_murmure_snapshot(document_id, covered));
    }

    /// Note the relay's number of a delta for a channel or a space document.
    pub(crate) fn murmure_seq(
        &mut self,
        document_id: Id<Document>,
        seq: Seq<Relay>,
        payload: Option<&ambre_core::Payload<ambre_core::DeltaBytes>>,
    ) {
        self.route(|m| m.record_channel_seq(document_id, seq, payload));
    }

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
            LogicCommand::LocalStoreFull { document_id, room } => {
                self.store_full(document_id, room);
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
                if doc_type == DocType::Murmure && self.route(|m| m.reads_channel_copy(document_id))
                {
                    self.route(|m| m.channel_copy_landed(document_id, snapshot, &deltas));
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
                        // The deltas a window holds leave before the
                        // session ends (`SYNC-195`).
                        self.flush_windows();
                        self.docs.clear();
                        self.evicted.clear();
                        self.store_full.clear();
                        self.route(|m| m.logout());
                        self.locale_seeds.clear();
                        // A preview fetch the relay has not answered belongs
                        // to this user (the Murmure engine spec).
                        self.picturing.clear();
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
                    RestCommand::LeaveDocument { document_id }
                        if self.route(|m| m.hand_off_before_leave(id, *document_id)) =>
                    {
                        return;
                    }
                    RestCommand::InviteToDocument { .. }
                        if self.route(|m| m.announce_before_invite(id, &cmd)) =>
                    {
                        return;
                    }
                    _ => {}
                }
                self.send_to_sync(SyncCommand::Rest { id, cmd });
            }

            // -- P3 -- Control plane: explicit (no correlatable response) -------
            // The pin is the Sync worker's to check (`SYNC-205`).
            LogicCommand::PinInviter {
                document_id,
                inviter,
            } => self.send_to_sync(SyncCommand::PinInviter {
                document_id,
                inviter,
            }),
            LogicCommand::PinMurmureInviter { inviter } => {
                self.send_to_sync(SyncCommand::PinMurmureInviter { inviter })
            }
            // The kept log is the Sync worker's to hold (`SYNC-290`).
            LogicCommand::KeepLog { document_id, kept } => {
                self.send_to_sync(SyncCommand::KeepLog { document_id, kept });
            }
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
                // A document this reader just created takes their locale, once
                // (`LOCALE-019`). It is an edit, held until the identity.
                self.seed_locale(document_id);
                self.send_to_sync(SyncCommand::OpenDocument {
                    document_id,
                    doc_type,
                    meta: None,
                });
                // An empty Prose document is one blank page, and the view
                // shows it before anything arrives (the Prose pagination spec).
                self.owe_layout(document_id);
            }
            LogicCommand::CloseDocument { document_id } => {
                // The last edits of the pass are saved before the state goes,
                // so closing a document never loses them (`ARCH-017`). It then
                // leaves memory, and Sync keeps it light (`RESID-001`).
                self.evict(document_id);
            }
            // A view that lost an event reads again (`BUS-041`, `BUS-042`).
            // Its document is resent from what this thread holds, so nothing
            // reaches the relay or the store.
            LogicCommand::ResendState { document_id } => {
                let events = self
                    .docs
                    .get(&document_id)
                    .map(|doc| doc.state.whole_state())
                    .unwrap_or_default();
                for event in events {
                    self.send_to_ui(UiEvent::Viewport { document_id, event });
                }
            }
            LogicCommand::OpenChannel { channel_id } => self.route(|m| m.open_channel(channel_id)),
            LogicCommand::CloseChannel { channel_id } => {
                self.route(|m| m.close_channel(channel_id))
            }
            LogicCommand::OpenDirectMessage { draft, now } => {
                self.route(|m| m.open_direct_message(draft, now))
            }
            LogicCommand::InviteToChannel {
                id,
                channel_id,
                invitee,
                now,
            } => self.route(|m| m.invite_to_channel(id, channel_id, invitee, now)),
            LogicCommand::FetchActivity => self.send_to_sync(SyncCommand::FetchActivity),
            LogicCommand::ListMemberChannels => {
                self.send_to_sync(SyncCommand::ListMemberChannels);
            }
            LogicCommand::MarkAllRead => self.send_to_sync(SyncCommand::MarkAllRead),
            LogicCommand::SearchMessages { query } => self.route(|m| m.search_messages(query)),
            LogicCommand::CreateSpace {
                space_id,
                name,
                now,
            } => self.route(|m| m.create_space(space_id, &name, now)),
            LogicCommand::OpenSpace { space_id } => self.route(|m| m.open_space(space_id)),
            LogicCommand::CloseSpace { space_id } => self.route(|m| m.close_space(space_id)),
            LogicCommand::Space { space_id, command } => {
                self.route(|m| m.handle_space_command(space_id, command));
            }
            // -- Content plane: a Murmure command, addressed to its channel ---
            LogicCommand::Channel {
                channel_id,
                command,
            } => self.route(|m| m.handle_channel_command(channel_id, command)),
            LogicCommand::SetReaderLocale { reader } => self.set_reader_locale(reader),
            LogicCommand::LoadFonts { faces } => {
                // The Logic thread cannot fetch, so this is the only way a face
                // reaches it (`CANVAS-067`). Local-only: nothing is sent to
                // Sync. The registry keeps the one copy of each face, and every
                // consumer below takes a handle on it (`CANVAS-097`). A face
                // already held is dropped here, so no consumer takes it twice.
                tracing::info!("Logic: {} faces delivered", faces.len());
                let faces: Vec<ambre_draw::FontFace> = faces
                    .into_iter()
                    .filter_map(|face| self.faces.admit(face))
                    .collect();
                // Every open document that measures text. A Prose paginator
                // shapes on a sealed context and holds nothing until now, as
                // the Prose pagination spec states.
                for doc in self.docs.values_mut() {
                    doc.state.load_faces(&faces);
                }
                // A new face re-shapes the runs it covers (the Prose pagination
                // spec).
                let open: Vec<Id<Document>> = self.docs.keys().copied().collect();
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
                for face in &faces {
                    ambre_draw::register_face(export_cx, face);
                }
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
            LogicEvent::Rest { id, mut evt } => {
                // An answer no request asked for awaits nothing of this
                // thread: only its session hook below applies.
                if let Some(id) = id {
                    // The answer to a sealed `.ambre` export this thread asked for
                    // (`DOCFILE-014`). It becomes a display event of the owning
                    // application here, and never reaches the engine, which knows
                    // nothing of the seal.
                    if self.resolve_pending_seal(id, &evt) {
                        return;
                    }
                    // The owner role passed on: the leave it held goes now.
                    if self.route(|m| m.handed_off(id, &evt)) {
                        return;
                    }
                    // The erase of a deleted message's file, which no view awaits.
                    if self.route(|m| m.erased(id, &evt)) {
                        return;
                    }
                    // The image of a preview, which the view awaits decoded.
                    // The event moves in, so its bytes reach the job uncopied.
                    evt = match self.pictured(id, evt) {
                        Some(evt) => evt,
                        None => return,
                    };
                    // A response to a request this Logic thread issued on behalf of a
                    // document (`toile-media`) is fed back into the owning engine. It
                    // is not forwarded to the UI. UI-issued requests are absent from
                    // `rest_routes` and fall through to the verbatim forward.
                    if let Some(document_id) = self.rest_routes.remove(&id) {
                        self.route_service_event(document_id, id, evt);
                        return;
                    }
                }
                // A document this reader created waits for its locale seed.
                if let RestEvent::DocumentCreated { document_id, .. } = &evt
                    && !self.reader_locale.own.is_none()
                {
                    self.locale_seeds
                        .insert(*document_id, self.reader_locale.own.clone());
                }
                // `SessionReady` also seeds the canonical `Id<User>`. Capture it
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
                    self.route(|m| m.session_ready(*user_id));
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

            // A report dropped a snapshot (the Murmure segments spec).
            LogicEvent::SnapshotDropped {
                document_id,
                author,
                reporter,
                after,
            } => {
                self.route(|m| m.snapshot_dropped(document_id, author, reporter, after));
            }
            LogicEvent::ThreadActivity { document_id } => {
                self.route(|m| m.thread_activity(document_id))
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
                if !self.route(|m| m.space_presence_heard(document_id, from_user_id, &payload))
                    && !self.route(|m| m.channel_sample_heard(document_id, from_user_id, &payload))
                {
                    self.apply_ephemeral(document_id, from_user_id, &payload);
                }
            }

            LogicEvent::PeerLeft {
                document_id,
                user_id,
            } => {
                if !self.route(|m| m.space_peer_left(document_id, user_id))
                    && !self.route(|m| m.channel_peer_left(document_id, user_id))
                {
                    self.peer_left(document_id, user_id);
                }
            }

            LogicEvent::MembersChanged {
                document_id,
                members,
            } => {
                // A channel and a space show their members live; any other
                // document reads them when its view asks.
                if let Some(members) =
                    self.route(|m| m.channel_members_changed(document_id, members))
                {
                    self.route(|m| m.space_members_changed(document_id, members));
                }
            }

            LogicEvent::SnapshotNeeded { document_id } => {
                let covered = self
                    .export_covered_snapshot(document_id)
                    .or_else(|| self.route(|m| m.channel_snapshot(document_id)))
                    .or_else(|| self.route(|m| m.space_snapshot(document_id)));
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
                if connected.is_connected() {
                    self.route(|m| m.retry_erases());
                }
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
                if !self.route(|m| m.count_thread(document_id, counters))
                    && !self.route(|m| m.count_closed(document_id, counters))
                {
                    self.send_to_ui(UiEvent::Activity {
                        document_id,
                        counters,
                    });
                }
                self.route(|m| m.closed_meta_moved(document_id));
            }
            // The cue of a call in a document this thread does not hold (the Murmure calls
            // spec, the ring). A document of no closed channel cues nothing.
            LogicEvent::CallCue { document_id, cue } => {
                self.route(|m| m.cue_closed(document_id, cue));
            }
            LogicEvent::Released { document_id } => {
                self.store_sent(document_id);
            }

            // The local store keeps the pin the open proved (`SYNC-238`).
            LogicEvent::InviterProven {
                document_id,
                inviter,
            } => self.send_to_persist(document_id, PersistWrite::KeepInviter(inviter)),

            // The local store keeps where the log stands (`SYNC-290`).
            LogicEvent::LogKept { document_id, kept } => {
                self.send_to_persist(document_id, PersistWrite::KeepLog(kept));
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
                    self.route(|m| m.flush_unsent(document_id));
                    if self.route(|m| m.channel_document_ready(document_id))
                        || self.route(|m| m.space_document_ready(document_id, role))
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

            LogicEvent::SyncWarning { document_id, code } => {
                self.send_to_ui(UiEvent::SyncWarning { document_id, code });
            }

            // -- Sharing (server push) -----------------------------------------
            LogicEvent::MemberRevoked {
                document_id,
                revoked_user_id,
            } => {
                if Some(revoked_user_id) == self.my_user_id {
                    self.route(|m| m.channel_revoked(document_id));
                    self.route(|m| m.space_revoked(document_id));
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
            } => self.route(|m| m.channel_listed(channel_id, &documents)),
            LogicEvent::ChannelOpenFailed {
                channel_id,
                failure,
            } => {
                self.route(|m| m.opening_failed(channel_id, &failure));
            }
            LogicEvent::ChannelRefused { channel_id } => {
                self.route(|m| m.channel_refused(channel_id))
            }
            LogicEvent::ChannelActivity { documents } => {
                self.route(|m| m.channel_activity(&documents))
            }
            LogicEvent::ChannelCreated { channel_id, key } => {
                self.route(|m| m.channel_created(channel_id, key));
            }
            LogicEvent::ChannelKeyResealed {
                space_id,
                channel_id,
                key,
            } => {
                self.route(|m| m.key_resealed(space_id, channel_id, &key));
            }
            LogicEvent::SegmentClosed {
                channel_id,
                closing,
                active,
                successor,
            } => self.route(|m| m.segment_closed(channel_id, closing, active, successor)),
            LogicEvent::SegmentCloseFailed {
                channel_id,
                closing,
                failure,
            } => self.route(|m| m.segment_close_failed(channel_id, closing, failure)),
            LogicEvent::AmendmentsLinked {
                channel_id,
                segment,
                amendments,
            } => self.route(|m| m.amendments_linked(channel_id, segment, amendments)),
            LogicEvent::AmendmentsLinkFailed {
                channel_id,
                segment,
                amendments,
                failure,
            } => self.route(|m| m.amendments_link_failed(channel_id, segment, amendments, failure)),
            LogicEvent::ThreadLinked {
                channel_id,
                segment,
                root,
                thread,
            } => self.route(|m| m.thread_linked(channel_id, segment, root, thread)),
            LogicEvent::ThreadLinkFailed {
                channel_id,
                segment: _,
                root,
                thread,
                failure,
            } => self.route(|m| m.thread_link_failed(channel_id, root, thread, failure)),
            LogicEvent::FinalizeLeaseAnswered {
                channel_id,
                segment,
                answer,
            } => self.route(|m| m.finalize_lease_answered(channel_id, segment, answer)),
            LogicEvent::SegmentFinalized {
                channel_id,
                segment,
            } => self.route(|m| m.segment_finalized(channel_id, segment)),
            LogicEvent::SegmentFinalizeFailed {
                channel_id,
                segment,
                end,
                failure,
            } => self.route(|m| m.segment_finalize_failed(channel_id, segment, end, failure)),
            LogicEvent::FoldLeaseAnswered {
                channel_id,
                segment,
                answer,
            } => self.route(|m| m.fold_lease_answered(channel_id, segment, answer)),
            LogicEvent::SegmentFolded {
                channel_id,
                segment,
            } => self.route(|m| m.segment_folded(channel_id, segment)),
            LogicEvent::SegmentFoldFailed {
                channel_id,
                segment,
                failure,
            } => self.route(|m| m.segment_fold_failed(channel_id, segment, failure)),
            LogicEvent::AmendmentsFolded {
                document_id,
                amendments,
            } => self.route(|m| m.amendments_folded(document_id, amendments)),
            LogicEvent::DocumentGone { document_id } => {
                self.route(|m| m.document_gone(document_id))
            }
            LogicEvent::KeyRotated { document_id } => self.route(|m| m.key_rotated(document_id)),
            // A revoke capped a writer below a delta the document applied
            // (`SYNC-300`). A document of a channel reads its whole channel
            // again in the Murmure runtime, and any other reads alone.
            LogicEvent::DocumentReread { document_id } => {
                if !self.route(|m| m.channel_reread(document_id)) {
                    self.reread(document_id);
                }
            }
            LogicEvent::ConverterFetched { family, module } => {
                self.converter_fetched(family, module);
            }
            LogicEvent::ConverterRefused { family, cause } => {
                self.converter_refused(family, cause);
            }
            LogicEvent::DeltasLocked {
                document_id,
                deltas,
            } => self.route(|m| m.deltas_locked(document_id, &deltas)),
            LogicEvent::ChannelJoined { channel_id } => {
                self.route(|m| m.channel_joined(channel_id))
            }
            LogicEvent::ChannelDeleted { channel_id } => {
                self.route(|m| m.channel_deleted(channel_id))
            }
            LogicEvent::ChannelLeft { channel_id } => self.route(|m| m.channel_left(channel_id)),
            LogicEvent::DirectMessageOpened {
                requested,
                channel_id,
            } => self.route(|m| m.direct_message_opened(requested, channel_id)),
            LogicEvent::ChannelRequestFailed {
                channel_id,
                failure,
            } => self.route(|m| m.channel_request_failed(channel_id, failure)),
            LogicEvent::SpaceChannels { space_id, channels } => {
                self.route(|m| m.space_channels(space_id, &channels));
            }
            // The sidebar learns of the channels, and none of them opens
            // (the Murmure channels spec). Each is watched unopened.
            LogicEvent::MemberChannels {
                channels,
                spaces,
                names,
                peers,
                metas,
            } => {
                self.send_to_ui(UiEvent::MemberChannels {
                    channels,
                    spaces,
                    names: names.clone(),
                    peers,
                });
                // The names stay hints: an open channel seals what its
                // `meta` reads when they differ.
                self.route(|m| m.channels_listed(&metas, &names));
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
            LogicEvent::DocumentOpenFailed { document_id, cause } => {
                if !self.route(|m| m.channel_document_failed(document_id, cause))
                    && !self.route(|m| m.space_document_failed(document_id, &cause))
                {
                    self.send_to_ui(UiEvent::DocumentOpenFailed { document_id, cause });
                }
            }
            LogicEvent::AuthError { cause } => {
                self.send_to_ui(UiEvent::AuthError { cause });
            }
            LogicEvent::KeyNotShared {
                document_id,
                members,
            } => {
                self.send_to_ui(UiEvent::KeyNotShared {
                    document_id,
                    members,
                });
            }
            LogicEvent::SetupRequired => self.send_to_ui(UiEvent::SetupRequired),
            LogicEvent::UpdateRequired => self.send_to_ui(UiEvent::UpdateRequired),
            // The UI thread reads a call signal on arrival, and none reaches this thread.
            LogicEvent::Call(_) => {}
            LogicEvent::CallStarted { call, channel, at } => {
                self.route(|m| m.call_started(call, channel, at))
            }
            LogicEvent::CallEnded { call, channel, at } => {
                self.route(|m| m.call_ended(call, channel, at))
            }
            LogicEvent::CallRoster {
                call,
                channel,
                members,
            } => self.route(|m| m.call_roster(call, channel, members)),
            LogicEvent::CallMetaAsked { call, channel } => {
                self.route(|m| m.call_meta_asked(call, channel))
            }
            LogicEvent::ApiError {
                id,
                request,
                failure,
            } => {
                // A failure correlated to a document's own request
                // (`toile-media`) aborts its pending import rather than
                // surfacing a raw API error to the UI. Uncorrelated errors (id
                // absent, or not routed) forward verbatim, exactly as before.
                if let Some(id) = id
                    && let Some(document_id) = self.rest_routes.remove(&id)
                {
                    self.route_service_error(document_id, id, failure);
                    return;
                }
                if id
                    .is_some_and(|id| self.route(|m| m.erase_failed(id)) || self.picture_failed(id))
                {
                    return;
                }
                // A hand-off the relay refused fails the leave it held, which
                // never left (the Murmure spaces spec).
                let id = match id.map(|id| self.route(|m| m.hand_off_refused(id))) {
                    Some(Some(leave)) => Some(leave),
                    _ => id,
                };
                self.send_to_ui(UiEvent::ApiError {
                    id,
                    request,
                    failure,
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
    use ambre_bus::{Authored, Instant};
    use ambre_bus::{Col, Row};
    use ambre_bus::{Request, RestCommand, RestEvent};
    use ambre_core::Seq;
    use ambre_core::index::Pos;
    use ambre_prose::message::DocChar;
    use ambre_toile::message::ToileCommand;
    use ambre_trame::message::TrameCommand;
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

    /// The events a resend hands the view of `document_id`, and whether it
    /// sent anything to the Sync worker.
    fn resent(doc_type: DocType) -> (Vec<AppEvent>, bool) {
        let (mut state, ui, sync) = harness();
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, doc_type);
        ui.try_iter().for_each(drop);
        sync.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::ResendState { document_id });
        let events = ui
            .try_iter()
            .map(|ev| match ev {
                UiEvent::Viewport {
                    document_id: id,
                    event,
                } if id == document_id => event,
                other => panic!("a resend sent {other:?}"),
            })
            .collect();
        (events, sync.try_recv().is_ok())
    }

    /// `BUS-042`, `BUS-045`: a resend hands a Trame view what draws it whole.
    /// That is the re-fetch of its window, its tabs, its cell toggles and its
    /// undo state. Nothing reaches the relay.
    #[test]
    fn a_resend_hands_a_trame_view_its_whole_state() {
        use ambre_trame::message::TrameEvent;
        let (events, synced) = resent(DocType::Trame);
        assert!(!synced, "a resend reached the Sync worker");
        let has = |want: fn(&TrameEvent) -> bool| {
            events
                .iter()
                .any(|ev| matches!(ev, AppEvent::Trame(e) if want(e)))
        };
        assert!(has(|e| matches!(e, TrameEvent::DataReset)));
        assert!(has(|e| matches!(e, TrameEvent::SheetList { .. })));
        assert!(has(|e| matches!(e, TrameEvent::StyleApplied { .. })));
        assert!(has(|e| matches!(e, TrameEvent::UndoState { .. })));
    }

    /// `BUS-042`, `BUS-045`: a resend hands a Toile view what draws it whole.
    /// That is its viewport, the changes its panels re-read on, and its undo
    /// state. Nothing reaches the relay.
    #[test]
    fn a_resend_hands_a_toile_view_its_whole_state() {
        use ambre_toile::message::ToileEvent;
        let (events, synced) = resent(DocType::Toile);
        assert!(!synced, "a resend reached the Sync worker");
        let has = |want: fn(&ToileEvent) -> bool| {
            events
                .iter()
                .any(|ev| matches!(ev, AppEvent::Toile(e) if want(e)))
        };
        assert!(has(|e| matches!(e, ToileEvent::ViewportData(_))));
        assert!(has(|e| matches!(e, ToileEvent::BoardChanged)));
        assert!(has(|e| matches!(e, ToileEvent::DeckChanged)));
        assert!(has(|e| matches!(e, ToileEvent::UndoState { .. })));
    }

    /// `BUS-042`, `BUS-045`: a resend of a document this thread does not hold
    /// answers nothing.
    #[test]
    fn a_resend_of_a_document_not_open_answers_nothing() {
        let (mut state, ui, sync) = harness();
        state.handle_logic_command(LogicCommand::ResendState {
            document_id: Id::<Document>::mint(),
        });
        assert!(ui.try_recv().is_err());
        assert!(sync.try_recv().is_err());
    }

    /// Drive the native export on a live document of `doc_type` and hand back
    /// the bytes the UI would download.
    ///
    /// Goes through the real dispatcher rather than calling the engine, because
    /// the rule under test is exactly what the dispatcher adds. The header's
    /// identity comes from the registry that keyed the document, and an engine
    /// has no way to supply it.
    fn exported_file(doc_type: DocType, document_id: Id<Document>) -> Vec<u8> {
        let (mut state, ui, _sync) = harness();
        state.open_with_identity(document_id, doc_type);
        let command = match doc_type {
            DocType::Trame => AppCommand::Trame(TrameCommand::export(
                ambre_trame::message::TrameExport::Ambre,
            )),
            DocType::Toile => AppCommand::Toile(ToileCommand::export(
                ambre_toile::message::ToileExport::Ambre,
            )),
            DocType::Prose => AppCommand::Prose(ambre_prose::message::ProseCommand::Interop(
                ambre_bus::interop::InteropRequest::Export(
                    ambre_prose::message::ExportFormat::Ambre,
                ),
            )),
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
                UiEvent::Viewport { event, .. } => event
                    .exported_file()
                    .map(|file| file.data.as_bytes().to_vec()),
                _ => None,
            })
            .expect("the export answered with bytes")
    }

    /// The bytes the native export would answer with, for a document already
    /// live in `state`. The command path itself is driven by
    /// [`exported_file`]; this reads the same answer without a second registry.
    fn exported_file_from(state: &LogicState, document_id: Id<Document>) -> Vec<u8> {
        let doc = state.docs.get(&document_id).expect("a live document");
        let event = doc.state.ambre_file_event(document_id);
        match event.exported_file() {
            Some(file) => file.data.as_bytes().to_vec(),
            None => panic!("the export answered with {event:?}"),
        }
    }

    /// `PROSE-IO-020`, `PROSE-IO-028`: the save command reaches the
    /// dispatcher, and the file it writes opens in a fresh Prose document.
    #[test]
    fn a_prose_document_saves_to_a_native_file_and_reopens() {
        let document_id = Id::<Document>::mint();
        let bytes = exported_file(DocType::Prose, document_id);
        let mut reopened = ambre_prose::logic::ProseState::with_peer(ambre_core::PeerId::new(9))
            .expect("a document");
        let (id, _) = reopened
            .import_ambre_plaintext(&ambre_core::Payload::<ambre_core::PlaintextContainer>::new(
                bytes,
            ))
            .expect("the file opens");
        assert_eq!(id, document_id);
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
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, DocType::Prose);
        // The opening's own sends leave with its pass.
        state.answer_pass();
        while sync.try_recv().is_ok() {}
        let block = match &state.docs.get(&document_id).expect("a live document").state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ambre_prose::message::ProseCommand::InsertText {
                block_id: block,
                pos: Pos::<DocChar>::ZERO,
                text: "typed".to_owned(),
            }),
        });
        // The delta leaves at the end of the pass (`SYNC-192`).
        state.answer_pass();

        let confirmed = std::iter::from_fn(|| ui.try_recv().ok())
            .find_map(|evt| match evt {
                UiEvent::Viewport {
                    event:
                        AppEvent::Prose(ambre_prose::message::ProseEvent::DocUpdated { payload }),
                    document_id: id,
                } if id == document_id => Some(payload),
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
        assert_eq!(ambre_core::LoroPayload::Delta(broadcast), confirmed);
    }

    /// A picture of a Prose document leaves for the Sync worker with the
    /// document's id. Its answer comes back to the document, which tells the
    /// view what the address shows (`features/ambre-spec-prose-images.md
    /// section 3`).
    /// The spell checker of Prose
    /// (`4-Prose/features/ambre-spec-prose-spell-check.md`). A Prose document
    /// in French is checked on the converter thread, in the French spelling
    /// module the Sync worker fetched for it. Sync is asked for the module by
    /// its family alone, and no command it receives carries a word of the
    /// text.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_prose_spell_check_runs_on_the_converter_thread_and_sync_sees_no_word() {
        use ambre_prose::message::{DocChar, ProseCommand, ProseEvent};
        let (mut state, ui, sync) = harness();
        let mut thread = converter_thread(&mut state);
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, DocType::Prose);
        state.answer_pass();
        let block = match &state.docs.get(&document_id).expect("a live document").state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        for command in [
            ProseCommand::SetDocumentLocale {
                locale: ambre_client_core::DocumentLocale::parse("fr").expect("a tag"),
            },
            ProseCommand::InsertText {
                block_id: block,
                pos: ambre_core::index::Pos::<DocChar>::ZERO,
                text: "Une maisn bleue".to_owned(),
            },
        ] {
            state.handle_logic_command(LogicCommand::App {
                document_id,
                command: AppCommand::Prose(command),
            });
        }
        state.answer_pass();
        let to_sync: Vec<SyncCommand> = sync.try_iter().collect();
        let fetches: Vec<_> = to_sync
            .iter()
            .filter_map(|cmd| match cmd {
                SyncCommand::FetchConverter { family } => Some(*family),
                _ => None,
            })
            .collect();
        assert_eq!(fetches, vec![ambre_core::ConverterFamily::SpellFr]);
        for cmd in &to_sync {
            assert!(
                !format!("{cmd:?}").contains("maisn"),
                "no command to Sync names a word"
            );
        }
        state.handle_logic_event(LogicEvent::ConverterFetched {
            family: ambre_core::ConverterFamily::SpellFr,
            module: ambre_core::Payload::new(Vec::new()),
        });
        thread.serve();
        state.take_converted();
        state.answer_pass();
        let underlined = ui.try_iter().find_map(|event| match event {
            UiEvent::Viewport {
                event: AppEvent::Prose(ProseEvent::Misspelled { block_id, ranges }),
                ..
            } if block_id == block => Some(ranges),
            _ => None,
        });
        let ranges = underlined.expect("the view was told the misspelled word");
        assert_eq!(
            ranges
                .iter()
                .map(|r| r.to_usize_range())
                .collect::<Vec<_>>(),
            vec![4..9]
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_prose_picture_is_asked_for_with_its_document_and_answered_to_it() {
        use ambre_prose::message::{PictureShown, ProseCommand, ProseEvent};
        let (mut state, ui, sync) = harness();
        let mut thread = converter_thread(&mut state);
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, DocType::Prose);
        state.answer_pass();
        let block = match &state.docs.get(&document_id).expect("a live document").state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        let address = ambre_core::Validated::<ambre_core::ContentHash>::parse("c".repeat(64))
            .expect("a hash");
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ProseCommand::InsertImage {
                parent_id: None,
                after_id: Some(block),
                content_hash: address.clone(),
                alt: ambre_core::Validated::default(),
                size: ambre_core::Size::new(ambre_core::Px::new(4.0), ambre_core::Px::new(4.0)),
            }),
        });
        state.answer_pass();
        let asked = sync.try_iter().find_map(|cmd| match cmd {
            SyncCommand::Rest {
                id,
                cmd:
                    RestCommand::DownloadMedia {
                        document_id: on,
                        content_hash,
                    },
            } => Some((id, on, content_hash)),
            _ => None,
        });
        let (id, on, content_hash) = asked.expect("the layout asked for the picture");
        assert_eq!((on, content_hash), (document_id, address.clone()));
        assert_eq!(state.rest_routes.get(&id), Some(&document_id));
        ui.try_iter().for_each(drop);
        state.handle_logic_event(LogicEvent::Rest {
            id: Some(id),
            evt: RestEvent::MediaDownloaded {
                document_id,
                content_hash: address,
                data: ambre_core::Payload::new(solid_png(3, 2)),
            },
        });
        state.answer_pass();
        // The PNG decodes on the converter thread, never here (`ARCH-097`).
        state.handle_logic_event(LogicEvent::ConverterFetched {
            family: ambre_core::ConverterFamily::Images,
            module: ambre_core::Payload::new(Vec::new()),
        });
        thread.serve();
        state.take_converted();
        state.answer_pass();
        let ready = ui.try_iter().any(|event| {
            matches!(
                event,
                UiEvent::Viewport {
                    event: AppEvent::Prose(ProseEvent::PictureState {
                        shown: PictureShown::Ready(_),
                        ..
                    }),
                    ..
                }
            )
        });
        assert!(ready, "the view was told the picture decoded");
        assert!(state.rest_routes.is_empty(), "the route was consumed");
    }

    /// `PROSE-UI-006`: the layout reaches the view computed and shaped. The
    /// view adopts it from the event alone, and holds nothing that could lay
    /// text out. A caret move writes nothing, so it sends no layout.
    #[test]
    fn the_view_draws_a_layout_the_logic_thread_computed() {
        use ambre_prose::message::ProseEvent;
        let (mut state, ui, _sync) = harness();
        let document_id = Id::<Document>::mint();
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
        state.handle_logic_command(prose(ambre_prose::message::ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
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
            view.layout_version,
            ambre_core::Seq::nth(2),
            "the later layout is the one in force"
        );
        assert_eq!(view.shaped.layout.page_count.get(), 1);
        assert!(
            !view.shaped.blocks.is_empty(),
            "the runs arrive shaped with the layout"
        );

        state.handle_logic_command(prose(ambre_prose::message::ProseCommand::UpdateCursor {
            pos: ambre_prose::message::CursorPosition {
                block_id: block,
                offset: Pos::<DocChar>::new(1),
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
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, DocType::Prose);
        let block = match &state.docs.get(&document_id).expect("a live document").state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ambre_prose::message::ProseCommand::UpdateCursor {
                pos: ambre_prose::message::CursorPosition {
                    block_id: block,
                    offset: Pos::<DocChar>::ZERO,
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
            let document_id = Id::<Document>::mint();
            let bytes = exported_file(doc_type, document_id);
            let header = ambre_client_core::docfile::read_ambre_plaintext(&ambre_core::Payload::<
                ambre_core::PlaintextContainer,
            >::new(
                bytes.to_vec()
            ))
            .expect("the command wrote a valid container");
            assert_eq!(header.doc_type, doc_type, "the header names the wrong app");
            assert_eq!(
                header.document_id, document_id,
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
        let document_id = Id::<Document>::mint();
        let (mut state, _ui, _sync) = harness();
        state.open_with_identity(document_id, DocType::Trame);
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Trame(TrameCommand::SetCellText {
                row: Pos::<Row>::new(3),
                col: Pos::<Col>::new(1),
                text: "kept".to_owned(),
            }),
        });
        let saved = exported_file_from(&state, document_id);
        let header =
            ambre_client_core::docfile::read_ambre_plaintext(&ambre_core::Payload::<
                ambre_core::PlaintextContainer,
            >::new(saved.to_vec()))
            .expect("valid container");

        // A second session, as a page load after the open gesture. Nothing is
        // live, and the file's payload arrives the way the store hands it over.
        let (mut reopened, ui, _sync) = harness();
        reopened.handle_logic_command(LogicCommand::RestoreLocalSnapshot {
            document_id: header.document_id,
            doc_type: header.doc_type,
            snapshot: Some(header.payload),
            deltas: Vec::new(),
            sessions: Vec::new(),
        });
        reopened.grant_identity(document_id);
        reopened.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Trame(TrameCommand::ScrollViewport {
                row_start: Pos::<Row>::new(0),
                col_start: Pos::<Col>::new(0),
                row_end: Pos::<Row>::new(16),
                col_end: Pos::<Col>::new(8),
                version: ambre_core::Seq::nth(1),
            }),
        });
        let kept =
            std::iter::from_fn(|| ui.try_recv().ok()).any(|evt| match evt {
                UiEvent::Viewport {
                    event:
                        AppEvent::Trame(ambre_trame::message::TrameEvent::ViewportData {
                            cells, ..
                        }),
                    ..
                } => cells.iter().any(|c| {
                    c.row == Pos::<Row>::new(3) && c.col == Pos::<Col>::new(1) && c.text == "kept"
                }),
                _ => false,
            });
        assert!(kept, "the saved file reopened without the edit it carried");
    }

    // -- The sealed file (`DOCFILE-014`) ---------------------------------------

    /// Open a document and ask for a sealed `.ambre`, returning the state, the
    /// UI queue, and the service request the seal produced.
    fn sealing(doc_type: DocType) -> (LogicState, Receiver<UiEvent>, Seq<Request>, Vec<u8>) {
        let document_id = Id::<Document>::mint();
        let (mut state, ui, sync_rx) = harness();
        state.open_with_identity(document_id, doc_type);
        let passphrase =
            ambre_core::Confidential::<ambre_core::Passphrase>::new("a passphrase".to_owned());
        let command = match doc_type {
            DocType::Trame => AppCommand::Trame(TrameCommand::export(
                ambre_trame::message::TrameExport::AmbreSealed { passphrase },
            )),
            DocType::Toile => AppCommand::Toile(ToileCommand::export(
                ambre_toile::message::ToileExport::AmbreSealed { passphrase },
            )),
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
                } => Some((id, plaintext.into_plaintext())),
                _ => None,
            })
            .expect("the sealed export asked Sync to seal");
        (state, ui, id, plaintext)
    }

    /// The file bytes of the one export event on `ui`, if there is one.
    fn export_bytes(ui: &Receiver<UiEvent>) -> Option<Vec<u8>> {
        std::iter::from_fn(|| ui.try_recv().ok()).find_map(|evt| match evt {
            UiEvent::Viewport { event, .. } => event
                .exported_file()
                .map(|file| file.data.as_bytes().to_vec()),
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
            let header = ambre_client_core::docfile::read_ambre_plaintext(&ambre_core::Payload::<
                ambre_core::PlaintextContainer,
            >::new(
                plaintext.to_vec()
            ))
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
                id: Some(id),
                evt: RestEvent::DocFileSealed {
                    data: ambre_core::Payload::<ambre_core::SealedContainer>::new(
                        b"sealed bytes".to_vec(),
                    ),
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
                id: Some(id),
                evt: RestEvent::DocFileSealFailed,
            });
            let events: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    UiEvent::Viewport { event, .. } if event.interop_failure().is_some()
                )),
                "{doc_type:?}: the failure did not reach the surface",
            );
            assert!(
                !events.iter().any(|e| matches!(
                    e,
                    UiEvent::Viewport { event, .. } if event.exported_file().is_some()
                )),
                "{doc_type:?}: a failed seal produced a file",
            );
        }
    }

    /// `DOCFILE-022`: a seal the Sync worker refused for a weak passphrase
    /// reaches the UI as that refusal, so the prompt asks again. The
    /// application raises no failure and no file, and the seal waits no more.
    #[test]
    fn a_weak_seal_passphrase_reaches_the_prompt_and_no_file() {
        for doc_type in [DocType::Trame, DocType::Toile] {
            let (mut state, ui, id, _plaintext) = sealing(doc_type);
            let weakness = ambre_core::PassphraseWeakness::TooShort;
            state.handle_logic_event(LogicEvent::Rest {
                id: Some(id),
                evt: RestEvent::DocFileSealRefused { weakness },
            });
            let events: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    UiEvent::Rest {
                        evt: RestEvent::DocFileSealRefused { weakness: named },
                        ..
                    } if *named == weakness
                )),
                "{doc_type:?}: the refusal did not reach the UI",
            );
            assert!(
                !events.iter().any(|e| matches!(
                    e,
                    UiEvent::Viewport { event, .. }
                        if event.interop_failure().is_some() || event.exported_file().is_some()
                )),
                "{doc_type:?}: a refused seal raised a failure or a file",
            );
            assert!(
                state.pending_seals.is_empty(),
                "{doc_type:?}: the seal still waits"
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
            id: Some(id),
            evt: RestEvent::DocFileSealed {
                data: ambre_core::Payload::<ambre_core::SealedContainer>::new(
                    b"sealed bytes".to_vec(),
                ),
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
            id: ambre_bus::next_request(),
            cmd,
        }
    }

    fn scroll(version: u64) -> AppCommand {
        AppCommand::Trame(TrameCommand::ScrollViewport {
            version: ambre_core::Seq::nth(version),
            row_start: Pos::<Row>::new(0),
            row_end: Pos::<Row>::new(4),
            col_start: Pos::<Col>::new(0),
            col_end: Pos::<Col>::new(4),
        })
    }

    #[test]
    fn rest_command_forwards_to_sync_with_id() {
        let (mut state, _ui, sync_rx) = harness();
        let id = ambre_bus::next_request();
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
            name: ambre_core::Validated::<ambre_core::Name>::parse("Budget").unwrap(),
        }));
        match sync_rx.try_recv() {
            Ok(SyncCommand::Rest {
                cmd: RestCommand::CreateDocument { doc_type, name },
                ..
            }) => {
                assert_eq!(doc_type, DocType::Trame);
                assert_eq!(name.as_str(), "Budget");
            }
            other => panic!("unexpected SyncCommand: {other:?}"),
        }
    }

    /// Holds `SYNC-236` on the Logic side: the pin of a Murmure link reaches
    /// the Sync worker before the opens the Murmure page asks after it.
    #[test]
    fn a_murmure_inviter_reaches_sync_before_the_spaces_it_roots() {
        let (mut state, _ui, sync_rx) = harness();
        let pin = ambre_core::FixedBytes::<ambre_core::InviterPin>::from_array([8; 32]);
        let space_id = Id::<ambre_core::id::Space>::mint();
        state.handle_logic_command(LogicCommand::PinMurmureInviter { inviter: pin });
        state.handle_logic_command(LogicCommand::OpenSpace { space_id });
        let sent: Vec<SyncCommand> = std::iter::from_fn(|| sync_rx.try_recv().ok()).collect();
        let pinned = sent.iter().position(
            |cmd| matches!(cmd, SyncCommand::PinMurmureInviter { inviter } if *inviter == pin),
        );
        let opened = sent.iter().position(|cmd| {
            matches!(cmd, SyncCommand::OpenDocument { document_id, .. } if *document_id == space_id.document_id())
        });
        assert!(
            pinned.is_some_and(|p| opened.is_some_and(|o| p < o)),
            "{sent:?}"
        );
    }

    /// Holds `SYNC-238` on the Logic side: a pin an open proved goes to the
    /// local store as a row of its document.
    #[test]
    fn a_proven_inviter_goes_to_the_local_store() {
        let (to_ui, _ui) = test_link();
        let (to_sync, _sync) = test_link();
        let (to_persist, persist_rx) = test_link();
        let mut state = LogicState::new(to_ui, to_sync, to_persist);
        let document_id = Id::<Document>::mint();
        let pin = ambre_core::FixedBytes::<ambre_core::InviterPin>::from_array([7; 32]);
        state.handle_logic_event(LogicEvent::InviterProven {
            document_id,
            inviter: pin,
        });
        assert_eq!(
            persist_rx.try_iter().collect::<Vec<_>>(),
            [(document_id, PersistWrite::KeepInviter(pin))]
        );
    }

    /// Holds `SYNC-290` on the Logic side. A kept log goes to the local store
    /// as a row of its document. One the store hands back reaches the Sync
    /// worker unchanged.
    #[test]
    fn a_kept_log_goes_to_the_store_and_back_to_sync() {
        let (to_ui, _ui) = test_link();
        let (to_sync, sync_rx) = test_link();
        let (to_persist, persist_rx) = test_link();
        let mut state = LogicState::new(to_ui, to_sync, to_persist);
        let document_id = Id::<Document>::mint();
        let kept = ambre_bus::KeptLog {
            anchor: ambre_core::FixedBytes::from_array([1; 32]),
            head: ambre_bus::LogHead {
                position: ambre_core::Seq::new(4),
                previous: Some(ambre_core::FixedBytes::from_array([2; 32])),
            },
            key: ambre_core::FixedBytes::from_array([3; 16]),
            links: Vec::new(),
        };
        state.handle_logic_event(LogicEvent::LogKept {
            document_id,
            kept: kept.clone(),
        });
        assert_eq!(
            persist_rx.try_iter().collect::<Vec<_>>(),
            [(document_id, PersistWrite::KeepLog(kept.clone()))]
        );
        state.handle_logic_command(LogicCommand::KeepLog {
            document_id,
            kept: kept.clone(),
        });
        let sent: Vec<SyncCommand> = std::iter::from_fn(|| sync_rx.try_recv().ok()).collect();
        assert!(
            sent.iter().any(|cmd| matches!(
                cmd,
                SyncCommand::KeepLog { document_id: d, kept: k } if *d == document_id && *k == kept
            )),
            "{sent:?}"
        );
    }

    /// Holds `SYNC-300` on the Logic side. A document read again loses its
    /// state and its local copy, then closes and opens again in Sync. The
    /// relay replays it under the cap. One this thread does not hold
    /// reads nothing.
    #[test]
    fn a_document_read_again_drops_its_copy_and_opens_anew() {
        let (to_ui, _ui) = test_link();
        let (to_sync, sync_rx) = test_link();
        let (to_persist, persist_rx) = test_link();
        let mut state = LogicState::new(to_ui, to_sync, to_persist);
        let document_id = Id::<Document>::mint();
        state.handle_logic_command(LogicCommand::OpenDocument {
            document_id,
            doc_type: DocType::Prose,
        });
        let _ = std::iter::from_fn(|| sync_rx.try_recv().ok()).count();
        let _ = persist_rx.try_iter().count();

        state.handle_logic_event(LogicEvent::DocumentReread { document_id });
        let written: Vec<_> = persist_rx.try_iter().collect();
        assert!(
            written.contains(&(document_id, PersistWrite::Erase)),
            "{written:?}"
        );
        let sent: Vec<SyncCommand> = std::iter::from_fn(|| sync_rx.try_recv().ok()).collect();
        let closed = sent.iter().position(
            |cmd| matches!(cmd, SyncCommand::CloseDocument { document_id: d } if *d == document_id),
        );
        let opened = sent.iter().position(
            |cmd| matches!(cmd, SyncCommand::OpenDocument { document_id: d, .. } if *d == document_id),
        );
        assert!(
            closed.is_some_and(|c| opened.is_some_and(|o| c < o)),
            "{sent:?}"
        );

        let stranger = Id::<Document>::mint();
        state.handle_logic_event(LogicEvent::DocumentReread {
            document_id: stranger,
        });
        assert!(persist_rx.try_iter().next().is_none());
        assert!(sync_rx.try_recv().is_err());
    }

    /// Holds `SYNC-209` on the Logic side: a pin reaches the Sync worker
    /// before the open sent after it, unchanged.
    #[test]
    fn a_pinned_inviter_reaches_sync_before_its_open() {
        let (mut state, _ui, sync_rx) = harness();
        let document_id = Id::<Document>::mint();
        let pin = ambre_core::FixedBytes::<ambre_core::InviterPin>::from_array([7; 32]);
        state.handle_logic_command(LogicCommand::PinInviter {
            document_id,
            inviter: pin,
        });
        state.handle_logic_command(LogicCommand::OpenDocument {
            document_id,
            doc_type: DocType::Prose,
        });
        let sent: Vec<SyncCommand> = std::iter::from_fn(|| sync_rx.try_recv().ok()).collect();
        let pinned = sent.iter().position(|cmd| {
            matches!(cmd, SyncCommand::PinInviter { document_id: d, inviter } if *d == document_id && *inviter == pin)
        });
        let opened = sent
            .iter()
            .position(|cmd| matches!(cmd, SyncCommand::OpenDocument { .. }));
        assert!(
            pinned.is_some_and(|p| opened.is_some_and(|o| p < o)),
            "{sent:?}"
        );
    }

    #[test]
    fn close_document_drops_state_and_forwards() {
        let (mut state, _ui, sync_rx) = harness();
        let doc_id = Id::<Document>::mint();
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
            let doc_id = Id::<Document>::mint();
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
        let already_open = Id::<Document>::mint();
        state.docs.insert(
            already_open,
            LiveDoc::new(DocState::Toile(Box::new(ToileState::new()))),
        );

        let me = Id::<User>::mint();
        state.handle_logic_event(LogicEvent::Rest {
            id: Some(ambre_bus::next_request()),
            evt: RestEvent::SessionReady {
                user_id: me,
                display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada")
                    .unwrap(),
                email: "ada@example.com".into(),
            },
        });

        let toile_user_id = |state: &LogicState, id: &Id<Document>| match state.docs.get(id) {
            Some(LiveDoc {
                state: DocState::Toile(s),
                ..
            }) => s.user_id(),
            other => panic!("expected an open Toile doc, got {}", other.is_some()),
        };
        assert_eq!(toile_user_id(&state, &already_open), Some(me));

        // A document opened after authentication is seeded immediately.
        let opened_later = Id::<Document>::mint();
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
        let doc_id = Id::<Document>::mint();
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
        let id = ambre_bus::next_request();
        state.handle_logic_event(LogicEvent::Rest {
            id: Some(id),
            evt: RestEvent::DocumentList { documents: vec![] },
        });
        match ui_rx.try_recv() {
            Ok(UiEvent::Rest {
                id: got,
                evt: RestEvent::DocumentList { .. },
            }) => {
                assert_eq!(got, Some(id));
            }
            other => panic!("unexpected UiEvent: {other:?}"),
        }
    }

    #[test]
    fn app_command_on_live_doc_yields_viewport() {
        let (mut state, ui_rx, _sync) = harness();
        let doc_id = Id::<Document>::mint();
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
        let unknown = Id::<Document>::from_bytes([9; 16]);
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
        let id = Id::<Document>::from_bytes([7; 16]);

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
            row: Pos::<Row>::new(0),
            col: Pos::<Col>::new(0),
            text: text.into(),
        })
    }

    /// Seed a live document already carrying `role`, skipping the open handshake.
    fn live_trame(state: &mut LogicState, role: Role) -> Id<Document> {
        let doc_id = Id::<Document>::mint();
        let mut doc = LiveDoc::new(DocState::Trame(Box::new(TrameState::new())));
        doc.role = role;
        state.docs.insert(doc_id, doc);
        state.grant_identity(doc_id);
        doc_id
    }

    /// The Trame state of an open document.
    fn trame_of(state: &LogicState, id: Id<Document>) -> &TrameState {
        match state.docs.get(&id).map(|doc| &doc.state) {
            Some(DocState::Trame(trame)) => trame,
            other => panic!("expected an open Trame doc, got {other:?}"),
        }
    }

    fn reader(own: &str) -> ambre_client_core::ReaderLocale {
        ambre_client_core::ReaderLocale {
            own: ambre_client_core::DocumentLocale::parse(own).unwrap(),
            convention: ambre_client_core::NumberConvention::default(),
        }
    }

    /// A compaction starts the sweep of the lines an ended session hid. It
    /// starts it on an editor's workbook, and never on a viewer's: a viewer
    /// writes nothing.
    #[test]
    fn only_an_editor_sweeps_the_lines_an_ended_session_hid() {
        // Another session hid two rows, and no user of it is live here.
        let mut other = TrameState::with_peer(ambre_core::PeerId::new(7));
        for row in 0..4u32 {
            other.handle_command(TrameCommand::SetCellText {
                row: ambre_core::index::Pos::new(row),
                col: ambre_core::index::Pos::new(0),
                text: row.to_string(),
            });
        }
        other.handle_command(TrameCommand::SetActiveCell {
            row: ambre_core::index::Pos::new(2),
            col: ambre_core::index::Pos::new(0),
            anchor_row: ambre_core::index::Pos::new(1),
            anchor_col: ambre_core::index::Pos::new(0),
            extra: Vec::new(),
            kind: ambre_trame::SelectionKind::Cells,
        });
        other.handle_command(TrameCommand::DeleteSelectedRows);
        let snapshot = other.export_snapshot();
        for (role, sweeps) in [(Role::Viewer, false), (Role::Editor, true)] {
            let (mut state, _ui, _sync) = harness();
            let doc_id = live_trame(&mut state, role);
            if let Some(DocState::Trame(trame)) = state.docs.get_mut(&doc_id).map(|d| &mut d.state)
            {
                trame
                    .import_snapshot(&snapshot)
                    .expect("a snapshot this build wrote");
            }
            state.handle_logic_event(LogicEvent::SnapshotNeeded {
                document_id: doc_id,
            });
            assert_eq!(state.has_pending_work(), sweeps, "{role:?}");
        }
    }

    /// `LOCALE-007`: a viewer's locale set is an edit, and it is dropped
    /// before the engine.
    #[test]
    fn a_viewers_locale_set_is_dropped() {
        let (mut state, _ui, sync_rx) = harness();
        let doc_id = live_trame(&mut state, Role::Viewer);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Trame(TrameCommand::SetDocumentLocale {
                locale: ambre_client_core::DocumentLocale::parse("fr").unwrap(),
            }),
        });
        assert!(sync_rx.try_recv().is_err(), "no delta leaves");
        assert_eq!(
            trame_of(&state, doc_id).document_locale(),
            ambre_client_core::DocumentLocale::NONE
        );
    }

    /// `LOCALE-006`: a locale set reaches Sync as one CRDT delta, which the
    /// worker seals like every other, and as no field of its own.
    #[test]
    fn a_locale_set_leaves_as_one_delta_and_nothing_else() {
        let (mut state, _ui, sync_rx) = harness();
        let doc_id = live_trame(&mut state, Role::Editor);
        while sync_rx.try_recv().is_ok() {}
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Trame(TrameCommand::SetDocumentLocale {
                locale: ambre_client_core::DocumentLocale::parse("fr").unwrap(),
            }),
        });
        state.answer_pass();
        let sent: Vec<SyncCommand> = std::iter::from_fn(|| sync_rx.try_recv().ok()).collect();
        assert_eq!(sent.len(), 1, "one command reaches Sync");
        assert!(matches!(sent[0], SyncCommand::SendUpdate { .. }));
    }

    /// `LOCALE-019`: a document this reader creates takes their locale once,
    /// as its first edit, and a document they open is not written.
    #[test]
    fn a_created_document_is_seeded_with_its_creators_locale() {
        let (mut state, _ui, _sync) = harness();
        state.handle_logic_command(LogicCommand::SetReaderLocale {
            reader: reader("fr-CA"),
        });
        let created = Id::<Document>::mint();
        state.handle_logic_event(LogicEvent::Rest {
            id: Some(ambre_bus::next_request()),
            evt: RestEvent::DocumentCreated {
                document_id: created,
                doc_type: DocType::Trame,
            },
        });
        let opened = Id::<Document>::mint();
        for document_id in [created, opened] {
            state.handle_logic_command(LogicCommand::OpenDocument {
                document_id,
                doc_type: DocType::Trame,
            });
            state.grant_identity(document_id);
        }
        assert_eq!(
            trame_of(&state, created).document_locale(),
            ambre_client_core::DocumentLocale::parse("fr-CA").unwrap()
        );
        assert_eq!(
            trame_of(&state, opened).document_locale(),
            ambre_client_core::DocumentLocale::NONE
        );
        // A second open of the created document seeds nothing again.
        assert!(state.locale_seeds.is_empty());
    }

    /// `LOCALE-017`: the reader's half reaches every open document and one
    /// opened later, and no delta leaves for it.
    #[test]
    fn the_readers_locale_reaches_each_document_and_writes_nothing() {
        let (mut state, _ui, sync_rx) = harness();
        let before = live_trame(&mut state, Role::Editor);
        while sync_rx.try_recv().is_ok() {}
        state.handle_logic_command(LogicCommand::SetReaderLocale {
            reader: reader("de"),
        });
        assert!(sync_rx.try_recv().is_err(), "nothing is sent to Sync");
        let after = Id::<Document>::mint();
        state.handle_logic_command(LogicCommand::OpenDocument {
            document_id: after,
            doc_type: DocType::Trame,
        });
        for id in [before, after] {
            assert_eq!(*trame_of(&state, id).reader_locale(), reader("de"));
        }
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
        state.answer_pass();

        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::SendUpdate { .. })
        ));
    }

    /// An import hands the Sync worker a CRDT delta, never the bytes of the
    /// file the user opened. The worker seals that delta before it leaves.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_import_hands_sync_a_delta_and_never_the_file() {
        let (mut state, _ui, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        let doc_id = live_trame(&mut state, Role::Editor);
        load_converter(&mut state, ambre_core::ConverterFamily::Tables);
        let file = b"Quarterly report,Owner\nNorth region,Alice Marchand\n".to_vec();
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Trame(TrameCommand::import_csv(ambre_trame::message::CsvImport {
                data: ambre_core::Payload::<ambre_client_core::FileBytes>::new(file.clone()),
                delimiter: None,
                header: ambre_trame::message::HeaderRow::Absent,
                target_sheet_id: None,
                start_row: Pos::<Row>::new(0),
                start_col: Pos::<Col>::new(0),
            })),
        });
        state.answer_pass();
        // The read runs on the converter thread, and its answer writes the
        // workbook on the next pass (`ARCH-123`).
        thread.serve();
        state.take_converted();
        state.answer_pass();
        let sent: Vec<SyncCommand> = std::iter::from_fn(|| sync_rx.try_recv().ok()).collect();
        assert!(
            sent.iter()
                .any(|c| matches!(c, SyncCommand::SendUpdate { .. })),
            "the import reached no peer",
        );
        // A payload prints its length alone, so the search reads the bytes
        // each command carries: the file's run must appear in none of them.
        let carried = |cmd: &SyncCommand| -> Vec<u8> {
            match cmd {
                SyncCommand::SendUpdate { payload, .. }
                | SyncCommand::SendUnsent { payload, .. } => payload.as_bytes().to_vec(),
                SyncCommand::SendSnapshot { payload, .. } => payload.as_bytes().to_vec(),
                _ => Vec::new(),
            }
        };
        let holds_file = |bytes: &[u8]| bytes.windows(file.len()).any(|w| w == file.as_slice());
        let forwarded = SyncCommand::SendUpdate {
            document_id: doc_id,
            payload: file.clone().into(),
            summary: None,
        };
        assert!(
            holds_file(&carried(&forwarded)),
            "the search sees a forwarded file"
        );
        for cmd in &sent {
            assert!(!holds_file(&carried(cmd)), "the file left as it was opened");
        }
    }

    #[test]
    fn document_ready_supersedes_the_optimistic_owner_role() {
        let (mut state, ui_rx, sync_rx) = harness();
        let doc_id = Id::<Document>::from_bytes([3; 16]);

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
    fn live_toile(state: &mut LogicState, role: Role) -> Id<Document> {
        let doc_id = Id::<Document>::mint();
        let mut toile = ToileState::new();
        toile.set_user_id(Id::<User>::mint());
        let mut doc = LiveDoc::new(DocState::Toile(Box::new(toile)));
        doc.role = role;
        state.docs.insert(doc_id, doc);
        state.grant_identity(doc_id);
        doc_id
    }

    /// Drain every `SendUpdate` and report which documents produced one -- which
    /// document a delta belongs to is exactly what these tests are about.
    fn updated_docs(sync_rx: &Receiver<SyncCommand>) -> Vec<Id<Document>> {
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
        state.answer_pass();

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
            document_id: Id::<Document>::mint(),
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
        let doc_id = Id::<Document>::mint();
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
        let doc_id = Id::<Document>::mint();
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
            !payload
                .as_bytes()
                .starts_with(&ambre_client_core::AMBRE_MAGIC),
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
        assert_eq!(
            reopened.element_count(),
            ambre_core::Count::<ambre_core::Elements>::new(1)
        );
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

    /// Import a solid PNG into `doc_id` and return the single `Seq<Request>` the
    /// lowered `UploadMedia` was correlated with.
    fn import_one_image(state: &mut LogicState, doc_id: Id<Document>) -> Seq<Request> {
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::import(
                ambre_toile::message::ToileImportFormat::Image("image/png".into()),
                ambre_core::Payload::<ambre_client_core::FileBytes>::new(solid_png(8, 8)),
                kurbo::Point::ZERO,
            )),
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
            id: Some(id),
            evt: RestEvent::MediaUploaded {
                document_id: doc_id,
                content_hash: ambre_core::Validated::<ambre_core::ContentHash>::parse(
                    "b2".repeat(32),
                )
                .expect("a hash"),
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
            id: Some(ambre_bus::next_request()),
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
            request: ambre_bus::ApiRequest::UploadMedia,
            failure: ambre_bus::ApiFailure::Relay {
                status: ambre_bus::HttpStatus::new(500),
                code: None,
            },
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
            "the abort should emit an import failure viewport event"
        );
    }

    #[test]
    fn an_uncorrelated_api_error_still_forwards_to_the_ui() {
        let (mut state, ui_rx, _sync) = harness();
        state.handle_logic_event(LogicEvent::ApiError {
            id: None,
            request: ambre_bus::ApiRequest::ListDocuments,
            failure: ambre_bus::ApiFailure::Relay {
                status: ambre_bus::HttpStatus::new(500),
                code: None,
            },
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
            id: Some(id),
            evt: RestEvent::MediaUploaded {
                document_id: doc_id,
                content_hash: ambre_core::Validated::<ambre_core::ContentHash>::parse(
                    "b2".repeat(32),
                )
                .expect("a hash"),
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
    fn relayed_requests(doc: Id<Document>) -> Vec<RestCommand> {
        use ambre_bus::Role;
        use ambre_bus::rest::{Confidential, Plaintext, RecoveryWords};
        use ambre_core::id::Meeting;
        let secret = || Confidential::<Plaintext>::new(b"pass".to_vec());
        let typed = || ambre_core::Confidential::<ambre_core::Passphrase>::new("pass".to_owned());
        let (user, meeting) = (Id::<User>::mint(), Id::<Meeting>::mint());
        let all = vec![
            RestCommand::Unlock {
                passphrase: typed(),
            },
            RestCommand::SetUpKeys {
                passphrase: typed(),
            },
            RestCommand::DevLogin {
                username: ambre_core::Validated::<ambre_core::DisplayName>::parse("a").unwrap(),
            },
            RestCommand::ChangePassphrase {
                current_passphrase: typed(),
                new_passphrase: typed(),
            },
            RestCommand::RecoverAccount {
                recovery_words: Confidential::<RecoveryWords>::new("words".to_owned()),
                new_passphrase: typed(),
            },
            RestCommand::ListDocuments,
            RestCommand::CreateDocument {
                doc_type: DocType::Trame,
                name: ambre_core::Validated::<ambre_core::Name>::parse("n").unwrap(),
            },
            RestCommand::RenameDocument {
                document_id: doc,
                name: ambre_core::Validated::<ambre_core::Name>::parse("n").unwrap(),
            },
            RestCommand::FetchInviteeKey {
                document_id: doc,
                invitee_email: ambre_core::Email::from("a@b.c"),
            },
            RestCommand::FetchMemberKey { user_id: user },
            RestCommand::InviteToDocument {
                document_id: doc,
                invitee_email: Some(ambre_core::Email::from("a@b.c")),
                invitee: user,
                role: Role::Editor,
                expect_exchange_pub_key: FixedBytes::from_array([0; 32]),
                now: Instant::<Authored>::new(0),
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
                query: ambre_core::Validated::<ambre_core::UserSearch>::parse("ab")
                    .expect("two characters"),
            },
            RestCommand::UploadMedia {
                document_id: doc,
                data: vec![1].into(),
            },
            RestCommand::DownloadMedia {
                document_id: doc,
                content_hash: ambre_core::Validated::<ambre_core::ContentHash>::parse(
                    "a".repeat(64),
                )
                .expect("a hash"),
            },
            RestCommand::EraseMedia {
                document_id: doc,
                content_hash: ambre_core::Validated::<ambre_core::ContentHash>::parse(
                    "a".repeat(64),
                )
                .expect("a hash"),
            },
            RestCommand::SealDocFile {
                plaintext: secret(),
                passphrase: typed(),
            },
            RestCommand::OpenDocFile {
                sealed: ambre_core::Payload::<ambre_core::SealedContainer>::new(vec![1]),
                passphrase: typed(),
            },
            RestCommand::SealLocalRow {
                place: ambre_bus::RowPlace::Document(doc),
                plaintext: secret(),
            },
            RestCommand::OpenLocalRows {
                rows: vec![ambre_bus::SealedRow {
                    place: ambre_bus::RowPlace::Document(doc),
                    sealed: ambre_core::Payload::<ambre_core::SealedRowBytes>::new(vec![1]),
                }],
            },
            RestCommand::CreateMeeting {
                name: ambre_core::Validated::<ambre_core::Name>::parse("m").unwrap(),
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
                provider: ambre_core::OidcProvider::Google,
            },
            RestCommand::ProviderReturn {
                provider: ambre_core::OidcProvider::Google,
                code: ambre_core::Credential::new("c".to_owned()),
                state: ambre_core::Credential::new("s".to_owned()),
            },
            RestCommand::ResumeSession,
        ];
        for cmd in &all {
            let removes = match cmd {
                RestCommand::Logout | RestCommand::DeleteDocument { .. } => true,
                RestCommand::Unlock { .. }
                | RestCommand::SetUpKeys { .. }
                | RestCommand::DevLogin { .. }
                | RestCommand::ChangePassphrase { .. }
                | RestCommand::RecoverAccount { .. }
                | RestCommand::ListDocuments
                | RestCommand::CreateDocument { .. }
                | RestCommand::RenameDocument { .. }
                | RestCommand::FetchInviteeKey { .. }
                | RestCommand::FetchMemberKey { .. }
                | RestCommand::InviteToDocument { .. }
                | RestCommand::RevokeFromDocument { .. }
                | RestCommand::LeaveDocument { .. }
                | RestCommand::ListDocumentMembers { .. }
                | RestCommand::UpdateDocumentMemberRole { .. }
                | RestCommand::SearchUsers { .. }
                | RestCommand::UploadMedia { .. }
                | RestCommand::DownloadMedia { .. }
                | RestCommand::EraseMedia { .. }
                | RestCommand::SealDocFile { .. }
                | RestCommand::OpenDocFile { .. }
                | RestCommand::SealLocalRow { .. }
                | RestCommand::OpenLocalRows { .. }
                | RestCommand::CreateMeeting { .. }
                | RestCommand::JoinMeeting { .. }
                | RestCommand::LeaveMeeting { .. }
                | RestCommand::EndMeeting { .. }
                | RestCommand::ProviderSignIn { .. }
                | RestCommand::ProviderReturn { .. }
                | RestCommand::ResumeSession => false,
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
        let doc = Id::<Document>::mint();
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
    fn shipped_faces() -> Vec<ambre_core::Payload<ambre_core::FontBytes>> {
        faces_of_files(&["NotoSans-Regular.ttf", "DejaVuSerif.ttf"])
    }

    /// The faces of the files named, read off the tree.
    fn faces_of_files(files: &[&str]) -> Vec<ambre_core::Payload<ambre_core::FontBytes>> {
        const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
        files
            .iter()
            .filter_map(|file| std::fs::read(format!("{DIR}{file}")).ok())
            .map(ambre_core::Payload::<ambre_core::FontBytes>::new)
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

    /// Every handle on a face this thread holds is the registry's copy
    /// (`CANVAS-097`). The holders are the export context and each Prose
    /// document, opened before or after a delivery: its list and its measuring
    /// context. A face delivered twice stays one copy.
    ///
    /// The check is pointer identity. A holder that cloned the bytes reads an
    /// equal copy at another address, which the registry does not hold.
    #[test]
    fn the_logic_thread_holds_one_copy_of_each_face() {
        let (mut state, _ui, _sync) = harness();
        let early = Id::<Document>::mint();
        state.open_with_identity(early, DocType::Prose);
        state.handle_logic_command(LogicCommand::LoadFonts {
            faces: shipped_faces(),
        });
        let late = Id::<Document>::mint();
        state.open_with_identity(late, DocType::Prose);
        // Each document builds its measuring context on its first layout.
        for id in [early, late] {
            if let DocState::Prose(prose) = &mut state.docs.get_mut(&id).expect("open").state {
                prose.paginate(&[]);
            }
        }
        // A late face, then the base faces again.
        state.handle_logic_command(LogicCommand::LoadFonts {
            faces: faces_of_files(&["DejaVuSansMono.ttf"]),
        });
        state.handle_logic_command(LogicCommand::LoadFonts {
            faces: shipped_faces(),
        });

        let delivered = shipped_faces().len() + 1;
        assert_eq!(delivered, 3, "the workspace ships the faces");
        assert_eq!(state.faces.faces().len(), delivered, "one entry per face");

        let export_cx = state.export_cx.as_mut().expect("a delivery seeds it");
        let export = ambre_draw::faces_of(export_cx);
        assert_eq!(export.len(), delivered, "the export reads each face once");
        assert!(export.iter().all(|face| state.faces.holds(face)));

        for id in [early, late] {
            let DocState::Prose(prose) = &mut state.docs.get_mut(&id).expect("open").state else {
                panic!("a Prose document");
            };
            let copies = prose.face_copies();
            // The list, then the measuring context: two handles per face.
            assert_eq!(
                copies.len(),
                2 * delivered,
                "each face, listed and measured"
            );
            assert!(
                copies.iter().all(|face| state.faces.holds(face)),
                "a document reads a copy of its own"
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

        let before = Id::<Document>::mint();
        state.open_with_identity(before, DocType::Prose);
        assert_eq!(
            state.docs[&before].state.face_count(),
            ambre_core::Count::<ambre_prose::logic::Faces>::ZERO,
            "nothing has been delivered yet"
        );

        state.handle_logic_command(LogicCommand::LoadFonts {
            faces: faces.clone(),
        });
        assert_eq!(
            state.docs[&before].state.face_count(),
            ambre_core::Count::<ambre_prose::logic::Faces>::of(faces.len()),
            "a document open at delivery takes the faces"
        );

        let after = Id::<Document>::mint();
        state.open_with_identity(after, DocType::Prose);
        assert_eq!(
            state.docs[&after].state.face_count(),
            ambre_core::Count::<ambre_prose::logic::Faces>::of(faces.len()),
            "a document opened afterwards takes the same set"
        );

        // A later delivery (a script face) reaches both (`CANVAS-073`).
        state.handle_logic_command(LogicCommand::LoadFonts {
            faces: faces_of_files(&["DejaVuSansMono.ttf"]),
        });
        for id in [before, after] {
            assert_eq!(
                state.docs[&id].state.face_count(),
                ambre_core::Count::<ambre_prose::logic::Faces>::of(faces.len() + 1),
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

    /// The converter module of `family` as the Sync worker hands it over.
    /// The host links every engine, so the bytes are not read.
    fn load_converter(state: &mut LogicState, family: ambre_core::ConverterFamily) {
        state.handle_logic_event(LogicEvent::ConverterFetched {
            family,
            module: ambre_core::Payload::new(Vec::new()),
        });
    }

    /// The Toile events the UI received, in order, draining the queue.
    fn toile_answers(ui_rx: &Receiver<UiEvent>) -> Vec<ambre_toile::message::ToileEvent> {
        ui_rx
            .try_iter()
            .filter_map(|e| match e {
                UiEvent::Viewport {
                    event: AppEvent::Toile(te),
                    ..
                } => Some(te),
                _ => None,
            })
            .collect()
    }

    /// Ask a live board for its deck, and report the Toile event that came
    /// back. The converter thread serves the write of the file, then the
    /// next pass lands its answer (`ARCH-124`).
    fn export_pdf_answer(
        state: &mut LogicState,
        ui_rx: &Receiver<UiEvent>,
        doc_id: Id<Document>,
    ) -> ambre_toile::message::ToileEvent {
        load_converter(state, ambre_core::ConverterFamily::Pdf);
        #[cfg(not(target_arch = "wasm32"))]
        let mut thread = converter_thread(state);
        // Whatever seeded the board has already spoken; the export's own answer
        // is the only event this reads.
        ui_rx.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::export(
                ambre_toile::message::ToileExport::Pdf,
            )),
        });
        #[cfg(not(target_arch = "wasm32"))]
        {
            thread.serve();
            state.take_converted();
        }
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
    fn board_with_content(state: &mut LogicState) -> Id<Document> {
        let doc_id = live_toile(state, Role::Owner);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::CreateElement {
                kind: ambre_toile::logic::model::ElementKind::Shape(
                    ambre_toile::logic::model::ShapeKind::Rect,
                ),
                at: kurbo::Point::new(0.0, 0.0),
                size: kurbo::Size::new(100.0, 60.0),
                content: ambre_toile::message::InitContent::None,
                fill: None,
                correlate: None,
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
            matches!(
                answer,
                ambre_toile::message::ToileEvent::Interop(
                    ambre_bus::interop::InteropAnswer::Failed(_)
                )
            ),
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
            ambre_toile::message::ToileEvent::Interop(
                ambre_bus::interop::InteropAnswer::ExportReady { file, .. },
            ) => {
                assert_eq!(file.filename.as_str(), "deck.pdf");
                assert!(file.data.as_bytes().starts_with(b"%PDF-"), "a PDF header");
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
            ambre_toile::message::ToileEvent::Interop(
                ambre_bus::interop::InteropAnswer::ExportReady { .. }
            )
        ));
        assert!(sync_rx.try_recv().is_err(), "the export sent a delta");
    }

    /// A text that no held face covers still answers with the file, at each
    /// export (`PDF-020`). The export waits on no face: the thread has no
    /// window to fetch one.
    #[cfg(feature = "toile-pdf")]
    #[test]
    fn a_deck_export_over_a_face_the_thread_lacks_answers_with_the_file() {
        let (mut state, ui_rx, _sync) = harness();
        state.handle_logic_command(LogicCommand::LoadFonts {
            faces: shipped_faces(),
        });
        let doc_id = live_toile(&mut state, Role::Owner);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::CreateElement {
                kind: ambre_toile::logic::model::ElementKind::Text,
                at: kurbo::Point::new(0.0, 0.0),
                size: kurbo::Size::new(300.0, 80.0),
                content: ambre_toile::message::InitContent::Text("\u{4e2d}\u{6587}".to_owned()),
                fill: None,
                correlate: None,
            }),
        });
        for _ in 0..2 {
            assert!(
                matches!(
                    export_pdf_answer(&mut state, &ui_rx, doc_id),
                    ambre_toile::message::ToileEvent::Interop(
                        ambre_bus::interop::InteropAnswer::ExportReady { .. }
                    )
                ),
                "the export waited on a face"
            );
        }
    }

    /// The Prose PDF export, routed. The command waits on the PDF module, and
    /// the pages are walked one slice a pass. The last page leaves the write
    /// to the converter thread, and the Logic thread never waits for it
    /// (`ARCH-124`). The file comes back to the UI thread, and nothing
    /// reaches Sync but the module fetch (`PDF-017`).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_prose_pdf_export_answers_with_the_file_and_sends_no_delta() {
        use ambre_bus::interop::{InteropAnswer, InteropRequest};
        use ambre_prose::message::{ExportFormat, PdfExport, ProseCommand, ProseEvent};
        let (mut state, ui, sync) = harness();
        let mut thread = converter_thread(&mut state);
        state.handle_logic_command(LogicCommand::LoadFonts {
            faces: shipped_faces(),
        });
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, DocType::Prose);
        let block = match &state.docs.get(&document_id).expect("a live document").state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ProseCommand::InsertText {
                block_id: block,
                pos: Pos::<DocChar>::ZERO,
                text: "printed".to_owned(),
            }),
        });
        // The edit's delta leaves on its own pass, before the export.
        state.answer_pass();
        sync.try_iter().for_each(drop);
        ui.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ProseCommand::Interop(InteropRequest::Export(
                ExportFormat::Pdf(PdfExport::default()),
            ))),
        });
        assert!(matches!(
            sync.try_recv(),
            Ok(SyncCommand::FetchConverter {
                family: ambre_core::ConverterFamily::Pdf
            })
        ));
        load_converter(&mut state, ambre_core::ConverterFamily::Pdf);
        while state.has_pending_work() {
            state.resume_import(document_id);
        }
        // The pass answers the document: its requests leave then.
        state.answer_pass();
        assert!(
            !prose_answers(&ui)
                .iter()
                .any(|e| matches!(e, ProseEvent::Interop(InteropAnswer::ExportReady { .. }))),
            "the write is in flight"
        );
        assert!(!state.offload.is_idle(), "the pages left for the thread");
        thread.serve();
        state.take_converted();
        let file = ui.try_iter().find_map(|e| match e {
            UiEvent::Viewport {
                event:
                    AppEvent::Prose(ProseEvent::Interop(
                        ambre_bus::interop::InteropAnswer::ExportReady { file, .. },
                    )),
                ..
            } => Some(file),
            _ => None,
        });
        let file = file.expect("the export answers with its file");
        assert_eq!(file.filename.as_str(), "document.pdf");
        assert!(file.data.as_bytes().starts_with(b"%PDF-"), "a PDF header");
        assert!(sync.try_recv().is_err(), "the export sent a delta");
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

    // -- The deck's PowerPoint export (`toile-pptx-export`) --------------------

    /// The whole seam: the File menu's command asks the Sync worker for the
    /// Office module and waits for it. It comes back to the UI thread as a
    /// named PowerPoint file with the map's report. The report names the
    /// sticky note as approximated, never lost in silence.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_pptx_export_reaches_the_ui_as_a_file_and_its_report() {
        let (mut state, ui_rx, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        let doc_id = board_with_content(&mut state);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::CreateElement {
                kind: ambre_toile::logic::model::ElementKind::Sticky,
                at: kurbo::Point::new(200.0, 0.0),
                size: kurbo::Size::new(100.0, 100.0),
                content: ambre_toile::message::InitContent::None,
                fill: None,
                correlate: None,
            }),
        });
        sync_rx.try_iter().for_each(drop);
        ui_rx.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::export(
                ambre_toile::message::ToileExport::Pptx,
            )),
        });
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::FetchConverter {
                family: ambre_core::ConverterFamily::Office
            })
        ));
        assert!(toile_answers(&ui_rx).is_empty(), "the export waits");
        load_converter(&mut state, ambre_core::ConverterFamily::Office);
        // The write runs on the converter thread (`ARCH-129`).
        thread.serve();
        state.take_converted();
        let answer = toile_answers(&ui_rx).into_iter().next();
        let Some(ambre_toile::message::ToileEvent::Interop(
            ambre_bus::interop::InteropAnswer::ExportReady { file, report },
        )) = answer
        else {
            panic!("expected the file, got {answer:?}");
        };
        assert_eq!(file.filename.as_str(), "deck.pptx");
        assert_eq!(
            file.media_type.as_str(),
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        );
        assert!(file.data.as_bytes().starts_with(b"PK"), "a zip package");
        assert_eq!(report.format(), ambre_bus::interop::FileFormat::Pptx);
        assert_eq!(
            report.count(
                ambre_toile::message::ToileConstruct::StickyNote,
                ambre_bus::interop::Outcome::Approximated
            ),
            ambre_core::Count::of(1_usize)
        );
        assert!(sync_rx.try_recv().is_err(), "the export sent a delta");
    }

    /// Through the seam, a board with no element answers a failure that
    /// names the reason, and no file.
    #[test]
    fn a_pptx_export_of_an_empty_board_answers_a_reason() {
        let (mut state, ui_rx, _sync) = harness();
        let doc_id = live_toile(&mut state, Role::Viewer);
        load_converter(&mut state, ambre_core::ConverterFamily::Office);
        ui_rx.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::export(
                ambre_toile::message::ToileExport::Pptx,
            )),
        });
        assert!(matches!(
            toile_answers(&ui_rx).as_slice(),
            [ambre_toile::message::ToileEvent::Interop(
                ambre_bus::interop::InteropAnswer::Failed(_)
            )]
        ));
    }

    // -- The converter module (`ARCH-081`) -------------------------------------

    /// The first command that needs the converter asks the Sync worker once,
    /// and every such command waits. When the module lands, each runs in
    /// order, and its write answers once the converter thread served it.
    #[cfg(all(feature = "toile-pdf", not(target_arch = "wasm32")))]
    #[test]
    fn the_first_conversion_asks_for_the_converter_once_and_the_rest_wait() {
        let (mut state, ui_rx, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        state.handle_logic_command(LogicCommand::LoadFonts { faces: Vec::new() });
        let doc_id = board_with_content(&mut state);
        sync_rx.try_iter().for_each(drop);
        ui_rx.try_iter().for_each(drop);
        for _ in 0..2 {
            state.handle_logic_command(LogicCommand::App {
                document_id: doc_id,
                command: AppCommand::Toile(ToileCommand::export(
                    ambre_toile::message::ToileExport::Pdf,
                )),
            });
        }
        let asked: Vec<SyncCommand> = sync_rx.try_iter().collect();
        assert!(
            matches!(
                asked.as_slice(),
                [SyncCommand::FetchConverter {
                    family: ambre_core::ConverterFamily::Pdf
                }]
            ),
            "one fetch for two exports: {asked:?}"
        );
        assert!(
            toile_answers(&ui_rx).is_empty(),
            "the exports wait for the module"
        );
        load_converter(&mut state, ambre_core::ConverterFamily::Pdf);
        thread.serve();
        state.take_converted();
        let answers = toile_answers(&ui_rx);
        assert_eq!(answers.len(), 2);
        assert!(answers.iter().all(|a| matches!(
            a,
            ambre_toile::message::ToileEvent::Interop(
                ambre_bus::interop::InteropAnswer::ExportReady { .. }
            )
        )));
        // Loaded once, the converter serves the next export at once.
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::export(
                ambre_toile::message::ToileExport::Pdf,
            )),
        });
        assert!(sync_rx.try_recv().is_err(), "no second fetch");
        thread.serve();
        state.take_converted();
        assert_eq!(toile_answers(&ui_rx).len(), 1);
    }

    /// A refused fetch answers every waiting command with the reason, and the
    /// next command asks again.
    #[test]
    fn a_refused_converter_answers_each_waiting_command_and_the_next_asks_again() {
        let (mut state, ui_rx, sync_rx) = harness();
        let doc_id = board_with_content(&mut state);
        sync_rx.try_iter().for_each(drop);
        ui_rx.try_iter().for_each(drop);
        let export = || LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::export(
                ambre_toile::message::ToileExport::Pdf,
            )),
        };
        state.handle_logic_command(export());
        state.handle_logic_event(LogicEvent::ConverterRefused {
            family: ambre_core::ConverterFamily::Pdf,
            cause: ambre_core::ConverterRefusal::NotFetched,
        });
        match toile_answers(&ui_rx).as_slice() {
            [
                ambre_toile::message::ToileEvent::Interop(
                    failed @ ambre_bus::interop::InteropAnswer::Failed(_),
                ),
            ] => {
                assert!(
                    matches!(
                        failed,
                        ambre_bus::interop::InteropAnswer::Failed(
                            ambre_bus::interop::InteropFailure {
                                error: ambre_bus::interop::InteropError::Stopped(
                                    ambre_core::StopCause::ConverterRefused(
                                        ambre_core::ConverterRefusal::NotFetched
                                    )
                                ),
                                ..
                            }
                        )
                    ),
                    "the cause reaches the user: {failed:?}"
                );
            }
            other => panic!("expected one refusal, got {other:?}"),
        }
        sync_rx.try_iter().for_each(drop);
        state.handle_logic_command(export());
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::FetchConverter {
                family: ambre_core::ConverterFamily::Pdf
            })
        ));
    }

    /// Past the waiting limit, a command is told to try again rather than
    /// held without bound.
    #[test]
    fn a_command_past_the_waiting_limit_is_told_to_try_again() {
        let (mut state, ui_rx, _sync) = harness();
        let doc_id = board_with_content(&mut state);
        ui_rx.try_iter().for_each(drop);
        for _ in 0..=crate::converting::WAITING_LIMIT.get() {
            state.handle_logic_command(LogicCommand::App {
                document_id: doc_id,
                command: AppCommand::Toile(ToileCommand::export(
                    ambre_toile::message::ToileExport::Pdf,
                )),
            });
        }
        assert!(matches!(
            toile_answers(&ui_rx).as_slice(),
            [ambre_toile::message::ToileEvent::Interop(
                ambre_bus::interop::InteropAnswer::Failed(_)
            )]
        ));
    }

    /// A Trame CSV export waits for the converter like a deck export, and the
    /// workbook writes its file through it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_csv_export_converts_once_the_module_lands() {
        let (mut state, ui_rx, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        let doc_id = live_trame(&mut state, Role::Editor);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: set_cell("42"),
        });
        sync_rx.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Trame(TrameCommand::export(
                ambre_trame::message::TrameExport::Csv {
                    sheet_id: None,
                    delimiter: None,
                    range: None,
                },
            )),
        });
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::FetchConverter {
                family: ambre_core::ConverterFamily::Tables
            })
        ));
        ui_rx.try_iter().for_each(drop);
        load_converter(&mut state, ambre_core::ConverterFamily::Tables);
        // The write runs on the converter thread (`ARCH-123`).
        thread.serve();
        state.take_converted();
        let file = ui_rx.try_iter().find_map(|e| match e {
            UiEvent::Viewport {
                event:
                    AppEvent::Trame(ambre_trame::message::TrameEvent::Interop(
                        ambre_bus::interop::InteropAnswer::ExportReady { file, .. },
                    )),
                ..
            } => Some(file.data),
            _ => None,
        });
        assert_eq!(file.map(|d| d.as_bytes().to_vec()), Some(b"42\n".to_vec()));
    }

    /// A workbook of 150 labels down column A, with the shipped faces
    /// delivered, so a PDF export of it walks three pages.
    #[cfg(not(target_arch = "wasm32"))]
    fn printable_trame(state: &mut LogicState) -> Id<Document> {
        state.handle_logic_command(LogicCommand::LoadFonts {
            faces: shipped_faces(),
        });
        let doc_id = live_trame(state, Role::Editor);
        let cells = (0..150)
            .map(|r| {
                (
                    ambre_trame::CellPos::new(Pos::<Row>::new(r), Pos::<Col>::new(0)),
                    format!("r{r}"),
                )
            })
            .collect();
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Trame(TrameCommand::SetCellsBatch { cells }),
        });
        doc_id
    }

    /// The sheet export the File menu asks for.
    #[cfg(not(target_arch = "wasm32"))]
    fn sheet_pdf() -> AppCommand {
        AppCommand::Trame(TrameCommand::export(
            ambre_trame::message::TrameExport::Pdf(ambre_trame::message::PdfExport::default()),
        ))
    }

    /// The file of each `ExportReady` the UI received, draining the queue.
    #[cfg(not(target_arch = "wasm32"))]
    fn trame_files(ui_rx: &Receiver<UiEvent>) -> Vec<ambre_bus::interop::ExportedFile> {
        ui_rx
            .try_iter()
            .filter_map(|e| match e {
                UiEvent::Viewport {
                    event:
                        AppEvent::Trame(ambre_trame::message::TrameEvent::Interop(
                            ambre_bus::interop::InteropAnswer::ExportReady { file, .. },
                        )),
                    ..
                } => Some(file),
                _ => None,
            })
            .collect()
    }

    /// `PDF-002`, `ARCH-124`: the sheet export, routed. The command waits
    /// on the PDF module, then each pass of the loop draws one page. The last
    /// page leaves the write to the converter thread, and the Logic thread
    /// never waits for it. The file comes back to the UI thread, and nothing
    /// reaches Sync but the module fetch.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_sheet_pdf_export_answers_with_the_file_and_sends_no_delta() {
        let (mut state, ui_rx, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        let doc_id = printable_trame(&mut state);
        sync_rx.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: sheet_pdf(),
        });
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::FetchConverter {
                family: ambre_core::ConverterFamily::Pdf
            })
        ));
        ui_rx.try_iter().for_each(drop);
        load_converter(&mut state, ambre_core::ConverterFamily::Pdf);
        let mut passes = 0;
        while state.has_pending_work() {
            assert!(trame_files(&ui_rx).is_empty(), "a page is still owed");
            state.resume_import(doc_id);
            passes += 1;
        }
        assert_eq!(passes, 3, "one pass per page");
        assert!(trame_files(&ui_rx).is_empty(), "the write is in flight");
        assert!(!state.offload.is_idle(), "the pages left for the thread");
        thread.serve();
        state.take_converted();
        let files = trame_files(&ui_rx);
        let [file] = files.as_slice() else {
            panic!("one file, got {}", files.len());
        };
        assert_eq!(file.filename.as_str(), "Sheet1.pdf");
        assert!(file.data.as_bytes().starts_with(b"%PDF-"), "a PDF header");
        assert!(sync_rx.try_recv().is_err(), "the export sent a delta");
    }

    /// A scroll is served between two pages and leaves the export
    /// running. An edit lands the export whole first, so the write of the
    /// file leaves for the converter thread as the edit is applied.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_edit_lands_a_running_sheet_export_first() {
        let (mut state, ui_rx, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        let doc_id = printable_trame(&mut state);
        load_converter(&mut state, ambre_core::ConverterFamily::Pdf);
        sync_rx.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: sheet_pdf(),
        });
        state.resume_import(doc_id);
        ui_rx.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Trame(TrameCommand::ScrollViewport {
                version: ambre_core::Seq::ZERO,
                row_start: Pos::<Row>::new(0),
                row_end: Pos::<Row>::new(30),
                col_start: Pos::<Col>::new(0),
                col_end: Pos::<Col>::new(5),
            }),
        });
        assert!(state.has_pending_work(), "a scroll lets the export run");
        assert!(trame_files(&ui_rx).is_empty());
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: set_cell("edited"),
        });
        let printing = match &state.docs.get(&doc_id).expect("a live document").state {
            DocState::Trame(trame) => trame.is_printing(),
            other => panic!("the registry holds {other:?}"),
        };
        assert!(!printing, "the edit landed the export");
        assert!(!state.offload.is_idle(), "the write left first");
        thread.serve();
        state.take_converted();
        assert_eq!(trame_files(&ui_rx).len(), 1);
    }

    /// Each family loads its own module and holds its own queue. A CSV export
    /// runs once the tables module lands. A deck export of the same session
    /// still waits on the PDF module.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn each_family_waits_on_its_own_module() {
        let (mut state, ui_rx, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        let deck = board_with_content(&mut state);
        let book = live_trame(&mut state, Role::Editor);
        state.handle_logic_command(LogicCommand::App {
            document_id: book,
            command: set_cell("42"),
        });
        sync_rx.try_iter().for_each(drop);
        ui_rx.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id: deck,
            command: AppCommand::Toile(ToileCommand::export(
                ambre_toile::message::ToileExport::Pdf,
            )),
        });
        state.handle_logic_command(LogicCommand::App {
            document_id: book,
            command: AppCommand::Trame(TrameCommand::export(
                ambre_trame::message::TrameExport::Csv {
                    sheet_id: None,
                    delimiter: None,
                    range: None,
                },
            )),
        });
        let asked: Vec<SyncCommand> = sync_rx.try_iter().collect();
        assert!(
            matches!(
                asked.as_slice(),
                [
                    SyncCommand::FetchConverter {
                        family: ambre_core::ConverterFamily::Pdf
                    },
                    SyncCommand::FetchConverter {
                        family: ambre_core::ConverterFamily::Tables
                    }
                ]
            ),
            "one fetch per family: {asked:?}"
        );
        load_converter(&mut state, ambre_core::ConverterFamily::Tables);
        thread.serve();
        state.take_converted();
        let answered: Vec<UiEvent> = ui_rx.try_iter().collect();
        assert!(answered.iter().any(|e| matches!(
            e,
            UiEvent::Viewport {
                event: AppEvent::Trame(ambre_trame::message::TrameEvent::Interop(
                    ambre_bus::interop::InteropAnswer::ExportReady { .. }
                )),
                ..
            }
        )));
        assert!(
            !answered.iter().any(|e| matches!(
                e,
                UiEvent::Viewport {
                    event: AppEvent::Toile(_),
                    ..
                }
            )),
            "the deck still waits on its own module"
        );
        // A refusal of one family answers that family alone.
        state.handle_logic_event(LogicEvent::ConverterRefused {
            family: ambre_core::ConverterFamily::Pdf,
            cause: ambre_core::ConverterRefusal::NotFetched,
        });
        assert_eq!(toile_answers(&ui_rx).len(), 1);
        assert!(
            sync_rx.try_recv().is_err(),
            "the tables module stays loaded"
        );
    }

    /// A paste of HTML another application wrote waits on the markup module,
    /// then lands formatted. A paste of Ambre's own bytes waits on nothing.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_html_paste_waits_on_the_markup_module_and_lands_formatted() {
        use ambre_prose::message::{ProseCommand, ProseEvent};
        let (mut state, ui, sync) = harness();
        let mut thread = converter_thread(&mut state);
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, DocType::Prose);
        let block = match &state.docs.get(&document_id).expect("a live document").state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        sync.try_iter().for_each(drop);
        ui.try_iter().for_each(drop);
        let paste = || LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ProseCommand::Paste {
                at: (block, Pos::<DocChar>::ZERO),
                over: None,
                text: "plain".to_owned(),
                html: Some("<h1>Title</h1>".to_owned()),
                payload: None,
            }),
        };
        state.handle_logic_command(paste());
        assert!(matches!(
            sync.try_recv(),
            Ok(SyncCommand::FetchConverter {
                family: ambre_core::ConverterFamily::Markup
            })
        ));
        let text_of = |state: &LogicState| match &state.docs.get(&document_id).map(|d| &d.state) {
            Some(DocState::Prose(prose)) => prose.text_of(&block),
            _ => None,
        };
        assert_eq!(text_of(&state).as_deref(), Some(""), "the paste waits");
        load_converter(&mut state, ambre_core::ConverterFamily::Markup);
        thread.serve();
        state.take_converted();
        assert_eq!(text_of(&state).as_deref(), Some("Title"));
        assert!(ui.try_iter().any(|e| matches!(
            e,
            UiEvent::Viewport {
                event: AppEvent::Prose(ProseEvent::DocUpdated { .. }),
                ..
            }
        )));
    }

    /// `ARCH-130`: the HTML half of a paste reads on the converter thread.
    /// The command returns with the read in flight and the document
    /// untouched, and the paste lands formatted once the thread served it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_html_paste_calls_its_module_on_the_converter_thread() {
        use ambre_prose::message::ProseCommand;
        let (mut state, _ui, sync) = harness();
        let mut thread = converter_thread(&mut state);
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, DocType::Prose);
        let block = match &state.docs.get(&document_id).expect("a live document").state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        load_converter(&mut state, ambre_core::ConverterFamily::Markup);
        sync.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ProseCommand::Paste {
                at: (block, Pos::<DocChar>::ZERO),
                over: None,
                text: "plain".to_owned(),
                html: Some("<h1>Title</h1>".to_owned()),
                payload: None,
            }),
        });
        let text_of = |state: &LogicState| match &state.docs.get(&document_id).map(|d| &d.state) {
            Some(DocState::Prose(prose)) => prose.text_of(&block),
            _ => None,
        };
        assert!(!state.offload.is_idle(), "the read left for the thread");
        assert_eq!(
            text_of(&state).as_deref(),
            Some(""),
            "the document is untouched while the read is in flight"
        );
        assert!(
            !sync
                .try_iter()
                .any(|c| matches!(c, SyncCommand::SendUpdate { .. })),
            "no delta leaves before the answer"
        );
        thread.serve();
        state.take_converted();
        assert_eq!(
            text_of(&state).as_deref(),
            Some("Title"),
            "the answer lands"
        );
    }

    /// A paste whose markup module could not load lands its plain text: the
    /// user loses the formatting and keeps the words.
    #[test]
    fn a_paste_whose_markup_module_fails_lands_its_plain_text() {
        use ambre_prose::message::ProseCommand;
        let (mut state, _ui, sync) = harness();
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, DocType::Prose);
        let block = match &state.docs.get(&document_id).expect("a live document").state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        sync.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ProseCommand::Paste {
                at: (block, Pos::<DocChar>::ZERO),
                over: None,
                text: "plain".to_owned(),
                html: Some("<h1>Title</h1>".to_owned()),
                payload: None,
            }),
        });
        state.handle_logic_event(LogicEvent::ConverterRefused {
            family: ambre_core::ConverterFamily::Markup,
            cause: ambre_core::ConverterRefusal::NotFetched,
        });
        let text = match &state.docs.get(&document_id).map(|d| &d.state) {
            Some(DocState::Prose(prose)) => prose.text_of(&block),
            _ => None,
        };
        assert_eq!(text.as_deref(), Some("plain"));
    }

    /// A WebP drop waits on the images module, which decodes the picture; a
    /// PNG drop is read in place and fetches nothing.
    // The lossless encoder that writes the fixture is a host dev-dependency.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_webp_picture_waits_on_the_images_module() {
        let (mut state, _ui_rx, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        let doc_id = board_with_content(&mut state);
        sync_rx.try_iter().for_each(drop);
        let picture = |format: image::ImageFormat, mime: &str| {
            let mut bytes = Vec::new();
            if format == image::ImageFormat::WebP {
                image_webp::WebPEncoder::new(&mut bytes)
                    .encode(&[1, 2, 3].repeat(9), 3, 3, image_webp::ColorType::Rgb8)
                    .expect("the lossless encoder writes a picture");
            } else {
                let img = image::RgbaImage::from_pixel(3, 3, image::Rgba([1, 2, 3, 255]));
                image::DynamicImage::ImageRgba8(img)
                    .write_to(&mut std::io::Cursor::new(&mut bytes), format)
                    .expect("the encoder writes a picture");
            }
            LogicCommand::App {
                document_id: doc_id,
                command: AppCommand::Toile(ToileCommand::import(
                    ambre_toile::message::ToileImportFormat::Image(ambre_core::MediaType::from(
                        mime,
                    )),
                    ambre_core::Payload::<ambre_client_core::FileBytes>::new(bytes),
                    kurbo::Point::ZERO,
                )),
            }
        };
        state.handle_logic_command(picture(image::ImageFormat::Png, "image/png"));
        assert!(
            !sync_rx
                .try_iter()
                .any(|c| matches!(c, SyncCommand::FetchConverter { .. })),
            "a PNG is read in place"
        );
        state.handle_logic_command(picture(image::ImageFormat::WebP, "image/webp"));
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::FetchConverter {
                family: ambre_core::ConverterFamily::Images
            })
        ));
        load_converter(&mut state, ambre_core::ConverterFamily::Images);
        // The decode runs on the converter thread (`ARCH-129`).
        thread.serve();
        state.take_converted();
        assert!(
            sync_rx.try_iter().any(|c| matches!(
                c,
                SyncCommand::Rest {
                    cmd: RestCommand::UploadMedia { .. },
                    ..
                }
            )),
            "the decoded picture uploads its blob"
        );
    }

    /// A JPEG of `width` by `height`, written by the encoder of the `image`
    /// crate at test time.
    #[cfg(not(target_arch = "wasm32"))]
    fn jpeg_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image::RgbImage::from_pixel(
                width,
                height,
                image::Rgb([1, 2, 3]),
            ))
            .expect("a JPEG encodes");
        bytes
    }

    /// A JPEG drop on `doc_id`, at the origin.
    #[cfg(not(target_arch = "wasm32"))]
    fn jpeg_drop(doc_id: Id<Document>) -> LogicCommand {
        LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::import(
                ambre_toile::message::ToileImportFormat::Image(ambre_core::MediaType::from(
                    "image/jpeg",
                )),
                ambre_core::Payload::<ambre_client_core::FileBytes>::new(jpeg_bytes(40, 20)),
                kurbo::Point::ZERO,
            )),
        }
    }

    /// A converter thread linked to `state`, served by hand.
    #[cfg(not(target_arch = "wasm32"))]
    fn converter_thread(state: &mut LogicState) -> crate::converter_thread::ConverterThread {
        let (link, ends) = crate::converter_thread::converter_channels(&Doorbell::new());
        state.offload = crate::converter_thread::Offload::over(link);
        crate::converter_thread::ConverterThread::new(ends).0
    }

    /// Whether `sync` received an upload.
    #[cfg(not(target_arch = "wasm32"))]
    fn uploads(sync_rx: &Receiver<SyncCommand>) -> bool {
        sync_rx.try_iter().any(|c| {
            matches!(
                c,
                SyncCommand::Rest {
                    cmd: RestCommand::UploadMedia { .. },
                    ..
                }
            )
        })
    }

    /// `ARCH-097`, `ARCH-098`: a JPEG drop asks for the images module once,
    /// and its decode runs on the converter thread. The Logic thread hands
    /// the job over and returns: nothing lands until the thread answers, and
    /// the next pass lands it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_jpeg_decodes_on_the_converter_thread_and_the_logic_thread_never_waits() {
        let (mut state, _ui_rx, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        let doc_id = board_with_content(&mut state);
        sync_rx.try_iter().for_each(drop);
        state.handle_logic_command(jpeg_drop(doc_id));
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::FetchConverter {
                family: ambre_core::ConverterFamily::Images
            })
        ));
        load_converter(&mut state, ambre_core::ConverterFamily::Images);
        state.take_converted();
        assert!(!uploads(&sync_rx), "the drop waits on the converter thread");
        // A second drop asks for no module: the thread holds it.
        state.handle_logic_command(jpeg_drop(doc_id));
        assert!(
            !sync_rx
                .try_iter()
                .any(|c| matches!(c, SyncCommand::FetchConverter { .. })),
            "the module is asked once"
        );
        thread.serve();
        assert!(
            state.offload.has_answers(),
            "the answers ring the Logic thread"
        );
        state.take_converted();
        let uploaded = sync_rx
            .try_iter()
            .filter(|c| {
                matches!(
                    c,
                    SyncCommand::Rest {
                        cmd: RestCommand::UploadMedia { .. },
                        ..
                    }
                )
            })
            .count();
        assert_eq!(uploaded, 2, "each picture lands once its thumbnail does");
    }

    /// The PNG export of `scope`, sent to `doc_id` as the File menu sends it.
    #[cfg(all(feature = "toile-pdf", not(target_arch = "wasm32")))]
    fn png_export(doc_id: Id<Document>, scope: ambre_toile::message::PngScope) -> LogicCommand {
        LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::export(
                ambre_toile::message::ToileExport::Png(scope),
            )),
        }
    }

    /// The PNG export records
    /// its page on this thread and hands the raster to the converter thread.
    /// The command returns with the job in flight and nothing answered. The
    /// Logic thread never waits for a pixel. The UI thread receives the file
    /// alone, once the thread answers. No delta leaves for a peer.
    #[cfg(all(feature = "toile-pdf", not(target_arch = "wasm32")))]
    #[test]
    fn a_png_export_rasterizes_on_the_converter_thread_and_the_logic_thread_never_waits() {
        let (mut state, ui_rx, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        state.handle_logic_command(LogicCommand::LoadFonts { faces: Vec::new() });
        let doc_id = board_with_content(&mut state);
        load_converter(&mut state, ambre_core::ConverterFamily::Pdf);
        ui_rx.try_iter().for_each(drop);
        sync_rx.try_iter().for_each(drop);
        state.handle_logic_command(png_export(doc_id, ambre_toile::message::PngScope::Board));
        state.take_converted();
        assert!(
            toile_answers(&ui_rx).is_empty(),
            "nothing answers before the converter thread runs the job"
        );
        thread.serve();
        assert!(
            state.offload.has_answers(),
            "the answer rings the Logic thread"
        );
        state.take_converted();
        match toile_answers(&ui_rx).as_slice() {
            [
                ambre_toile::message::ToileEvent::Interop(
                    ambre_bus::interop::InteropAnswer::ExportReady { file, .. },
                ),
            ] => {
                assert_eq!(file.filename.as_str(), "board.png");
                assert!(file.data.as_bytes().starts_with(b"\x89PNG"), "a PNG header");
            }
            other => panic!("expected the file alone, got {other:?}"),
        }
        assert!(
            sync_rx.try_recv().is_err(),
            "the export sent something to Sync"
        );
    }

    /// A PNG export with nothing to draw answers at once with
    /// the reason, and hands the converter thread no job.
    #[cfg(all(feature = "toile-pdf", not(target_arch = "wasm32")))]
    #[test]
    fn a_png_export_of_an_empty_board_answers_with_a_reason_and_no_job() {
        let (mut state, ui_rx, _sync_rx) = harness();
        let _thread = converter_thread(&mut state);
        state.handle_logic_command(LogicCommand::LoadFonts { faces: Vec::new() });
        let doc_id = live_toile(&mut state, Role::Owner);
        ui_rx.try_iter().for_each(drop);
        state.handle_logic_command(png_export(doc_id, ambre_toile::message::PngScope::Board));
        assert!(matches!(
            toile_answers(&ui_rx).as_slice(),
            [ambre_toile::message::ToileEvent::Interop(
                ambre_bus::interop::InteropAnswer::Failed(_)
            )]
        ));
        assert!(!state.offload.has_answers());
        assert!(state.offload.release().is_empty());
    }

    /// The Prose events the UI received, in order, draining the queue.
    #[cfg(not(target_arch = "wasm32"))]
    fn prose_answers(ui: &Receiver<UiEvent>) -> Vec<ambre_prose::message::ProseEvent> {
        ui.try_iter()
            .filter_map(|e| match e {
                UiEvent::Viewport {
                    event: AppEvent::Prose(event),
                    ..
                } => Some(event),
                _ => None,
            })
            .collect()
    }

    /// `ARCH-122`: a Prose import and a Markdown export each call their
    /// module on the converter thread. The command returns with the job in
    /// flight and the document untouched. The answer lands the import, then
    /// the file, each on the pass after the thread served it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_prose_import_and_export_call_their_module_on_the_converter_thread() {
        use ambre_bus::interop::{InteropAnswer, InteropRequest};
        use ambre_prose::message::{
            ExportFormat, ImportFormat, MarkdownFlavor, ProseCommand, ProseEvent, ProseImport,
            TimeZone,
        };
        let (mut state, ui, sync) = harness();
        let mut thread = converter_thread(&mut state);
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, DocType::Prose);
        load_converter(&mut state, ambre_core::ConverterFamily::Markup);
        let blocks = |state: &LogicState| match &state.docs[&document_id].state {
            DocState::Prose(prose) => prose
                .root_blocks()
                .iter()
                .filter_map(|block| prose.text_of(block))
                .collect::<Vec<_>>(),
            other => panic!("the registry opened {other:?}"),
        };
        let before = blocks(&state);
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ProseCommand::Interop(InteropRequest::Import(
                ProseImport {
                    format: ImportFormat::Markdown,
                    data: ambre_core::Payload::new(b"# Read off the Logic thread\n".to_vec()),
                    now: ambre_core::Instant::new(1_800_000_000_000),
                    zone: TimeZone::UTC,
                },
            ))),
        });
        state.answer_pass();
        assert_eq!(blocks(&state), before, "the read is in flight");
        assert!(!state.offload.is_idle(), "the read left for the thread");
        ui.try_iter().for_each(drop);
        thread.serve();
        state.take_converted();
        state.answer_pass();
        assert!(
            prose_answers(&ui).iter().any(|e| matches!(
                e,
                ProseEvent::Interop(InteropAnswer::ImportCompleted { .. })
            )),
            "the answer lands the import"
        );
        assert_eq!(blocks(&state), vec!["Read off the Logic thread".to_owned()]);
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Prose(ProseCommand::Interop(InteropRequest::Export(
                ExportFormat::Markdown(MarkdownFlavor::Gfm),
            ))),
        });
        state.answer_pass();
        assert!(
            !prose_answers(&ui)
                .iter()
                .any(|e| matches!(e, ProseEvent::Interop(InteropAnswer::ExportReady { .. }))),
            "the write is in flight"
        );
        thread.serve();
        state.take_converted();
        let file = prose_answers(&ui).into_iter().find_map(|e| match e {
            ProseEvent::Interop(InteropAnswer::ExportReady { file, .. }) => Some(file),
            _ => None,
        });
        let file = file.expect("the answer hands the file over");
        assert!(
            file.data
                .as_bytes()
                .starts_with(b"# Read off the Logic thread")
        );
        assert!(
            !sync
                .try_iter()
                .any(|c| matches!(c, SyncCommand::FetchConverter { .. })),
            "the thread was handed the module the Logic thread held"
        );
    }

    /// The Trame events the UI received, in order, draining the queue.
    #[cfg(not(target_arch = "wasm32"))]
    fn trame_answers(ui: &Receiver<UiEvent>) -> Vec<ambre_trame::message::TrameEvent> {
        ui.try_iter()
            .filter_map(|e| match e {
                UiEvent::Viewport {
                    event: AppEvent::Trame(event),
                    ..
                } => Some(event),
                _ => None,
            })
            .collect()
    }

    /// `ARCH-123`: a CSV import and an Excel export of a workbook each call
    /// their module on the converter thread. The command returns with the
    /// job in flight and the workbook untouched. The answer lands the
    /// import, then the file, each once the thread served it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_workbook_import_and_export_call_their_module_on_the_converter_thread() {
        use ambre_bus::interop::{InteropAnswer, InteropRequest};
        use ambre_trame::message::{TrameEvent, TrameExport};
        let (mut state, ui, sync) = harness();
        let mut thread = converter_thread(&mut state);
        let document_id = live_trame(&mut state, Role::Editor);
        load_converter(&mut state, ambre_core::ConverterFamily::Tables);
        load_converter(&mut state, ambre_core::ConverterFamily::Office);
        sync.try_iter().for_each(drop);
        ui.try_iter().for_each(drop);
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Trame(TrameCommand::import_csv(ambre_trame::message::CsvImport {
                data: ambre_core::Payload::new(b"Read off the Logic thread\n".to_vec()),
                delimiter: None,
                header: ambre_trame::message::HeaderRow::Absent,
                target_sheet_id: None,
                start_row: Pos::<Row>::new(0),
                start_col: Pos::<Col>::new(0),
            })),
        });
        state.answer_pass();
        assert!(!state.offload.is_idle(), "the read left for the thread");
        assert!(
            !sync
                .try_iter()
                .any(|c| matches!(c, SyncCommand::SendUpdate { .. })),
            "the workbook is untouched while the read is in flight"
        );
        assert!(
            !trame_answers(&ui).iter().any(|e| matches!(
                e,
                TrameEvent::Interop(InteropAnswer::ImportCompleted { .. })
            )),
            "nothing lands before the answer"
        );
        thread.serve();
        state.take_converted();
        state.answer_pass();
        assert!(
            trame_answers(&ui).iter().any(|e| matches!(
                e,
                TrameEvent::Interop(InteropAnswer::ImportCompleted { .. })
            )),
            "the answer lands the import"
        );
        assert!(
            sync.try_iter()
                .any(|c| matches!(c, SyncCommand::SendUpdate { .. })),
            "the import reaches the peers"
        );
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Trame(TrameCommand::Interop(InteropRequest::Export(
                TrameExport::Xlsx,
            ))),
        });
        state.answer_pass();
        assert!(
            !trame_answers(&ui)
                .iter()
                .any(|e| matches!(e, TrameEvent::Interop(InteropAnswer::ExportReady { .. }))),
            "the write is in flight"
        );
        thread.serve();
        state.take_converted();
        let file = trame_answers(&ui).into_iter().find_map(|e| match e {
            TrameEvent::Interop(InteropAnswer::ExportReady { file, .. }) => Some(file),
            _ => None,
        });
        let file = file.expect("the answer hands the file over");
        let read = ambre_xlsx::read(&ambre_core::Payload::new(file.data.as_bytes().to_vec()))
            .expect("the file reads back");
        assert!(
            read.workbook
                .sheets
                .iter()
                .any(|sheet| sheet.cells.iter().any(|cell| matches!(
                    &cell.content,
                    ambre_xlsx::model::Content::Value(ambre_xlsx::model::Value::Text(t))
                        if t == "Read off the Logic thread"
                ))),
            "the file holds the imported cell"
        );
        assert!(
            !sync
                .try_iter()
                .any(|c| matches!(c, SyncCommand::FetchConverter { .. })),
            "the thread was handed the modules the Logic thread held"
        );
    }

    /// `ARCH-100`: a converter thread that is gone fails each job as a module
    /// that did not load. Nothing is fatal: the board is told, and the UI
    /// hears no fatal event.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_converter_thread_gone_fails_each_job_and_nothing_is_fatal() {
        let (mut state, ui_rx, sync_rx) = harness();
        drop(converter_thread(&mut state));
        let doc_id = board_with_content(&mut state);
        load_converter(&mut state, ambre_core::ConverterFamily::Images);
        ui_rx.try_iter().for_each(drop);
        state.handle_logic_command(jpeg_drop(doc_id));
        state.take_converted();
        let told = toile_answers(&ui_rx);
        assert!(
            told.iter().any(|a| matches!(
                a,
                ambre_toile::message::ToileEvent::Interop(
                    ambre_bus::interop::InteropAnswer::Failed(_)
                )
            )),
            "the board is told: {told:?}"
        );
        assert!(!uploads(&sync_rx));
        assert!(
            !ui_rx
                .try_iter()
                .any(|e| matches!(e, UiEvent::FatalError { .. })),
            "a lost converter thread is no fatal error"
        );
    }

    /// `ARCH-100`: a thread lost while a job runs fails that job at the next
    /// pass, so no picture waits for ever.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_converter_thread_lost_mid_job_fails_it_at_the_next_pass() {
        let (mut state, ui_rx, _sync_rx) = harness();
        let thread = converter_thread(&mut state);
        let doc_id = board_with_content(&mut state);
        load_converter(&mut state, ambre_core::ConverterFamily::Images);
        ui_rx.try_iter().for_each(drop);
        state.handle_logic_command(jpeg_drop(doc_id));
        assert!(toile_answers(&ui_rx).is_empty(), "the job is in flight");
        drop(thread);
        state.take_converted();
        assert!(toile_answers(&ui_rx).iter().any(|a| matches!(
            a,
            ambre_toile::message::ToileEvent::Interop(ambre_bus::interop::InteropAnswer::Failed(_))
        )));
    }

    /// `ARCH-108`: a thread that never answers fails its job at the deadline,
    /// and the board is told. The next drop runs on the thread spawned after
    /// it, which takes the module again from the Logic thread, and lands.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_thread_past_its_deadline_fails_the_job_and_the_next_thread_lands_the_next() {
        let (mut state, ui_rx, sync_rx) = harness();
        let (link, _stuck) = crate::converter_thread::converter_channels(&Doorbell::new());
        let spawned = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = std::sync::Arc::clone(&spawned);
        let spawner = crate::converter_thread::ConverterSpawner::new(move || {
            let (link, ends) = crate::converter_thread::converter_channels(&Doorbell::new());
            kept.lock()
                .expect("the test holds the lock")
                .push(crate::converter_thread::ConverterThread::new(ends).0);
            Some(link)
        });
        state.offload = crate::converter_thread::Offload::over(link)
            .respawning(spawner)
            .with_deadline(ambre_core::TimeSpan::ZERO);
        let doc_id = board_with_content(&mut state);
        load_converter(&mut state, ambre_core::ConverterFamily::Images);
        ui_rx.try_iter().for_each(drop);
        sync_rx.try_iter().for_each(drop);
        state.handle_logic_command(jpeg_drop(doc_id));
        state.take_converted();
        assert!(toile_answers(&ui_rx).is_empty(), "the pass dates the job");
        state.take_converted();
        assert!(
            toile_answers(&ui_rx).iter().any(|a| matches!(
                a,
                ambre_toile::message::ToileEvent::Interop(
                    ambre_bus::interop::InteropAnswer::Failed(_)
                )
            )),
            "the job fails at its deadline"
        );
        state.handle_logic_command(jpeg_drop(doc_id));
        assert!(
            !sync_rx
                .try_iter()
                .any(|c| matches!(c, SyncCommand::FetchConverter { .. })),
            "the Logic thread holds the module, and asks Sync for none"
        );
        spawned
            .lock()
            .expect("the test holds the lock")
            .iter_mut()
            .for_each(crate::converter_thread::ConverterThread::serve);
        state.take_converted();
        assert!(uploads(&sync_rx), "the next drop lands on the next thread");
    }

    /// `ARCH-112`: a session that converts nothing spawns no converter
    /// thread. The first JPEG drop spawns it while the images module is
    /// still asked of the Sync worker, and the drop lands on it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_first_drop_spawns_the_converter_thread_while_its_module_is_asked() {
        let (mut state, ui_rx, sync_rx) = harness();
        let spawned = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = std::sync::Arc::clone(&spawned);
        let spawner = crate::converter_thread::ConverterSpawner::new(move || {
            let (link, ends) = crate::converter_thread::converter_channels(&Doorbell::new());
            kept.lock()
                .expect("the test holds the lock")
                .push(crate::converter_thread::ConverterThread::new(ends).0);
            Some(link)
        });
        state.offload = crate::converter_thread::Offload::started(
            crate::converter_thread::ConverterThreads::on_demand(spawner),
        );
        let doc_id = board_with_content(&mut state);
        state.take_converted();
        assert!(
            spawned.lock().expect("the test holds the lock").is_empty(),
            "an edit and a pass spawn no thread"
        );
        ui_rx.try_iter().for_each(drop);
        sync_rx.try_iter().for_each(drop);
        state.handle_logic_command(jpeg_drop(doc_id));
        assert!(
            sync_rx
                .try_iter()
                .any(|c| matches!(c, SyncCommand::FetchConverter { .. })),
            "the module is asked"
        );
        assert_eq!(
            spawned.lock().expect("the test holds the lock").len(),
            1,
            "the thread starts before its module arrives"
        );
        load_converter(&mut state, ambre_core::ConverterFamily::Images);
        spawned
            .lock()
            .expect("the test holds the lock")
            .iter_mut()
            .for_each(crate::converter_thread::ConverterThread::serve);
        state.take_converted();
        assert!(uploads(&sync_rx), "the drop lands on the first thread");
        assert_eq!(spawned.lock().expect("the test holds the lock").len(), 1);
    }

    /// `ARCH-081`: offline on a first use, the images module
    /// does not arrive. Each JPEG waiting on it fails with the reason, and the
    /// next drop asks for the module again.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_refused_images_module_fails_each_waiting_jpeg_and_the_next_asks_again() {
        let (mut state, ui_rx, sync_rx) = harness();
        let _thread = converter_thread(&mut state);
        let doc_id = board_with_content(&mut state);
        sync_rx.try_iter().for_each(drop);
        ui_rx.try_iter().for_each(drop);
        state.handle_logic_command(jpeg_drop(doc_id));
        state.handle_logic_command(jpeg_drop(doc_id));
        assert_eq!(
            sync_rx
                .try_iter()
                .filter(|c| matches!(c, SyncCommand::FetchConverter { .. }))
                .count(),
            1
        );
        state.handle_logic_event(LogicEvent::ConverterRefused {
            family: ambre_core::ConverterFamily::Images,
            cause: ambre_core::ConverterRefusal::NotFetched,
        });
        let failed = toile_answers(&ui_rx)
            .into_iter()
            .filter(|a| {
                matches!(
                    a,
                    ambre_toile::message::ToileEvent::Interop(
                        ambre_bus::interop::InteropAnswer::Failed(_)
                    )
                )
            })
            .count();
        assert_eq!(failed, 2, "each waiting drop is told");
        state.handle_logic_command(jpeg_drop(doc_id));
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::FetchConverter {
                family: ambre_core::ConverterFamily::Images
            })
        ));
    }

    /// An SVG drop waits on the images module, and the drawing lands once it
    /// does.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_svg_import_waits_on_the_images_module() {
        let (mut state, ui_rx, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        let doc_id = board_with_content(&mut state);
        sync_rx.try_iter().for_each(drop);
        ui_rx.try_iter().for_each(drop);
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="4" height="4"/></svg>"#;
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::import(
                ambre_toile::message::ToileImportFormat::Svg,
                ambre_core::Payload::<ambre_client_core::FileBytes>::new(svg.to_vec()),
                kurbo::Point::ZERO,
            )),
        });
        assert!(matches!(
            sync_rx.try_recv(),
            Ok(SyncCommand::FetchConverter {
                family: ambre_core::ConverterFamily::Images
            })
        ));
        assert!(toile_answers(&ui_rx).is_empty(), "the drop waits");
        load_converter(&mut state, ambre_core::ConverterFamily::Images);
        // The read runs on the converter thread (`ARCH-129`).
        thread.serve();
        state.take_converted();
        state.settle_import(doc_id);
        assert!(toile_answers(&ui_rx).iter().any(|a| matches!(
            a,
            ambre_toile::message::ToileEvent::Interop(
                ambre_bus::interop::InteropAnswer::ImportCompleted { .. }
            )
        )));
    }

    /// `ARCH-129`: an SVG import and a PowerPoint export of a board each call
    /// their module on the converter thread. The command returns with the job
    /// in flight and the board untouched. The answer lands the import, then
    /// the file, each once the thread served it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_board_import_and_export_call_their_module_on_the_converter_thread() {
        use ambre_bus::interop::InteropAnswer;
        use ambre_toile::message::ToileEvent;
        let (mut state, ui_rx, sync_rx) = harness();
        let mut thread = converter_thread(&mut state);
        let doc_id = live_toile(&mut state, Role::Editor);
        load_converter(&mut state, ambre_core::ConverterFamily::Images);
        load_converter(&mut state, ambre_core::ConverterFamily::Office);
        sync_rx.try_iter().for_each(drop);
        ui_rx.try_iter().for_each(drop);
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="4" height="4"/></svg>"#;
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::import(
                ambre_toile::message::ToileImportFormat::Svg,
                ambre_core::Payload::<ambre_client_core::FileBytes>::new(svg.to_vec()),
                kurbo::Point::ZERO,
            )),
        });
        assert!(!state.offload.is_idle(), "the read left for the thread");
        assert!(
            !sync_rx
                .try_iter()
                .any(|c| matches!(c, SyncCommand::SendUpdate { .. })),
            "the board is untouched while the read is in flight"
        );
        assert!(
            toile_answers(&ui_rx).is_empty(),
            "nothing lands before the answer"
        );
        thread.serve();
        state.take_converted();
        state.settle_import(doc_id);
        assert!(
            toile_answers(&ui_rx).iter().any(|a| matches!(
                a,
                ToileEvent::Interop(InteropAnswer::ImportCompleted { .. })
            )),
            "the answer lands the import"
        );
        let Some(DocState::Toile(board)) = state.docs.get(&doc_id).map(|d| &d.state) else {
            panic!("the board is open");
        };
        assert!(
            board.element_count() > ambre_core::Count::<ambre_core::Elements>::ZERO,
            "the answer writes the board"
        );
        state.handle_logic_command(LogicCommand::App {
            document_id: doc_id,
            command: AppCommand::Toile(ToileCommand::export(
                ambre_toile::message::ToileExport::Pptx,
            )),
        });
        assert!(
            !toile_answers(&ui_rx)
                .iter()
                .any(|a| matches!(a, ToileEvent::Interop(InteropAnswer::ExportReady { .. }))),
            "the write is in flight"
        );
        thread.serve();
        state.take_converted();
        let file = toile_answers(&ui_rx).into_iter().find_map(|a| match a {
            ToileEvent::Interop(InteropAnswer::ExportReady { file, .. }) => Some(file),
            _ => None,
        });
        let file = file.expect("the answer hands the file over");
        assert!(file.data.as_bytes().starts_with(b"PK"), "a zip package");
        assert!(
            !sync_rx
                .try_iter()
                .any(|c| matches!(c, SyncCommand::FetchConverter { .. })),
            "the thread was handed the modules the Logic thread held"
        );
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
        let failed = Id::<Document>::mint();
        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: failed,
            cause: ambre_bus::OpenFailure::Transient,
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
