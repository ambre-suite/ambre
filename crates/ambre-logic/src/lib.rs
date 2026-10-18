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
// Transport: UI <-> Logic uses `ambre_bus::conduit` queues in the shared memory
// of the Main instance, carrying typed Rust values by move (no postcard, no
// `Any`). Logic-bound `SyncCommand`s are sent to a link whose receiver is on
// the Main thread, which relays them to the separate Sync worker via
// `postMessage`. `LogicEvent`s arrive the same way in reverse.
// ============================================================================

use std::collections::{HashMap, HashSet};

/// IndexedDB-backed local snapshot store. Compiled only for wasm, since its
/// callbacks need a live event loop. The store itself is driven by the Main
/// thread (see `ambre-app`), not by this blocking Logic loop.
#[cfg(target_arch = "wasm32")]
pub mod persistence;

// The `impl LogicState` block is spread across sibling modules:
//   - `data`:     application data plane (app commands, remote deltas, snapshots)
//   - `dispatch`: the two inbound dispatchers (+ their host-side tests)
//   - `channels`: Murmure channels, each a set of documents (the Murmure engine spec)
//   - `direct`:   Murmure direct messages, found or created by their members
mod channels;
mod data;
mod direct;
mod dispatch;
/// A history replay imported in slices (`SYNC-131`).
mod replay;
/// Which documents this thread holds, and the reopen of one that left
/// (`ambre-spec-residency.md`).
mod residency;
mod segments;
/// The snapshot check against the signed deltas (`AUTHOR-016`).
mod snapshot_check;
mod spaces;
/// What the local store holds, and which copies leave it (`RESID-002`).
pub mod store_ledger;
/// The threads of a channel: a reply, and the link of its thread.
mod threads;
/// The deltas that wait on an operation no delta carries (`AUTHOR-025`).
mod waiting;

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
    RestEvent, Role, Seq, SignedDelta, SnapshotId, SnapshotPurpose, SnapshotVerdict, SyncCommand,
    UiEvent, UserId,
};

use ambre_core::SessionNonce;
use ambre_core::schema::SchemaError;

use ambre_murmure::Order;
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
    /// `LogicState::export_covered_snapshot` cannot serve that belt: it takes a
    /// `DocumentId` and needs a live registry entry, where the belt quantifies
    /// over a bare `DocState`.
    pub fn export_snapshot(&self) -> Vec<u8> {
        match self {
            DocState::Trame(state) => state.export_snapshot(),
            DocState::Toile(state) => state.export_snapshot(),
            DocState::Prose(state) => state.export_snapshot(),
        }
    }

    /// The CRDT document, read-only, whichever application owns it. The
    /// snapshot check reads it and never writes (`AUTHOR-018`).
    fn crdt(&self) -> &loro::LoroDoc {
        match self {
            DocState::Trame(state) => state.crdt(),
            DocState::Toile(state) => state.crdt(),
            DocState::Prose(state) => state.crdt(),
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

    /// Write under the session identity Sync issued (`AUTHOR-002`), whichever
    /// application owns the document. `false` when the engine refused it.
    fn adopt_session(&mut self, peer: u64, entry: Vec<u8>) -> bool {
        match self {
            DocState::Trame(state) => state.adopt_session(peer, entry),
            DocState::Toile(state) => state.adopt_session(peer, entry),
            DocState::Prose(state) => state.adopt_session(peer, entry),
        }
    }
}

/// Whether every operation `payload` carries was written under `peer`, the
/// PeerID Sync derived from the delta's signer and nonce (`AUTHOR-012`).
///
/// Read before any engine imports the delta, from the change headers alone, so
/// a refusal leaves the document untouched (`AUTHOR-014`). Only an update blob
/// passes: a snapshot carries the whole history, every author's included.
/// Bytes the engine cannot read are refused too, since nothing in them can be
/// attributed.
fn authored_by(payload: &[u8], peer: u64) -> bool {
    match loro::LoroDoc::decode_import_blob_meta(payload, false) {
        Ok(meta) => {
            meta.mode == loro::EncodedBlobMode::Updates
                && meta.partial_end_vv.keys().all(|p| *p == peer)
        }
        Err(_) => false,
    }
}

/// [`authored_by`] for a payload of either form: a binary update, or the JSON
/// update a segment delta travels as (the Murmure messages spec).
#[cfg(test)]
fn sent_by(payload: &[u8], peer: u64) -> bool {
    authored_by(payload, peer)
        || serde_json::from_slice::<loro::json::JsonSchema>(payload).is_ok_and(|update| {
            !update.changes.is_empty() && update.changes.iter().all(|c| c.id.peer == peer)
        })
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
    /// The PeerID this session writes the document under, once Sync issued it
    /// (`AUTHOR-002`). Until then no edit is taken: an operation written under
    /// the provisional PeerID could never travel (`AUTHOR-008`).
    identity: Option<u64>,
    /// The local restores that arrived before the identity, applied once it
    /// lands. A restore may write under the document's PeerID, and that write
    /// must be one the session can send.
    held_restores: Vec<HeldRestore>,
    /// The edits made before the identity, applied under it once it lands
    /// (`AUTHOR-026`). None reaches the document before (`AUTHOR-029`).
    held_edits: Vec<AppCommand>,
    /// The relay's number of each delta applied since the last snapshot, and
    /// what it carries. A snapshot check reads it (`AUTHOR-016`).
    ledger: snapshot_check::SeqLedger,
    /// The deltas still missing a dependency (`AUTHOR-025`).
    waiting: waiting::Waiting,
    /// Set by `DocumentReady`: every delta the relay kept is replayed, so a
    /// delta still waiting below the ledger's contiguous end waits for ever.
    ready: bool,
    /// The operations this client has seen arrive from the relay: a lower
    /// bound of what the relay holds. An operation of an earlier session
    /// past it is sent again (`AUTHOR-028`).
    relay: loro::VersionVector,
    /// The earlier sessions of this user on the document, read back from the
    /// local store: each PeerID with the nonce it derives from.
    earlier: Vec<(u64, SessionNonce)>,
    /// The document reopens from the local store after leaving memory, at
    /// this horizon. Sync is asked to open it once the read lands
    /// (`RESID-010`).
    restoring: Option<u64>,
    /// The deltas of a replay not yet imported, one slice a pass
    /// (`SYNC-131`).
    replay: replay::ReplayQueue,
    /// `DocumentReady` landed while the replay was still being imported:
    /// what reads the whole replay runs after its last slice.
    ready_held: bool,
}

/// What a local restore leaves behind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Restore {
    /// Shown before the session identity, then dropped when the identity
    /// rebuilds the document (`AUTHOR-027`). Nothing of it is saved.
    Shown,
    /// Applied under the session identity, and saved like any other state.
    Kept,
}

/// A local restore held until the document has its session identity.
struct HeldRestore {
    snapshot: Option<Vec<u8>>,
    deltas: Vec<Vec<u8>>,
}

