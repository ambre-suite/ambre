//! `CommandSink` implementation over a [`flume::Sender`].
//!
//! Plugs the existing `AppController` API into the channel-based inter-thread
//! transport introduced in Phase 3 of the multi-threaded WASM migration. The
//! controller stays unaware of the transport: it still calls `sink.send(cmd)`,
//! but the bytes that used to be postcard-serialised and shoved through a
//! `MessagePort` now flow as a typed Rust value through a `flume` channel.

use crate::{CommandSink, SyncCommand};

/// `CommandSink` backed by a [`flume::Sender<SyncCommand>`].
///
/// Created once per controller by the Logic thread when it constructs its
/// controller registry, then handed to each controller via
/// `AppController::init` (or equivalent) as `Box<dyn CommandSink>`.
///
/// On `send`, the [`SyncCommand`] is moved into the channel — zero copies,
/// zero serialisation. Dropped commands (channel closed) are logged at warn
/// level rather than panicking, since channel closure means the Sync thread
/// has shut down and the controller is mid-teardown.
pub struct SendCommandSink {
    tx: flume::Sender<SyncCommand>,
}

impl SendCommandSink {
    pub fn new(tx: flume::Sender<SyncCommand>) -> Self {
        Self { tx }
    }
}

impl CommandSink for SendCommandSink {
    fn send(&self, cmd: SyncCommand) {
        if let Err(flume::SendError(cmd)) = self.tx.send(cmd) {
            tracing::warn!(
                "SendCommandSink: Sync thread receiver dropped, discarding {:?}",
                std::mem::discriminant(&cmd),
            );
        }
    }
}
