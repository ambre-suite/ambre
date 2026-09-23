//! The UI thread's own state, and the one path a mutation leaves it by
//! (`features/ambre-spec-prose-engine.md section 3.G`).
//!
//! This thread holds view state and what arrived as an event, and no document
//! data (`PROSE-UI-001`). The scroll position and the selection stay here:
//! neither enters the document, so two collaborators scroll independently and
//! read the same document (`PROSE-UI-010`).
//!
//! **It never blocks on the Logic channel** (`PROSE-UI-003`). Every send is a
//! `try_send`, and a full channel drops the command rather than holding the
//! frame (`PROSE-UI-149`). The lossless class of `ambre-spec-message-bus.md`
//! (`BUS-011`) holds a command instead, and refuses input while it does. It
//! lands with the "Input and cursor" Story (`ambre-roadmap.md` E5).
//!
//! **Engine-core scope** (`ARCH P2`). The draft below holds what was typed and
//! not yet confirmed, because the send path needs it to answer
//! `PROSE-UI-150`. Its whole state machine lands with the "Input and cursor"
//! Story. That is the re-anchor over a remote change, the input-method
//! composition, and the collapse on a deleted block.

use ambre_core::DocumentId;

use ambre_bus::logic::{AppCommand, LogicCommand};

use crate::events::{
    BandSet, BlockId, CommentThread, CursorPosition, FindMatch, FootnoteEntry, ProseCommand,
    ProseEvent, RemoteCursor, ResolvedCrossRef, TableOfContents, WordCount,
};

/// How long the text must stay still before the four tallies are recounted,
/// in milliseconds (`PROSE-FEAT-124`).
///
/// A fast typist then pays for one recount per pause rather than one per
/// keystroke (`features/ambre-spec-prose-word-count.md section 4`).
pub const RECOUNT_DELAY_MS: f64 = 500.0;

/// What was typed and not yet confirmed, held apart from the document rather
/// than written into a second copy of it.
///
/// It carries text and an anchor and no structure: it cannot express a split, a
/// deletion or a mark (`PROSE-UI-135`). It is painted under the caret and it
/// never reaches the wire (`PROSE-UI-150`): what reaches the wire is the
/// command the same keystroke emitted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DraftBuffer {
    /// Typed since the last confirming event, drawn over the confirmed runs.
    pub pending_text: String,
    pub block_id: Option<BlockId>,
    pub insert_pos: u32,
}

impl DraftBuffer {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending_text.is_empty()
    }

    /// Append committed text. The draft re-anchors only when it moves to
    /// another block, so consecutive keystrokes in one block accumulate in one
    /// draft (`PROSE-UI-138`).
    pub fn push_str(&mut self, text: &str, block_id: &BlockId, pos: u32) {
        if self.block_id != Some(*block_id) {
            self.pending_text.clear();
            self.block_id = Some(*block_id);
            self.insert_pos = pos;
        }
        self.pending_text.push_str(text);
    }

    pub fn clear(&mut self) {
        self.pending_text.clear();
        self.block_id = None;
        self.insert_pos = 0;
    }
}

/// Where the reader is looking. Local, and never sent (`PROSE-UI-010`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
    pub scroll_y: f32,
    pub zoom: f32,
}

/// The UI thread's whole state for one open Prose document.
///
/// Every field is either handed over by the Logic thread or local to this
/// thread. None of them is document content (`PROSE-UI-001`).
#[derive(Clone, Debug, Default)]
pub struct ProseUiState {
    // Handed over by the Logic thread.
    /// The version of the last layout this view adopted. An older one is
    /// dropped (`features/ambre-spec-prose-page-rendering.md`).
    pub layout_version: u64,
    /// The whole live peer set, replaced on each event (`PRESENCE-009`).
    pub remote_cursors: Vec<RemoteCursor>,
    /// The five figures as last answered, for the shared status bar slot.
    pub word_count: WordCount,
    /// The entries of every table of contents, as last answered, by block.
    pub tocs: Vec<(BlockId, TableOfContents)>,
    /// The matches of this peer's own search, to highlight, and the current
    /// one. Empty once the search closes (`PROSE-FEAT-289`).
    pub find_matches: Vec<FindMatch>,
    pub find_current: Option<u32>,
    /// The four band blocks and their settings, as last answered: the blocks
    /// the ordinary text commands aim at to edit a band.
    pub bands: BandSet,
    /// Every footnote, numbered or orphaned, as last answered.
    pub footnotes: Vec<FootnoteEntry>,
    /// Every comment thread, marked or orphaned, as last answered.
    pub comments: Vec<CommentThread>,
    /// Every cross reference, as last resolved.
    pub cross_refs: Vec<ResolvedCrossRef>,
    /// The page a followed reference asked to bring into view, until the
    /// view scrolls there.
    pub reveal_page: Option<u32>,

    // Local, and never sent.
    pub viewport: Viewport,
    pub cursor: Option<CursorPosition>,
    pub draft: DraftBuffer,
    /// A change arrived since the last tick.
    changed: bool,
    /// When the text was last seen changing, while a recount is owed.
    quiet_since_ms: Option<f64>,
}

