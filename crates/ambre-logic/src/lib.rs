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

use ambre_bus::{
    AppCommand, AppEvent, DocType, DocumentId, LogicCommand, LogicEvent, RestCommand, RestEvent,
    Role, SyncCommand, UiEvent, UserId,
};

use ambre_toile::ToileState;
use ambre_trame::TrameState;

// -----------------------------------------------------------------------------
// DocState — typed per-document state
// -----------------------------------------------------------------------------

/// Per-document state, one variant per application document type.
///
/// A closed sum mirroring [`AppCommand`] / [`AppEvent`]: the dispatcher matches
/// the `(DocState, AppCommand)` pair to route a typed command to the right
/// document model — no trait object, no downcast.
enum DocState {
    // Boxed: the per-document states differ widely in size (a `TrameState` is
    // several KB, a `ToileState` far smaller), so a bare enum would size every
    // slot to the largest variant. One heap indirection per open document is
    // negligible and keeps the registry entry pointer-sized.
    Trame(Box<TrameState>),
    Toile(Box<ToileState>),
    // Future: Prose(ProseState), Murmure(MurmureState), …
}

impl DocState {
    /// Build the state for a freshly-opened document of `doc_type`.
    ///
    /// Returns `None` for document types whose logic is not yet implemented.
    fn new(doc_type: DocType) -> Option<Self> {
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

    /// Seed the canonical `UserId` (§7) into the underlying app state once the
    /// session is authenticated. Only apps that key per-user state on it react;
    /// Trame's own `my_user_id` (`trame-logic §7`) is a separate slice.
    fn set_user_id(&mut self, user_id: UserId) {
        match self {
            DocState::Toile(state) => state.set_user_id(user_id),
            DocState::Trame(_) => {}
        }
    }
}

// -----------------------------------------------------------------------------
// LiveDoc — an open document and the caller's role on it
// -----------------------------------------------------------------------------

/// One open document: its typed state plus this client's role on **that**
/// document (architecture §2.D).
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
    /// (§7). `None` until the session is authenticated. Seeded into every open
    /// `DocState` on arrival and into any document opened afterwards, so an app
    /// that keys per-user state (Toile reactions, presence identity) always has
    /// it before the first edit.
    my_user_id: Option<UserId>,
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
