//! Toile IPC contract (§8) and the data-oriented display structures (§9).
//!
//! The command / event / display types themselves live in `ambre-bus`
//! (`ambre_bus::app::toile`) so the `ambre-logic` dispatcher can match on
//! `(DocState, AppCommand::Toile)` without a dependency cycle. This module
//! re-exports them (keeping the engine's `crate::logic::command::...` paths) and adds
//! the one engine-local type: [`ToileOutcome`].
//!
//! **Outcome shape.** [`ToileOutcome`] mirrors `TrameOutcome` (which likewise
//! stays in `ambre-trame`, not `ambre-bus`): it returns raw CRDT / ephemeral
//! bytes, and the `ambre-logic` dispatcher wraps them into `SyncCommand`s with
//! the owning `document_id` (which the standalone engine does not hold). Same
//! division of labour as Trame.

pub use ambre_bus::app::toile::{
    ClipConnector, ClipElement, ClipMembership, ClipValue, ClipboardPayload, DisplayConnector,
    DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, End, FindOptions, FindResults,
    InitContent, PresencePayload, PresenceUpdate, SelectionStats, TextDelta, ToileCommand,
    ToileError, ToileEvent, ZTarget,
};

// -----------------------------------------------------------------------------
// Outcome (§8.C) -- engine-local, like `TrameOutcome`.
// -----------------------------------------------------------------------------

/// Result of processing a [`ToileCommand`]: display events for the UI plus
/// optional CRDT / ephemeral byte payloads the dispatcher forwards to Sync.
pub struct ToileOutcome {
    pub display: Vec<ToileEvent>,
    /// Binary CRDT delta (`ExportMode::Updates`) to encrypt and broadcast.
    pub update: Option<Vec<u8>>,
    /// Ephemeral (presence) payload to encrypt and broadcast.
    pub ephemeral: Option<Vec<u8>>,
}

impl ToileOutcome {
    pub fn display_only(display: Vec<ToileEvent>) -> Self {
        Self {
            display,
            update: None,
            ephemeral: None,
        }
    }

    /// An outcome that only broadcasts an ephemeral (presence) frame: no display
    /// event and no CRDT delta. The local cursor is drawn by the UI itself, so a
    /// `Presence` command produces nothing for the local UI to repaint (§7).
    pub fn ephemeral_only(ephemeral: Vec<u8>) -> Self {
        Self {
            display: Vec::new(),
            update: None,
            ephemeral: Some(ephemeral),
        }
    }
}
