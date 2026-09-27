// ambre-logic/src/lib.rs
// ============================================================================
// Logic thread entry point -- runs in a dedicated wasm_thread, shares the heap
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
//   - Cryptography, compression, network -- handled exclusively by ambre-sync.
//   - Rendering -- handled by the UI Thread (wgpu + QuadRenderer + GlyphAtlas).
//
// Transport: UI <-> Logic uses `ambre_bus::conduit` channels in the shared memory
// of the Main instance, carrying typed Rust values by move (no postcard, no
// `Any`). Logic-bound `SyncCommand`s are sent to a link whose receiver is on
// the Main thread, which relays them to the separate Sync worker via
// `postMessage`. `LogicEvent`s arrive the same way in reverse.
// ============================================================================

use std::collections::HashMap;

/// IndexedDB-backed local snapshot store. Compiled only for wasm, since its
/// callbacks need a live event loop. The store itself is driven by the Main
/// thread (see `ambre-app`), not by this blocking Logic loop.
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

use ambre_bus::conduit::{Doorbell, Receiver, WorkerSender};
use ambre_bus::{
    AppCommand, AppEvent, DocType, DocumentId, LogicCommand, LogicEvent, RequestId, RestCommand,
    RestEvent, Role, SyncCommand, UiEvent, UserId,
};

use ambre_core::schema::SchemaError;

use ambre_prose::ProseState;
use ambre_toile::{ToileRequest, ToileState};
use ambre_trame::TrameState;

// -----------------------------------------------------------------------------
// DocState -- typed per-document state
// -----------------------------------------------------------------------------

/// Per-document state, one variant per application document type.
///
/// A closed sum mirroring [`AppCommand`] / [`AppEvent`]. The dispatcher matches
/// the `(DocState, AppCommand)` pair to route a typed command to the right
/// document model: no trait object, no downcast.
///
/// `pub` since `ambre-spec-fixtures.md`: the fixture loader hands one back and
/// the round-trip belt of quantifies over its variants, neither of which a
/// crate-private enum allows.
pub enum DocState {
    // Boxed, because the per-document states differ widely in size (a
    // `TrameState` is several KB, a `ToileState` far smaller). A bare enum
    // would size every slot to the largest variant. One heap indirection per
    // open document is negligible and keeps the registry entry pointer-sized.
    Trame(Box<TrameState>),
    Toile(Box<ToileState>),
    Prose(Box<ProseState>),
    // Future: Murmure(MurmureState), ...
}

