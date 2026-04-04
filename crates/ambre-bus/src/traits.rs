//! Core traits for the Logic Worker dispatch layer.
//!
//! - [`AppController`]: implemented once per document type by each application
//!   crate (`ambre-trame`, `ambre-prose`, `ambre-toile`, `ambre-murmure`).
//! - [`CommandSink`]: abstraction that lets an `AppController` send
//!   [`SyncCommand`]s to the Sync Worker without depending on web-sys.

use crate::{DocType, LogicEvent, SyncCommand};

// ─────────────────────────────────────────────────────────────────────────────
// CommandSink
// ─────────────────────────────────────────────────────────────────────────────

/// Abstraction for sending [`SyncCommand`]s to the Sync Worker.
///
/// The concrete implementation wraps a `web_sys::MessagePort` and serializes
/// the command with postcard before calling `postMessage` (see
/// `ambre-app/src/bridge_sync.rs`). `AppController` implementations receive a
/// boxed `CommandSink` via [`AppController::init`] and store it for the lifetime
/// of the document session.
pub trait CommandSink {
    /// Serialize `cmd` with postcard and transfer it to the Sync Worker.
    fn send(&self, cmd: SyncCommand);
}

// ─────────────────────────────────────────────────────────────────────────────
// AppController
// ─────────────────────────────────────────────────────────────────────────────

/// Per-document-type controller living in the Logic Worker.
///
/// One `AppController` instance is created for each open document and stored
/// in the `ControllerRegistry` keyed by `document_id`. The Logic Worker
/// dispatcher routes:
/// - [`UiEnvelope::AppCommand`] → [`handle_ui_command`]
/// - [`UiEnvelope::ScrollTo`] → [`handle_ui_command`] (scroll params serialized by
///   the dispatcher)
/// - [`LogicEvent::RemoteSnapshot`] / [`RemoteUpdate`] / [`RemoteEphemeral`] →
///   [`on_logic_event`]
/// - [`LogicEvent::SnapshotNeeded`] → [`export_snapshot`]
///
/// All return values are postcard-serialized display lists forwarded to the UI
/// Thread as [`LogicEnvelope::ViewportUpdate`] or [`LogicEnvelope::PresenceUpdate`].
///
/// [`UiEnvelope::AppCommand`]: crate::UiEnvelope::AppCommand
/// [`UiEnvelope::ScrollTo`]: crate::UiEnvelope::ScrollTo
/// [`handle_ui_command`]: AppController::handle_ui_command
/// [`on_logic_event`]: AppController::on_logic_event
/// [`export_snapshot`]: AppController::export_snapshot
/// [`RemoteUpdate`]: crate::LogicEvent::RemoteUpdate
/// [`RemoteEphemeral`]: crate::LogicEvent::RemoteEphemeral
/// [`LogicEnvelope::ViewportUpdate`]: crate::LogicEnvelope::ViewportUpdate
/// [`LogicEnvelope::PresenceUpdate`]: crate::LogicEnvelope::PresenceUpdate
pub trait AppController {
    /// Returns the document type this controller handles.
    ///
    /// Used by the controller factory to verify the correct type is being
    /// constructed for a given document.
    fn doc_type(&self) -> DocType;

    /// Process a UI command and return display updates for the UI Thread.
    ///
    /// `cmd` is a postcard-serialized command whose schema is defined by the
    /// implementing crate. The return value is zero or more postcard-serialized
    /// display lists. Each one is wrapped in a [`LogicEnvelope::ViewportUpdate`]
    /// by the Logic Worker dispatcher before being sent to the UI Thread.
    ///
    /// This method must complete in < 50 ms (P2 budget). For scroll requests
    /// (P1) the dispatcher serializes the scroll parameters and calls this
    /// method too; the controller must respond in < 10 ms in that case.
    ///
    /// [`LogicEnvelope::ViewportUpdate`]: crate::LogicEnvelope::ViewportUpdate
    fn handle_ui_command(&mut self, cmd: Vec<u8>) -> Vec<Vec<u8>>;

    /// Receive an event from the Sync Worker and return display updates.
    ///
    /// Called for [`LogicEvent::RemoteSnapshot`], [`LogicEvent::RemoteUpdate`],
    /// and [`LogicEvent::RemoteEphemeral`]. The controller applies the CRDT
    /// delta to the local `LoroDoc`, recomputes the affected display region,
    /// and returns postcard-serialized display lists. The dispatcher wraps each
    /// one in [`LogicEnvelope::ViewportUpdate`] (or [`LogicEnvelope::PresenceUpdate`]
    /// for ephemeral data) before forwarding to the UI Thread.
    ///
    /// [`LogicEvent::RemoteSnapshot`]: crate::LogicEvent::RemoteSnapshot
    /// [`LogicEvent::RemoteUpdate`]: crate::LogicEvent::RemoteUpdate
    /// [`LogicEvent::RemoteEphemeral`]: crate::LogicEvent::RemoteEphemeral
    /// [`LogicEnvelope::ViewportUpdate`]: crate::LogicEnvelope::ViewportUpdate
    /// [`LogicEnvelope::PresenceUpdate`]: crate::LogicEnvelope::PresenceUpdate
    fn on_logic_event(&mut self, event: LogicEvent) -> Vec<Vec<u8>>;

    /// Export the full CRDT state for snapshot compaction.
    ///
    /// Called by the Logic Worker dispatcher when the Sync Worker emits
    /// [`LogicEvent::SnapshotNeeded`]. The return value must be a valid
    /// `LoroDoc::export_snapshot()` payload. The dispatcher wraps it in
    /// [`SyncCommand::SendSnapshot`] and forwards it to the Sync Worker.
    ///
    /// [`LogicEvent::SnapshotNeeded`]: crate::LogicEvent::SnapshotNeeded
    /// [`SyncCommand::SendSnapshot`]: crate::SyncCommand::SendSnapshot
    fn export_snapshot(&self) -> Vec<u8>;
}
