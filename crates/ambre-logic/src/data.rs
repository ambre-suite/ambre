// ambre-logic/src/data.rs
// ============================================================================
// Application data plane: route typed app commands to a document's state, apply
// remote CRDT deltas / ephemeral payloads, and export snapshots for compaction.
// All methods borrow `LogicState` and emit through its outbound senders.
// ============================================================================

use super::*;
// The deck's PDF export is answered here rather than inside `ToileState`.
// This file therefore names the two Toile bus types the rest of the module
// reaches only through `AppCommand` / `AppEvent`. A command that needs the
// converter module waits in `converting` first.
use ambre_bus::Mutation;
use ambre_bus::interop::{
    Direction, ExportedFile, FileFormat, InteropAnswer, InteropError, InteropRequest,
};
use ambre_bus::sync_warning::SyncWarningCode;
use ambre_toile::message::{ToileCommand, ToileEvent, ToileExport, ToileReport};

use ambre_core::LoroPayloadRef;

impl LogicState {
    /// Start the export of a Toile deck to a PDF (`toile-pdf-export`,
    /// `TOILE-IO-029`). The board is read once, here, and the pages are then
    /// recorded one per pass (the `deck_export` module).
    ///
    /// Every refusal reaches the user as a shared failure. A build without the
    /// export compiled in and a session whose fonts have not landed answer by
    /// name. So do a board with nothing to draw and a serializer that gave up.
    pub(crate) fn export_toile_pdf(&mut self, document_id: Id<Document>) {
        self.settle_import(document_id);
        let Some(DocState::Toile(state)) = self.docs.get(&document_id).map(|d| &d.state) else {
            tracing::warn!("Logic: PDF export asked of a document that is not a board");
            return;
        };
        match record_deck_of(state, self.export_cx.as_ref()) {
            Ok(recording) => self.start_deck_export(document_id, recording),
            Err(cause) => {
                let answer = deck_file_answer(
                    Err(cause),
                    ToileReport::new(Direction::Export, FileFormat::Pdf),
                );
                self.dispatch_outcome(
                    document_id,
                    vec![AppEvent::Toile(answer)],
                    None,
                    None,
                    Vec::new(),
                );
            }
        }
    }

    /// Start the PNG export of a Toile board or frame (`toile-import-export`).
    /// The page is recorded now, in one step, and the converter thread
    /// rasterizes it: this thread never waits for the image (`ARCH-098`).
    /// A refusal answers at once, by name.
    pub(crate) fn export_toile_png(
        &mut self,
        document_id: Id<Document>,
        scope: &ambre_toile::message::PngScope,
    ) {
        self.settle_import(document_id);
        let Self {
            docs, export_cx, ..
        } = self;
        let Some(DocState::Toile(state)) = docs.get(&document_id).map(|d| &d.state) else {
            tracing::warn!("Logic: PNG export asked of a document that is not a board");
            return;
        };
        match record_png_of(state, scope, export_cx.as_mut()) {
            Ok(conversion) => self.offload(crate::converter_thread::Job {
                ticket: ambre_bus::next_request(),
                owner: crate::converter_thread::JobOwner::ToilePng(document_id),
                conversion,
            }),
            Err(cause) => {
                let answer = png_file_answer(Err(cause));
                self.dispatch_outcome(
                    document_id,
                    vec![AppEvent::Toile(answer)],
                    None,
                    None,
                    Vec::new(),
                );
            }
        }
    }

    /// Hand the converter thread's answer to a PNG export to its board, as
    /// the file or as the reason there is none. A board closed meanwhile
    /// drops it.
    pub(crate) fn answer_toile_png(
        &mut self,
        document_id: Id<Document>,
        answer: Result<ambre_convert::Converted, ambre_convert::JobFailure>,
    ) {
        if !self.docs.contains_key(&document_id) {
            return;
        }
        let file = match answer {
            Ok(ambre_convert::Converted::File(bytes)) => Ok(bytes),
            Ok(_) => Err(ambre_core::StopCause::ConverterAnswered),
            Err(ambre_convert::JobFailure::Unavailable) => {
                Err(ambre_core::StopCause::ConverterUnavailable)
            }
            Err(ambre_convert::JobFailure::Refused) => Err(ambre_core::StopCause::FileRefused(
                ambre_core::FileRefusal::Unwritable,
            )),
        };
        let answer = png_file_answer(file);
        self.dispatch_outcome(
            document_id,
            vec![AppEvent::Toile(answer)],
            None,
            None,
            Vec::new(),
        );
    }

    /// Write a document to a native `.ambre` file and answer with the bytes
    /// (`ambre-spec-docfile.md`). The second app command this thread serves
    /// itself, and for a reason of its own. The container's header carries the
    /// document's identity. An engine does not know its own; only this
    /// registry, which keyed it, does.
    ///
    /// App-agnostic by construction: one container serves every application
    /// (`DOCFILE-007`), so the only per-app value is the event that carries the
    /// bytes out, which [`DocState::ambre_file_event`] mints.
    fn export_ambre_file(&mut self, document_id: Id<Document>) {
        self.settle_import(document_id);
        let Some(answer) = self
            .docs
            .get(&document_id)
            .map(|d| d.state.ambre_file_event(document_id))
        else {
            tracing::warn!("Logic: .ambre export asked of a document that is not open");
            return;
        };
        self.dispatch_outcome(document_id, vec![answer], None, None, Vec::new());
    }

    /// Write a document to a native `.ambre` file **sealed under a passphrase**
    /// and answer with the sealed bytes (`ambre-spec-docfile.md section 5`).
    ///
    /// Three threads and no crypto here: this one writes the plaintext
    /// container exactly as [`export_ambre_file`](Self::export_ambre_file)
    /// does. The Sync worker seals it because cryptography runs nowhere else
    /// (`ARCH-004`), and the browser downloads what comes back. The file name
    /// never leaves this thread -- it waits in `pending_seals` with the id that
    /// will bring the bytes home.
    ///
    /// The passphrase travels as `Confidential<Passphrase>`:
    /// every hop that drops it wipes it, and it prints as nothing in a log
    /// line.
    fn export_ambre_file_sealed(
        &mut self,
        document_id: Id<Document>,
        passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) {
        self.settle_import(document_id);
        let Some(doc) = self.docs.get(&document_id) else {
            tracing::warn!("Logic: sealed .ambre export asked of a document that is not open");
            return;
        };
        let Some((filename, plaintext)) = doc.state.ambre_file_bytes(document_id) else {
            // Nothing to seal: a document the engine could not read becomes no
            // file, sealed or not (`TOILE-IO-069`).
            let answer = doc
                .state
                .export_failed_event(ambre_core::StopCause::DocumentUnreadable);
            self.dispatch_outcome(document_id, vec![answer], None, None, Vec::new());
            return;
        };
        let id = ambre_bus::next_request();
        self.pending_seals.insert(
            id,
            crate::PendingSeal {
                document_id,
                filename,
            },
        );
        self.send_to_sync(SyncCommand::Rest {
            id,
            cmd: RestCommand::SealDocFile {
                plaintext: plaintext.into_bytes().into(),
                passphrase,
            },
        });
    }

    /// Turn the Sync worker's answer to a pending seal back into the display
    /// event the owning application surfaces (`DOCFILE-014`). Returns whether
    /// `id` named a pending seal, so the caller can fall through to its other
    /// routes when it did not.
    ///
    /// A document closed while its seal was in flight drops the answer.
    /// Nothing remains to download into, and the bytes go out of scope sealed.
    pub(crate) fn resolve_pending_seal(&mut self, id: Seq<Request>, evt: &RestEvent) -> bool {
        let answer = match evt {
            RestEvent::DocFileSealed { data } => {
                Ok(data.clone().into_role::<ambre_client_core::FileBytes>())
            }
            RestEvent::DocFileSealFailed => Err(ambre_core::StopCause::SealFailed),
            _ => return false,
        };
        let Some(crate::PendingSeal {
            document_id,
            filename,
        }) = self.pending_seals.remove(&id)
        else {
            return false;
        };
        let Some(doc) = self.docs.get(&document_id) else {
            tracing::warn!("Logic: sealed .ambre landed for closed doc {document_id}, dropping");
            return true;
        };
        let event = match answer {
            Ok(data) => doc.state.export_complete_event(filename, data),
            Err(cause) => doc.state.export_failed_event(cause),
        };
        self.dispatch_outcome(document_id, vec![event], None, None, Vec::new());
        true
    }

