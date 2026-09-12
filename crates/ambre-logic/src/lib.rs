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

// The `impl LogicState` block is spread across sibling modules:
//   - `data`:     application data plane (app commands, remote deltas, snapshots)
//   - `dispatch`: the two inbound dispatchers (+ their host-side tests)
mod data;
mod dispatch;

/// Document-fixture loader, registry and driver (`ambre-spec-fixtures.md`).
///
/// Fixture code (I4): behind the non-default `fixtures` feature, because this
/// crate **does** ship to wasm. `DocState::import_snapshot` / `export_snapshot`
/// are the ungated production exceptions and live in this file, not here.
#[cfg(feature = "fixtures")]
pub mod fixtures;

use ambre_bus::{
    AppCommand, AppEvent, DocType, DocumentId, LogicCommand, LogicEvent, RequestId, RestCommand,
    RestEvent, Role, SyncCommand, UiEvent, UserId,
};

use ambre_core::schema::SchemaError;

use ambre_toile::{ToileRequest, ToileState};
use ambre_trame::TrameState;

// -----------------------------------------------------------------------------
// DocState — typed per-document state
// -----------------------------------------------------------------------------

/// Per-document state, one variant per application document type.
///
/// A closed sum mirroring [`AppCommand`] / [`AppEvent`]: the dispatcher matches
/// the `(DocState, AppCommand)` pair to route a typed command to the right
/// document model — no trait object, no downcast.
///
/// `pub` since `ambre-spec-fixtures.md`: the fixture loader hands one back
/// and the round-trip belt of quantifies over its variants, neither of which
/// a crate-private enum allows.
pub enum DocState {
    // Boxed: the per-document states differ widely in size (a `TrameState` is
    // several KB, a `ToileState` far smaller), so a bare enum would size every
    // slot to the largest variant. One heap indirection per open document is
    // negligible and keeps the registry entry pointer-sized.
    Trame(Box<TrameState>),
    Toile(Box<ToileState>),
    // Future: Prose(ProseState), Murmure(MurmureState), …
}

/// The variant, never the contents. Neither `TrameState` nor `ToileState` is
/// `Debug` — both hold a whole `LoroDoc` and printing one would dump a document
/// into a log line, which the zero-knowledge posture rules out. What a caller
/// actually wants here is which engine it is holding, which is what an error
/// message (`FixtureError`, the dispatcher's mismatch warning) needs.
impl std::fmt::Debug for DocState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DocState::{:?}", self.doc_type())
    }
}

impl DocState {
    /// Build the state for a freshly-opened document of `doc_type`.
    ///
    /// Returns `None` for document types whose logic is not yet implemented.
    pub fn new(doc_type: DocType) -> Option<Self> {
        match doc_type {
            DocType::Trame => Some(DocState::Trame(Box::new(TrameState::new()))),
            DocType::Toile => Some(DocState::Toile(Box::new(ToileState::new()))),
            DocType::Prose | DocType::Murmure => {
                tracing::warn!(
                    "Logic: no state implemented for {:?}, document will not render",
                    doc_type
                );
                None
            }
        }
    }

    /// The document's `doc_type`, recovered from the live variant. The inverse of
    /// [`new`](Self::new), and what the fixture driver writes into a manifest.
    pub fn doc_type(&self) -> DocType {
        match self {
            DocState::Trame(_) => DocType::Trame,
            DocState::Toile(_) => DocType::Toile,
        }
    }

    /// Load a full Loro snapshot into this state, returning the display events
    /// the app emits.
    ///
    /// **Production code, ungated** (`FIXTURE-007`): `ambre-logic` ships to
    /// wasm, and this is the enum-level entry point a local restore goes through.
    /// Its Toile arm is why `ToileState::import_snapshot` is ungated too --
    /// gating that would make an ordinary client build fail to compile here.
    ///
    /// Lifted to the enum because a loader cannot address two engines that spell
    /// the same operation differently (`TrameState::import_snapshot` versus, up
    /// to this phase, `ToileState::apply_remote`).
    ///
    /// # Errors
    /// The schema gate's verdict, unchanged (`ambre-spec-schema-version.md`): a
    /// document written by a newer client, a migration the owning application
    /// cannot run, or bytes the CRDT engine refuses. On any of them the state is
    /// untouched (`SCHEMA-012`), which is why this is a `Result` and not an
    /// event: a caller that ignored it would carry on with a document it never
    /// loaded.
    pub fn import_snapshot(&mut self, payload: &[u8]) -> Result<Vec<AppEvent>, SchemaError> {
        match self {
            DocState::Trame(state) => Ok(state
                .import_snapshot(payload)?
                .into_iter()
                .map(AppEvent::Trame)
                .collect()),
            DocState::Toile(state) => Ok(state
                .import_snapshot(payload)?
                .into_iter()
                .map(AppEvent::Toile)
                .collect()),
        }
    }