/// The variant, never the contents. Neither `TrameState` nor `ToileState` is
/// `Debug`. Both hold a whole `LoroDoc`, and printing one would dump a document
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
            // `None` when the CRDT engine refused the scaffolding: the document
            // is not opened rather than opened empty (`PROSE-027`).
            DocType::Prose => ProseState::new().map(|state| DocState::Prose(Box::new(state))),
            DocType::Murmure => {
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
            DocState::Prose(_) => DocType::Prose,
        }
    }

    /// Load a full Loro snapshot into this state, returning the display events
    /// the app emits.
    ///
    /// **Production code, ungated** (`FIXTURE-007`): `ambre-logic` ships to wasm,
    /// and this is the enum-level entry point a local restore goes through. Its
    /// Toile arm is why `ToileState::import_snapshot` is ungated too -- gating
    /// that would make an ordinary client build fail to compile here.
    ///
    /// Lifted to the enum because a loader cannot address two engines that spell
    /// the same operation differently (`TrameState::import_snapshot` versus, up
    /// to this phase, `ToileState::apply_remote`).
    ///
    /// # Errors
    /// The schema gate's verdict, unchanged (`ambre-spec-schema-version.md`). It
    /// is a document written by a newer client, a migration the owning
    /// application cannot run, or bytes the CRDT engine refuses. On any of them
    /// the state is untouched (`SCHEMA-012`). That is why this is a `Result` and
    /// not an event. A caller that ignored it would carry on with a document it
    /// never loaded.
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
            DocState::Prose(state) => Ok(state
                .import_snapshot(payload)?
                .into_iter()
                .map(AppEvent::Prose)
                .collect()),
        }
    }

    /// Import a batch of deltas, whichever application owns the document, and
    /// answer its display events.
    pub fn apply_remote_batch(&mut self, payloads: &[Vec<u8>]) -> Vec<AppEvent> {
        match self {
            DocState::Trame(state) => state
                .apply_remote_batch(payloads)
                .into_iter()
                .map(AppEvent::Trame)
                .collect(),
            DocState::Toile(state) => state
                .apply_remote_batch(payloads)
                .into_iter()
                .map(AppEvent::Toile)
                .collect(),
            DocState::Prose(state) => state
                .apply_remote_batch(payloads)
                .into_iter()
                .map(AppEvent::Prose)
                .collect(),
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
            DocState::Prose(state) => state.schema_version(),
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
            DocState::Prose(_) => {
                AppEvent::Prose(ambre_bus::app::prose::ProseEvent::DocumentRefused {
                    reason: error.to_string(),
                })
            }
        }
    }

    /// The whole document as a native `.ambre` file, as the display event the
    /// owning application surfaces (`DOCFILE-001`, `DOCFILE-007`).
    ///
    /// Minted here for the same reason as [`refusal_event`](Self::refusal_event).
    /// The container is one for every application, and the event that carries its
    /// bytes to the browser is each application's own. `document_id` comes from
    /// the registry that keyed this state -- the engine does not know its own
    /// identity, and the header needs it.
    ///
    /// The file name is a constant per application, like every other export in
    /// the suite (`board.svg`, `deck.pdf`). The document's own name is encrypted
    /// and lives on the Sync thread, which this one cannot read (`ZK-001`).
    fn ambre_file_event(&self, document_id: DocumentId) -> AppEvent {
        match self.ambre_file_bytes(document_id) {
            Some((filename, data)) => self.export_complete_event(filename, data),
            // A document the engine could not read becomes no file
            // (`TOILE-IO-069`). The user is told instead.
            None => self.export_failed_event("the document could not be read".to_owned()),
        }
    }

    /// The file name and the plaintext container behind
    /// [`ambre_file_event`](Self::ambre_file_event), apart from the event.
    ///
    /// The sealed export needs exactly these two, and needs them *before*
    /// there is an event to carry them. The bytes travel to the Sync worker and
    /// come back changed, while the name waits here (`DOCFILE-014`).
    /// `None` when the engine could not read the document: no file is written
    /// at all (`TOILE-IO-069`).
    fn ambre_file_bytes(&self, document_id: DocumentId) -> Option<(String, Vec<u8>)> {
        match self {
            DocState::Trame(state) => Some((
                "workbook.ambre".to_owned(),
                state.export_ambre_plaintext(document_id.into())?,
            )),
            DocState::Toile(state) => Some((
                "board.ambre".to_owned(),
                state.export_ambre_plaintext(document_id.into())?,
            )),
            DocState::Prose(state) => Some((
                "document.ambre".to_owned(),
                state.export_ambre_plaintext(document_id.into())?,
            )),
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
                    skipped: Vec::new(),
                })
            }
            DocState::Prose(_) => AppEvent::Prose(ambre_bus::app::prose::ProseEvent::ExportReady {
                filename,
                media_type: ambre_bus::app::prose::ExportFormat::Ambre.media_type(),
                data,
                report: ambre_bus::app::prose::ConversionReport::clean(
                    ambre_bus::app::prose::ConversionFormat::Ambre,
                ),
            }),
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
            DocState::Prose(_) => AppEvent::Prose(ambre_bus::app::prose::ProseEvent::Error(
                ambre_bus::app::prose::ProseError::new(
                    "exporting",
                    ambre_bus::app::prose::ProseRefusal::WriteFailed(reason),
                ),
            )),
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
            DocState::Prose(state) => state.export_snapshot(),
        }
    }

    /// Seed the canonical `UserId` into the underlying app state once the
    /// session is authenticated. Only apps that key per-user state on it react;
    /// Trame's own `my_user_id` (`trame-engine`) is a separate slice.
    fn set_user_id(&mut self, user_id: UserId) {
        match self {
            DocState::Toile(state) => state.set_user_id(user_id),
            DocState::Prose(state) => state.set_user_id(user_id),
            DocState::Trame(_) => {}
        }
    }

    /// Hand the document the font faces the application has loaded
    /// (`CANVAS-067`, `PROSE-115`).
    ///
    /// Only Prose measures text on the Logic thread: pagination shapes on a
    /// sealed context, which holds nothing until these arrive (`PROSE-114`).
    /// Trame and Toile measure on the UI thread, which owns its own context.
    fn load_faces(&mut self, faces: &[Vec<u8>]) {
        match self {
            DocState::Prose(state) => state.load_faces(faces),
            DocState::Trame(_) | DocState::Toile(_) => {}
        }
    }

    /// The layout the view draws, when it changed since the last one handed
    /// over (`PROSE-UI-006`). Only Prose lays text out on this thread.
    fn layout_event(&mut self) -> Option<AppEvent> {
        match self {
            DocState::Prose(state) => state.layout_event().map(AppEvent::Prose),
            DocState::Trame(_) | DocState::Toile(_) => None,
        }
    }

    /// How many faces this document measures on, zero for an engine that
    /// measures nothing on this thread (`PROSE-115`).
    #[cfg(test)]
    fn face_count(&self) -> usize {
        match self {
            DocState::Prose(state) => state.face_count(),
            DocState::Trame(_) | DocState::Toile(_) => 0,
        }
    }

    /// Seed the session's display name beside the `UserId`, from the same
    /// `SessionReady`. Toile stamps it on every outbound presence sample, so a
    /// collaborator reads a name rather than a hex prefix (`TOILE-UI-076`). It
    /// is cosmetic and decides nothing (`PRESENCE-004`).
    fn set_display_name(&mut self, name: &str) {
        match self {
            DocState::Toile(state) => state.set_display_name(name.to_owned()),
            DocState::Prose(state) => state.set_display_name(name.to_owned()),
            DocState::Trame(_) => {}
        }
    }
}