    /// Drop every pending seal of `document_id` (it closed, or the session
    /// ended): an answer that arrives afterwards has nowhere to go.
    pub(crate) fn forget_pending_seals(&mut self, document_id: Option<Id<Document>>) {
        match document_id {
            Some(document_id) => self
                .pending_seals
                .retain(|_, seal| seal.document_id != document_id),
            None => self.pending_seals.clear(),
        }
    }

    /// Route a typed application command to its document's state, send display
    /// events to the UI Thread, and forward CRDT/ephemeral payloads to Sync.
    ///
    /// The registry is **flat**. A document embedded in a Toile via a `doc_embed`
    /// is an ordinary entry here, keyed by its own `Id<Document>`. Its commands
    /// arrive addressed straight to it. Nesting adds no routing step. The child's
    /// view holds the child's id and emits through the same queue, so a command
    /// never travels through the parent.
    pub(crate) fn handle_app_command(&mut self, document_id: Id<Document>, command: AppCommand) {
        // The store refuses an unsent change: an edit taken now could be lost
        // with it (`RESID-018`). Reading, selecting and scrolling still pass.
        if command.is_mutating() && self.edits_paused() {
            tracing::warn!("Logic: dropping an edit on doc {document_id}: the local store is full");
            return;
        }
        // A running sheet export lets a scroll and a selection through. Any
        // other command could change what a page shows, so the export lands
        // whole first (`ARCH-031`).
        if let AppCommand::Trame(cmd) = &command
            && !cmd.lets_print_run()
        {
            self.settle_trame_print(document_id);
        }
        let Some(doc) = self.docs.get_mut(&document_id) else {
            tracing::warn!("Logic: no live document {document_id} for app command");
            return;
        };

        // Role gate (architecture): a read-only caller never reaches the
        // engine with a mutating command, so we do not offer an edit the server
        // would reject. UX only -- the server re-checks every write and stays the
        // sole authority. Read-only and ephemeral commands still pass, so a
        // viewer keeps scrolling, selecting, copying and showing a cursor.
        if command.is_mutating() && !doc.role.can_write_document() {
            tracing::warn!(
                "Logic: dropping mutating command on doc {document_id}: role {:?} is not writable",
                doc.role,
            );
            return;
        }
        // No edit before the session identity (`AUTHOR-002`): it would be
        // written under a PeerID no delta of this session may carry
        // (`AUTHOR-008`). Sync issues the identity as it handles the open, and
        // the edit is applied under it then (`AUTHOR-026`, `AUTHOR-029`).
        if command.is_mutating() && doc.identity.is_none() {
            if ambre_core::Count::of(doc.held_edits.len()) < HELD_EDITS_LIMIT {
                doc.held_edits.push(command);
            } else {
                tracing::warn!(
                    "Logic: dropping mutating command on doc {document_id}: {HELD_EDITS_LIMIT} \
                     edits already wait for the session identity"
                );
            }
            return;
        }
        // A command that reads or writes a file runs through the converter
        // module of its family. This thread loads it on the first such command
        // (`ARCH-081`). The deck's PDF export also needs the base font
        // faces the Main thread fetched at boot, which live here
        // (`toile-pdf-export`).
        if let Some(family) = converting::needs_converter(&command) {
            self.convert_for(family, document_id, command);
            return;
        }
        // The PNG export records its page here, beside the base font faces,
        // and the converter thread rasterizes it (`toile-import-export`). Its job
        // asks for the module itself, so no command waits on it.
        if let AppCommand::Toile(ToileCommand::Interop(InteropRequest::Export(ToileExport::Png(
            scope,
        )))) = command
        {
            self.export_toile_png(document_id, &scope);
            return;
        }
        // The native file is the same operation whichever engine owns the
        // document, so the two variants are intercepted together rather than
        // implemented twice (`DOCFILE-007`).
        if matches!(
            command,
            AppCommand::Toile(ToileCommand::Interop(InteropRequest::Export(
                ToileExport::Ambre
            ))) | AppCommand::Trame(ambre_trame::message::TrameCommand::Interop(
                InteropRequest::Export(ambre_trame::message::TrameExport::Ambre)
            )) | AppCommand::Prose(ambre_prose::message::ProseCommand::Interop(
                InteropRequest::Export(ambre_prose::message::ExportFormat::Ambre)
            ))
        ) {
            self.export_ambre_file(document_id);
            return;
        }
        // The sealed file is the same operation once more, and intercepted in
        // the same place for the same reason (`DOCFILE-007`, `DOCFILE-014`).
        // Matched by value rather than by discriminant: this moves the
        // passphrase out of the command and sends it no further than Sync.
        let command = match command {
            AppCommand::Toile(ToileCommand::Interop(InteropRequest::Export(
                ToileExport::AmbreSealed { passphrase },
            )))
            | AppCommand::Trame(ambre_trame::message::TrameCommand::Interop(
                InteropRequest::Export(ambre_trame::message::TrameExport::AmbreSealed {
                    passphrase,
                }),
            )) => {
                self.export_ambre_file_sealed(document_id, passphrase);
                return;
            }
            other => other,
        };
        let doc = &mut doc.state;

        // Each arm produces the same triple: display events, an optional CRDT
        // `update`, an optional `ephemeral` payload. The fan-out to UI / Sync /
        // persist is therefore factored into `dispatch_outcome`. That keeps the
        // arms to the single differing line (which `state` and which `AppEvent`
        // wrap).
        match (doc, command) {
            (DocState::Trame(state), AppCommand::Trame(cmd)) => {
                let outcome = state.handle_command(cmd);
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Trame).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    // Trame issues no service-plane requests (no interop plane).
                    Vec::new(),
                );
            }
            (DocState::Toile(state), AppCommand::Toile(cmd)) => {
                let outcome = state.handle_command(cmd);
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Toile).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    outcome.requests,
                );
            }
            (DocState::Prose(state), AppCommand::Prose(cmd)) => {
                let outcome = state.handle_view_command(cmd);
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Prose).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    // Prose's own media requests leave with the pass's
                    // answers (`flush_prose_media`).
                    Vec::new(),
                );
                self.owe_layout(document_id);
            }
            // A mismatched (state, command) pair is impossible by routing. The
            // `document_id` selects a single `DocState`, and the UI only ever
            // sends the matching `AppCommand` variant for that document.
            (state, command) => tracing::warn!(
                "Logic: mismatched (state, command) pair for doc {document_id}: {:?} / {:?}",
                std::mem::discriminant(state),
                std::mem::discriminant(&command),
            ),
        }
    }

    /// The documents a conversion is still being applied to: a board, a
    /// workbook or a Prose document (`ARCH-030`).
    pub(crate) fn boards_with_pending_work(&self) -> Vec<Id<Document>> {
        self.docs
            .iter()
            .filter(|(_, doc)| state_has_pending_work(&doc.state))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Whether any board still has an import to apply, or a deck export to
    /// record. The loop then comes back without waiting for a message
    /// (`ARCH-030`, `toile-pdf-export`).
    pub(crate) fn has_pending_work(&self) -> bool {
        self.docs.values().any(|doc| {
            !doc.replay.is_empty()
                || doc.deck_export.as_deref().is_some_and(|recording| {
                    deck_export::stage(recording) == deck_export::Stage::Recording
                })
                || state_has_pending_work(&doc.state)
        })
    }

    /// Apply the next slice of a board's running import and fan it out
    /// (`ARCH-030`). A document with nothing running answers nothing.
    pub(crate) fn resume_import(&mut self, document_id: Id<Document>) {
        match self.docs.get_mut(&document_id).map(|d| &mut d.state) {
            Some(DocState::Toile(state)) if state.has_pending_work() => {
                let outcome = state.resume();
                self.dispatch_toile_outcome(document_id, outcome);
            }
            // A sheet's PDF export, one page per pass (`ARCH-030`).
            Some(DocState::Trame(state)) if state.is_printing() => {
                self.run_trame_print(document_id, ambre_trame::PrintPace::Slice);
            }
            // An Excel import, one slice per pass.
            Some(DocState::Trame(state)) if state.has_pending_work() => {
                let outcome = state.resume();
                self.dispatch_trame_outcome(document_id, outcome);
            }
            // A Prose import or export, one slice per pass.
            Some(DocState::Prose(_)) => self.run_prose_conversion(document_id, Pace::Slice),
            _ => {}
        }
    }

    /// Take `pace` of a Prose document's running conversion, through the
    /// module its export still needs, and fan it out.
    fn run_prose_conversion(&mut self, document_id: Id<Document>, pace: Pace) {
        let Self {
            docs, converters, ..
        } = self;
        let Some(DocState::Prose(state)) = docs.get_mut(&document_id).map(|d| &mut d.state) else {
            return;
        };
        if !state.has_pending_work() {
            return;
        }
        let converter = state.pending_family().and_then(|f| converters.ready(f));
        let outcome = match pace {
            Pace::Slice => state.resume_conversion(converter),
            Pace::Whole => state.settle_conversion(converter),
        };
        // A layout measures the whole document, so one per slice would cost
        // the import the square of its length. The view is owed one when the
        // conversion ends, and a page it asks for meanwhile is served.
        let ended = !state.has_pending_work();
        self.dispatch_outcome(
            document_id,
            outcome.display.into_iter().map(AppEvent::Prose).collect(),
            outcome.update,
            outcome.ephemeral,
            Vec::new(),
        );
        // The walk of a text export that ended owes the converter thread
        // its write.
        self.offload_prose_files(document_id);
        if ended {
            self.owe_layout(document_id);
        }
    }

    /// Land a board's running import whole, now (`ARCH-031`). Called
    /// before what reads the board here rather than in the engine: the exports
    /// this thread serves itself, a remote delta, and a close. A remote delta
    /// merged mid-import would also end the import's undo group in Loro.
    ///
    /// A history replay still being imported lands whole first too
    /// (`SYNC-131`).
    pub(crate) fn settle_import(&mut self, document_id: Id<Document>) {
        self.settle_replay(document_id);
        self.settle_board_import(document_id);
    }

    /// Land a board's running import whole, now: [`settle_import`] without
    /// the replay, for the import of one replay slice.
    ///
    /// [`settle_import`]: Self::settle_import
    fn settle_board_import(&mut self, document_id: Id<Document>) {
        self.settle_trame_print(document_id);
        match self.docs.get_mut(&document_id).map(|d| &mut d.state) {
            Some(DocState::Toile(state)) if state.has_pending_work() => {
                let outcome = state.settle();
                self.dispatch_toile_outcome(document_id, outcome);
            }
            Some(DocState::Trame(state)) if state.has_pending_work() => {
                let outcome = state.settle();
                self.dispatch_trame_outcome(document_id, outcome);
            }
            Some(DocState::Prose(_)) => self.run_prose_conversion(document_id, Pace::Whole),
            _ => {}
        }
    }

    /// Start a sheet's PDF export (`ARCH-030`). The pages are cut now,
    /// and each later pass of the loop draws one. A running import or an
    /// earlier export lands whole first. A refusal answers at once, by name.
    pub(crate) fn start_trame_print(
        &mut self,
        document_id: Id<Document>,
        options: ambre_trame::message::PdfExport,
    ) {
        self.settle_import(document_id);
        let fonts_ready = self.export_cx.is_some();
        let Some(DocState::Trame(state)) = self.docs.get_mut(&document_id).map(|d| &mut d.state)
        else {
            tracing::warn!("Logic: sheet PDF export asked of a document that is not a workbook");
            return;
        };
        // The faces are fetched once at boot, so a missing context is a race
        // with startup: the next attempt succeeds.
        let started = if fonts_ready {
            state.print_start(options)
        } else {
            Err(ambre_core::StopCause::FontsLoading)
        };
        if let Err(cause) = started {
            let answer = ambre_trame::TrameEvent::Interop(InteropAnswer::failed(
                Direction::Export,
                InteropError::Stopped(cause),
            ));
            self.dispatch_outcome(
                document_id,
                vec![AppEvent::Trame(answer)],
                None,
                None,
                Vec::new(),
            );
        }
    }

    /// Walk `pace` of a sheet's running PDF export, through the export
    /// context and the PDF module, and fan out what it answers. The export
    /// writes nothing, so no slice carries a delta (`PDF-002`).
    pub(crate) fn run_trame_print(
        &mut self,
        document_id: Id<Document>,
        pace: ambre_trame::PrintPace,
    ) {
        let Self {
            docs,
            export_cx,
            converters,
            ..
        } = self;
        let Some(DocState::Trame(state)) = docs.get_mut(&document_id).map(|d| &mut d.state) else {
            return;
        };
        if !state.is_printing() {
            return;
        }
        let outcome = match (
            export_cx.as_mut(),
            converters.ready(ambre_core::ConverterFamily::Pdf),
        ) {
            (Some(font_cx), Some(converter)) => state.print_slice(font_cx, converter, pace),
            (None, _) => state.print_stop(ambre_core::StopCause::FontsLoading),
            (_, None) => state.print_stop(ambre_core::StopCause::ConverterRefused(
                ambre_core::ConverterRefusal::NotLoaded,
            )),
        };
        self.dispatch_trame_outcome(document_id, outcome);
    }

    /// Land a sheet's running PDF export whole, now: before a command that
    /// could change what a page shows, a remote delta, or a close.
    pub(crate) fn settle_trame_print(&mut self, document_id: Id<Document>) {
        self.run_trame_print(document_id, ambre_trame::PrintPace::Whole);
    }

    fn dispatch_trame_outcome(
        &mut self,
        document_id: Id<Document>,
        outcome: ambre_trame::TrameOutcome,
    ) {
        self.dispatch_outcome(
            document_id,
            outcome.display.into_iter().map(AppEvent::Trame).collect(),
            outcome.update,
            outcome.ephemeral,
            Vec::new(),
        );
    }

    fn dispatch_toile_outcome(
        &mut self,
        document_id: Id<Document>,
        outcome: ambre_toile::ToileOutcome,
    ) {
        self.dispatch_outcome(
            document_id,
            outcome.display.into_iter().map(AppEvent::Toile).collect(),
            outcome.update,
            outcome.ephemeral,
            outcome.requests,
        );
    }

    /// Fan an already-typed outcome out to the UI Thread (display events), to
    /// Sync (`SendUpdate` / `SendEphemeral`), and to local persistence, which
    /// logs the same delta. Shared by every `AppCommand` arm; the arm only
    /// decides which `AppEvent` variant wraps the display events.
    pub(crate) fn dispatch_outcome(
        &mut self,
        document_id: Id<Document>,
        display: Vec<AppEvent>,
        update: Option<ambre_core::Payload<ambre_core::DeltaBytes>>,
        ephemeral: Option<ambre_core::Payload<ambre_core::EphemeralBytes>>,
        requests: Vec<ToileRequest>,
    ) {
        for event in display {
            self.send_to_ui(UiEvent::Viewport { document_id, event });
        }
        if let Some(payload) = update {
            tracing::debug!(
                "[Logic] owing SendUpdate doc={document_id} delta_size={}",
                payload.len().get(),
            );
            // The delta that leaves for the relay is the one saved: it holds
            // every operation the command wrote. It is saved in its own pass,
            // and leaves through the send window (`SYNC-192`, `SYNC-196`).
            self.owe_delta(document_id, payload.clone());
            self.owe_send(document_id, payload);
        }
        if let Some(payload) = ephemeral {
            self.send_to_sync(SyncCommand::SendEphemeral {
                document_id,
                payload,
            });
        }
        // Lower each Toile-level service request to a `RestCommand`, supplying the
        // owning `document_id` (which the engine does not hold). Record the route
        // as well, so the answering `RestEvent` / `ApiError` is fed back into the
        // engine instead of forwarded to the UI (`toile-media`).
        for request in requests {
            // A JPEG thumbnail leaves for the converter thread, never the
            // relay (`ARCH-097`).
            if let ToileRequest::DecodePicture { id, request } = request {
                self.offload(crate::converter_thread::Job {
                    ticket: id,
                    owner: crate::converter_thread::JobOwner::Toile(document_id),
                    conversion: ambre_convert::Conversion::DecodeJpeg(request),
                });
                continue;
            }
            let Some((id, cmd)) = self.lower_request(document_id, request) else {
                continue;
            };
            self.rest_routes.insert(id, document_id);
            self.send_to_sync(SyncCommand::Rest { id, cmd });
        }
    }

    /// Lower one [`ToileRequest`] into the `(Seq<Request>, RestCommand)` pair the
    /// Sync plane expects, filling in the `document_id` the engine cannot name.
    /// The `Seq<Request>` is the one Toile allocated -- it owns the correlation.
    /// A request the converter thread serves lowers to nothing here.
    fn lower_request(
        &self,
        document_id: Id<Document>,
        request: ToileRequest,
    ) -> Option<(Seq<Request>, RestCommand)> {
        match request {
            ToileRequest::UploadMedia { id, data } => {
                Some((id, RestCommand::UploadMedia { document_id, data }))
            }
            ToileRequest::DownloadMedia { id, content_hash } => Some((
                id,
                RestCommand::DownloadMedia {
                    document_id,
                    content_hash,
                },
            )),
            ToileRequest::DecodePicture { .. } => None,
        }
    }

    /// Feed a service-plane response routed back to a document
    /// (`toile-media`) into its engine and fan the resulting outcome out.
    /// Only a Toile document issues service requests, so a route pointing at any
    /// other state is a bug; we log and drop rather than panic. The engine
    /// silently ignores an id it no longer tracks (import undone / doc reopened).
    pub(crate) fn route_service_event(
        &mut self,
        document_id: Id<Document>,
        id: Seq<Request>,
        evt: RestEvent,
    ) {
        // A stored file a deck export asked for (`TOILE-IO-100`). Any answer
        // other than the opened bytes leaves the picture to its thumbnail.
        if self.deck_fetch_owed(document_id, id).is_some() {
            let data = match evt {
                RestEvent::MediaDownloaded { data, .. } => Some(data),
                _ => None,
            };
            self.answer_deck_fetch(document_id, id, data);
            return;
        }
        let Some(doc) = self.docs.get_mut(&document_id) else {
            tracing::warn!("Logic: routed RestEvent for closed doc {document_id}, dropping");
            return;
        };
        // A picture's bytes or address, answered to the Prose document that
        // asked (`features/ambre-spec-prose-images.md section 3`).
        if let DocState::Prose(state) = &mut doc.state {
            if let Some(outcome) = state.handle_media_event(id, evt) {
                self.dispatch_prose_media(document_id, outcome);
            }
            return;
        }
        let DocState::Toile(state) = &mut doc.state else {
            tracing::warn!("Logic: routed RestEvent for non-Toile doc {document_id}, dropping");
            return;
        };
        let outcome = state.handle_service_event(id, evt);
        self.dispatch_outcome(
            document_id,
            outcome.display.into_iter().map(AppEvent::Toile).collect(),
            outcome.update,
            outcome.ephemeral,
            outcome.requests,
        );
    }

    /// Fan out what a Prose document answered to its media flow. A picture
    /// chosen from a file writes its block here, so the view is owed a
    /// layout like after any command.
    pub(crate) fn dispatch_prose_media(
        &mut self,
        document_id: Id<Document>,
        outcome: ambre_prose::ProseOutcome,
    ) {
        self.dispatch_outcome(
            document_id,
            outcome.display.into_iter().map(AppEvent::Prose).collect(),
            outcome.update,
            outcome.ephemeral,
            Vec::new(),
        );
        self.owe_layout(document_id);
    }

    /// Lower and send the media requests a Prose document owes the Sync
    /// worker, each routed back to it by its id. Prose makes no network call
    /// and names no key: the document id is added here.
    pub(crate) fn flush_prose_media(&mut self, document_id: Id<Document>) {
        let Some(DocState::Prose(state)) = self.docs.get_mut(&document_id).map(|d| &mut d.state)
        else {
            return;
        };
        for request in state.take_media_requests() {
            let (id, cmd) = match request {
                ambre_prose::message::MediaRequest::Upload { id, data } => {
                    (id, RestCommand::UploadMedia { document_id, data })
                }
                ambre_prose::message::MediaRequest::Download { id, content_hash } => (
                    id,
                    RestCommand::DownloadMedia {
                        document_id,
                        content_hash,
                    },
                ),
            };
            self.rest_routes.insert(id, document_id);
            self.send_to_sync(SyncCommand::Rest { id, cmd });
        }
        // The picture decodes leave for the converter thread, and the Logic
        // thread never waits for them (`ARCH-098`).
        let Some(DocState::Prose(state)) = self.docs.get_mut(&document_id).map(|d| &mut d.state)
        else {
            return;
        };
        for (ticket, conversion) in state.take_decode_requests() {
            self.offload(crate::converter_thread::Job {
                ticket,
                owner: crate::converter_thread::JobOwner::Prose(document_id),
                conversion,
            });
        }
        self.offload_prose_files(document_id);
    }

    /// Hand the converter thread the file reads and writes a Trame workbook
    /// owes, and never wait for them (`ARCH-123`, `ARCH-098`).
    pub(crate) fn offload_trame_files(&mut self, document_id: Id<Document>) {
        let Some(DocState::Trame(state)) = self.docs.get_mut(&document_id).map(|d| &mut d.state)
        else {
            return;
        };
        for (ticket, conversion) in state.take_file_requests() {
            self.offload(crate::converter_thread::Job {
                ticket,
                owner: crate::converter_thread::JobOwner::TrameFile(document_id),
                conversion,
            });
        }
    }

    /// Hand the converter thread the file reads and writes a Prose document
    /// owes, and never wait for them (`ARCH-122`, `ARCH-098`).
    pub(crate) fn offload_prose_files(&mut self, document_id: Id<Document>) {
        let Some(DocState::Prose(state)) = self.docs.get_mut(&document_id).map(|d| &mut d.state)
        else {
            return;
        };
        for (ticket, conversion) in state.take_file_requests() {
            self.offload(crate::converter_thread::Job {
                ticket,
                owner: crate::converter_thread::JobOwner::ProseFile(document_id),
                conversion,
            });
        }
    }

    /// Feed a correlated service-plane failure back into a document's engine
    /// (`toile-media`): a failed media upload aborts its pending import.
    pub(crate) fn route_service_error(
        &mut self,
        document_id: Id<Document>,
        id: Seq<Request>,
        failure: ambre_bus::ApiFailure,
    ) {
        // A stored file the Sync worker could not fetch or open: a blob the
        // relay altered, or one no key of the chain opens. The picture goes
        // out from its thumbnail, counted in the report (`TOILE-IO-100`).
        if self.deck_fetch_owed(document_id, id).is_some() {
            tracing::debug!("Logic: a deck export's stored file failed: {failure}");
            self.answer_deck_fetch(document_id, id, None);
            return;
        }
        let Some(doc) = self.docs.get_mut(&document_id) else {
            tracing::warn!("Logic: routed ApiError for closed doc {document_id}, dropping");
            return;
        };
        // A picture the Sync worker did not store or serve: the reason stays
        // here, and the document holds the failure by its kind alone.
        if let DocState::Prose(state) = &mut doc.state {
            tracing::debug!("Logic: a Prose picture request failed: {failure}");
            if let Some(outcome) = state.handle_media_error(id) {
                self.dispatch_prose_media(document_id, outcome);
            }
            return;
        }
        let DocState::Toile(state) = &mut doc.state else {
            tracing::warn!("Logic: routed ApiError for non-Toile doc {document_id}, dropping");
            return;
        };
        let outcome = state.handle_service_error(id, failure);
        self.dispatch_outcome(
            document_id,
            outcome.display.into_iter().map(AppEvent::Toile).collect(),
            outcome.update,
            outcome.ephemeral,
            outcome.requests,
        );
    }

    /// Import a locally persisted snapshot and the deltas logged after it, and
    /// forward the resulting viewport events to the UI. Local-only: never
    /// forwarded to Sync, and never checked for authorship, since this client
    /// wrote or accepted every byte of it.
    ///
    /// A [`Restore::Shown`] restore is never saved back: the session identity
    /// replaces the state it built (`AUTHOR-027`).
    ///
    /// Returns `false` when the snapshot was refused, so a reopen fetches the
    /// document from the relay instead (`RESID-012`).
    pub(crate) fn restore_local(
        &mut self,
        document_id: Id<Document>,
        snapshot: Option<ambre_core::Payload<ambre_core::SnapshotBytes>>,
        deltas: Vec<ambre_core::Payload<ambre_core::DeltaBytes>>,
        restore: Restore,
    ) -> bool {
        let Some(doc) = self.docs.get_mut(&document_id).map(|d| &mut d.state) else {
            return false;
        };
        let mut opened = true;
        // A snapshot import is the same operation whichever engine owns the
        // document, and it is a full-state read. The schema gate runs inside
        // `DocState::import_snapshot` before any container is used
        // (`SCHEMA-006`). A refused document leaves the state alone
        // (`SCHEMA-012`) and reaches the user as the owning app's own refusal
        // event. A restore that silently does nothing is indistinguishable from
        // a document that never existed.
        let mut events: Vec<AppEvent> = match snapshot.map(|s| doc.import_snapshot(&s)) {
            Some(Ok(events)) => events,
            Some(Err(e)) => {
                tracing::warn!("Logic: doc {document_id} refused its snapshot: {e}");
                opened = false;
                vec![doc.refusal_event(&e)]
            }
            None => Vec::new(),
        };
        // The deltas logged since the snapshot, imported the way a relay's
        // replay is. The log is folded into a snapshot at once, so the next
        // restore reads one row.
        if !deltas.is_empty() {
            events.extend(doc.apply_remote_batch(&deltas));
            if restore == Restore::Kept {
                self.owe_snapshot(document_id);
            }
        }
        for event in events {
            self.send_to_ui(UiEvent::Viewport { document_id, event });
        }
        self.owe_layout(document_id);
        opened
    }

    /// Hold the reader's own locale and convention, and hand them to every
    /// open document, which repaints under them (`LOCALE-015`). Nothing is
    /// sent to Sync (`LOCALE-017`).
    pub(crate) fn set_reader_locale(&mut self, reader: ambre_client_core::ReaderLocale) {
        self.reader_locale = reader;
        let open: Vec<Id<Document>> = self
            .docs
            .iter()
            .filter(|(_, doc)| matches!(doc.state, DocState::Trame(_)))
            .map(|(id, _)| *id)
            .collect();
        for document_id in open {
            let command = AppCommand::Trame(ambre_trame::TrameCommand::SetReaderLocale {
                reader: self.reader_locale.clone(),
            });
            self.handle_app_command(document_id, command);
        }
    }

    /// Write the creator's locale into a document this reader created, once,
    /// as its first edit (`LOCALE-019`). The seed is no resolution: every
    /// later read takes the register (`LOCALE-012`).
    pub(crate) fn seed_locale(&mut self, document_id: Id<Document>) {
        let Some(locale) = self.locale_seeds.remove(&document_id) else {
            return;
        };
        let Some(doc) = self.docs.get(&document_id) else {
            return;
        };
        let command = doc.state.locale_command(locale);
        self.handle_app_command(document_id, command);
    }

    /// A new state for a document of `doc_type`, seeded with what the session
    /// already knows: the user, the display name and the faces.
    pub(crate) fn fresh_state(&self, doc_type: DocType) -> Option<DocState> {
        let mut state = DocState::new(doc_type)?;
        // Seed the identity if the session is already authenticated; otherwise
        // the later `SessionReady` sweep covers this state.
        if let Some(user_id) = self.my_user_id {
            state.set_user_id(user_id);
        }
        if let Some(name) = &self.my_display_name {
            state.set_display_name(name);
        }
        // The faces that already arrived, so a document opened after boot
        // measures on the same set as one opened before it. A sealed measuring
        // context holds nothing until this runs.
        state.load_faces(self.faces.faces());
        // The reader's convention, so the first viewport reads under it.
        state.set_reader_locale(&self.reader_locale);
        Some(state)
    }

    /// Write the document under the session identity Sync issued
    /// (`AUTHOR-002`), then apply the local restores that waited for it.
    ///
    /// A restore shown before the identity may have written under the
    /// provisional PeerID, and such an operation never travels (`AUTHOR-008`).
    /// The document is then rebuilt from a fresh state, and the restores run
    /// again under the identity (`AUTHOR-027`). Sync issues the identity before
    /// it fetches anything, so the rebuild drops nothing from the relay.
    ///
    /// The PeerID and its nonce are saved before any operation written under
    /// them, so a later open can send what this session leaves unsent
    /// (`AUTHOR-028`).
    ///
    /// The same identity issued twice is nothing new: the document reopened
    /// in the same session, and its unsent cursor stays where it is.
    pub(crate) fn adopt_session(
        &mut self,
        document_id: Id<Document>,
        peer: ambre_core::PeerId,
        entry: ambre_core::PeerEntry,
        nonce: FixedBytes<SessionNonce>,
    ) {
        let Some(doc) = self.docs.get(&document_id) else {
            // A channel or a space document is no entry of `docs`.
            return self.murmure_session(document_id, peer, entry);
        };
        if doc.identity == Some(peer) {
            return;
        }
        let rebuilt = if doc.held_restores.is_empty() {
            None
        } else {
            self.fresh_state(doc.state.doc_type())
        };
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        if let Some(state) = rebuilt {
            doc.state = state;
        }
        if !doc.state.adopt_session(peer, entry) {
            return;
        }
        doc.identity = Some(peer);
        let held = std::mem::take(&mut doc.held_restores);
        let edits = std::mem::take(&mut doc.held_edits);
        self.send_to_persist(document_id, PersistWrite::Session { peer, nonce });
        for restore in held {
            self.restore_local(document_id, restore.snapshot, restore.deltas, Restore::Kept);
        }
        // The edits made before the identity, in the order they were made, on
        // top of the restored document (`AUTHOR-026`).
        for command in edits {
            self.handle_app_command(document_id, command);
        }
        self.send_unsent(document_id);
    }

    /// Take in the earlier sessions the local store kept for a document, and
    /// send what they left unsent once the relay's replay is done.
    pub(crate) fn note_earlier_sessions(
        &mut self,
        document_id: Id<Document>,
        sessions: Vec<(ambre_core::PeerId, FixedBytes<SessionNonce>)>,
    ) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        for (peer, nonce) in sessions {
            if !doc.earlier.iter().any(|(p, _)| *p == peer) {
                doc.earlier.push((peer, nonce));
            }
        }
        self.send_unsent(document_id);
    }

    /// Send, for each earlier session, the operations it wrote past what the
    /// relay is known to hold, sealed with that session's nonce
    /// (`AUTHOR-028`). A session the relay holds whole is forgotten.
    ///
    /// Runs once the relay's replay is done, since only then is its lower
    /// bound worth reading. An operation sent twice is harmless: an import
    /// skips what the document already holds.
    pub(crate) fn send_unsent(&mut self, document_id: Id<Document>) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        if !doc.ready || doc.identity.is_none() || !doc.held_restores.is_empty() {
            return;
        }
        let held = doc.state.crdt().oplog_vv();
        let mut sends = Vec::new();
        let mut forgotten = Vec::new();
        for (peer, nonce) in std::mem::take(&mut doc.earlier) {
            if Some(peer) == doc.identity {
                continue;
            }
            let end = held.get(&peer.get()).copied().unwrap_or(0);
            let start = doc.relay.get(&peer.get()).copied().unwrap_or(0);
            if end <= start {
                forgotten.push(peer);
                continue;
            }
            let span = loro::IdSpan::new(peer.get(), start, end);
            match doc
                .state
                .crdt()
                .export(loro::ExportMode::updates_in_range(vec![span]))
            {
                Ok(payload) => sends.push((payload, nonce)),
                Err(e) => tracing::warn!(
                    "Logic: doc {document_id} could not export an earlier session's edits: {e}"
                ),
            }
        }
        for (payload, nonce) in sends {
            self.send_to_sync(SyncCommand::SendUnsent {
                document_id,
                payload: payload.into(),
                nonce,
            });
        }
        for peer in forgotten {
            self.send_to_persist(document_id, PersistWrite::ForgetSession { peer });
        }
    }

    /// Apply a remote delta only when every operation it carries was written
    /// under the PeerID of its signer (`AUTHOR-012`). A refused delta changes
    /// nothing (`AUTHOR-014`). The log names the document and the signer and
    /// never a byte of the payload (`ZK-001`).
    pub(crate) fn apply_signed(&mut self, document_id: Id<Document>, delta: &SignedDelta) {
        // A channel document runs the engine's own check, which reads what
        // each operation writes (the Murmure engine spec).
        if !self.docs.contains_key(&document_id)
            && self.murmure_signed(document_id, std::slice::from_ref(delta))
        {
            return;
        }
        let accepted = authored_by(&delta.payload, delta.peer);
        self.record_seq(document_id, delta.seq, accepted.then_some(&delta.payload));
        if !accepted {
            tracing::warn!(
                "Logic: refused a delta on doc {document_id}: an operation is not its signer's \
                 (user {:?})",
                delta.user
            );
            return;
        }
        self.apply_remote(
            document_id,
            delta.seq,
            LoroPayloadRef::Delta(&delta.payload),
        );
        self.track_waiting(document_id, &[(delta.seq, &delta.payload)]);
    }

    /// Note what the deltas just applied wait on, and drop what the document
    /// now holds. A document left waiting for good is reported (`AUTHOR-025`).
    pub(crate) fn track_waiting(
        &mut self,
        document_id: Id<Document>,
        applied: &[(Seq<Relay>, &ambre_core::Payload<ambre_core::DeltaBytes>)],
    ) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        let crdt = doc.state.crdt();
        for (seq, payload) in applied {
            doc.waiting.note(*seq, payload, crdt);
        }
        doc.waiting.settle(crdt);
        self.report_stalled(document_id);
    }

    /// Tell the user, once, that a delta waits on an operation no delta
    /// carries (`AUTHOR-025`). Only after the replay: before it, the
    /// operation may still be on its way.
    pub(crate) fn report_stalled(&mut self, document_id: Id<Document>) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        if !doc.ready || !doc.waiting.stalled(doc.ledger.contiguous_end()) {
            return;
        }
        tracing::warn!(
            "Logic: doc {document_id} holds a delta waiting on an operation no delta carries"
        );
        self.send_to_ui(UiEvent::SyncWarning {
            document_id,
            code: SyncWarningCode::DocumentUnreadable,
        });
    }

    /// Note the relay's number of a delta, and what it carries when it was
    /// accepted. A snapshot check reads which deltas a horizon covers from
    /// it (`AUTHOR-016`).
    fn record_seq(
        &mut self,
        document_id: Id<Document>,
        seq: Seq<Relay>,
        payload: Option<&ambre_core::Payload<ambre_core::DeltaBytes>>,
    ) {
        if let Some(doc) = self.docs.get_mut(&document_id) {
            doc.ledger.record(seq, payload);
            if let Some(payload) = payload {
                doc.relay_saw(LoroPayloadRef::Delta(payload));
            }
        } else {
            self.murmure_seq(document_id, seq, payload);
        }
    }

    /// The batch form of [`apply_signed`](Self::apply_signed): each delta is
    /// checked on its own, and only the refused ones are left out.
    pub(crate) fn apply_signed_batch(
        &mut self,
        document_id: Id<Document>,
        deltas: Vec<SignedDelta>,
    ) {
        if !self.docs.contains_key(&document_id) && self.murmure_signed(document_id, &deltas) {
            return;
        }
        let accepted: Vec<(Seq<Relay>, ambre_core::Payload<ambre_core::DeltaBytes>)> = deltas
            .into_iter()
            .filter_map(|delta| {
                let accepted = authored_by(&delta.payload, delta.peer);
                self.record_seq(document_id, delta.seq, accepted.then_some(&delta.payload));
                if accepted {
                    Some((delta.seq, delta.payload))
                } else {
                    tracing::warn!(
                        "Logic: refused a replayed delta on doc {document_id}: an operation is \
                         not its signer's (user {:?})",
                        delta.user
                    );
                    None
                }
            })
            .collect();
        if accepted.is_empty() {
            return;
        }
        // A replay queues on an open document only. A space document takes
        // the whole batch now. A channel's never reaches here.
        if !self.docs.contains_key(&document_id) {
            let borrowed: Vec<LoroPayloadRef<'_>> = accepted
                .iter()
                .map(|(_, p)| LoroPayloadRef::Delta(p))
                .collect();
            if !self.murmure_replay(document_id, &borrowed) {
                tracing::warn!(
                    "Logic: replay for unopened doc {document_id} ({} delta(s)), dropping",
                    accepted.len()
                );
            }
            return;
        }
        // One slice now, the rest one a pass (`SYNC-131`).
        self.replay(document_id, accepted);
    }

    /// Apply a remote CRDT delta, or a snapshot whose contents are trusted,
    /// to a document and forward viewport events. `seq` is where the relay
    /// placed it: the update's number, or the last one the snapshot folds in.
    /// Only a channel reads it.
    pub(crate) fn apply_remote(
        &mut self,
        document_id: Id<Document>,
        seq: Seq<Relay>,
        payload: LoroPayloadRef<'_>,
    ) {
        self.settle_import(document_id);
        // A document no entry of `docs` names may belong to an open channel.
        if !self.docs.contains_key(&document_id) && self.murmure_remote(document_id, seq, payload) {
            return;
        }
        let Some(doc) = self.docs.get_mut(&document_id).map(|d| &mut d.state) else {
            tracing::warn!(
                "Logic: apply_remote for unopened doc {document_id} ({} bytes), dropping",
                payload.as_bytes().len()
            );
            return;
        };
        let (events, persist): (Vec<AppEvent>, bool) = match doc {
            DocState::Trame(state) => {
                let events = match payload {
                    LoroPayloadRef::Delta(delta) => state.apply_remote(delta),
                    LoroPayloadRef::Snapshot(snapshot) => state.apply_remote_snapshot(snapshot),
                };
                // Persist the merged state locally whenever the remote delta
                // actually changed the document (it emitted display events). A
                // refresh then restores it even for a client that only ever
                // receives, never edits. Otherwise such a client has no local
                // snapshot and depends entirely on a fresh server replay.
                let persist = !events.is_empty();
                (events.into_iter().map(AppEvent::Trame).collect(), persist)
            }
            DocState::Toile(state) => {
                let events = match payload {
                    LoroPayloadRef::Delta(delta) => state.apply_remote(delta),
                    LoroPayloadRef::Snapshot(snapshot) => state.apply_remote_snapshot(snapshot),
                };
                let persist = !events.is_empty();
                (events.into_iter().map(AppEvent::Toile).collect(), persist)
            }
            DocState::Prose(state) => {
                let events = match payload {
                    LoroPayloadRef::Delta(delta) => state.apply_remote(delta),
                    LoroPayloadRef::Snapshot(snapshot) => state.apply_remote_snapshot(snapshot),
                };
                let persist = !events.is_empty();
                (events.into_iter().map(AppEvent::Prose).collect(), persist)
            }
        };
        for event in events {
            self.send_to_ui(UiEvent::Viewport { document_id, event });
        }
        if persist {
            // A delta joins the local log. A snapshot holds the whole document,
            // so the store takes one of its own.
            match payload {
                LoroPayloadRef::Delta(delta) => self.owe_delta(document_id, delta.clone()),
                LoroPayloadRef::Snapshot(_) => self.owe_snapshot(document_id),
            }
            self.owe_layout(document_id);
        }
    }

    /// Apply a batch of remote deltas (one slice of a history replay) as one
    /// merge+reseed, so the UI repaints once instead of once per delta. Mirrors
    /// [`apply_remote`] for the local-persist and viewport-forward tail.
    ///
    /// It settles a board's import, never the replay it is a slice of. A
    /// channel or space document never queues a replay, so it never comes here.
    pub(crate) fn merge_batch(
        &mut self,
        document_id: Id<Document>,
        payloads: &[ambre_core::Payload<ambre_core::DeltaBytes>],
    ) {
        self.settle_board_import(document_id);
        let Some(doc) = self.docs.get_mut(&document_id).map(|d| &mut d.state) else {
            tracing::warn!(
                "Logic: merge_batch for unopened doc {document_id} ({} delta(s)), dropping",
                payloads.len()
            );
            return;
        };
        let (events, persist): (Vec<AppEvent>, bool) = match doc {
            DocState::Trame(state) => {
                let events = state.apply_remote_batch(payloads);
                let persist = !events.is_empty();
                (events.into_iter().map(AppEvent::Trame).collect(), persist)
            }
            DocState::Toile(state) => {
                let events = state.apply_remote_batch(payloads);
                let persist = !events.is_empty();
                (events.into_iter().map(AppEvent::Toile).collect(), persist)
            }
            DocState::Prose(state) => {
                let events = state.apply_remote_batch(payloads);
                let persist = !events.is_empty();
                (events.into_iter().map(AppEvent::Prose).collect(), persist)
            }
        };
        for event in events {
            self.send_to_ui(UiEvent::Viewport { document_id, event });
        }
        if persist {
            self.owe_snapshot(document_id);
            self.owe_layout(document_id);
        }
    }

    /// Apply a remote ephemeral payload to a document and forward presence events.
    /// `from_user_id` is the sender's canonical `Id<User>`, resolved from the
    /// authenticated `pub_key` in `ambre-sync` so presence cannot be spoofed.
    pub(crate) fn apply_ephemeral(
        &mut self,
        document_id: Id<Document>,
        from_user_id: Id<User>,
        payload: &ambre_core::Payload<ambre_core::EphemeralBytes>,
    ) {
        let Some(doc) = self.docs.get_mut(&document_id).map(|d| &mut d.state) else {
            return;
        };
        let events: Vec<AppEvent> = match doc {
            // Trame draws no collaborator yet, and keeps who has a session
            // open for its sweep.
            DocState::Trame(state) => {
                state.note_live(from_user_id);
                state
                    .apply_ephemeral(payload)
                    .into_iter()
                    .map(AppEvent::Trame)
                    .collect()
            }
            DocState::Toile(state) => state
                .apply_ephemeral(from_user_id, payload)
                .into_iter()
                .map(AppEvent::Toile)
                .collect(),
            DocState::Prose(state) => state
                .apply_ephemeral(from_user_id, payload)
                .into_iter()
                .map(AppEvent::Prose)
                .collect(),
        };
        for event in events {
            self.send_to_ui(UiEvent::Presence { document_id, event });
        }
        // A Prose or a Trame session answers each newcomer once, so a silent
        // one is known to it (`ambre-spec-prose-schema.md` section 2).
        let greeting = match self.docs.get_mut(&document_id).map(|d| &mut d.state) {
            Some(DocState::Prose(state)) => state.take_greeting(),
            Some(DocState::Trame(state)) => state.take_greeting(),
            _ => None,
        };
        if let Some(payload) = greeting {
            self.send_to_sync(SyncCommand::SendEphemeral {
                document_id,
                payload,
            });
        }
    }

    /// Tell the other Prose or Trame sessions on a document that this one is
    /// open, once it is ready (`ambre-spec-prose-schema.md` section 2).
    pub(crate) fn announce(&mut self, document_id: Id<Document>) {
        let frame = match self.docs.get_mut(&document_id).map(|d| &mut d.state) {
            Some(DocState::Prose(state)) => state.announce(),
            Some(DocState::Trame(state)) => Some(state.announce()),
            _ => None,
        };
        if let Some(payload) = frame {
            self.send_to_sync(SyncCommand::SendEphemeral {
                document_id,
                payload,
            });
        }
    }

    /// A peer stopped watching a document (`PRESENCE-014`): drop them from the
    /// document's live set and hand the view the set that remains. Ephemeral
    /// throughout -- nothing here touches the CRDT or the history.
    pub(crate) fn peer_left(&mut self, document_id: Id<Document>, user_id: Id<User>) {
        let Some(doc) = self.docs.get_mut(&document_id).map(|d| &mut d.state) else {
            return;
        };
        let events: Vec<AppEvent> = match doc {
            // Trame draws no peer yet, and forgets the user for its sweep.
            DocState::Trame(state) => {
                state.drop_live(user_id);
                Vec::new()
            }
            DocState::Toile(state) => state
                .peer_left(user_id)
                .into_iter()
                .map(AppEvent::Toile)
                .collect(),
            DocState::Prose(state) => state
                .drop_presence(user_id)
                .into_iter()
                .map(AppEvent::Prose)
                .collect(),
        };
        for event in events {
            self.send_to_ui(UiEvent::Presence { document_id, event });
        }
    }

    /// The snapshot for compaction, and the last delta it folds in: the last
    /// one held with no gap below it (`SRV-083`).
    pub(crate) fn export_covered_snapshot(
        &self,
        document_id: Id<Document>,
    ) -> Option<(ambre_core::Payload<ambre_core::SnapshotBytes>, Seq<Relay>)> {
        let doc = self.docs.get(&document_id)?;
        Some((doc.state.export_snapshot(), doc.ledger.contiguous_end()))
    }

    /// Remove for good what an ended session hid, as an edit of this
    /// session. That is the hidden Prose blocks (`ambre-spec-prose-schema.md`
    /// section 2), or the hidden Trame lines, one slice per pass (the Trame
    /// rows-and-columns spec, section 3). It waits for the session identity,
    /// since a write under the provisional PeerID never travels. A viewer
    /// writes nothing, so it sweeps nothing.
    pub(crate) fn sweep_hidden(&mut self, document_id: Id<Document>) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        if doc.identity.is_none() || !doc.role.can_write_document() {
            return;
        }
        let state = match &mut doc.state {
            DocState::Prose(state) => state,
            DocState::Trame(state) => {
                // The pass loop takes each slice (`ARCH-030`).
                state.start_sweep();
                return;
            }
            DocState::Toile(_) => return,
        };
        let outcome = state.sweep_hidden();
        let display = outcome.display.into_iter().map(AppEvent::Prose).collect();
        self.dispatch_outcome(document_id, display, outcome.update, None, Vec::new());
    }

    /// Import a snapshot whose contents are trusted, and count the deltas a
    /// later snapshot must hold from its horizon on (`AUTHOR-015`).
    pub(crate) fn adopt_snapshot(
        &mut self,
        document_id: Id<Document>,
        payload: &ambre_core::Payload<ambre_core::SnapshotBytes>,
        covered: Seq<Relay>,
    ) {
        self.apply_remote(document_id, covered, LoroPayloadRef::Snapshot(payload));
        if let Some(doc) = self.docs.get_mut(&document_id) {
            let base = doc.state.crdt().oplog_vv();
            doc.ledger.reset(covered, base);
            doc.relay_saw(LoroPayloadRef::Snapshot(payload));
        } else {
            self.murmure_snapshot_served(document_id, covered);
        }
        self.track_waiting(document_id, &[]);
    }

    /// Check a snapshot against the deltas this client holds, and answer the
    /// verdict to Sync (`AUTHOR-016`, `AUTHOR-017`, `AUTHOR-022`). The live
    /// document is only read (`AUTHOR-018`). A match on a pending snapshot
    /// folds the deltas it covers into the ledger's base.
    pub(crate) fn check_pending_snapshot(
        &mut self,
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        covered: Seq<Relay>,
        payload: &ambre_core::Payload<ambre_core::SnapshotBytes>,
        purpose: SnapshotPurpose,
    ) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            self.murmure_check_snapshot(document_id, snapshot_id, covered, payload, purpose);
            return;
        };
        let verdict = snapshot_check::check_snapshot(
            doc.state.crdt(),
            &doc.ledger,
            payload,
            covered,
            purpose,
        );
        if purpose == SnapshotPurpose::Confirm && matches!(verdict, SnapshotVerdict::Match { .. }) {
            doc.ledger.fold_through(covered);
        }
        self.send_to_sync(SyncCommand::SnapshotVerdict {
            document_id,
            snapshot_id,
            purpose,
            verdict,
        });
    }
}

