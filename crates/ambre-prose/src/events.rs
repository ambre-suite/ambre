//! Prose's IPC contract, and the one engine-local value that is not part of it.
//!
//! The command and event types live in `ambre-bus` (`ambre_bus::app::prose`) so
//! the `ambre-logic` dispatcher can match on `(DocState, AppCommand::Prose)`
//! without a dependency cycle. This module re-exports them, keeping the
//! engine's `crate::events::...` paths, and adds [`ProseOutcome`].
//!
//! [`ProseOutcome`] stays here, like `ToileOutcome` and `TrameOutcome` stay in
//! their own crates. It carries raw CRDT and ephemeral bytes; the `ambre-logic`
//! dispatcher wraps them into `SyncCommand`s under the owning `DocumentId`,
//! which the engine does not hold (`PROSE-024`).

pub use ambre_bus::app::prose::{
    ActiveMarks, Align, BandSet, BlockId, BlockInit, BlockType, CellBorder, CommentReply,
    CommentThread, CrossRefKind, CursorPosition, DateFieldFormat, FindMatch, FindQuery,
    FootnoteEntry, HeaderField, HfTarget, LineHeight, LineHeightMode, LinkSpan, ListStyle,
    MarkAction, OrderedStyle, ParagraphStyle, PresencePayload, ProseCommand, ProseError,
    ProseEvent, ProseRefusal, RemoteCursor, ResolvedCrossRef, Suggestion, SuggestionKind,
    SuggestionSpan, TableOfContents, TextEdit, TocEntry, TriState, VAlign, WordCount,
};

use loro::TreeID;

/// The CRDT id a transport block id names.
///
/// The two carry the same pair, so this renames a value rather than resolving
/// one. It is the whole of the boundary `ambre-bus` draws to keep the CRDT out
/// of the Sync worker's dependency graph (`SYNC-011`).
#[must_use]
pub fn tree_id(id: BlockId) -> TreeID {
    TreeID::new(id.peer.0, id.counter)
}

/// The transport block id a CRDT id names. The inverse of [`tree_id`].
#[must_use]
pub fn block_id(id: TreeID) -> BlockId {
    BlockId::new(ambre_core::PeerId(id.peer), id.counter)
}

/// What one command answers with (`PROSE-046`).
///
/// One value carries every display event the command produced, the delta it
/// exported and the presence sample it built. A handler sends on no channel: the
/// dispatcher fans this out (`PROSE-024`, `PROSE-062`).
#[derive(Debug, Default)]
pub struct ProseOutcome {
    pub display: Vec<ProseEvent>,
    /// The CRDT delta to seal and broadcast. `None` when the command wrote
    /// nothing (`PROSE-047`).
    pub update: Option<ambre_core::DeltaBytes>,
    /// The presence sample to seal and broadcast.
    pub ephemeral: Option<ambre_core::EphemeralBytes>,
}

impl ProseOutcome {
    /// An outcome that repaints and broadcasts nothing.
    #[must_use]
    pub fn nothing() -> Self {
        Self::default()
    }

    /// An outcome that repaints the local view alone: no delta, no sample.
    #[must_use]
    pub fn display_only(display: Vec<ProseEvent>) -> Self {
        Self {
            display,
            update: None,
            ephemeral: None,
        }
    }

    /// An outcome that broadcasts a presence sample alone. The local caret is
    /// drawn by the view itself, so nothing repaints here.
    #[must_use]
    pub fn ephemeral_only(ephemeral: ambre_core::EphemeralBytes) -> Self {
        Self {
            display: Vec::new(),
            update: None,
            ephemeral: Some(ephemeral),
        }
    }

    /// The refusal of a command, by its reason alone (`PROSE-105`).
    ///
    /// The reason is known deep in a handler; the command is known only where
    /// the engine was entered. So a refusal is built unnamed here and
    /// [`ProseOutcome::named`] writes the command in on the way out, which is
    /// the one place that holds it.
    #[must_use]
    pub fn refused(reason: ProseRefusal) -> Self {
        Self::display_only(vec![ProseEvent::Error(ProseError::new(UNNAMED, reason))])
    }

    /// Name every refusal this outcome carries after the command that drew it
    /// (`PROSE-105`). An outcome that refused nothing passes through.
    #[must_use]
    pub fn named(mut self, refused: &'static str) -> Self {
        for event in &mut self.display {
            if let ProseEvent::Error(error) = event {
                error.refused = refused;
            }
        }
        self
    }
}

/// What a refusal names before the entry point names the command it came from.
/// It reaches a screen only if a handler refuses somewhere the engine is not
/// driven by a command, and no such path exists today.
const UNNAMED: &str = "that";