// -----------------------------------------------------------------------------
// LiveDoc -- an open document and the caller's role on it
// -----------------------------------------------------------------------------

/// One open document: its typed state plus this client's role on **that**
/// document (architecture).
///
/// The role gates mutating `AppCommand`s in [`LogicState::handle_app_command`].
/// This is a **UX gate only** -- it avoids offering edits the server would
/// reject; the server remains the sole authority and re-checks every write.
struct LiveDoc {
    state: DocState,
    /// How many deltas local persistence holds past the snapshot this session
    /// last saved. At [`DELTAS_PER_SNAPSHOT`] the next save is a snapshot.
    /// `None` until this session saved one, since a delta only makes sense on
    /// top of a snapshot. A document freshly seeded holds a root block of its
    /// own, which a replay of deltas alone would duplicate.
    logged: Option<usize>,
    /// Starts `Role::Owner` because a document is materialised optimistically at
    /// `OpenDocument`, before Sync has fetched the members response. Edits route
    /// immediately, exactly as they did before this gate existed. The real role
    /// arrives on `DocumentReady` and supersedes it. A downgrade landing a beat
    /// late is harmless -- the server rejects any write we should not have made.
    role: Role,
}

impl LiveDoc {
    fn new(state: DocState) -> Self {
        Self {
            state,
            logged: None,
            role: Role::Owner,
        }
    }
}

/// How many deltas a document's local log holds before the next save folds
/// them into a snapshot. A keystroke then costs a delta, not a whole export,
/// while a restore never replays more than this many.
const DELTAS_PER_SNAPSHOT: usize = 256;

