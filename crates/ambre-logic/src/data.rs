// ambre-logic/src/data.rs
// ============================================================================
// Application data plane: route typed app commands to a document's state, apply
// remote CRDT deltas / ephemeral payloads, and export snapshots for compaction.
// All methods borrow `LogicState` and emit through its outbound senders.
// ============================================================================

use super::*;
// The deck's PDF export is answered here rather than inside `ToileState`.
// This file therefore names the two Toile bus types the rest of the module
// reaches only through `AppCommand` / `AppEvent`.
use ambre_bus::SecretBytes;
use ambre_bus::app::toile::{ToileCommand, ToileEvent};

impl LogicState {
    /// Render a Toile deck to a PDF and answer with the bytes
    /// (`toile-pdf-export`, `TOILE-IO-029`). The board is read through a shared
    /// reference and the fonts through a mutable one. This borrows the two
    /// fields apart rather than going through `handle_app_command`'s single
    /// `get_mut`.
    ///
    /// Every refusal reaches the user as a `ExportFailed`. A build without the
    /// export compiled in and a session whose fonts have not landed answer by
    /// name. So do a board with nothing to draw and a serializer that gave up.
    fn export_toile_pdf(&mut self, document_id: DocumentId) {
        let Self {
            docs, export_cx, ..
        } = self;
        let Some(DocState::Toile(state)) = docs.get(&document_id).map(|d| &d.state) else {
            tracing::warn!("Logic: PDF export asked of a document that is not a board");
            return;
        };
        let answer = match export_pdf_of(state, export_cx.as_mut()) {
            Ok(data) => ToileEvent::ExportComplete {
                filename: PDF_FILENAME.to_owned(),
                data,
                // The deck walk draws every element through the shared surface,
                // so a page holds what the slide holds.
                skipped: Vec::new(),
            },
            Err(reason) => ToileEvent::ExportFailed(reason),
        };
        self.dispatch_outcome(
            document_id,
            vec![AppEvent::Toile(answer)],
            None,
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
    fn export_ambre_file(&mut self, document_id: DocumentId) {
        let Some(answer) = self
            .docs
            .get(&document_id)
            .map(|d| d.state.ambre_file_event(document_id))
        else {
            tracing::warn!("Logic: .ambre export asked of a document that is not open");
            return;
        };
        self.dispatch_outcome(document_id, vec![answer], None, None, None, Vec::new());
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
    /// The passphrase travels as [`SecretBytes`](ambre_bus::SecretBytes):
    /// every hop that drops it wipes it, and it prints as nothing in a log
    /// line.
    fn export_ambre_file_sealed(&mut self, document_id: DocumentId, passphrase: SecretBytes) {
        let Some(doc) = self.docs.get(&document_id) else {
            tracing::warn!("Logic: sealed .ambre export asked of a document that is not open");
            return;
        };
        let Some((filename, plaintext)) = doc.state.ambre_file_bytes(document_id) else {
            // Nothing to seal: a document the engine could not read becomes no
            // file, sealed or not (`TOILE-IO-069`).
            let answer = doc
                .state
                .export_failed_event("the document could not be read".to_owned());
            self.dispatch_outcome(document_id, vec![answer], None, None, None, Vec::new());
            return;
        };
        let id = RequestId::next();
        self.pending_seals.insert(id, (document_id, filename));
        self.send_to_sync(SyncCommand::Rest {
            id,
            cmd: RestCommand::SealDocFile {
                plaintext: plaintext.into(),
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
    pub(crate) fn resolve_pending_seal(&mut self, id: RequestId, evt: &RestEvent) -> bool {
        let answer = match evt {
            RestEvent::DocFileSealed { data } => Ok(data.clone()),
            RestEvent::DocFileSealFailed => Err("the file could not be encrypted".to_owned()),
            _ => return false,
        };
        let Some((document_id, filename)) = self.pending_seals.remove(&id) else {
            return false;
        };
        let Some(doc) = self.docs.get(&document_id) else {
            tracing::warn!("Logic: sealed .ambre landed for closed doc {document_id}, dropping");
            return true;
        };
        let event = match answer {
            Ok(data) => doc.state.export_complete_event(filename, data),
            Err(reason) => doc.state.export_failed_event(reason),
        };
        self.dispatch_outcome(document_id, vec![event], None, None, None, Vec::new());
        true
    }

    /// Drop every pending seal of `document_id` (it closed, or the session
    /// ended): an answer that arrives afterwards has nowhere to go.
    pub(crate) fn forget_pending_seals(&mut self, document_id: Option<DocumentId>) {
        match document_id {
            Some(document_id) => self.pending_seals.retain(|_, (d, _)| *d != document_id),
            None => self.pending_seals.clear(),
        }
    }

    /// Route a typed application command to its document's state, send display
    /// events to the UI Thread, and forward CRDT/ephemeral payloads to Sync.
    ///
    /// The registry is **flat**. A document embedded in a Toile via a `doc_embed`
    /// is an ordinary entry here, keyed by its own `DocumentId`. Its commands
    /// arrive addressed straight to it. Nesting adds no routing step. The child's
    /// view holds the child's id and emits through the same queue, so a command
    /// never travels through the parent.
    pub(crate) fn handle_app_command(&mut self, document_id: DocumentId, command: AppCommand) {
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
        // The deck's PDF export is the one app command this thread serves itself
        // (`toile-pdf-export`). The render walk needs the base font faces the
        // Main thread fetched at boot. Those live here, beside the documents
        // rather than inside one.
        if matches!(command, AppCommand::Toile(ToileCommand::ExportPdf)) {
            self.export_toile_pdf(document_id);
            return;
        }
        // The native file is the same operation whichever engine owns the
        // document, so the two variants are intercepted together rather than
        // implemented twice (`DOCFILE-007`).
        if matches!(
            command,
            AppCommand::Toile(ToileCommand::ExportAmbreFile)
                | AppCommand::Trame(ambre_bus::TrameCommand::ExportAmbreFile)
        ) {
            self.export_ambre_file(document_id);
            return;
        }
        // The sealed file is the same operation once more, and intercepted in
        // the same place for the same reason (`DOCFILE-007`, `DOCFILE-014`).
        // Matched by value rather than by discriminant: this moves the
        // passphrase out of the command and sends it no further than Sync.
        let command = match command {
            AppCommand::Toile(ToileCommand::ExportAmbreFileSealed { passphrase })
            | AppCommand::Trame(ambre_bus::TrameCommand::ExportAmbreFileSealed { passphrase }) => {
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
                let snapshot = outcome.update.is_some().then(|| state.export_snapshot());
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Trame).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    snapshot,
                    // Trame issues no service-plane requests (no interop plane).
                    Vec::new(),
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
                    outcome.requests,
                );
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

    /// Fan an already-typed outcome out to the UI Thread (display events), to
    /// Sync (`SendUpdate` / `SendEphemeral`), and to local persistence. The
    /// snapshot is a full one, captured while the `state` borrow was live.
    /// Shared by every `AppCommand` arm; the arm only decides which `AppEvent`
    /// variant wraps the display events and whether a snapshot was taken.
    fn dispatch_outcome(
        &mut self,
        document_id: DocumentId,
        display: Vec<AppEvent>,
        update: Option<Vec<u8>>,
        ephemeral: Option<Vec<u8>>,
        snapshot: Option<Vec<u8>>,
        requests: Vec<ToileRequest>,
    ) {
        for event in display {
            self.send_to_ui(UiEvent::Viewport { document_id, event });
        }
        if let Some(payload) = update {
            tracing::info!(
                "[Logic->Sync] pushing SendUpdate doc={document_id} delta_size={}",
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
        // Lower each Toile-level service request to a `RestCommand`, supplying the
        // owning `document_id` (which the engine does not hold). Record the route
        // as well, so the answering `RestEvent` / `ApiError` is fed back into the
        // engine instead of forwarded to the UI (`toile-media`).
        for request in requests {
            let (id, cmd) = self.lower_request(document_id, request);
            self.rest_routes.insert(id, document_id);
            self.send_to_sync(SyncCommand::Rest { id, cmd });
        }
    }

    /// Lower one [`ToileRequest`] into the `(RequestId, RestCommand)` pair the
    /// Sync plane expects, filling in the `document_id` the engine cannot name.
    /// The `RequestId` is the one Toile allocated -- it owns the correlation.
    fn lower_request(
        &self,
        document_id: DocumentId,
        request: ToileRequest,
    ) -> (RequestId, RestCommand) {
        match request {
            ToileRequest::UploadMedia { id, data, mime } => (
                id,
                RestCommand::UploadMedia {
                    document_id,
                    data,
                    mime_type: mime,
                },
            ),
            ToileRequest::DownloadMedia { id, content_hash } => (
                id,
                RestCommand::DownloadMedia {
                    document_id,
                    content_hash,
                },
            ),
        }
    }

    /// Feed a service-plane response routed back to a document
    /// (`toile-media`) into its engine and fan the resulting outcome out.
    /// Only a Toile document issues service requests, so a route pointing at any
    /// other state is a bug; we log and drop rather than panic. The engine
    /// silently ignores an id it no longer tracks (import undone / doc reopened).
    pub(crate) fn route_service_event(
        &mut self,
        document_id: DocumentId,
        id: RequestId,
        evt: RestEvent,
    ) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            tracing::warn!("Logic: routed RestEvent for closed doc {document_id}, dropping");
            return;
        };
        let DocState::Toile(state) = &mut doc.state else {
            tracing::warn!("Logic: routed RestEvent for non-Toile doc {document_id}, dropping");
            return;
        };
        let outcome = state.handle_service_event(id, evt);
        let snapshot = outcome.update.is_some().then(|| state.export_snapshot());
        self.dispatch_outcome(
            document_id,
            outcome.display.into_iter().map(AppEvent::Toile).collect(),
            outcome.update,
            outcome.ephemeral,
            snapshot,
            outcome.requests,
        );
    }

    /// Feed a correlated service-plane failure back into a document's engine
    /// (`toile-media`): a failed media upload aborts its pending import.
    pub(crate) fn route_service_error(
        &mut self,
        document_id: DocumentId,
        id: RequestId,
        reason: String,
    ) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            tracing::warn!("Logic: routed ApiError for closed doc {document_id}, dropping");
            return;
        };
        let DocState::Toile(state) = &mut doc.state else {
            tracing::warn!("Logic: routed ApiError for non-Toile doc {document_id}, dropping");
            return;
        };
        let outcome = state.handle_service_error(id, reason);
        let snapshot = outcome.update.is_some().then(|| state.export_snapshot());
        self.dispatch_outcome(
            document_id,
            outcome.display.into_iter().map(AppEvent::Toile).collect(),
            outcome.update,
            outcome.ephemeral,
            snapshot,
            outcome.requests,
        );
    }

    /// Apply a remote CRDT delta to a document and forward viewport events.
    pub(crate) fn apply_remote(&mut self, document_id: DocumentId, payload: &[u8]) {
        let Some(doc) = self.docs.get_mut(&document_id).map(|d| &mut d.state) else {
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
                // actually changed the document (it emitted display events). A
                // refresh then restores it even for a client that only ever
                // receives, never edits. Otherwise such a client has no local
                // snapshot and depends entirely on a fresh server replay.
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
        let Some(doc) = self.docs.get_mut(&document_id).map(|d| &mut d.state) else {
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
    /// `from_user_id` is the sender's canonical `UserId`, resolved from the
    /// authenticated `pub_key` in `ambre-sync` so presence cannot be spoofed.
    pub(crate) fn apply_ephemeral(
        &mut self,
        document_id: DocumentId,
        from_user_id: UserId,
        payload: &[u8],
    ) {
        let Some(doc) = self.docs.get_mut(&document_id).map(|d| &mut d.state) else {
            return;
        };
        let events: Vec<AppEvent> = match doc {
            // Trame's own `UserId`-keyed presence is a separate slice
            // (`trame-engine`); it ignores the resolved id for now.
            DocState::Trame(state) => state
                .apply_ephemeral(payload)
                .into_iter()
                .map(AppEvent::Trame)
                .collect(),
            DocState::Toile(state) => state
                .apply_ephemeral(from_user_id, payload)
                .into_iter()
                .map(AppEvent::Toile)
                .collect(),
        };
        for event in events {
            self.send_to_ui(UiEvent::Presence { document_id, event });
        }
    }

    /// A peer stopped watching a document (`PRESENCE-014`): drop them from the
    /// document's live set and hand the view the set that remains. Ephemeral
    /// throughout -- nothing here touches the CRDT or the history.
    pub(crate) fn peer_left(&mut self, document_id: DocumentId, user_id: UserId) {
        let Some(doc) = self.docs.get_mut(&document_id).map(|d| &mut d.state) else {
            return;
        };
        let events: Vec<AppEvent> = match doc {
            // Trame keeps no peer set yet (`trame-engine`).
            DocState::Trame(_) => Vec::new(),
            DocState::Toile(state) => state
                .peer_left(user_id)
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
        match &self.docs.get(&document_id)?.state {
            DocState::Trame(state) => Some(state.export_snapshot()),
            DocState::Toile(state) => Some(state.export_snapshot()),
        }
    }
}

/// The name a deck export carries. The board's title is a launcher concern, so
/// the file is named for what it holds, like the SVG export.
const PDF_FILENAME: &str = "deck.pdf";

/// The deck as PDF bytes, or the reason there are none. Split out of the
/// handler. The build with the export compiled in and the build without differ
/// in one function, not in the routing around it.
#[cfg(feature = "toile-pdf")]
fn export_pdf_of(
    state: &ToileState,
    font_cx: Option<&mut parley::FontContext>,
) -> Result<Vec<u8>, String> {
    let Some(font_cx) = font_cx else {
        // The faces are fetched once at boot and never re-requested, so this is
        // a race with startup, not a dead end: the next attempt succeeds.
        return Err("the fonts are still loading".to_owned());
    };
    state
        .export_pdf_document(Default::default(), font_cx)
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "toile-pdf"))]
fn export_pdf_of(
    _state: &ToileState,
    _font_cx: Option<&mut parley::FontContext>,
) -> Result<Vec<u8>, String> {
    Err("PDF export is not compiled into this build".to_owned())
}