impl LiveDoc {
    fn new(state: DocState) -> Self {
        Self {
            state,
            logged: None,
            role: Role::Owner,
            identity: None,
            held_restores: Vec::new(),
            held_edits: Vec::new(),
            ledger: snapshot_check::SeqLedger::default(),
            waiting: waiting::Waiting::default(),
            ready: false,
            relay: loro::VersionVector::default(),
            earlier: Vec::new(),
            restoring: None,
            replay: replay::ReplayQueue::new(),
            ready_held: false,
        }
    }

    /// Note the operations a blob from the relay carries (`AUTHOR-028`). A
    /// PeerID's bound grows only when the blob starts at or below it, so a gap
    /// the relay never received is not counted as held. A blob the engine
    /// cannot read adds nothing, which keeps the bound low.
    fn relay_saw(&mut self, payload: &[u8]) {
        let Ok(meta) = loro::LoroDoc::decode_import_blob_meta(payload, false) else {
            return;
        };
        for (peer, end) in meta.partial_end_vv.iter() {
            let start = meta.partial_start_vv.get(peer).copied().unwrap_or(0);
            let held = self.relay.get(peer).copied().unwrap_or(0);
            if start <= held && *end > held {
                self.relay.insert(*peer, *end);
            }
        }
    }
}

#[cfg(test)]
impl LogicState {
    /// Open a document as the client does, Sync's answer included: the
    /// `OpenDocument`, then the session identity Sync issues for it.
    pub(crate) fn open_with_identity(&mut self, document_id: DocumentId, doc_type: DocType) {
        self.handle_logic_command(LogicCommand::OpenDocument {
            document_id,
            doc_type,
        });
        self.grant_identity(document_id);
    }

    /// The session identity Sync would issue for `document_id`, under a PeerID
    /// no other call of this test binary hands out.
    pub(crate) fn grant_identity(&mut self, document_id: DocumentId) -> u64 {
        let peer = test_peer();
        self.handle_logic_event(LogicEvent::SessionIdentity {
            document_id,
            peer,
            entry: peer.to_le_bytes().to_vec(),
            nonce: SessionNonce([0; 16]),
        });
        peer
    }
}

/// A PeerID no other call of this test binary hands out.
#[cfg(test)]
fn test_peer() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0x1000);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// `channel` with every document under a session identity of its own, as
/// another client writing it would hold it.
#[cfg(test)]
fn identified(mut channel: ambre_murmure::ChannelState) -> ambre_murmure::ChannelState {
    for document_id in channel.document_ids() {
        let peer = test_peer();
        channel
            .adopt_session(document_id, peer, peer.to_le_bytes().to_vec())
            .expect("adopt");
    }
    channel
}

/// How many deltas a document's local log holds before the next save folds
/// them into a snapshot. A keystroke then costs a delta, not a whole export,
/// while a restore never replays more than this many.
const DELTAS_PER_SNAPSHOT: usize = 256;

/// How many edits a document holds while it waits for its session identity
/// (`ambre-spec-budgets.md` section 6). The wait is one message from the Sync
/// worker, or the unlock of a session opened before it. Past the cap a new
/// edit is dropped and logged, so the ones held keep the order they were made
/// in.
const HELD_EDITS_LIMIT: usize = 256;

/// One write to local persistence, which the Main thread performs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistWrite {
    /// The whole document. It replaces the stored one and every delta logged
    /// since, which it holds.
    Snapshot(Vec<u8>),
    /// A change appended to the document's log, imported after the snapshot on
    /// restore.
    Delta(Vec<u8>),
    /// A session's PeerID and the nonce it derives from, kept so a later
    /// open sends what the session left unsent (`AUTHOR-028`).
    Session { peer: u64, nonce: SessionNonce },
    /// An earlier session whose operations the relay holds: nothing of it is
    /// left to send.
    ForgetSession { peer: u64 },
    /// Not a write: the document left memory and reopens. The Main thread
    /// reads it back and answers with a `RestoreLocalSnapshot`, empty when
    /// the store holds nothing (`RESID-010`, `RESID-012`).
    ReadBack { doc_type: DocType },
    /// Not a write: a view shows the document, so its copy never leaves the
    /// local store (`RESID-003`).
    Shown,
    /// Not a write: the document's view closed, so its copy may leave the
    /// local store once the relay holds every change (`RESID-008`).
    Hidden,
    /// Not a write: the relay holds every change of the document, so its
    /// copy is one the relay can serve again (`RESID-019`).
    Sent,
}

impl PersistWrite {
    /// Whether it writes to the store, rather than read it back or tell the
    /// store's ledger what the document is.
    #[must_use]
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Self::Snapshot(_) | Self::Delta(_) | Self::Session { .. } | Self::ForgetSession { .. }
        )
    }
}

// -----------------------------------------------------------------------------
// LogicState
// -----------------------------------------------------------------------------