    /// The schema version this document carries (`SCHEMA-004`), whichever
    /// application owns it. Each declares its own number, so the value is only
    /// comparable against that application's own current version.
    #[must_use]
    pub fn schema_version(&self) -> u32 {
        match self {
            DocState::Trame(state) => state.schema_version(),
            DocState::Toile(state) => state.schema_version(),
        }
    }

    /// The refusal, as the display event the owning application surfaces
    /// (`SCHEMA-008`). The gate is shared; the wording and the toast are the
    /// app's, so the event is minted here rather than in the dispatcher.
    fn refusal_event(&self, error: &SchemaError) -> AppEvent {
        match self {
            DocState::Trame(_) => AppEvent::Trame(ambre_bus::TrameEvent::DocumentRefused {
                reason: error.to_string(),
            }),
            DocState::Toile(_) => AppEvent::Toile(
                ambre_bus::app::toile::ToileEvent::DocumentRefused(error.to_string()),
            ),
        }
    }

    /// The whole document as a native `.ambre` file, as the display event the
    /// owning application surfaces (`DOCFILE-001`, `DOCFILE-007`).
    ///
    /// Minted here for the same reason as [`refusal_event`](Self::refusal_event):
    /// the container is one for every application, and the event that carries
    /// its bytes to the browser is each application's own. `document_id` comes
    /// from the registry that keyed this state -- the engine does not know its
    /// own identity, and the header needs it.
    ///
    /// The file name is a constant per application, like every other export in
    /// the suite (`board.svg`, `deck.pdf`): the document's own name is encrypted
    /// and lives on the Sync thread, which this one cannot read (`ZK-001`).
    fn ambre_file_event(&self, document_id: DocumentId) -> AppEvent {
        let (filename, data) = self.ambre_file_bytes(document_id);
        self.export_complete_event(filename, data)
    }

    /// The file name and the plaintext container behind
    /// [`ambre_file_event`](Self::ambre_file_event), apart from the event.
    ///
    /// The sealed export needs exactly these two, and needs them *before*
    /// there is an event to carry them. The bytes travel to the Sync worker and
    /// come back changed, while the name waits here (`DOCFILE-014`).
    fn ambre_file_bytes(&self, document_id: DocumentId) -> (String, Vec<u8>) {
        match self {
            DocState::Trame(state) => (
                "workbook.ambre".to_owned(),
                state.export_ambre_plaintext(document_id.into()),
            ),
            DocState::Toile(state) => (
                "board.ambre".to_owned(),
                state.export_ambre_plaintext(document_id.into()),
            ),
        }
    }

    /// A finished export, as the display event the owning application surfaces.
    /// The bytes are whatever the caller has: a plaintext container, or the
    /// sealed one the Sync worker returned. The surface downloads either the
    /// same way, and never tells them apart.
    fn export_complete_event(&self, filename: String, data: Vec<u8>) -> AppEvent {
        match self {
            DocState::Trame(_) => {
                AppEvent::Trame(ambre_bus::TrameEvent::ExportComplete { filename, data })
            }
            DocState::Toile(_) => {
                AppEvent::Toile(ambre_bus::app::toile::ToileEvent::ExportComplete {
                    filename,
                    data,
                })
            }
        }
    }

    /// An export that produced no file, as the display event the owning
    /// application surfaces. Trame has no export-failure variant of its own, so
    /// it says it through the generic error the chrome already toasts.
    fn export_failed_event(&self, reason: String) -> AppEvent {
        match self {
            DocState::Trame(_) => AppEvent::Trame(ambre_bus::TrameEvent::Error { message: reason }),
            DocState::Toile(_) => {
                AppEvent::Toile(ambre_bus::app::toile::ToileEvent::ExportFailed(reason))
            }
        }
    }

    /// Export this state's full Loro snapshot.
    ///
    /// **Production code, ungated** (`FIXTURE-007`), and the other half of the
    /// round trip asserts on every variant.
    /// [`LogicState::export_snapshot`] cannot serve that belt: it takes a
    /// `DocumentId` and needs a live registry entry, where the belt quantifies
    /// over a bare `DocState`.
    pub fn export_snapshot(&self) -> Vec<u8> {
        match self {
            DocState::Trame(state) => state.export_snapshot(),
            DocState::Toile(state) => state.export_snapshot(),
        }
    }

