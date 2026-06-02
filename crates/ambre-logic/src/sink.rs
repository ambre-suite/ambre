// ambre-logic/src/sink.rs
// ============================================================================
// `CommandSink` implementation over a `flume::Sender`.
//
// The `CommandSink` trait (the contract an AppController is handed) lives in
// ambre-bus. The concrete flume-backed transport lives here, next to the Logic
// thread that constructs it — that keeps ambre-bus free of any transport
// dependency (no flume, no tracing).
//
// On `send`, the SyncCommand is moved into the channel — zero copies, zero
// serialisation. Dropped commands (channel closed) are logged at warn level
// rather than panicking, since channel closure means the Sync thread has shut
// down and the controller is mid-teardown.
// ============================================================================

use ambre_bus::{CommandSink, SyncCommand};

/// `CommandSink` backed by a [`flume::Sender<SyncCommand>`].
///
/// Created once per controller by the Logic thread when it constructs its
/// controller registry, then handed to each controller as `Box<dyn CommandSink>`.
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