/// All mutable state owned by the Logic thread.
struct LogicState {
    /// Outbound queue: Logic -> UI Thread.
    to_ui: WorkerSender<UiEvent>,
    /// Outbound queue: Logic -> Sync (drained by the Main thread, relayed to
    /// the Sync worker via postMessage).
    to_sync: WorkerSender<SyncCommand>,
    /// Outbound queue: Logic -> Main, carrying document snapshots and deltas
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
    /// Open Murmure channels, keyed by `channel_id`. Apart from `docs`: a
    /// channel is several documents, and a remote delta finds its channel by
    /// the document it names (the Murmure engine spec). Removed on `CloseChannel` and
    /// `Logout`.
    channels: HashMap<ambre_bus::ChannelId, ambre_murmure::ChannelState>,
    /// Channels on their way to open (the Murmure engine spec). `None` while
    /// the relay has not listed the documents yet. Moved to `channels` once
    /// every live document is in, and removed on a failure, `CloseChannel` or
    /// `Logout`.
    opening: HashMap<ambre_bus::ChannelId, Option<ambre_murmure::OpeningChannel>>,
    /// The relay's number of each delta a channel document took in, so its
    /// snapshot names the last delta it folds in (`SRV-083`). Keyed by
    /// document, dropped with its channel.
    channel_ledgers: HashMap<DocumentId, snapshot_check::SeqLedger>,
    /// This user's read cursor on each channel document (the Murmure unread
    /// spec). The relay's activity answer sets it, and this client's own
    /// marks move it. Cleared on `Logout`.
    read_cursors: HashMap<DocumentId, channels::ReadCursor>,
    /// Channels whose view closed: their documents left memory, and Sync
    /// counts them from their summaries alone (`RESID-004`). Removed on a
    /// reopen, a delete, a leave or `Logout`.
    closed_channels: HashMap<ambre_bus::ChannelId, channels::ClosedChannel>,
    /// The close each open channel asked of the Sync worker, with the time
    /// of the send that filled its segment (the Murmure segments spec).
    /// Removed on the answer, and with the channel.
    closing_segments: HashMap<ambre_bus::ChannelId, ambre_bus::Timestamp>,
    /// The successor each channel's close names, kept across a failed close
    /// so the next ask registers the one already created. Removed once the
    /// relay answers, and with the channel.
    close_successors: HashMap<ambre_bus::ChannelId, ambre_bus::SegmentId>,
    /// The edits and deletes a segment's lock refused, owed again as
    /// amendments. They leave at the channel's next tick, which stamps them.
    /// Removed with the channel.
    owed_replays: HashMap<ambre_bus::ChannelId, Vec<ambre_murmure::Replay>>,
    /// Who watches each open channel and who types in it, with this client's
    /// own samples (the Murmure typing and presence spec). Taken on the
    /// first tick, removed with the channel and on `Logout`.
    channel_watchers: HashMap<ambre_bus::ChannelId, ambre_murmure::Watchers>,
    /// The members of each open or opening channel, as Sync lists them for
    /// its `meta` (`SYNC-150`). The `@` list offers them, and a mention of
    /// anyone else notifies no one (the Murmure mentions spec). Removed with
    /// the channel and on `Logout`.
    channel_members: HashMap<ambre_bus::ChannelId, Vec<ambre_bus::MemberInfo>>,
    /// Open Murmure spaces, keyed by `space_id`: one document each, apart
    /// from `docs` (the Murmure spaces spec). Removed on `CloseSpace` and
    /// `Logout`.
    spaces: HashMap<ambre_bus::SpaceId, spaces::OpenSpace>,
    /// Spaces on their way to open, with the payloads and the member rows
    /// that arrived before their document was ready.
    space_opening: HashMap<ambre_bus::SpaceId, spaces::Opening>,
    /// Public channels the relay is creating, until it answers.
    creating: HashMap<ambre_bus::ChannelId, spaces::Creating>,
    /// Self-joins the relay has not answered yet, with their space.
    joining: HashMap<ambre_bus::ChannelId, ambre_bus::SpaceId>,
    /// Channel deletes the relay has not answered yet, with their space.
    deleting: HashMap<ambre_bus::ChannelId, ambre_bus::SpaceId>,
    /// Channel leaves the relay has not answered yet, with their space.
    leaving: HashMap<ambre_bus::ChannelId, ambre_bus::SpaceId>,
    /// Leaves of a space held while its owner role passes on (the Murmure
    /// spaces spec). Each is keyed by the role change this thread asked for,
    /// and holds the leave the view asked for.
    handing_off: HashMap<ambre_bus::RequestId, (ambre_bus::RequestId, DocumentId)>,
    /// Direct message requests the relay has not answered yet, keyed by the
    /// channel id each minted, with the channel it would create.
    direct: HashMap<ambre_bus::ChannelId, ambre_murmure::ChannelState>,
    /// The whole content of a document this client created, sent as its first
    /// update once the Sync worker holds its session.
    unsent: HashMap<DocumentId, Vec<u8>>,
    /// The session identity Sync issued for a channel or a space document
    /// that no open channel or space holds yet, adopted once one does
    /// (`AUTHOR-002`).
    murmure_identities: HashMap<DocumentId, (u64, Vec<u8>)>,
    /// The channel and space commands that write a document whose session
    /// identity has not landed, in the order they came (`AUTHOR-026`).
    murmure_waiting: HashMap<DocumentId, Vec<channels::HeldCommand>>,
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
    /// The documents that left memory when their view closed, each with the
    /// horizon its local copy holds. Sync keeps a light subscription for each
    /// (`RESID-004`), and a reopen resumes after the horizon (`RESID-010`).
    evicted: HashMap<DocumentId, u64>,
    /// The documents whose unsent change the browser refuses to store. While
    /// any is, no local edit is taken (`RESID-018`).
    store_full: HashSet<DocumentId>,
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
            channels: HashMap::new(),
            opening: HashMap::new(),
            channel_ledgers: HashMap::new(),
            read_cursors: HashMap::new(),
            closed_channels: HashMap::new(),
            closing_segments: HashMap::new(),
            close_successors: HashMap::new(),
            owed_replays: HashMap::new(),
            channel_watchers: HashMap::new(),
            channel_members: HashMap::new(),
            spaces: HashMap::new(),
            space_opening: HashMap::new(),
            creating: HashMap::new(),
            joining: HashMap::new(),
            deleting: HashMap::new(),
            leaving: HashMap::new(),
            handing_off: HashMap::new(),
            direct: HashMap::new(),
            unsent: HashMap::new(),
            murmure_identities: HashMap::new(),
            murmure_waiting: HashMap::new(),
            my_user_id: None,
            my_display_name: None,
            rest_routes: HashMap::new(),
            pending_seals: HashMap::new(),
            base_faces: Vec::new(),
            export_cx: None,
            owed: HashMap::new(),
            evicted: HashMap::new(),
            store_full: HashSet::new(),
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
    ///
    /// A board whose import was running when the pass began then takes one
    /// more slice of it (`ARCH-030`). An import the pass started already
    /// applied its first slice, so no pass applies two. A history replay
    /// takes its next slice the same way (`SYNC-131`).
    fn drain_pass(&mut self, from_ui: &Receiver<LogicCommand>, from_sync: &Receiver<LogicEvent>) {
        let importing = self.boards_with_pending_work();
        let replaying = self.docs_replaying();
        for command in from_ui.try_iter().take(from_ui.len()) {
            self.handle_logic_command(command);
        }
        for event in from_sync.try_iter().take(from_sync.len()) {
            self.handle_logic_event(event);
        }
        for document_id in importing {
            self.resume_import(document_id);
        }
        for document_id in replaying {
            self.resume_replay(document_id);
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
// Entry point: drain both inbound queues, then park on the doorbell.
// -----------------------------------------------------------------------------

/// Run the Logic thread to completion.
///
/// Binds `bell`, the doorbell both inbound queues ring, then loops. While
/// nothing is queued it parks. Once a message is queued it applies every
/// message waiting, then answers each document it wrote once (`ARCH-017`).
/// Exits when **both** inbound queues are drained and disconnected (the UI
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
        // The thread cannot wait for work, so it ends here. The UI thread
        // hears why before the channels close (`ARCH-009`).
        let message = "the Logic doorbell is bound to another thread";
        tracing::error!("Ambre Logic thread: {message}");
        let _ = to_ui.try_send(UiEvent::FatalError {
            message: message.to_owned(),
        });
        return;
    };
    let mut state = LogicState::new(to_ui, to_sync, to_persist);

    loop {
        // A board still importing brings the loop back without a message: the
        // next pass applies its next slice (`ARCH-030`).
        while from_ui.is_empty() && from_sync.is_empty() && !state.has_pending_work() {
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

/// A delta the relay delivers at `seq`, signed under the one PeerID its
/// operations carry, as Sync would hand it over.
#[cfg(test)]
fn remote_delta(seq: u64, payload: Vec<u8>) -> ambre_bus::SignedDelta {
    let peer = loro::LoroDoc::decode_import_blob_meta(&payload, false)
        .ok()
        .and_then(|meta| meta.partial_end_vv.keys().next().copied())
        .unwrap_or_default();
    ambre_bus::SignedDelta {
        seq,
        payload,
        nonce: ambre_core::SessionNonce([0; 16]),
        peer,
        user: None,
        summary: None,
    }
}

/// `ARCH-009`: a Logic thread that cannot bind its doorbell says so to the
/// UI thread before it returns. `ARCH-015`: on that way out it sends nothing
/// to the relay and writes nothing to local persistence.
#[cfg(test)]
#[test]
fn a_logic_thread_that_cannot_wait_emits_a_fatal_event() {
    let bell = Doorbell::new();
    let taken = bell.clone();
    let bound = std::thread::spawn(move || taken.bind().is_some()).join();
    assert!(matches!(bound, Ok(true)), "another thread holds the bell");
    let (_from_ui_tx, from_ui) = ambre_bus::conduit::local(4);
    let (to_ui, ui) = test_link();
    let (to_sync, sync) = test_link();
    let (_from_sync_tx, from_sync) = ambre_bus::conduit::local(4);
    let (to_persist, persist) = test_link();
    run(bell, from_ui, to_ui, to_sync, from_sync, to_persist);
    let told: Vec<_> = ui.try_iter().collect();
    assert!(
        matches!(told.as_slice(), [UiEvent::FatalError { .. }]),
        "one fatal event, then the end: {told:?}"
    );
    assert!(
        sync.try_recv().is_err(),
        "the failing thread sent to the relay"
    );
    assert!(
        persist.try_recv().is_err(),
        "the failing thread wrote the document"
    );
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
        rig.saved = rig
            .persist
            .try_iter()
            .map(|(_, w)| w)
            .filter(PersistWrite::is_write)
            .collect();
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
        state.open_with_identity(document_id, DocType::Prose);
        state.answer_pass();
        let block = match &state.docs[&document_id].state {
            DocState::Prose(prose) => prose.root_blocks()[0],
            other => panic!("the registry opened {other:?}"),
        };
        ui.try_iter().for_each(drop);
        // The session saved at its identity is not a save of the document.
        assert!(
            persist
                .try_iter()
                .all(|(_, w)| !w.is_write() || matches!(w, PersistWrite::Session { .. }))
        );
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
            panic!("the rig holds a Prose document");
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
                PersistWrite::Session { .. }
                | PersistWrite::ForgetSession { .. }
                | PersistWrite::ReadBack { .. }
                | PersistWrite::Shown
                | PersistWrite::Hidden
                | PersistWrite::Sent => {}
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
            sessions: Vec::new(),
        });
        state.grant_identity(document_id);
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
        let writes: Vec<PersistWrite> = rig
            .persist
            .try_iter()
            .map(|(_, w)| w)
            .filter(PersistWrite::is_write)
            .collect();
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
        let writes: Vec<PersistWrite> = rig
            .persist
            .try_iter()
            .map(|(_, w)| w)
            .filter(PersistWrite::is_write)
            .collect();
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
            writes.extend(
                rig.persist
                    .try_iter()
                    .map(|(_, w)| w)
                    .filter(PersistWrite::is_write),
            );
        }
        let snapshots = writes
            .iter()
            .filter(|w| matches!(w, PersistWrite::Snapshot(_)))
            .count();
        assert_eq!(snapshots, 2, "the first save and one fold");
        assert_eq!(restored(writes), dump(&rig));
    }