/// The stem of the name a deck export carries. The board's title is a launcher
/// concern, so the file is named for what it holds, like the SVG export.
const PDF_STEM: &str = "deck";

/// The answer a deck export ends on: the file, or the reason there is none.
pub(crate) fn deck_file_answer(
    file: Result<ambre_core::Payload<ambre_client_core::FileBytes>, ambre_core::StopCause>,
    report: ToileReport,
) -> ToileEvent {
    match file {
        Ok(data) => ToileEvent::Interop(InteropAnswer::ExportReady {
            file: ExportedFile {
                filename: ambre_client_core::ExportFilename::named(
                    PDF_STEM,
                    ambre_client_core::ExportExtension::Pdf,
                ),
                media_type: ambre_core::MediaType::from("application/pdf"),
                data,
            },
            // The deck walk draws every element through the shared surface,
            // so a page holds what the slide holds. A picture drawn without
            // its stored file is counted (`TOILE-IO-100`).
            report,
        }),
        Err(cause) => ToileEvent::Interop(InteropAnswer::failed(
            Direction::Export,
            InteropError::Stopped(cause),
        )),
    }
}

/// The answer a PNG export ends on: the file, or the reason there is none.
pub(crate) fn png_file_answer(
    file: Result<ambre_core::Payload<ambre_client_core::FileBytes>, ambre_core::StopCause>,
) -> ToileEvent {
    match file {
        Ok(data) => ToileEvent::Interop(InteropAnswer::ExportReady {
            file: ExportedFile {
                // Named for what it holds, as the SVG export of the board is.
                filename: ambre_client_core::ExportFilename::named(
                    "board",
                    ambre_client_core::ExportExtension::Png,
                ),
                media_type: ambre_core::MediaType::from("image/png"),
                data,
            },
            // The page walk draws every element through the shared surface,
            // so the image holds what the board shows.
            report: ToileReport::new(Direction::Export, FileFormat::Image),
        }),
        Err(cause) => ToileEvent::Interop(InteropAnswer::failed(
            Direction::Export,
            InteropError::Stopped(cause),
        )),
    }
}

