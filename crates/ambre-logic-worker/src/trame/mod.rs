// ambre-logic-worker/src/trame/mod.rs
// ============================================================================
// Trame (spreadsheet) AppController -- Logic Worker side.
//
// Lives here rather than in ambre-trame because it requires heavy deps
// (loro, petgraph, postcard) that must not pollute the UI Thread binary.
// The shared IPC types (TrameCommand, TrameViewUpdate) live in ambre-trame
// which has serde as its only dependency.
// ============================================================================

use ambre_bus::{AppController, CommandSink, DocType, LogicEvent};
use ambre_trame::{TrameCommand, TrameViewUpdate};

// -----------------------------------------------------------------------------
// TrameController
// -----------------------------------------------------------------------------

/// Controller for Trame (spreadsheet) documents.
///
/// Owned by [`super::LogicWorkerState::controllers`], keyed by `document_id`.
/// Created when the first [`LogicEvent::RemoteSnapshot`] arrives for a document
/// of type [`DocType::Trame`] (via [`super::LogicWorkerState::make_controller`]).
pub struct TrameController {
    /// Sink to the Sync Worker. Used to emit [`ambre_bus::SyncCommand::SendUpdate`]
    /// and [`ambre_bus::SyncCommand::SendEphemeral`] in response to user edits.
    sync_tx: Box<dyn CommandSink>,
    /// Total number of key presses received since this controller was created.
    key_count: u64,
    // -- Future fields --------------------------------------------------------
    // doc:     loro::LoroDoc,
    // grid:    Grid2D,
    // formula: FormulaEngine,
}

impl TrameController {
    pub fn new(sync_tx: Box<dyn CommandSink>) -> Self {
        Self { sync_tx, key_count: 0 }
    }
}

impl AppController for TrameController {
    fn doc_type(&self) -> DocType {
        DocType::Trame
    }

    /// Handle a P1 scroll or P2 application command from the UI Thread.
    ///
    /// Returns postcard-serialized view updates wrapped by the dispatcher
    /// into [`ambre_bus::LogicEnvelope::ViewportUpdate`].
    fn handle_ui_command(&mut self, cmd: Vec<u8>) -> Vec<Vec<u8>> {
        let cmd = match postcard::from_bytes::<TrameCommand>(&cmd) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("TrameController: deserialize TrameCommand: {e}");
                return vec![];
            }
        };
        match cmd {
            TrameCommand::KeyPress { seq, t0_ms } => {
                self.key_count += 1;
                let update = TrameViewUpdate::KeyCount {
                    count: self.key_count,
                    seq,
                    t0_ms,
                };
                match postcard::to_allocvec(&update) {
                    Ok(bytes) => vec![bytes],
                    Err(e) => {
                        log::error!("TrameController: serialize TrameViewUpdate: {e}");
                        vec![]
                    }
                }
            }
        }
    }

    /// Apply a remote CRDT delta from the Sync Worker.
    ///
    /// Not yet implemented: returns an empty list until LoroDoc is wired.
    fn on_logic_event(&mut self, _event: LogicEvent) -> Vec<Vec<u8>> {
        vec![]
    }

    /// Export the full CRDT state for Sync Worker snapshot compaction.
    ///
    /// Not yet implemented: returns an empty payload until LoroDoc is wired.
    fn export_snapshot(&self) -> Vec<u8> {
        vec![]
    }
}