    /// `ARCH-027`: a document closed in the pass that edited it folds the edit
    /// into its closing snapshot. The answer the edit is owed leaves before the
    /// state does.
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
        let writes: Vec<PersistWrite> = rig
            .persist
            .try_iter()
            .map(|(_, w)| w)
            .filter(PersistWrite::is_write)
            .collect();
        assert!(
            matches!(writes.as_slice(), [PersistWrite::Snapshot(_)]),
            "the close folds the edit into a snapshot: {writes:?}"
        );
        assert!(!rig.state.docs.contains_key(&rig.document_id));
        assert_eq!(restored(writes).len(), before.len() + 1);
    }

    /// An SVG of `n` rectangles, more than one import slice holds.
    fn drawing(n: usize) -> Vec<u8> {
        let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg">"#);
        for i in 0..n {
            svg.push_str(&format!(r#"<rect x="{i}" y="0" width="5" height="5"/>"#));
        }
        svg.push_str("</svg>");
        svg.into_bytes()
    }

    /// A state with one open board, and the queues a pass drains.
    struct Board {
        state: LogicState,
        document_id: DocumentId,
        to_logic: ambre_bus::conduit::Sender<LogicCommand>,
        from_ui: Receiver<LogicCommand>,
        from_sync: Receiver<LogicEvent>,
        persist: Receiver<(DocumentId, PersistWrite)>,
    }

    fn board() -> Board {
        let (to_persist, persist) = test_link();
        let mut state = LogicState::new(test_link().0, test_link().0, to_persist);
        let document_id = DocumentId::new();
        state.open_with_identity(document_id, DocType::Toile);
        state.answer_pass();
        let (to_logic, from_ui) = ambre_bus::conduit::to_worker(64, &Doorbell::new());
        let (_to_logic_events, from_sync) =
            ambre_bus::conduit::to_worker::<LogicEvent>(64, &Doorbell::new());
        Board {
            state,
            document_id,
            to_logic,
            from_ui,
            from_sync,
            persist,
        }
    }

    fn import(document_id: DocumentId, n: usize) -> LogicCommand {
        LogicCommand::App {
            document_id,
            command: AppCommand::Toile(ambre_bus::app::toile::ToileCommand::ImportSvg {
                bytes: drawing(n),
                at: kurbo::Point::ZERO,
            }),
        }
    }

    fn elements(state: &LogicState, document_id: DocumentId) -> usize {
        match &state.docs[&document_id].state {
            DocState::Toile(board) => board.element_count(),
            other => panic!("the registry opened {other:?}"),
        }
    }

    /// `ARCH-030`: a large import takes several passes, and the loop needs
    /// no message to come back for the next slice. Each pass applies one slice,
    /// so it ends long before the import does.
    #[test]
    fn a_large_import_lands_over_several_passes() {
        let Board {
            mut state,
            document_id,
            to_logic,
            from_ui,
            from_sync,
            ..
        } = board();
        to_logic
            .try_send(import(document_id, 1_000))
            .expect("queued");
        state.drain_pass(&from_ui, &from_sync);
        let first = elements(&state, document_id);
        assert!(first > 0 && first < 1_000, "one slice landed: {first}");
        assert!(state.has_pending_work(), "the loop comes back unasked");
        let mut passes = 1;
        while state.has_pending_work() {
            state.drain_pass(&from_ui, &from_sync);
            passes += 1;
        }
        assert!(passes > 2, "the import took {passes} passes");
        assert_eq!(elements(&state, document_id), 1_000);
    }

    /// `ARCH-031`: a board closed mid-import saves the whole import, never
    /// half of it.
    #[test]
    fn a_board_closed_mid_import_saves_it_whole() {
        let Board {
            mut state,
            document_id,
            to_logic,
            from_ui,
            from_sync,
            persist,
        } = board();
        to_logic
            .try_send(import(document_id, 1_000))
            .expect("queued");
        state.drain_pass(&from_ui, &from_sync);
        assert!(state.has_pending_work());
        to_logic
            .try_send(LogicCommand::CloseDocument { document_id })
            .expect("queued");
        state.drain_pass(&from_ui, &from_sync);
        assert!(!state.has_pending_work());
        let snapshot = persist
            .try_iter()
            .filter_map(|(_, write)| match write {
                PersistWrite::Snapshot(bytes) => Some(bytes),
                _ => None,
            })
            .last()
            .expect("the close saves a snapshot");
        let mut reopened = board();
        reopened
            .state
            .apply_remote(reopened.document_id, Order::Snapshot(Seq::ZERO), &snapshot);
        assert_eq!(elements(&reopened.state, reopened.document_id), 1_000);
    }

    /// `ARCH-031`: a peer's delta that arrives mid-import lands the import
    /// first. Merged inside it, the delta would end the import's undo group,
    /// and one Undo would no longer take the whole file back.
    #[test]
    fn a_remote_delta_mid_import_lands_the_import_first() {
        let Board {
            mut state,
            document_id,
            to_logic,
            from_ui,
            from_sync,
            ..
        } = board();
        to_logic
            .try_send(import(document_id, 1_000))
            .expect("queued");
        state.drain_pass(&from_ui, &from_sync);
        assert!(state.has_pending_work());
        let mut peer = ambre_toile::ToileState::new();
        let delta = peer
            .handle_command(ambre_bus::app::toile::ToileCommand::ImportSvg {
                bytes: drawing(1),
                at: kurbo::Point::new(-50.0, 0.0),
            })
            .update
            .expect("the peer's delta");
        state.apply_remote(document_id, Order::Update(Seq(1)), &delta);
        assert!(!state.has_pending_work(), "the import landed first");
        assert_eq!(elements(&state, document_id), 1_001);
        state.handle_logic_command(LogicCommand::App {
            document_id,
            command: AppCommand::Toile(ambre_bus::app::toile::ToileCommand::Undo),
        });
        assert_eq!(
            elements(&state, document_id),
            1,
            "one Undo takes the file back"
        );
    }
}

#[cfg(test)]
mod authorship_tests {
    //! The authorship check on the Logic side (`ambre-spec-op-authorship.md`):
    //! Sync derived the PeerID from the signer, and Logic compares it with
    //! every operation a delta carries.

    use super::*;
    use ambre_bus::TrameCommand;
    use ambre_core::SessionNonce;
    use ambre_core::authorship::{PEERS_MAP, peer_key};
    use loro::{ExportMode, LoroDoc, LoroValue, VersionVector};

    const ALICE: u64 = 0xA11CE;
    const MALLORY: u64 = 0xBAD;

    struct Rig {
        state: LogicState,
        sync: Receiver<SyncCommand>,
        ui: Receiver<UiEvent>,
        document_id: DocumentId,
        peer: u64,
    }

    /// One open workbook holding its session identity, its opening drained.
    fn rig() -> Rig {
        let (to_ui, ui) = test_link();
        let (to_sync, sync) = test_link();
        let (to_persist, persist) = test_link();
        std::mem::forget(persist);
        let mut state = LogicState::new(to_ui, to_sync, to_persist);
        let document_id = DocumentId::new();
        state.handle_logic_command(LogicCommand::OpenDocument {
            document_id,
            doc_type: DocType::Trame,
        });
        let peer = state.grant_identity(document_id);
        sync.try_iter().for_each(drop);
        Rig {
            state,
            sync,
            ui,
            document_id,
            peer,
        }
    }

    /// The delta a session writing under `peer` sends for one cell edit.
    fn written_by(peer: u64, text: &str) -> Vec<u8> {
        let mut author = TrameState::new();
        assert!(author.adopt_session(peer, peer.to_le_bytes().to_vec()));
        author
            .handle_command(TrameCommand::SetCellText {
                row: 0,
                col: 0,
                text: text.to_owned(),
            })
            .update
            .expect("an edit writes a delta")
    }

    /// One delta carrying the operations of every delta in `deltas`.
    fn merged(deltas: &[Vec<u8>]) -> Vec<u8> {
        let doc = LoroDoc::new();
        doc.import(&TrameState::new().export_snapshot())
            .expect("the scaffolding");
        let base = doc.oplog_vv();
        for delta in deltas {
            doc.import(delta).expect("a delta");
        }
        doc.export(ExportMode::updates(&base)).expect("an export")
    }

    fn signed(payload: Vec<u8>, peer: u64) -> SignedDelta {
        SignedDelta {
            seq: 0,
            payload,
            nonce: SessionNonce([0; 16]),
            peer,
            user: None,
            summary: None,
        }
    }

    /// Every operation the document holds, read back from its snapshot.
    fn held(rig: &Rig) -> VersionVector {
        let doc = LoroDoc::new();
        doc.import(&rig.state.docs[&rig.document_id].state.export_snapshot())
            .expect("the document's own snapshot");
        doc.oplog_vv()
    }

    fn edit(rig: &mut Rig, text: &str) -> Vec<u8> {
        rig.state.handle_logic_command(LogicCommand::App {
            document_id: rig.document_id,
            command: AppCommand::Trame(TrameCommand::SetCellText {
                row: 1,
                col: 1,
                text: text.to_owned(),
            }),
        });
        rig.sync
            .try_iter()
            .find_map(|cmd| match cmd {
                SyncCommand::SendUpdate { payload, .. } => Some(payload),
                _ => None,
            })
            .expect("a local edit sends a delta")
    }

    /// The PeerIDs whose operations a delta carries.
    fn authors(delta: &[u8]) -> Vec<u64> {
        let meta = LoroDoc::decode_import_blob_meta(delta, false).expect("a readable delta");
        meta.partial_end_vv.keys().copied().collect()
    }

    /// `AUTHOR-012`, `AUTHOR-014`: a delta Mallory signs over Alice's
    /// operation is refused, and so is one that mixes both. Neither changes
    /// the document, its registry or its export cursor.
    #[test]
    fn a_delta_carrying_an_operation_not_its_signers_is_refused_whole() {
        let mut rig = rig();
        let before = held(&rig);
        let forged = written_by(ALICE, "forged");
        let mixed = merged(&[written_by(MALLORY, "mine"), written_by(ALICE, "hers")]);
        rig.state.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: rig.document_id,
            delta: signed(forged, MALLORY),
        });
        rig.state.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: rig.document_id,
            delta: signed(mixed.clone(), MALLORY),
        });
        rig.state.handle_logic_event(LogicEvent::RemoteUpdateBatch {
            document_id: rig.document_id,
            deltas: vec![signed(mixed, MALLORY)],
        });
        assert_eq!(held(&rig), before, "a refused delta reached the document");
        assert!(rig.sync.try_recv().is_err(), "a refusal sent something");
        // The cursor did not move: the next edit carries this session alone.
        assert_eq!(authors(&edit(&mut rig, "after")), vec![rig.peer]);
    }

    /// `AUTHOR-012`: the delta its signer wrote is imported, and in a batch
    /// only the refused delta is left out.
    #[test]
    fn a_delta_its_signer_wrote_reaches_the_document() {
        let mut rig = rig();
        rig.state.handle_logic_event(LogicEvent::RemoteUpdateBatch {
            document_id: rig.document_id,
            deltas: vec![
                signed(written_by(ALICE, "hers"), ALICE),
                signed(written_by(MALLORY, "forged"), ALICE),
            ],
        });
        let held = held(&rig);
        assert!(held.get(&ALICE).is_some(), "the honest delta was dropped");
        assert!(held.get(&MALLORY).is_none(), "the forged delta landed");
    }

    /// `AUTHOR-007`: the verdict reads the delta and its signer alone, so two
    /// peers fed the same deltas keep the same ones.
    #[test]
    fn two_peers_given_the_same_deltas_keep_the_same_ones() {
        let deltas = [
            signed(written_by(ALICE, "hers"), ALICE),
            signed(written_by(ALICE, "forged"), MALLORY),
            signed(
                merged(&[written_by(MALLORY, "a"), written_by(ALICE + 1, "b")]),
                MALLORY,
            ),
        ];
        let kept: Vec<VersionVector> = (0..2)
            .map(|_| {
                let mut rig = rig();
                for delta in deltas.clone() {
                    rig.state.handle_logic_event(LogicEvent::RemoteUpdate {
                        document_id: rig.document_id,
                        delta,
                    });
                }
                let mut held = held(&rig);
                held.remove(&rig.peer);
                held
            })
            .collect();
        assert_eq!(kept.first(), kept.get(1));
    }

    /// `ARCH-013`: two tabs of one user are two sessions under two PeerIDs.
    /// They edit one cell at once, each takes the other's delta as the relay
    /// hands it on, and both end on the same document.
    #[test]
    fn two_tabs_of_one_user_converge_as_two_users_do() {
        let user = Some(UserId::new());
        let mut tabs = [rig(), rig()];
        assert_ne!(tabs[0].peer, tabs[1].peer, "two tabs share a PeerID");
        let sent = [edit(&mut tabs[0], "left"), edit(&mut tabs[1], "right")];
        for (tab, other) in [(0, 1), (1, 0)] {
            let document_id = tabs[tab].document_id;
            let mut delta = signed(sent[other].clone(), tabs[other].peer);
            delta.user = user;
            tabs[tab]
                .state
                .handle_logic_event(LogicEvent::RemoteUpdate { document_id, delta });
        }
        let read = |rig: &Rig| {
            let doc = LoroDoc::new();
            doc.import(&rig.state.docs[&rig.document_id].state.export_snapshot())
                .expect("the document's own snapshot");
            (doc.oplog_vv(), doc.get_deep_value())
        };
        let [left, right] = tabs.each_ref().map(read);
        assert_eq!(left, right, "the two tabs diverged");
        assert!(left.0.get(&tabs[0].peer).is_some() && left.0.get(&tabs[1].peer).is_some());
    }

    /// `AUTHOR-008`, `AUTHOR-009`, `AUTHOR-011`: a session that never edits
    /// sends nothing. Its first delta writes its own registry entry and
    /// carries its own operations alone, though the document holds Alice's.
    #[test]
    fn the_first_delta_carries_the_registry_entry_and_this_sessions_operations_alone() {
        let mut rig = rig();
        let hers = written_by(ALICE, "hers");
        rig.state.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: rig.document_id,
            delta: signed(hers.clone(), ALICE),
        });
        assert!(rig.sync.try_recv().is_err(), "an unedited session sent");

        let first = edit(&mut rig, "mine");
        assert_eq!(authors(&first), vec![rig.peer]);
        let reader = LoroDoc::new();
        reader
            .import(&TrameState::new().export_snapshot())
            .expect("the scaffolding");
        reader.import(&hers).expect("Alice's delta");
        let registry = |doc: &LoroDoc| match doc.get_map(PEERS_MAP).get_deep_value() {
            LoroValue::Map(map) => (*map).clone(),
            _ => Default::default(),
        };
        let hers_only = registry(&reader);
        reader.import(&first).expect("the first delta");
        let after = registry(&reader);
        assert_eq!(
            after.get(&peer_key(rig.peer)),
            Some(&LoroValue::Binary(rig.peer.to_le_bytes().to_vec().into()))
        );
        assert_eq!(
            after.len(),
            hers_only.len() + 1,
            "another entry was written"
        );
        assert_eq!(after.get(&peer_key(ALICE)), hers_only.get(&peer_key(ALICE)));

        let second = edit(&mut rig, "again");
        assert_eq!(authors(&second), vec![rig.peer]);
    }

    /// A workbook snapshot holding the scaffolding and `deltas`.
    fn snapshot_of(deltas: &[Vec<u8>]) -> Vec<u8> {
        let doc = LoroDoc::new();
        doc.import(&TrameState::new().export_snapshot())
            .expect("the scaffolding");
        for delta in deltas {
            doc.import(delta).expect("a delta");
        }
        doc.export(ExportMode::Snapshot).expect("a snapshot")
    }

    /// Alice's delta, numbered `seq` by the relay, reaches the rig.
    fn receive(rig: &mut Rig, payload: Vec<u8>, seq: u64) {
        let mut delta = signed(payload, ALICE);
        delta.seq = seq;
        rig.state.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id: rig.document_id,
            delta,
        });
    }

    /// The verdict Logic answers a pending snapshot covering `covered` with.
    fn verdict(rig: &mut Rig, payload: Vec<u8>, covered: u64) -> SnapshotVerdict {
        rig.state.handle_logic_event(LogicEvent::PendingSnapshot {
            document_id: rig.document_id,
            snapshot_id: SnapshotId::genesis(rig.document_id),
            covered_seq: covered,
            payload,
            purpose: SnapshotPurpose::Confirm,
        });
        rig.sync
            .try_iter()
            .find_map(|cmd| match cmd {
                SyncCommand::SnapshotVerdict { verdict, .. } => Some(verdict),
                _ => None,
            })
            .expect("a pending snapshot is answered")
    }

    /// A delta Alice writes on a scaffolding another build wrote under `peer`.
    fn on_foreign_scaffold(peer: u64) -> Vec<u8> {
        let doc = LoroDoc::new();
        doc.set_peer_id(peer)
            .expect("a fresh document takes a peer id");
        doc.get_map("meta")
            .insert("schema_version", 99)
            .expect("an insert");
        doc.commit();
        let base = doc.oplog_vv();
        doc.set_peer_id(ALICE).expect("a peer id");
        doc.get_map("meta").insert("note", "x").expect("an insert");
        doc.commit();
        doc.export(ExportMode::updates(&base)).expect("an export")
    }

    /// Whether the UI was told the document cannot be read.
    fn told_unreadable(rig: &Rig) -> bool {
        rig.ui.try_iter().any(|event| {
            matches!(
                event,
                UiEvent::SyncWarning {
                    code: ambre_bus::sync_warning::SyncWarningCode::DocumentUnreadable,
                    ..
                }
            )
        })
    }

    fn ready(rig: &mut Rig) {
        rig.state.handle_logic_event(LogicEvent::DocumentReady {
            document_id: rig.document_id,
            role: Role::Owner,
        });
    }

    /// `AUTHOR-025`: a delta built on a scaffolding this client never wrote
    /// is reported once the replay is done, and only then. A delta on this
    /// build's scaffolding is not.
    #[test]
    fn a_delta_waiting_on_a_foreign_scaffold_reports_the_document() {
        let mut rig = rig();
        receive(&mut rig, written_by(ALICE, "ours"), 1);
        receive(&mut rig, on_foreign_scaffold(0x5CAF), 2);
        assert!(
            !told_unreadable(&rig),
            "reported before the replay was done"
        );
        ready(&mut rig);
        assert!(told_unreadable(&rig), "the document was not reported");
        receive(&mut rig, on_foreign_scaffold(0x5CAF), 3);
        assert!(!told_unreadable(&rig), "the document was reported twice");

        let mut clean = rig_with_ready();
        receive(&mut clean, written_by(ALICE, "ours"), 1);
        assert!(!told_unreadable(&clean), "a readable delta was reported");
    }

    /// A delta waiting on one the client does not hold yet is not reported:
    /// the gap may still fill.
    #[test]
    fn a_delta_past_a_gap_is_not_reported() {
        let mut rig = rig_with_ready();
        receive(&mut rig, on_foreign_scaffold(0x5CAF), 3);
        assert!(!told_unreadable(&rig));
    }

    fn rig_with_ready() -> Rig {
        let mut rig = rig();
        ready(&mut rig);
        rig.ui.try_iter().for_each(drop);
        rig
    }

    /// `AUTHOR-016`, `AUTHOR-017`, `AUTHOR-018`: a pending snapshot holding
    /// the covered delta matches, a forged one does not, and neither reaches
    /// the document.
    #[test]
    fn a_pending_snapshot_is_answered_and_never_imported() {
        let mut rig = rig();
        let hers = written_by(ALICE, "hers");
        receive(&mut rig, hers.clone(), 1);
        let before = held(&rig);
        let forged = snapshot_of(&[written_by(ALICE, "forged")]);
        assert_eq!(verdict(&mut rig, forged, 1), SnapshotVerdict::Mismatch);
        assert_eq!(
            verdict(&mut rig, snapshot_of(&[hers]), 1),
            SnapshotVerdict::Match { extras: vec![] }
        );
        assert_eq!(held(&rig), before, "a pending snapshot was imported");
    }

    /// `AUTHOR-015`: a snapshot adopted at a horizon answers for the deltas
    /// below it, so a later one covering less is left unchecked.
    #[test]
    fn an_adopted_snapshot_moves_the_horizon() {
        let mut rig = rig();
        let hers = written_by(ALICE, "hers");
        rig.state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id: rig.document_id,
            payload: snapshot_of(std::slice::from_ref(&hers)),
            covered_seq: 5,
        });
        assert_eq!(
            verdict(&mut rig, snapshot_of(&[hers]), 3),
            SnapshotVerdict::Unchecked
        );
    }

    /// `SRV-083`: a snapshot this client builds names the last delta it holds
    /// with no gap below it, so no trim drops a delta it lacks.
    #[test]
    fn a_built_snapshot_names_the_last_delta_held_without_a_gap() {
        let mut rig = rig();
        receive(&mut rig, written_by(ALICE, "one"), 1);
        receive(&mut rig, written_by(ALICE, "three"), 3);
        rig.state.handle_logic_event(LogicEvent::SnapshotNeeded {
            document_id: rig.document_id,
        });
        let covered = rig.sync.try_iter().find_map(|cmd| match cmd {
            SyncCommand::SendSnapshot { covered_seq, .. } => Some(covered_seq),
            _ => None,
        });
        assert_eq!(covered, Some(1));
    }

    /// One open document still waiting for its session identity, with the
    /// receiver of its local persistence.
    fn unidentified(doc_type: DocType) -> (Rig, Receiver<(DocumentId, PersistWrite)>) {
        let (to_ui, ui) = test_link();
        let (to_sync, sync) = test_link();
        let (to_persist, persist) = test_link();
        let mut state = LogicState::new(to_ui, to_sync, to_persist);
        let document_id = DocumentId::new();
        state.handle_logic_command(LogicCommand::OpenDocument {
            document_id,
            doc_type,
        });
        sync.try_iter().for_each(drop);
        ui.try_iter().for_each(drop);
        let rig = Rig {
            state,
            sync,
            ui,
            document_id,
            peer: 0,
        };
        (rig, persist)
    }

    /// `AUTHOR-026`, `AUTHOR-029`: an edit made before the identity reaches
    /// neither the document nor Sync, and travels under the identity once it
    /// lands.
    #[test]
    fn an_edit_before_the_identity_is_applied_under_it() {
        let (mut rig, _persist) = unidentified(DocType::Trame);
        let before = held(&rig);
        rig.state.handle_logic_command(LogicCommand::App {
            document_id: rig.document_id,
            command: AppCommand::Trame(TrameCommand::SetCellText {
                row: 1,
                col: 1,
                text: "early".to_owned(),
            }),
        });
        assert_eq!(held(&rig), before, "a held edit reached the document");
        assert!(rig.sync.try_recv().is_err(), "a held edit was sent");

        let peer = rig.state.grant_identity(rig.document_id);
        let sent: Vec<Vec<u8>> = rig
            .sync
            .try_iter()
            .filter_map(|cmd| match cmd {
                SyncCommand::SendUpdate { payload, .. } => Some(payload),
                _ => None,
            })
            .collect();
        assert_eq!(sent.len(), 1, "the held edit did not travel once");
        assert_eq!(
            sent.iter().flat_map(|d| authors(d)).collect::<Vec<_>>(),
            vec![peer]
        );
    }

    /// A page Alice emptied of every block. Restoring it grows a paragraph, a
    /// write under whatever PeerID the document holds then (`PROSE-027`).
    fn emptied_page() -> Vec<u8> {
        let doc = LoroDoc::new();
        doc.set_peer_id(ALICE).expect("a PeerID");
        let page = DocState::new(DocType::Prose).expect("a page");
        doc.import(&page.export_snapshot())
            .expect("the scaffolding");
        let tree = doc.get_tree("blocks");
        for root in tree.roots() {
            tree.delete(root).expect("a block deleted");
        }
        doc.commit();
        doc.export(ExportMode::Snapshot).expect("a snapshot")
    }

    /// `AUTHOR-027`: a cached document shows before the identity and saves
    /// nothing. Once the identity lands, the document holds the cache and
    /// nothing written under a PeerID but the cache's and the session's.
    #[test]
    fn a_cached_document_shows_before_the_identity_and_is_rebuilt_under_it() {
        let logged = written_by(ALICE + 1, "logged");
        let cases = [
            (
                DocType::Trame,
                snapshot_of(&[written_by(ALICE, "hers")]),
                vec![logged],
            ),
            (DocType::Prose, emptied_page(), Vec::new()),
        ];
        for (doc_type, cached, deltas) in cases {
            let (mut rig, persist) = unidentified(doc_type);
            let restored = {
                let doc = LoroDoc::new();
                doc.import(&cached).expect("the cached snapshot");
                for delta in &deltas {
                    doc.import(delta).expect("a logged delta");
                }
                doc.oplog_vv()
            };
            rig.state
                .handle_logic_command(LogicCommand::RestoreLocalSnapshot {
                    document_id: rig.document_id,
                    doc_type,
                    snapshot: Some(cached),
                    deltas,
                    sessions: Vec::new(),
                });
            assert!(
                rig.ui
                    .try_iter()
                    .any(|e| matches!(e, UiEvent::Viewport { .. })),
                "{doc_type:?}: the cache was not shown before the identity"
            );
            assert!(
                !persist.try_iter().any(|(_, w)| w.is_write()),
                "{doc_type:?}: a shown restore was saved"
            );

            let peer = rig.state.grant_identity(rig.document_id);
            let held = held(&rig);
            assert!(
                held.includes_vv(&restored),
                "{doc_type:?}: the cache was lost"
            );
            let strangers: Vec<u64> = held
                .keys()
                .copied()
                .filter(|p| *p != peer && restored.get(p).is_none())
                .collect();
            assert!(
                strangers.is_empty(),
                "{doc_type:?}: written under {strangers:x?}"
            );
        }
    }

    /// The deltas one session writing under `peer` sends, one per text.
    fn session_wrote(peer: u64, texts: &[&str]) -> Vec<Vec<u8>> {
        let mut author = TrameState::new();
        assert!(author.adopt_session(peer, peer.to_le_bytes().to_vec()));
        texts
            .iter()
            .map(|text| {
                author
                    .handle_command(TrameCommand::SetCellText {
                        row: 0,
                        col: 0,
                        text: (*text).to_owned(),
                    })
                    .update
                    .expect("an edit writes a delta")
            })
            .collect()
    }

    /// A document cached with both of Alice's deltas and her session, then
    /// identified. The relay replays the first `relayed` of her deltas before
    /// the replay ends. Returns what Logic sent and saved.
    fn reopened(relayed: usize) -> (Vec<SyncCommand>, Vec<PersistWrite>, Vec<Vec<u8>>, u64) {
        let hers = session_wrote(ALICE, &["one", "two"]);
        let (mut rig, persist) = unidentified(DocType::Trame);
        rig.state
            .handle_logic_command(LogicCommand::RestoreLocalSnapshot {
                document_id: rig.document_id,
                doc_type: DocType::Trame,
                snapshot: Some(snapshot_of(&hers)),
                deltas: Vec::new(),
                sessions: vec![(ALICE, SessionNonce([7; 16]))],
            });
        let peer = rig.state.grant_identity(rig.document_id);
        for (seq, delta) in hers.iter().take(relayed).enumerate() {
            receive(&mut rig, delta.clone(), seq as u64 + 1);
        }
        assert!(
            !rig.sync
                .try_iter()
                .any(|cmd| matches!(cmd, SyncCommand::SendUnsent { .. })),
            "sent before the replay was done"
        );
        ready(&mut rig);
        let sent = rig.sync.try_iter().collect();
        let saved = persist.try_iter().map(|(_, write)| write).collect();
        (sent, saved, hers, peer)
    }

    /// `AUTHOR-028`: what an earlier session wrote past what the relay holds
    /// travels once the replay is done. It is sealed with that session's
    /// nonce and carries its operations alone. The current session is saved
    /// with its nonce at the identity.
    #[test]
    fn an_earlier_sessions_unsent_edits_travel_with_its_nonce() {
        let (sent, saved, hers, peer) = reopened(1);
        let unsent: Vec<(Vec<u8>, SessionNonce)> = sent
            .into_iter()
            .filter_map(|cmd| match cmd {
                SyncCommand::SendUnsent { payload, nonce, .. } => Some((payload, nonce)),
                _ => None,
            })
            .collect();
        let [(payload, nonce)] = unsent.as_slice() else {
            panic!("expected one unsent delta, got {}", unsent.len());
        };
        assert_eq!(*nonce, SessionNonce([7; 16]));
        assert_eq!(authors(payload), vec![ALICE]);
        let meta = LoroDoc::decode_import_blob_meta(payload, false).expect("a readable delta");
        let first = LoroDoc::decode_import_blob_meta(&hers[0], false).expect("her first delta");
        assert_eq!(
            meta.partial_start_vv.get(&ALICE),
            first.partial_end_vv.get(&ALICE),
            "the unsent delta does not start where the relay's copy ends"
        );
        assert!(
            saved
                .iter()
                .any(|w| matches!(w, PersistWrite::Session { peer: p, .. } if *p == peer)),
            "the session was not saved at the identity"
        );
        assert!(
            !saved
                .iter()
                .any(|w| matches!(w, PersistWrite::ForgetSession { .. })),
            "a session with unsent edits was forgotten"
        );
    }

    /// `AUTHOR-028`: an earlier session the relay holds whole sends nothing
    /// and is forgotten.
    #[test]
    fn an_earlier_session_the_relay_holds_whole_is_forgotten() {
        let (sent, saved, _, _) = reopened(2);
        assert!(
            !sent
                .iter()
                .any(|cmd| matches!(cmd, SyncCommand::SendUnsent { .. })),
            "a held session was sent again"
        );
        assert!(
            saved
                .iter()
                .any(|w| matches!(w, PersistWrite::ForgetSession { peer } if *peer == ALICE)),
            "a held session was not forgotten"
        );
    }
}