/// The PNG job of `scope`, its page recorded off the board now, or the
/// reason there is none.
#[cfg(feature = "toile-pdf")]
fn record_png_of(
    state: &ToileState,
    scope: &ambre_toile::message::PngScope,
    fonts: Option<&mut parley::FontContext>,
) -> Result<ambre_convert::Conversion, ambre_core::StopCause> {
    use ambre_toile::logic::interop::PngExportError;
    // A race with startup, as for the deck: the next attempt succeeds.
    let fonts = fonts.ok_or(ambre_core::StopCause::FontsLoading)?;
    let request = state.record_png(scope, fonts).map_err(|e| match e {
        PngExportError::Empty => ambre_core::StopCause::NothingToConvert,
        PngExportError::NoFrame => ambre_core::StopCause::SourceGone,
    })?;
    Ok(ambre_convert::Conversion::WritePng {
        document: request.document,
        size: request.size,
    })
}

#[cfg(not(feature = "toile-pdf"))]
fn record_png_of(
    _state: &ToileState,
    _scope: &ambre_toile::message::PngScope,
    _fonts: Option<&mut parley::FontContext>,
) -> Result<ambre_convert::Conversion, ambre_core::StopCause> {
    Err(ambre_core::StopCause::NotInBuild)
}

/// The recording of the deck, read off the board now, or the reason there is
/// none. Split out of the handler. The build with the export compiled in and
/// the build without differ in one function, not in the routing around it.
#[cfg(feature = "toile-pdf")]
fn record_deck_of(
    state: &ToileState,
    fonts: Option<&parley::FontContext>,
) -> Result<deck_export::Recording, ambre_core::StopCause> {
    if fonts.is_none() {
        // The faces are fetched once at boot and never re-requested, so this is
        // a race with startup, not a dead end: the next attempt succeeds.
        return Err(ambre_core::StopCause::FontsLoading);
    }
    state.record_deck(Default::default()).map_err(|e| {
        tracing::warn!("Logic: the PDF export stopped: {e}");
        pdf_stop_cause(e)
    })
}