/// What a send to the Logic thread did.
///
/// The view reflects a command in its own state only when it was taken. A
/// dropped command never happened, so painting its echo would show the user a
/// change no peer will ever see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendResult {
    /// The command reached the channel.
    Sent,
    /// The channel was full, and the command was dropped rather than the frame
    /// held (`PROSE-UI-149`).
    Dropped,
    /// The Logic thread is gone. This channel carries nothing again.
    Disconnected,
}

impl SendResult {
    /// Whether the command is on its way to the Logic thread.
    #[must_use]
    pub fn accepted(self) -> bool {
        self == Self::Sent
    }
}

/// Send one command to the Logic thread (`PROSE-UI-002`).
///
/// Every mutation leaves this thread this way, and no other. The send never
/// blocks (`PROSE-UI-003`), and the draft echo the view painted stays here
/// (`PROSE-UI-150`): what crosses is the typed command alone.
pub fn send_command(
    to_logic: &flume::Sender<LogicCommand>,
    document_id: DocumentId,
    command: ProseCommand,
) -> SendResult {
    let envelope = LogicCommand::App {
        document_id,
        command: AppCommand::Prose(command),
    };
    match to_logic.try_send(envelope) {
        Ok(()) => SendResult::Sent,
        Err(flume::TrySendError::Full(_)) => {
            tracing::warn!("Prose: the Logic channel is full, the command is dropped");
            SendResult::Dropped
        }
        Err(flume::TrySendError::Disconnected(_)) => SendResult::Disconnected,
    }
}

impl ProseUiState {
    /// Take in one display event.
    ///
    /// A confirming change clears the draft it covers: the text is then in the
    /// confirmed runs (`PROSE-UI-005`). A split leaves the caret at the start of
    /// the new paragraph (`PROSE-UI-141`). The view can do that only because the
    /// event names the block the engine minted.
    pub fn apply_event(&mut self, event: &ProseEvent) {
        match event {
            ProseEvent::DocUpdated { .. } => {
                self.draft.clear();
                self.changed = true;
            }
            ProseEvent::CellEntered { block, .. } => {
                self.cursor = Some(CursorPosition {
                    block_id: *block,
                    offset: 0,
                    anchor: None,
                });
            }
            ProseEvent::WordCountUpdated { count } => self.word_count = *count,
            ProseEvent::TocUpdated { block_id, toc } => {
                match self.tocs.iter_mut().find(|(id, _)| id == block_id) {
                    Some(entry) => entry.1 = toc.clone(),
                    None => self.tocs.push((*block_id, toc.clone())),
                }
            }
            ProseEvent::FootnotesUpdated { notes } => self.footnotes.clone_from(notes),
            ProseEvent::CrossRefsUpdated { refs } => self.cross_refs.clone_from(refs),
            ProseEvent::CommentsUpdated { threads } => self.comments.clone_from(threads),
            ProseEvent::RevealBlock { page, .. } => self.reveal_page = Some(*page),
            ProseEvent::CaretPlaced { block, offset } => {
                self.cursor = Some(CursorPosition {
                    block_id: *block,
                    offset: *offset,
                    anchor: None,
                });
            }
            ProseEvent::BlockSplit { new_block, .. } => {
                self.cursor = Some(CursorPosition {
                    block_id: *new_block,
                    offset: 0,
                    anchor: None,
                });
            }
            ProseEvent::FindResults { matches, current } => {
                self.find_matches = matches.clone();
                self.find_current = *current;
            }
            ProseEvent::FindUpdated { current } => self.find_current = *current,
            ProseEvent::BandsUpdated { bands } => self.bands = bands.clone(),
            ProseEvent::RemoteCursors { cursors } => self.remote_cursors = cursors.clone(),
            // A picture arriving, a masked cell, a refusal and an error each
            // repaint or warn, and none of them touches what is being typed
            // (`PROSE-UI-027`).
            ProseEvent::MediaArrived { .. }
            | ProseEvent::MergeConflictDetected { .. }
            | ProseEvent::DocumentRefused { .. }
            | ProseEvent::Error(_) => {}
        }
    }

