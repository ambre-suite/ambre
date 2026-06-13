//! `CommandSink` -- the Logic thread's outbound handle to the Sync thread.
//!
//! A controller running in the Logic thread produces [`SyncCommand`]s (CRDT
//! deltas, ephemeral presence, snapshots) that must reach the Sync worker. In
//! production that hop is a `postMessage` boundary; in native tests it is a
//! `flume` channel or an in-memory buffer. `CommandSink` is the single seam
//! both share, so controller logic never names a concrete transport.
//!
//! The trait is object-safe (`send` takes `&self`), so a controller holds its
//! sink as `Box<dyn CommandSink>` and pushes through it without interior
//! plumbing of its own -- the implementation owns whatever buffering or
//! channel send it needs (hence the shared-reference signature).
//!
//! See `specs/2-Trame/ambre-spec-trame-logic.md` (the `sync_tx` field and the
//! `AppController::init` injection point).

use crate::SyncCommand;

/// Outbound channel abstraction: Logic thread -> Sync thread.
///
/// Implementors:
/// - production WASM: a wrapper that `postMessage`s the (postcard-encoded)
///   `SyncCommand` to the Sync worker;
/// - native integration: a `flume::Sender<SyncCommand>` wrapper;
/// - tests: `MockSyncHandle` (in `ambre-trame`), which buffers the commands for
///   inspection and peer-to-peer relay.
pub trait CommandSink {
    /// Hand a command to the Sync thread. Best-effort: a closed transport drops
    /// the command (the controller keeps running), so this cannot fail.
    fn send(&self, cmd: SyncCommand);
}
