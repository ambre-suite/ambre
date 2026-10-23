//! Toile IPC contract and the data-oriented display structures.
//!
//! The command / event / display types themselves live in `ambre-bus`
//! (`ambre_bus::app::toile`) so the `ambre-logic` dispatcher can match on `(DocState,
//! AppCommand::Toile)` without a dependency cycle. This module re-exports them
//! (keeping the engine's `crate::logic::command::...` paths) and adds the one
//! engine-local type: [`ToileOutcome`].
//!
//! **Outcome shape.** [`ToileOutcome`] mirrors `TrameOutcome`, which likewise stays
//! in `ambre-trame`, not `ambre-bus`. It returns raw CRDT / ephemeral bytes. The
//! `ambre-logic` dispatcher wraps them into `SyncCommand`s with the owning
//! `document_id`, which the standalone engine does not hold. Same division of labour
//! as Trame.

pub use ambre_bus::app::toile::{
    BoolOp, Broadcast, ClipConnector, ClipElement, ClipMembership, ClipValue, ClipboardPayload,
    CommentAnchor, CommentMessage, CommentPin, CommentSummary, CommentThread, CorrelationToken,
    DeckFrame, DeckView, DisplayConnector, DisplayElement, DisplayGeom, DisplayNode,
    DisplayViewport, End, FindOptions, FindResults, InitContent, PlaybackState, PresencePayload,
    PresenceUpdate, PresentFocus, PresentMode, Reaction, ReactionKind, ReactionTally,
    SelectionStats, SlideChange, SlideCount, SlideIndex, SlidePosition, Sources, StatsRequestKind,
    TextDelta, ToileCommand, ToileError, ToileEvent, Viewport, ZTarget,
};
pub use ambre_bus::rest::RequestId;

// -----------------------------------------------------------------------------
// Outcome -- engine-local, like `TrameOutcome`.
// -----------------------------------------------------------------------------

/// A Logic->Sync service-plane request a Toile command needs to make
/// (`toile-media`). Toile allocates the [`RequestId`], owning the correlation so
/// it can match the answering `RestEvent` back to a pending import. The
/// `ambre-logic` dispatcher supplies the `document_id` and lowers this to a
/// `RestCommand`. Toile never constructs a `RestCommand` nor knows its own
/// `DocumentId`, the same division of labour as the CRDT delta in
/// [`ToileOutcome::update`].
#[derive(Debug)]
pub enum ToileRequest {
    UploadMedia {
        id: RequestId,
        data: ambre_core::MediaBytes,
    },
    DownloadMedia {
        id: RequestId,
        content_hash: ambre_core::ContentHash,
    },
}

/// Result of processing a [`ToileCommand`]: display events for the UI plus
/// optional CRDT / ephemeral byte payloads the dispatcher forwards to Sync.
pub struct ToileOutcome {
    pub display: Vec<ToileEvent>,
    /// Binary CRDT delta (`ExportMode::Updates`) to encrypt and broadcast.
    pub update: Option<ambre_core::DeltaBytes>,
    /// Ephemeral (presence) payload to encrypt and broadcast.
    pub ephemeral: Option<ambre_core::EphemeralBytes>,
    /// Service-plane requests to lower into `RestCommand`s (`toile-media`).
    /// Empty for every command except an import that carries media.
    pub requests: Vec<ToileRequest>,
}

impl ToileOutcome {
    pub fn display_only(display: Vec<ToileEvent>) -> Self {
        Self {
            display,
            update: None,
            ephemeral: None,
            requests: Vec::new(),
        }
    }

    /// An outcome that only broadcasts a presence sample: no display
    /// event and no CRDT delta. The local cursor is drawn by the UI itself, so a
    /// `Presence` command produces nothing for the local UI to repaint.
    pub fn ephemeral_only(ephemeral: ambre_core::EphemeralBytes) -> Self {
        Self {
            display: Vec::new(),
            update: None,
            ephemeral: Some(ephemeral),
            requests: Vec::new(),
        }
    }

    /// An outcome with local display events plus an optional ephemeral and no
    /// CRDT delta. Used by the playback commands when "Follow presenter"
    /// broadcasting is on. The local `SlideChanged` repaints this viewer, while
    /// the ephemeral carries the active index to followers.
    pub fn display_and_ephemeral(
        display: Vec<ToileEvent>,
        ephemeral: Option<ambre_core::EphemeralBytes>,
    ) -> Self {
        Self {
            display,
            update: None,
            ephemeral,
            requests: Vec::new(),
        }
    }
}
