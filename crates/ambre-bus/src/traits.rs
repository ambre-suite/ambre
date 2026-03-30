//! Core traits for application controllers and command sinks.
//!
//! - `AppController`: implemented by each application controller in the Logic Worker.
//! - `CommandSink`: abstraction for sending `SyncCommand`s to the Sync Worker.

use crate::{DocType, LogicEvent, SyncCommand};

/// Abstraction for sending commands to the Sync Worker.
///
/// Implemented by a wrapper that serializes the command and sends it via
/// `postMessage` (see `ambre‑app/src/bridge_sync.rs`).
pub trait CommandSink {
    /// Send a command to the Sync Worker.
    fn send(&self, cmd: SyncCommand);
}

/// Implemented by each application controller in the Logic Worker.
///
/// Controllers are registered in the `ControllerRegistry` and receive
/// UI commands (as opaque `Vec<u8>`) and LogicEvents from the Sync Worker.
pub trait AppController {
    /// Returns the document type this controller handles.
    fn doc_type(&self) -> DocType;

    /// Initializes the controller with a sink to the Sync Worker.
    ///
    /// Called once when the controller is registered. The controller may store
    /// the sink for later use.
    fn init(&mut self, sync_tx: Box<dyn CommandSink>);

    /// Handles a UI command (serialized with postcard).
    ///
    /// The command is application‑specific; the bus does not interpret it.
    /// Returns zero or more events (also serialized with postcard) that should
    /// be dispatched to the UI Thread.
    fn handle_ui_command(&mut self, cmd: Vec<u8>) -> Vec<Vec<u8>>;

    /// Receives an event from the Sync Worker.
    ///
    /// Returns zero or more events (serialized with postcard) that should be
    /// dispatched to the UI Thread – symmetric with `handle_ui_command`.
    fn on_logic_event(&mut self, event: LogicEvent) -> Vec<Vec<u8>>;
}