/// One write to local persistence, which the Main thread performs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistWrite {
    /// The whole document. It replaces the stored one and every delta logged
    /// since, which it holds.
    Snapshot(Vec<u8>),
    /// A change appended to the document's log, imported after the snapshot on
    /// restore.
    Delta(Vec<u8>),
}

// -----------------------------------------------------------------------------
// LogicState
// -----------------------------------------------------------------------------

/// All mutable state owned by the Logic thread.
struct LogicState {
    /// Outbound channel: Logic -> UI Thread.
    to_ui: WorkerSender<UiEvent>,
    /// Outbound channel: Logic -> Sync (drained by the Main thread, relayed to
    /// the Sync worker via postMessage).
    to_sync: WorkerSender<SyncCommand>,
    /// Outbound channel: Logic -> Main, carrying document snapshots and deltas
    /// for local persistence. The Main thread owns the IndexedDB store, whose
    /// callbacks need a live event loop this blocking loop cannot offer. Here we
    /// only export the bytes and hand them over.
    to_persist: WorkerSender<(DocumentId, PersistWrite)>,
    /// Live document state, keyed by `document_id`.
    ///
    /// Pre-seeded on `OpenDocument` (which carries the `doc_type`) so UI edits
    /// route immediately, before any snapshot/delta arrives. Removed on
    /// `CloseDocument` / `DeleteDocument` / `Logout`.
    docs: HashMap<DocumentId, LiveDoc>,
    /// Canonical per-user identifier, captured from `LogicEvent::SessionReady`
    /// `None` until the session is authenticated. Seeded into every open
    /// `DocState` on arrival, and into any document opened afterwards. An app
    /// that keys per-user state (Toile reactions, presence identity) always has
    /// it before the first edit.
    my_user_id: Option<UserId>,
    /// The session's display name, from the same `SessionReady`. Seeded into a
    /// document opened afterwards, so a board opened later still labels this
    /// peer's cursor (`TOILE-UI-076`).
    my_display_name: Option<String>,
    /// P3 responses this Logic thread issued on behalf of a document
    /// (`toile-media`). A `RequestId` present here routes its answering
    /// `RestEvent` (or correlated `ApiError`) back into the owning engine,
    /// instead of forwarding it to the UI. UI-issued requests are absent from
    /// this map and forward verbatim, exactly as before. An entry is inserted
    /// when a `ToileRequest` is lowered and removed when its response arrives.
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
    /// Every font face the Main thread has fetched and shipped over
    /// (`LogicCommand::LoadFonts`). Kept as bytes, so a document opened later
    /// gets the same set (`CANVAS-067`, `PROSE-115`). The Logic thread cannot
    /// fetch, so this is the whole of what it can measure on.
    base_faces: Vec<Vec<u8>>,
    /// Base fonts for PDF export (`toile-pdf-export`), seeded once by
    /// [`LogicCommand::LoadFonts`]. `None` until the Main thread's boot-time
    /// fetch lands; a fresh `FontContext` here would have zero faces and emit no
    /// glyphs, so export waits on this. Lives on the Logic thread because that is
    /// where the export pass runs, and the thread cannot fetch it itself. It is
    /// sealed (`PDF-007`): no face of the machine is discoverable from it.
    ///
    /// Read by the PDF-export handler (`data::export_toile_pdf`), and seeded at
    /// boot so the faces are warm before the first export rather than fetched
    /// under it.
    export_cx: Option<parley::FontContext>,
    /// What each document written during the current pass is owed once the
    /// pass ends: a layout, a snapshot, or both (`ARCH-017`). A burst of
    /// keystrokes then costs one layout and one export, not one per key.
    owed: HashMap<DocumentId, Owed>,
}

/// The answers a document is owed at the end of a pass (`ARCH-017`).
#[derive(Default)]
struct Owed {
    /// The view's layout, handed over when it changed.
    layout: bool,
    /// A full snapshot for local persistence, in place of any delta.
    snapshot: bool,
    /// The changes the pass made, each logged as it came.
    deltas: Vec<Vec<u8>>,
}