#[cfg(not(feature = "toile-pdf"))]
fn record_deck_of(
    _state: &ToileState,
    _fonts: Option<&parley::FontContext>,
) -> Result<deck_export::Recording, ambre_core::StopCause> {
    Err(ambre_core::StopCause::NotInBuild)
}

/// The reason a refused deck export gives the user.
#[cfg(feature = "toile-pdf")]
pub(crate) fn pdf_stop_cause(
    error: ambre_toile::logic::interop::PdfExportError,
) -> ambre_core::StopCause {
    match error {
        ambre_toile::logic::interop::PdfExportError::Empty => {
            ambre_core::StopCause::NothingToConvert
        }
        ambre_toile::logic::interop::PdfExportError::Convert(e) => e.stop_cause(),
    }
}

/// How much of a Prose document's running conversion one call takes.
#[derive(Clone, Copy)]
enum Pace {
    /// One slice (`ARCH-030`).
    Slice,
    /// Every step left (`ARCH-031`).
    Whole,
}

/// Whether a document's engine holds an import still to apply, or a Prose
/// export still to walk (`ARCH-030`).
fn state_has_pending_work(state: &DocState) -> bool {
    match state {
        DocState::Toile(s) => s.has_pending_work(),
        DocState::Trame(s) => s.has_pending_work(),
        DocState::Prose(s) => s.has_pending_work(),
    }
}
