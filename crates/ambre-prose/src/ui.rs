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

use crate::events::{BlockId, CursorPosition, ProseCommand, ProseEvent, RemoteCursor};

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

    // Local, and never sent.
    pub viewport: Viewport,
    pub cursor: Option<CursorPosition>,
    pub draft: DraftBuffer,
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
            ProseEvent::DocUpdated { .. } => self.draft.clear(),
            ProseEvent::BlockSplit { new_block, .. } => {
                self.cursor = Some(CursorPosition {
                    block_id: *new_block,
                    offset: 0,
                    anchor: None,
                });
            }
            ProseEvent::RemoteCursors { cursors } => self.remote_cursors = cursors.clone(),
            // A picture arriving, a refusal and an error each repaint or warn,
            // and none of them touches what is being typed (`PROSE-UI-027`).
            ProseEvent::MediaArrived { .. }
            | ProseEvent::DocumentRefused { .. }
            | ProseEvent::Error(_) => {}
        }
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
}