impl LogicState {
    fn new(
        to_ui: WorkerSender<UiEvent>,
        to_sync: WorkerSender<SyncCommand>,
        to_persist: WorkerSender<(DocumentId, PersistWrite)>,
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
            base_faces: Vec::new(),
            export_cx: None,
            owed: HashMap::new(),
        }
    }

    // -- Outbound --------------------------------------------------------------

    fn send_to_ui(&self, event: UiEvent) {
        if let Err(error) = self.to_ui.send(event) {
            tracing::warn!("[Logic->UI] not delivered: {error:?}");
        }
    }

    /// Owe the view a document's layout. Called after every path that writes
    /// the document or its faces, so the view never lays text out itself
    /// (`PROSE-UI-006`). The layout leaves once, when the pass ends.
    fn owe_layout(&mut self, document_id: DocumentId) {
        self.owed.entry(document_id).or_default().layout = true;
    }

    /// Owe local persistence a snapshot of a document the pass changed.
    fn owe_snapshot(&mut self, document_id: DocumentId) {
        self.owed.entry(document_id).or_default().snapshot = true;
    }

    /// Owe local persistence one change the pass made to a document.
    fn owe_delta(&mut self, document_id: DocumentId, delta: Vec<u8>) {
        self.owed.entry(document_id).or_default().deltas.push(delta);
    }

    /// Hand every document the pass wrote its owed answers (`ARCH-017`).
    fn answer_pass(&mut self) {
        let owed: Vec<DocumentId> = self.owed.keys().copied().collect();
        for document_id in owed {
            self.answer_document(document_id, false);
        }
    }

    /// Hand one document what it is owed: its layout when that changed, then
    /// what local persistence needs. The view comes first, since the keystroke
    /// waits on it and the save does not.
    ///
    /// The save is the pass's deltas. A snapshot takes their place when one
    /// was asked for, once the log is long enough, or when `closing` finds
    /// the log holding anything. A document already closed is owed nothing.
    fn answer_document(&mut self, document_id: DocumentId, closing: bool) {
        let owed = self.owed.remove(&document_id).unwrap_or_default();
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        if owed.layout
            && let Some(event) = doc.state.layout_event()
        {
            self.send_to_ui(UiEvent::Viewport { document_id, event });
        }
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        let logged = doc.logged.map(|n| n + owed.deltas.len());
        let written = !owed.deltas.is_empty();
        let compact = owed.snapshot
            || match logged {
                None => written,
                Some(n) => n > DELTAS_PER_SNAPSHOT || (closing && n > 0),
            };
        if compact {
            doc.logged = Some(0);
            let snapshot = doc.state.export_snapshot();
            self.send_to_persist(document_id, PersistWrite::Snapshot(snapshot));
        } else {
            doc.logged = logged;
            for delta in owed.deltas {
                self.send_to_persist(document_id, PersistWrite::Delta(delta));
            }
        }
    }

    fn send_to_sync(&self, cmd: SyncCommand) {
        if let Err(error) = self.to_sync.send(cmd) {
            tracing::warn!("[Logic->Sync] not delivered: {error:?}");
        }
    }

    /// Hand one write to the Main thread for local persistence. Best-effort: a
    /// dropped receiver only means no local save, so it is non-fatal.
    fn send_to_persist(&self, document_id: DocumentId, write: PersistWrite) {
        if self.to_persist.send((document_id, write)).is_err() {
            tracing::warn!("[Logic->Persist] receiver dropped, write not saved");
        }
    }
}

impl LogicState {
    /// Apply the messages already queued on both channels, then answer.
    ///
    /// The pass takes only what waited when it began. A sender that never
    /// pauses therefore delays the answer by one queue's worth, not forever.
    fn drain_pass(&mut self, from_ui: &Receiver<LogicCommand>, from_sync: &Receiver<LogicEvent>) {
        for command in from_ui.try_iter().take(from_ui.len()) {
            self.handle_logic_command(command);
        }
        for event in from_sync.try_iter().take(from_sync.len()) {
            self.handle_logic_event(event);
        }
        self.answer_pass();
    }
}

