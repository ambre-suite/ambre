//! `CommandSink` -- the Logic thread's outbound handle to the Sync thread.
//!
//! A controller running in the Logic thread produces [`SyncCommand`]s (CRDT deltas,
//! ephemeral presence, snapshots) that must reach the Sync worker. In production that hop
//! is a `postMessage` boundary; in native tests it is a `conduit` channel or an in-memory
//! buffer. `CommandSink` is the single seam both share, so controller logic never names a
//! concrete transport.
//!
//! The trait is object-safe (`send` takes `&self`). A holder can keep its sink as
//! `Box<dyn CommandSink>` and push through it without interior plumbing of its own. The
//! implementation owns whatever buffering or channel send it needs, hence the
//! shared-reference signature.
//!
//! In production the Logic dispatcher (`ambre-logic::LogicState`) forwards `SyncCommand`s
//! on a bare `conduit::Sender`, reading them from the `TrameOutcome` a `TrameState` returns.
//! The state holds no sink itself. `CommandSink` is the **test seam**:
//! `ambre-trame::test_support::MockSyncHandle` implements it to capture emitted
//! `SyncCommand`s and relay CRDT deltas peer-to-peer. See `trame-engine`.

use crate::SyncCommand;

/// Outbound channel abstraction: Logic thread -> Sync thread.
///
/// Implementors:
/// - production WASM: a wrapper that `postMessage`s the (postcard-encoded)
///   `SyncCommand` to the Sync worker;
/// - native integration: a `conduit::Sender<SyncCommand>` wrapper;
/// - tests: `MockSyncHandle` (in `ambre-trame`), which buffers the commands for
///   inspection and peer-to-peer relay.
pub trait CommandSink {
    /// Hand a command to the Sync thread. Best-effort: a closed transport drops
    /// the command (the controller keeps running), so this cannot fail.
    fn send(&self, cmd: SyncCommand);
}