    /// Advance the recount clock by one frame, and answer the recount when
    /// one is due (`PROSE-FEAT-124`).
    ///
    /// A frame that saw a change restarts the wait, so a run of changes costs
    /// one recount, [`RECOUNT_DELAY_MS`] after the last of them. The caller
    /// sends the command it gets back.
    pub fn tick(&mut self, now_ms: f64) -> Option<ProseCommand> {
        if std::mem::take(&mut self.changed) {
            self.quiet_since_ms = Some(now_ms);
            return None;
        }
        let since = self.quiet_since_ms?;
        if now_ms - since < RECOUNT_DELAY_MS {
            return None;
        }
        self.quiet_since_ms = None;
        Some(ProseCommand::CountWords)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_document() -> DocumentId {
        DocumentId::from_bytes([7; 16])
    }

    #[test]
    fn a_full_channel_drops_the_command_rather_than_holding_the_frame() {
        let (tx, _rx) = flume::bounded::<LogicCommand>(1);
        assert_eq!(
            send_command(&tx, a_document(), ProseCommand::Noop),
            SendResult::Sent
        );
        // The channel holds one, so the second send finds it full. It answers
        // at once rather than parking this thread.
        assert_eq!(
            send_command(&tx, a_document(), ProseCommand::Noop),
            SendResult::Dropped
        );
    }

    #[test]
    fn a_closed_channel_is_told_apart_from_a_full_one() {
        let (tx, rx) = flume::bounded::<LogicCommand>(1);
        drop(rx);
        assert_eq!(
            send_command(&tx, a_document(), ProseCommand::Noop),
            SendResult::Disconnected
        );
    }

    #[test]
    fn the_draft_echo_never_reaches_the_wire() {
        let (tx, rx) = flume::bounded::<LogicCommand>(4);
        let block = BlockId::new(1, 0);
        let mut state = ProseUiState::default();
        state.draft.push_str("hel", &block, 0);
        send_command(
            &tx,
            a_document(),
            ProseCommand::InsertText {
                block_id: block,
                pos: 0,
                text: "hel".to_owned(),
            },
        );
        let sent = rx.try_recv().expect("the command crossed");
        let LogicCommand::App { command, .. } = sent else {
            panic!("the envelope carries the command");
        };
        // The draft is still held here, and the wire carries the typed command
        // rather than the echo.
        assert!(!state.draft.is_empty());
        assert!(matches!(
            command,
            AppCommand::Prose(ProseCommand::InsertText { .. })
        ));
    }

    #[test]
    fn a_confirming_change_clears_the_draft_it_covers() {
        let block = BlockId::new(1, 0);
        let mut state = ProseUiState::default();
        state.draft.push_str("hel", &block, 0);
        state.apply_event(&ProseEvent::DocUpdated { delta: vec![1] });
        assert!(state.draft.is_empty());
    }

    #[test]
    fn an_event_carrying_no_change_leaves_the_draft_alone() {
        let block = BlockId::new(1, 0);
        let mut state = ProseUiState::default();
        state.draft.push_str("hel", &block, 0);
        state.apply_event(&ProseEvent::MediaArrived {
            content_hash: "abc".to_owned(),
            blocks: Vec::new(),
        });
        assert_eq!(state.draft.pending_text, "hel");
    }

    #[test]
    fn a_split_leaves_the_caret_at_the_start_of_the_new_paragraph() {
        let source = BlockId::new(1, 0);
        let new_block = BlockId::new(1, 4);
        let mut state = ProseUiState::default();
        state.apply_event(&ProseEvent::BlockSplit { source, new_block });
        let cursor = state.cursor.expect("the caret moved");
        assert_eq!(cursor.block_id, new_block);
        assert_eq!(cursor.offset, 0);
    }

    #[test]
    fn consecutive_keystrokes_in_one_block_accumulate_in_one_draft() {
        let block = BlockId::new(1, 0);
        let other = BlockId::new(1, 1);
        let mut draft = DraftBuffer::default();
        draft.push_str("he", &block, 0);
        draft.push_str("llo", &block, 2);
        assert_eq!(draft.pending_text, "hello");
        assert_eq!(draft.insert_pos, 0);
        draft.push_str("x", &other, 0);
        assert_eq!(draft.pending_text, "x");
        assert_eq!(draft.block_id, Some(other));
    }

    #[test]
    fn a_run_of_changes_costs_one_recount_after_it_stops() {
        let mut state = ProseUiState::default();
        let mut recounts = 0;
        // A change on every frame for a second, one frame every sixteen
        // milliseconds: no recount while the run lasts.
        let mut now = 0.0;
        while now < 1000.0 {
            state.apply_event(&ProseEvent::DocUpdated { delta: vec![1] });
            recounts += usize::from(state.tick(now).is_some());
            now += 16.0;
        }
        assert_eq!(recounts, 0);
        // Then the text stays still for two seconds: one recount, once the
        // delay has run out, and no second one.
        let stopped = now;
        while now < stopped + 2000.0 {
            if let Some(command) = state.tick(now) {
                assert_eq!(command, ProseCommand::CountWords);
                assert!(now - stopped >= RECOUNT_DELAY_MS - 16.0);
                recounts += 1;
            }
            now += 16.0;
        }
        assert_eq!(recounts, 1);
    }

    #[test]
    fn a_closed_search_leaves_nothing_highlighted() {
        let mut ui = ProseUiState::default();
        let hit = FindMatch {
            block_id: BlockId::new(1, 0),
            range: (0, 3),
        };
        ui.apply_event(&ProseEvent::FindResults {
            matches: vec![hit],
            current: Some(0),
        });
        assert_eq!(ui.find_matches, vec![hit]);
        ui.apply_event(&ProseEvent::FindResults {
            matches: Vec::new(),
            current: None,
        });
        assert!(ui.find_matches.is_empty());
        assert_eq!(ui.find_current, None);
    }
}