/// The worker's clock, in milliseconds, or `None` off the browser.
#[cfg(target_arch = "wasm32")]
fn pass_clock() -> Option<f64> {
    let performance = js_sys::Reflect::get(&js_sys::global(), &"performance".into()).ok()?;
    let now = js_sys::Reflect::get(&performance, &"now".into()).ok()?;
    let now: js_sys::Function = wasm_bindgen::JsCast::dyn_into(now).ok()?;
    now.call0(&performance).ok()?.as_f64()
}

#[cfg(not(target_arch = "wasm32"))]
fn pass_clock() -> Option<f64> {
    None
}

/// Stamp one pass on the worker's timeline as `ambre:logic-pass`, beside the
/// keystroke probes of the view (`ambre-spec-budgets.md` section 1). It is
/// read on the worker's own clock, so it says how long the pass took, never
/// when against the view's marks.
#[cfg(target_arch = "wasm32")]
fn stamp_pass(start: Option<f64>) {
    let (Some(start), Some(end)) = (start, pass_clock()) else {
        return;
    };
    let Ok(performance) = js_sys::Reflect::get(&js_sys::global(), &"performance".into()) else {
        return;
    };
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"start".into(), &start.into());
    let _ = js_sys::Reflect::set(&options, &"end".into(), &end.into());
    let Ok(measure) = js_sys::Reflect::get(&performance, &"measure".into()) else {
        return;
    };
    let Ok(measure) = wasm_bindgen::JsCast::dyn_into::<js_sys::Function>(measure) else {
        return;
    };
    let _ = measure.call2(&performance, &"ambre:logic-pass".into(), &options);
}

#[cfg(not(target_arch = "wasm32"))]
fn stamp_pass(_start: Option<f64>) {}

// -----------------------------------------------------------------------------
// Entry point: drain both inbound channels, then park on the doorbell.
// -----------------------------------------------------------------------------

/// Run the Logic thread to completion.
///
/// Binds `bell`, the doorbell both inbound channels ring, then loops. While
/// nothing is queued it parks. Once a message is queued it applies every
/// message waiting, then answers each document it wrote once (`ARCH-017`).
/// Exits when **both** inbound channels are drained and disconnected (the UI
/// thread and the Sync relay both gone).
///
/// Parking is `Atomics.wait` on this worker, which the browser allows off the
/// main thread. A send from the main thread unparks it with `Atomics.notify`,
/// and never waits itself (`ARCH-022`).
pub fn run(
    bell: Doorbell,
    from_ui: Receiver<LogicCommand>,
    to_ui: WorkerSender<UiEvent>,
    to_sync: WorkerSender<SyncCommand>,
    from_sync: Receiver<LogicEvent>,
    to_persist: WorkerSender<(DocumentId, PersistWrite)>,
) {
    tracing::info!("Ambre Logic thread starting");
    let Some(waiter) = bell.bind() else {
        tracing::error!("Ambre Logic thread: the doorbell is bound to another thread");
        return;
    };
    let mut state = LogicState::new(to_ui, to_sync, to_persist);

    loop {
        while from_ui.is_empty() && from_sync.is_empty() {
            if from_ui.is_disconnected() && from_sync.is_disconnected() {
                // Re-check: a last message may have landed before the drop.
                if from_ui.is_empty() && from_sync.is_empty() {
                    tracing::info!(
                        "Ambre Logic thread exiting (both inbound channels disconnected)"
                    );
                    // Dropping `state` closes the outbound senders, which the
                    // UI thread and the Sync relay observe as `Disconnected`.
                    return;
                }
                break;
            }
            waiter.wait();
        }
        let start = pass_clock();
        state.drain_pass(&from_ui, &from_sync);
        stamp_pass(start);
    }
}