    /// Seed the canonical `UserId` into the underlying app state once the
    /// session is authenticated. Only apps that key per-user state on it react;
    /// Trame's own `my_user_id` (`trame-engine`) is a separate slice.
    fn set_user_id(&mut self, user_id: UserId) {
        match self {
            DocState::Toile(state) => state.set_user_id(user_id),
            DocState::Trame(_) => {}
        }
    }

    /// Seed the session's display name beside the `UserId`, from the same
    /// `SessionReady`. Toile stamps it on every outbound presence sample so a
    /// collaborator reads a name rather than a hex prefix (`TOILE-UI-076`); it
    /// is cosmetic and decides nothing (`PRESENCE-004`).
    fn set_display_name(&mut self, name: &str) {
        match self {
            DocState::Toile(state) => state.set_display_name(name.to_owned()),
            DocState::Trame(_) => {}
        }
    }
}

// -----------------------------------------------------------------------------
// LiveDoc — an open document and the caller's role on it
// -----------------------------------------------------------------------------

/// One open document: its typed state plus this client's role on **that**
/// document (architecture).
///
/// The role gates mutating `AppCommand`s in [`LogicState::handle_app_command`].
/// This is a **UX gate only** -- it avoids offering edits the server would
/// reject; the server remains the sole authority and re-checks every write.
struct LiveDoc {
    state: DocState,
    /// Starts `Role::Owner` because a document is materialised optimistically at
    /// `OpenDocument`, before Sync has fetched the members response: edits route
    /// immediately, exactly as they did before this gate existed. The real role
    /// arrives on `DocumentReady` and supersedes it. A downgrade landing a beat
    /// late is harmless -- the server rejects any write we should not have made.
    role: Role,
}

impl LiveDoc {
    fn new(state: DocState) -> Self {
        Self {
            state,
            role: Role::Owner,
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
    /// Pre-seeded on `OpenDocument` (which carries the `doc_type`) so UI edits
    /// route immediately, before any snapshot/delta arrives. Removed on
    /// `CloseDocument` / `DeleteDocument` / `Logout`.
    docs: HashMap<DocumentId, LiveDoc>,
    /// Canonical per-user identifier, captured from `LogicEvent::SessionReady`
    /// `None` until the session is authenticated. Seeded into every open
    /// `DocState` on arrival and into any document opened afterwards, so an app
    /// that keys per-user state (Toile reactions, presence identity) always has
    /// it before the first edit.
    my_user_id: Option<UserId>,
    /// The session's display name, from the same `SessionReady`. Seeded into a
    /// document opened afterwards, so a board opened later still labels this
    /// peer's cursor (`TOILE-UI-076`).
    my_display_name: Option<String>,
    /// P3 responses this Logic thread issued on behalf of a document
    /// (`toile-media`): a `RequestId` present here routes its answering
    /// `RestEvent` (or correlated `ApiError`) back into the owning engine instead
    /// of forwarding it to the UI. UI-issued requests are absent from this map
    /// and forward verbatim, exactly as before. An entry is inserted when a
    /// `ToileRequest` is lowered and removed when its response arrives.
    rest_routes: HashMap<RequestId, DocumentId>,
    /// Sealed `.ambre` exports awaiting the Sync worker's answer
    /// (`DOCFILE-014`): the document whose file the worker holds, and the name
    /// the download will carry. A separate map from
    /// [`rest_routes`](Self::rest_routes) because the answer is not the
    /// engine's. The engine never learns of the seal, and the response becomes
    /// a display event here.
    ///
    /// Cleared with the document that owns it, so a seal outlived by its
    /// document answers nothing rather than downloading into a closed tab.
    pending_seals: HashMap<RequestId, (DocumentId, String)>,
    /// Base fonts for PDF export (`toile-pdf-export`), seeded once by
    /// [`LogicCommand::LoadFonts`]. `None` until the Main thread's boot-time
    /// fetch lands; a fresh `FontContext` here would have zero faces and emit no
    /// glyphs, so export waits on this. Lives on the Logic thread because that is
    /// where the export pass runs, and the thread cannot fetch it itself.
    ///
    /// Read by the PDF-export handler (`data::export_toile_pdf`), and seeded at
    /// boot so the faces are warm before the first export rather than fetched
    /// under it.
    export_cx: Option<parley::FontContext>,
}

impl LogicState {
    fn new(
        to_ui: flume::Sender<UiEvent>,
        to_sync: flume::Sender<SyncCommand>,
        to_persist: flume::Sender<(DocumentId, Vec<u8>)>,
    ) -> Self {
        Self {
            to_ui,
            to_sync,
            to_persist,
            docs: HashMap::new(),
            my_user_id: None,
            my_display_name: None,
            rest_routes: HashMap::new(),
            pending_seals: HashMap::new(),
            export_cx: None,
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