/// An outbound link for a test, wide enough that no test fills it. No thread
/// binds its bell, so a full one hands the value back instead of parking.
#[cfg(test)]
fn test_link<T>() -> (WorkerSender<T>, Receiver<T>) {
    ambre_bus::conduit::to_main(4096, &Doorbell::new())
}

#[cfg(test)]
mod pass_tests {
    use super::*;
    use ambre_bus::app::prose::{ProseCommand, ProseEvent};
    use ambre_bus::{AppCommand, AppEvent, DocType};

    /// A state with every outbound receiver kept, and one open Prose document
    /// whose opening answers are already drained.
    struct Rig {
        state: LogicState,
        ui: Receiver<UiEvent>,
        persist: Receiver<(DocumentId, PersistWrite)>,
        /// What local persistence was handed before the test began.
        saved: Vec<PersistWrite>,
        _sync: Receiver<SyncCommand>,
        document_id: DocumentId,
        block: ambre_bus::app::prose::BlockId,
    }

    /// A rig whose first save already went, so the next ones are deltas.
    fn rig() -> Rig {
        let mut rig = Rig::fresh();
        rig.state.handle_logic_command(keystroke(&rig, 0));
        rig.state.answer_pass();
        rig.ui.try_iter().for_each(drop);
        rig.saved = rig.persist.try_iter().map(|(_, w)| w).collect();
        rig
    }

    impl Rig {
        fn fresh() -> Rig {
            fresh_rig()
        }
    }

    fn fresh_rig() -> Rig {
        let (to_ui, ui) = test_link();
        let (to_sync, sync) = test_link();
        let (to_persist, persist) = test_link();
        let mut state = LogicState::new(to_ui, to_sync, to_persist);
        let document_id = DocumentId::new();
        state.handle_logic_command(LogicCommand::OpenDocument {
            document_id,
            doc_type: DocType::Prose,
        });
        state.answer_pass();
        let block = match &state.docs[&document_id].state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        ui.try_iter().for_each(drop);
        Rig {
            state,
            ui,
            persist,
            saved: Vec::new(),
            _sync: sync,
            document_id,
            block,
        }
    }

    fn keystroke(rig: &Rig, pos: u32) -> LogicCommand {
        LogicCommand::App {
            document_id: rig.document_id,
            command: AppCommand::Prose(ProseCommand::InsertText {
                block_id: rig.block,
                pos,
                text: "a".to_owned(),
            }),
        }
    }

    /// The text of every top-level block of a Prose document, in order.
    fn text(state: &DocState) -> String {
        let DocState::Prose(prose) = state else {
            unreachable!("a Prose document");
        };
        prose
            .root_blocks()
            .iter()
            .map(|block| {
                let runs = prose.resolved_runs(block).into_iter();
                format!("[{}]", runs.map(|(run, _)| run).collect::<String>())
            })
            .collect()
    }

    /// The text of the rig's document.
    fn dump(rig: &Rig) -> String {
        text(&rig.state.docs[&rig.document_id].state)
    }

    /// The document a restore of `writes` rebuilds, in a state of its own.
    fn restored(writes: Vec<PersistWrite>) -> String {
        let mut snapshot = None;
        let mut deltas = Vec::new();
        for write in writes {
            match write {
                PersistWrite::Snapshot(bytes) => {
                    snapshot = Some(bytes);
                    deltas.clear();
                }
                PersistWrite::Delta(bytes) => deltas.push(bytes),
            }
        }
        let (to_persist, _persist) = test_link();
        let mut state = LogicState::new(test_link().0, test_link().0, to_persist);
        let document_id = DocumentId::new();
        state.handle_logic_command(LogicCommand::RestoreLocalSnapshot {
            document_id,
            doc_type: DocType::Prose,
            snapshot,
            deltas,
        });
        text(&state.docs[&document_id].state)
    }

    /// `ARCH-017`: three keystrokes queued before the thread wakes cost one
    /// layout, and the layout carries all three. Each is logged as the delta
    /// it wrote, and no snapshot is exported.
    #[test]
    fn a_burst_of_keystrokes_is_answered_once() {
        let mut rig = rig();
        let (to_logic, from_ui) = ambre_bus::conduit::to_worker(64, &Doorbell::new());
        let (_to_logic_events, from_sync) =
            ambre_bus::conduit::to_worker::<LogicEvent>(64, &Doorbell::new());
        for pos in 0..3 {
            to_logic.try_send(keystroke(&rig, pos)).expect("queued");
        }
        rig.state.drain_pass(&from_ui, &from_sync);

        let layouts = rig
            .ui
            .try_iter()
            .filter(|event| {
                matches!(
                    event,
                    UiEvent::Viewport {
                        event: AppEvent::Prose(ProseEvent::LayoutShaped { .. }),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(layouts, 1, "one layout for the pass");
        let writes: Vec<PersistWrite> = rig.persist.try_iter().map(|(_, w)| w).collect();
        assert_eq!(writes.len(), 3, "one delta per keystroke");
        assert!(writes.iter().all(|w| matches!(w, PersistWrite::Delta(_))));
        let log = rig.saved.iter().cloned().chain(writes).collect();
        assert_eq!(restored(log), dump(&rig), "the deltas hold all three");
    }

    /// The first save of a session is a snapshot, which every delta after it
    /// builds on.
    #[test]
    fn the_first_save_is_a_snapshot() {
        let mut rig = Rig::fresh();
        rig.state.handle_logic_command(keystroke(&rig, 0));
        rig.state.answer_pass();
        let writes: Vec<PersistWrite> = rig.persist.try_iter().map(|(_, w)| w).collect();
        assert!(
            matches!(writes.as_slice(), [PersistWrite::Snapshot(_)]),
            "{writes:?}"
        );
        assert_eq!(restored(writes), dump(&rig));
    }

    /// Past [`DELTAS_PER_SNAPSHOT`] deltas the save is a snapshot, which the
    /// restore reads with the deltas logged after it.
    #[test]
    fn a_long_log_is_folded_into_a_snapshot() {
        let mut rig = rig();
        let mut writes = rig.saved.clone();
        for pos in 0..u32::try_from(DELTAS_PER_SNAPSHOT + 3).expect("small") {
            rig.state.handle_logic_command(keystroke(&rig, pos));
            rig.state.answer_pass();
            writes.extend(rig.persist.try_iter().map(|(_, w)| w));
        }
        let snapshots = writes
            .iter()
            .filter(|w| matches!(w, PersistWrite::Snapshot(_)))
            .count();
        assert_eq!(snapshots, 2, "the first save and one fold");
        assert_eq!(restored(writes), dump(&rig));
    }

    /// `ARCH-017`: an edit closed in the same pass is still saved, since the
    /// answer it is owed leaves before the state does.
    #[test]
    fn a_document_closed_in_the_pass_keeps_its_last_edit() {
        let mut rig = rig();
        let (to_logic, from_ui) = ambre_bus::conduit::to_worker(64, &Doorbell::new());
        let (_to_logic_events, from_sync) =
            ambre_bus::conduit::to_worker::<LogicEvent>(64, &Doorbell::new());
        to_logic.try_send(keystroke(&rig, 0)).expect("queued");
        to_logic
            .try_send(LogicCommand::CloseDocument {
                document_id: rig.document_id,
            })
            .expect("queued");
        let before = dump(&rig);
        rig.state.drain_pass(&from_ui, &from_sync);
        let writes: Vec<PersistWrite> = rig.persist.try_iter().map(|(_, w)| w).collect();
        assert!(
            matches!(writes.as_slice(), [PersistWrite::Snapshot(_)]),
            "the close folds the edit into a snapshot: {writes:?}"
        );
        assert!(!rig.state.docs.contains_key(&rig.document_id));
        assert_eq!(restored(writes).len(), before.len() + 1);
    }
}
