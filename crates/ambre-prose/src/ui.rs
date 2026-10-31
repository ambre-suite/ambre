//! The UI thread's own state, and the one path a mutation leaves it by
//! (`features/ambre-spec-prose-engine.md section 3.G`).
//!
//! This thread holds view state and what arrived as an event, and no document
//! data (`PROSE-UI-001`). The scroll position and the selection stay here:
//! neither enters the document, so two collaborators scroll independently and
//! read the same document (`PROSE-UI-010`).
//!
//! **It never blocks on the Logic channel** (`UI-THREAD-002`). Every send is a
//! `try_send`. An edit goes through the lossless outbox of
//! `ambre-spec-message-bus.md`, which holds it on a full channel (`BUS-011`,
//! `PROSE-UI-149`). Any other command is dropped and asked again later.
//!
//! The draft below holds what was typed and not yet confirmed. A remote
//! change shifts it, a deleted block drops it, and a layout that carries its
//! text empties it. The input-method composition lands with the "Toolbar,
//! slash menu and composition" Story (`ambre-roadmap.md` E5).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use ambre_core::id::{Document, Id};
use ambre_core::{ByteCount, ByteOffset, ByteSpan, char_to_byte};

use ambre_bus::app::prose::{DocCount, DocDelta, DocOffset};
use ambre_bus::logic::{AppCommand, LogicCommand};
use ambre_bus::{Outbox, SendOutcome};

use crate::events::{
    BandSet, BlockId, ChangeOrigin, CommentThread, CursorPosition, FindMatch, FootnoteEntry,
    LinkSpan, PageIndex, ProseCommand, ProseEvent, RemoteCursor, ResolvedCrossRef, Suggestion,
    TableOfContents, TextEdit, WordCount,
};
use crate::layout::run::LayoutVersion;
use crate::layout::run::{RunOffset, ShapedLayout, ShapedRun};
use crate::rebase;
use crate::view::Highlight;
use crate::view::cache::RunCache;
use crate::view::column::PageColumn;

pub use crate::view::column::Viewport;

/// How long the text must stay still before the four tallies are recounted
/// (`PROSE-FEAT-124`).
///
/// A fast typist then pays for one recount per pause rather than one per
/// keystroke (`features/ambre-spec-prose-word-count.md section 4`).
pub const RECOUNT_DELAY: ambre_core::PageMillis = ambre_core::PageMillis::new(500.0);

/// The changes a draft went through. A paragraph shaped from the draft can
/// so tell it still reads what it was shaped from.
pub enum DraftRevision {}

impl ambre_core::GenerationKind for DraftRevision {
    type IssueRight = ();
    const NAME: &'static str = "DraftRevision";
}

/// What this view drew and the layout in force does not carry yet: the text
/// typed and the text deleted. It is held apart from the document, never
/// written into a second copy of it.
///
/// Each entry is one command sent, inside one block. It cannot express a split
/// or a mark (`PROSE-UI-135`). It never reaches the wire (`PROSE-UI-150`): what
/// reaches the wire is the command the same keystroke emitted.
///
/// An entry counts code points of the block as it reads once every entry
/// before it applied. Laid over the text of the layout in force, the entries
/// give the text the view shows (`PROSE-UI-201`, `PROSE-UI-202`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DraftBuffer {
    edits: Vec<DraftEdit>,
    /// Moves on every change, so a paragraph shaped from the draft can tell
    /// it still reads what it was shaped from.
    generation: ambre_core::Generation<DraftRevision>,
}

/// One command sent inside a block, and whether the Logic thread took it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DraftEdit {
    /// The number the Logic thread acknowledges this command by.
    seq: ambre_bus::app::prose::TextAck,
    block: BlockId,
    at: DocOffset,
    change: Change,
    /// Taken as drawn: the document holds it, and the next layout will.
    confirmed: Confirmation,
}

ambre_core::define_flag! {
    /// Whether the Logic thread took a draft entry as drawn.
    pub enum Confirmation {
        /// The entry waits for its answer.
        Pending,
        /// The document holds the entry, and the next layout will.
        Confirmed,
    }
}

/// What one draft entry does. The inserted text is the user's, so `Debug`
/// prints its length alone.
#[derive(Clone, PartialEq, Eq)]
enum Change {
    Insert(String),
    Delete(DocCount),
}

impl std::fmt::Debug for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Insert(text) => f
                .debug_tuple("Insert")
                .field(&crate::redact::Redacted(text))
                .finish(),
            Self::Delete(count) => f.debug_tuple("Delete").field(count).finish(),
        }
    }
}

impl DraftBuffer {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Each entry's acknowledgement number, and whether the Logic thread
    /// confirmed it. The keystroke probes read it (`ambre-spec-budgets.md`).
    pub fn entries(
        &self,
    ) -> impl Iterator<Item = (ambre_bus::app::prose::TextAck, Confirmation)> + '_ {
        self.edits.iter().map(|edit| (edit.seq, edit.confirmed))
    }

    /// A number that moves whenever what the draft shows does.
    #[must_use]
    pub fn generation(&self) -> ambre_core::Generation<DraftRevision> {
        self.generation
    }

    /// Text sent to `pos` in `block_id`, acknowledged as `seq`
    /// (`PROSE-UI-136`). Each keystroke is its own entry, so each confirms on
    /// its own (`PROSE-UI-138`).
    pub fn push_str(
        &mut self,
        seq: ambre_bus::app::prose::TextAck,
        text: &str,
        block_id: &BlockId,
        pos: DocOffset,
    ) {
        self.push(seq, *block_id, pos, Change::Insert(text.to_owned()));
    }

    /// A deletion sent over `start..end` in `block_id`, acknowledged as
    /// `seq`. The text typed before it stays drawn (`PROSE-UI-206`).
    pub fn push_delete(
        &mut self,
        seq: ambre_bus::app::prose::TextAck,
        block_id: &BlockId,
        start: DocOffset,
        end: DocOffset,
    ) {
        let (start, end) = (start.min(end), start.max(end));
        if end > start {
            self.push(seq, *block_id, start, Change::Delete(end.count_from(start)));
        }
    }

    fn push(
        &mut self,
        seq: ambre_bus::app::prose::TextAck,
        block: BlockId,
        at: DocOffset,
        change: Change,
    ) {
        self.edits.push(DraftEdit {
            seq,
            block,
            at,
            change,
            confirmed: Confirmation::Pending,
        });
        self.bump();
    }

    fn bump(&mut self) {
        self.generation = self.generation.next();
    }

    /// The command acknowledged as `seq` was taken as drawn
    /// (`PROSE-UI-005`). The entries confirm in the order they were sent.
    pub fn confirm(&mut self, seq: ambre_bus::app::prose::TextAck) {
        if let Some(edit) = self.edits.iter_mut().find(|edit| edit.seq == seq) {
            edit.confirmed = Confirmation::Confirmed;
        }
    }

    /// Move every entry by what a collaborator's change did before it
    /// (`PROSE-UI-028`).
    pub fn shift(&mut self, edits: &[TextEdit]) {
        for edit in &mut self.edits {
            edit.at = rebase::through_all(edit.at, edit.block, edits);
        }
        self.bump();
    }

    /// A layout was taken: what the Logic thread confirmed is in its runs
    /// now. Confirmed entries lead the log, since they confirm in order.
    pub fn laid(&mut self) {
        let before = self.edits.len();
        self.edits
            .retain(|edit| edit.confirmed == Confirmation::Pending);
        if self.edits.len() != before {
            self.bump();
        }
    }

    /// Drop what the draft holds of a block, the block being gone
    /// (`PROSE-UI-143`).
    pub fn forget(&mut self, block: BlockId) {
        self.edits.retain(|edit| edit.block != block);
        self.bump();
    }

    /// The blocks the draft changes, each once, in the order first touched.
    pub fn blocks(&self) -> Vec<BlockId> {
        let mut blocks: Vec<BlockId> = Vec::new();
        for edit in &self.edits {
            if !blocks.contains(&edit.block) {
                blocks.push(edit.block);
            }
        }
        blocks
    }

    /// Whether the draft changes `block`.
    #[must_use]
    pub fn touches(&self, block: BlockId) -> bool {
        self.edits.iter().any(|edit| edit.block == block)
    }

    fn of(&self, block: BlockId) -> impl DoubleEndedIterator<Item = &DraftEdit> {
        self.edits.iter().filter(move |edit| edit.block == block)
    }

    /// The text `block` shows: `laid`, as the layout in force holds it, with
    /// every entry laid over it in order.
    #[must_use]
    pub fn text_of(&self, block: BlockId, laid: &str) -> String {
        let mut text = laid.to_owned();
        for edit in self.of(block) {
            let start = char_to_byte(&text, edit.at.as_char());
            match &edit.change {
                Change::Insert(inserted) => text.insert_str(start.to_usize(), inserted),
                Change::Delete(len) => {
                    let end = char_to_byte(&text, (edit.at + *len).as_char());
                    text.replace_range(ByteSpan::new(start, end).to_usize_range(), "");
                }
            }
        }
        text
    }

    /// The marked spans of the text [`Self::text_of`] gives, from those of
    /// the laid text. Text typed after a mark takes it (`PROSE-UI-208`).
    #[must_use]
    pub fn marks_of(
        &self,
        block: BlockId,
        laid: &str,
        marks: &[crate::layout::measure::MarkedSpan],
    ) -> Vec<crate::layout::measure::MarkedSpan> {
        let mut text = laid.to_owned();
        let mut marks = marks.to_vec();
        for edit in self.of(block) {
            let start = char_to_byte(&text, edit.at.as_char());
            match &edit.change {
                Change::Insert(inserted) => {
                    let n = ByteCount::from_len(inserted.len());
                    // A mark expands after its range: text typed at its end
                    // joins it, text typed at its start does not.
                    for mark in &mut marks {
                        if mark.start >= start {
                            mark.start += n;
                            mark.end += n;
                        } else if mark.end >= start {
                            mark.end += n;
                        }
                    }
                    text.insert_str(start.to_usize(), inserted);
                }
                Change::Delete(len) => {
                    let end = char_to_byte(&text, (edit.at + *len).as_char());
                    let gone = end.count_from(start);
                    let squeeze = |at: ByteOffset| {
                        if at <= start {
                            at
                        } else if at >= end {
                            at - gone
                        } else {
                            start
                        }
                    };
                    for mark in &mut marks {
                        mark.start = squeeze(mark.start);
                        mark.end = squeeze(mark.end);
                    }
                    marks.retain(|mark| mark.end > mark.start);
                    text.replace_range(ByteSpan::new(start, end).to_usize_range(), "");
                }
            }
        }
        marks
    }

    /// How many code points `block` holds past the layout in force, negative
    /// when the draft deleted more than it typed.
    #[must_use]
    pub fn delta(&self, block: BlockId) -> DocDelta {
        self.of(block)
            .map(|edit| match &edit.change {
                Change::Insert(text) => DocDelta::from_count(DocCount::of(text)),
                Change::Delete(len) => DocDelta::from_count(*len).reversed(),
            })
            .sum()
    }

    /// An offset of the layout in force, as the view's text counts it. An
    /// offset at an insertion stays before it.
    #[must_use]
    pub fn to_doc(&self, block: BlockId, laid: RunOffset) -> DocOffset {
        let start = laid.as_doc();
        self.of(block).fold(start, |at, edit| match &edit.change {
            Change::Insert(text) if at > edit.at => at + DocCount::of(text),
            Change::Delete(len) if at > edit.at => (at - *len).max(edit.at),
            _ => at,
        })
    }

    /// An offset of the view's text, as the layout in force counts it.
    /// Inside text the layout lacks, it lands where that text goes.
    #[must_use]
    pub fn to_laid(&self, block: BlockId, offset: DocOffset) -> RunOffset {
        let laid = self
            .of(block)
            .rev()
            .fold(offset, |at, edit| match &edit.change {
                Change::Insert(text) if at > edit.at => (at - DocCount::of(text)).max(edit.at),
                Change::Delete(len) if at > edit.at => at + *len,
                _ => at,
            });
        RunOffset::from_doc(laid)
    }

    pub fn clear(&mut self) {
        if !self.edits.is_empty() {
            self.edits.clear();
            self.bump();
        }
    }
}

/// A file the Logic thread wrote, for the host to hand to the browser
/// (`PROSE-IO-056`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportFile {
    /// The name the download carries.
    pub filename: ambre_core::ExportFilename,
    /// The media type the Logic thread named for the bytes.
    pub media_type: ambre_core::MediaType,
    /// The file itself.
    pub bytes: ambre_core::Payload<ambre_core::FileBytes>,
}

ambre_core::define_flag! {
    /// Whether a press waits for the first block it asked for.
    pub(crate) enum BlockWait {
        /// No press waits.
        Idle,
        /// A press waits, and the caret goes to the block once a layout
        /// carries it.
        Awaiting,
    }

    /// Whether a sync holds the input until it is answered.
    pub(crate) enum SyncHold {
        /// The input flows on while the sync is in flight.
        Passes,
        /// The sync follows a structural command, and the input waits.
        Holds,
    }

    /// Whether the document changed since the view last ticked.
    enum Changed {
        /// No change arrived.
        Unchanged,
        /// A change arrived, and a recount is owed.
        Changed,
    }
}

/// The UI thread's whole state for one open Prose document.
///
/// Every field is either handed over by the Logic thread or local to this
/// thread. None of them is the document itself (`PROSE-UI-001`).
///
/// Many fields carry copies of its text: the runs, the drafts, the comments,
/// the text in composition and its shape. So `Debug` prints a length or a
/// count for each of those, and the text of none.
#[derive(Clone, Default)]
pub struct ProseUiState {
    // Handed over by the Logic thread.
    /// The version of the last layout this view adopted. An older one is
    /// dropped (`features/ambre-spec-prose-page-rendering.md`).
    pub layout_version: ambre_core::Generation<LayoutVersion>,
    /// The whole live peer set, replaced on each event (`PRESENCE-009`).
    pub remote_cursors: Vec<RemoteCursor>,
    /// The five figures as last answered, for the shared status bar slot.
    pub word_count: WordCount,
    /// The entries of every table of contents, as last answered, by block.
    pub tocs: Vec<(BlockId, TableOfContents)>,
    /// The matches of this peer's own search, to highlight, and the current
    /// one. Empty once the search closes (`PROSE-FEAT-289`).
    pub find_matches: Vec<FindMatch>,
    pub find_current: Option<ambre_bus::app::find::MatchIndex>,
    /// The four band blocks and their settings, as last answered: the blocks
    /// the ordinary text commands aim at to edit a band.
    pub bands: BandSet,
    /// Every footnote, numbered or orphaned, as last answered.
    pub footnotes: Vec<FootnoteEntry>,
    /// Every comment thread, marked or orphaned, as last answered.
    pub comments: Vec<CommentThread>,
    /// Every pending suggestion, and whether this session is suggesting, as
    /// last answered.
    pub suggestions: Vec<Suggestion>,
    pub edit_mode: crate::events::EditMode,
    /// Every run of a link mark and its target, as last answered.
    pub links: Vec<LinkSpan>,
    /// Every cross reference, as last resolved.
    pub cross_refs: Vec<ResolvedCrossRef>,
    /// The page a followed reference asked to bring into view, until the
    /// view scrolls there.
    pub reveal_page: Option<PageIndex>,
    /// The layout in force and the runs it carries, as last taken
    /// (`features/ambre-spec-prose-page-rendering.md`).
    pub shaped: Arc<ShapedLayout>,
    /// A file the Logic thread wrote, waiting for the host to hand it to the
    /// browser (`PROSE-IO-056`).
    pub pending_export: Option<ExportFile>,

    // Local, and never sent.
    pub viewport: Viewport,
    /// The page column, rebuilt from each layout taken (`PROSE-UI-131`).
    pub column: PageColumn,
    /// The drawing state of the runs painted lately (`PROSE-UI-019`).
    pub runs: RunCache,
    /// The inline highlights, by the block they sit in (`PROSE-UI-020`).
    pub(crate) highlights: HashMap<BlockId, Vec<Highlight>>,
    /// The other peers' carets, by the block each sits in (`PROSE-UI-020`).
    pub(crate) remote_at: HashMap<BlockId, Vec<RemoteCursor>>,
    /// How many highlights paints have read, for the tests to hold
    /// `PROSE-UI-020` against.
    pub(crate) highlights_walked: ambre_core::Tally<crate::view::walk::Highlights>,
    /// A link target a press asked to open, until the host takes it
    /// (`PROSE-UI-071`).
    pub followed: Option<ambre_core::LinkUrl>,
    /// The payload of the last copy, until the host mirrors it to the system
    /// clipboard (`PROSE-UI-041`).
    pub(crate) pending_clip: Option<ambre_bus::ClipboardBlob>,
    /// The payload of the last copy, kept for a paste back into the same tab
    /// while the system clipboard still holds its text (`PROSE-UI-043`).
    pub(crate) last_clip: Option<ambre_bus::ClipboardBlob>,
    /// The suggestion under the pointer, for the accept or reject popover
    /// (`PROSE-UI-148`).
    pub hovered: Option<crate::view::annotate::HoveredSuggestion>,
    pub cursor: Option<CursorPosition>,
    /// The caret sits at the end of the line before a wrap point rather than
    /// at the start of the line after it (`PROSE-UI-159`).
    pub caret_affinity: crate::layout::run::Affinity,
    /// Where the last paint drew the local caret, in canvas pixels. The host
    /// places the input pipe there (`CHROME-007`).
    pub caret_drawn: Option<ambre_draw::Rect>,
    /// The canvas transform of the page being painted, which takes a caret
    /// drawn on it to `caret_drawn`.
    pub(crate) page_frame: ambre_draw::Affine,
    /// The column a run of vertical moves keeps (`PROSE-UI-034`).
    pub(crate) x_goal: Option<ambre_core::Px<ambre_core::Doc>>,
    /// The page whose header or footer holds the caret. A band is drawn on
    /// every page, and the caret shows on the one it was placed on
    /// (`PROSE-UI-033`).
    pub(crate) band_page: Option<PageIndex>,
    /// A drag in flight: its anchor is frozen while it lasts (`PROSE-UI-152`).
    pub(crate) dragging: Option<(BlockId, DocOffset)>,
    /// The last pointer of the drag in flight, and its autoscroll
    /// (`PROSE-UI-038`). Both go with the drag.
    pub(crate) drag_pointer: Option<ambre_core::Point<ambre_core::Px<ambre_core::View>>>,
    pub(crate) autoscroll: ambre_draw::kinetic::EdgeScroll,
    /// A finger on the canvas, and the motion a flick left
    /// (`PROSE-UI-217`).
    pub(crate) kinetic: ambre_draw::kinetic::Kinetic,
    /// The last press, to count a second and a third one in place.
    pub(crate) last_press: Option<crate::input::Press>,
    /// A press asked for a first block, and the caret goes to it once a
    /// layout carries it (`PROSE-UI-072`).
    pub(crate) awaiting_block: BlockWait,
    /// A block asked to show that the layout in force does not carry. The
    /// view scrolls to it once a layout does (`PROSE-UI-212`).
    pub(crate) awaited_reveal: Option<BlockId>,
    /// The pages last asked of the Logic thread, first and last, so a frame
    /// does not ask twice for the same (`PROSE-UI-211`).
    pub(crate) asked_pages: Option<(PageIndex, PageIndex)>,
    /// The block last asked for in a [`ProseCommand::ShowBlock`].
    pub(crate) asked_block: Option<BlockId>,
    pub draft: DraftBuffer,
    /// The blocks the draft or a split changes, shaped again on this thread
    /// (`PROSE-UI-201`).
    pub(crate) provisional: crate::view::provisional::Provisional,
    /// A split sent and not yet laid out (`PROSE-UI-203`).
    pub(crate) splitting: Option<crate::view::provisional::Splitting>,
    /// The composition as last shaped, and the text it was shaped from.
    pub(crate) compose_shape: Option<(String, ShapedRun)>,
    /// The marks the Logic thread read for a selection, and that selection
    /// (`PROSE-UI-057`).
    pub(crate) marks: Option<(
        (crate::view::caret::Spot, crate::view::caret::Spot),
        crate::events::ActiveMarks,
    )>,
    /// The selection and the layout version the last read was asked for.
    pub(crate) marks_asked: Option<(
        (crate::view::caret::Spot, crate::view::caret::Spot),
        ambre_core::Generation<LayoutVersion>,
    )>,
    /// The slash menu, while it is open.
    pub(crate) slash: Option<crate::view::slash::SlashMenu>,
    /// The input method's text still being composed. It is drawn at the
    /// caret and sends nothing until the composition ends (`PROSE-UI-142`).
    pub(crate) composing: String,
    /// A change arrived since the last tick.
    changed: Changed,
    /// When the text was last seen changing, while a recount is owed.
    quiet_since: Option<ambre_core::PageTime>,
    /// The number of the last change this view did not draw and applied.
    pub(crate) seen: ambre_core::Generation<ambre_bus::app::prose::TextChangeSeq>,
    /// The number last handed back in a [`ProseCommand::Sync`].
    pub(crate) sent_seen: ambre_core::Generation<ambre_bus::app::prose::TextChangeSeq>,
    /// The text commands sent and not yet taken, in the text this view shows.
    pub(crate) inflight: VecDeque<TextEdit>,
    /// How many text commands the Logic thread said it took.
    pub(crate) popped: ambre_bus::app::prose::TextAck,
    /// The syncs sent and not yet answered, oldest first. One sent after a
    /// structural command holds the input until it is answered.
    pub(crate) syncs: VecDeque<SyncHold>,
    /// The input held meanwhile, replayed in order once it is answered.
    pub(crate) queued: VecDeque<crate::input::ProseInput>,
}

impl std::fmt::Debug for ProseUiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProseUiState")
            .field("layout_version", &self.layout_version)
            .field("remote_cursors", &self.remote_cursors.len())
            .field("tocs", &self.tocs.len())
            .field("find_matches", &self.find_matches.len())
            .field("find_current", &self.find_current)
            .field("footnotes", &self.footnotes.len())
            .field("comments", &self.comments.len())
            .field("suggestions", &self.suggestions.len())
            .field("edit_mode", &self.edit_mode)
            .field("links", &self.links.len())
            .field("cross_refs", &self.cross_refs.len())
            .field("reveal_page", &self.reveal_page)
            .field("blocks", &self.shaped.blocks.len())
            .field("pending_export", &self.pending_export.is_some())
            .field("viewport", &self.viewport)
            .field("cursor", &self.cursor)
            .field("caret_affinity", &self.caret_affinity)
            .field("draft", &self.draft)
            .field("composing", &crate::redact::Redacted(&self.composing))
            .field("compose_shape", &self.compose_shape.is_some())
            .field("slash", &self.slash.is_some())
            .field("seen", &self.seen)
            .field("sent_seen", &self.sent_seen)
            .field("popped", &self.popped)
            .field("inflight", &self.inflight.len())
            .field("syncs", &self.syncs)
            .field("queued", &self.queued.len())
            .finish_non_exhaustive()
    }
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
    /// The channel was full, and the outbox holds the edit for a later frame
    /// (`BUS-011`). It is on its way, and the view refuses input meanwhile.
    Held,
    /// Nothing carries the command: a full channel dropped a command outside
    /// the edit class, or the outbox refused an edit at its bound (`BUS-018`).
    Dropped,
    /// The Logic thread is gone. This channel carries nothing again.
    Disconnected,
}

impl SendResult {
    /// Whether the command is on its way to the Logic thread.
    #[must_use]
    pub fn accepted(self) -> bool {
        matches!(self, Self::Sent | Self::Held)
    }
}

/// Send one command to the Logic thread (`PROSE-UI-002`).
///
/// Every mutation leaves this thread this way, and no other. The send never
/// blocks (`UI-THREAD-002`), and the draft echo the view painted stays here
/// (`PROSE-UI-150`): what crosses is the typed command alone.
pub fn send_command(
    to_logic: &ambre_bus::conduit::Sender<LogicCommand>,
    document_id: Id<Document>,
    command: ProseCommand,
) -> SendResult {
    let envelope = LogicCommand::App {
        document_id,
        command: AppCommand::Prose(command),
    };
    match to_logic.try_send(envelope) {
        Ok(()) => SendResult::Sent,
        Err(ambre_bus::conduit::TrySendError::Full(_)) => {
            tracing::warn!("Prose: the Logic channel is full, the command is dropped");
            SendResult::Dropped
        }
        Err(ambre_bus::conduit::TrySendError::Disconnected(_)) => SendResult::Disconnected,
    }
}

/// Send one edit to the Logic thread through the lossless outbox
/// (`PROSE-UI-149`, `BUS-011`).
///
/// A full channel holds the edit rather than the frame or the keystroke. The
/// host refuses input while the outbox holds anything (`BUS-016`), so the
/// order the user typed in is the order the edits land.
pub fn send_edit(
    outbox: &mut Outbox,
    document_id: Id<Document>,
    command: ProseCommand,
) -> SendResult {
    let envelope = LogicCommand::App {
        document_id,
        command: AppCommand::Prose(command),
    };
    match outbox.send(envelope) {
        SendOutcome::Sent => SendResult::Sent,
        SendOutcome::Held => {
            tracing::warn!("Prose: the Logic channel is full, the edit is held");
            SendResult::Held
        }
        SendOutcome::Refused => {
            tracing::error!("Prose: the outbox is full, the edit is refused");
            SendResult::Dropped
        }
        SendOutcome::Disconnected => SendResult::Disconnected,
    }
}

impl ProseUiState {
    /// Hand the file the Logic thread wrote to the host, once.
    pub fn take_export(&mut self) -> Option<ExportFile> {
        self.pending_export.take()
    }

    /// Take in one display event.
    ///
    /// A change to the text confirms what the draft sent, or shifts the draft
    /// and the caret by what a collaborator did (`PROSE-UI-005`,
    /// `PROSE-UI-028`). An event that changes no text leaves the draft alone
    /// (`PROSE-UI-027`). A split leaves the caret at the start of the new
    /// paragraph (`PROSE-UI-141`), because the event names the block minted.
    pub fn apply_event(&mut self, event: &ProseEvent) {
        match event {
            ProseEvent::Clipboard {
                text,
                html,
                payload,
            } => {
                let blob = ambre_bus::ClipboardBlob {
                    text: text.clone(),
                    html: Some(html.clone()),
                    ambre: Some(payload.clone()),
                    form: ambre_core::PasteForm::Rich,
                };
                self.last_clip = Some(blob.clone());
                self.pending_clip = Some(blob);
            }
            // The view does not interpret the bytes; the host saves them.
            ProseEvent::ExportReady {
                filename,
                media_type,
                data,
                ..
            } => {
                self.pending_export = Some(ExportFile {
                    filename: filename.clone(),
                    media_type: media_type.clone(),
                    bytes: data.clone(),
                });
            }
            // Whole and already shaped: the view adopts it and lays nothing
            // out (`PROSE-UI-006`), by its pointer (`PROSE-UI-205`). A handle
            // of another type is not a layout.
            ProseEvent::LayoutShaped { layout } => {
                if let Some(shaped) = layout.shared::<ShapedLayout>() {
                    self.apply_layout(shaped);
                }
            }
            ProseEvent::TextChanged {
                origin,
                seq,
                acked,
                edits,
                removed,
            } => {
                let origin = *origin;
                self.take_acked(*acked, origin);
                if origin == ChangeOrigin::Remote {
                    self.seen = *seq;
                }
                self.take_text_change(origin, edits, removed);
            }
            ProseEvent::Synced { acked } => {
                self.take_acked(*acked, ChangeOrigin::Local);
                // A split the document refused mints nothing to wait for.
                if self.syncs.pop_front() == Some(SyncHold::Holds)
                    && self
                        .splitting
                        .is_some_and(|split| split.new_block.is_none())
                {
                    self.splitting = None;
                }
            }
            ProseEvent::DocUpdated { .. } => self.changed = Changed::Changed,
            ProseEvent::CellEntered { block, .. } => self.place_caret(*block, DocOffset::ZERO),
            ProseEvent::WordCountUpdated { count } => self.word_count = *count,
            ProseEvent::SelectionMarks {
                anchor,
                focus,
                marks,
            } => self.take_marks(*anchor, *focus, marks),
            ProseEvent::TocUpdated { block_id, toc } => {
                match self.tocs.iter_mut().find(|(id, _)| id == block_id) {
                    Some(entry) => entry.1 = toc.clone(),
                    None => self.tocs.push((*block_id, toc.clone())),
                }
            }
            ProseEvent::FootnotesUpdated { notes } => self.footnotes.clone_from(notes),
            ProseEvent::CrossRefsUpdated { refs } => self.cross_refs.clone_from(refs),
            ProseEvent::LinksUpdated { links } => self.links.clone_from(links),
            ProseEvent::CommentsUpdated { threads } => {
                self.comments.clone_from(threads);
                self.index_highlights();
            }
            ProseEvent::SuggestionsUpdated { mode, suggestions } => {
                self.edit_mode = *mode;
                self.suggestions.clone_from(suggestions);
                // A suggestion accepted or rejected takes its popover along.
                if let Some(hovered) = self.hovered
                    && !suggestions
                        .iter()
                        .any(|s| s.suggestion_id == hovered.suggestion_id)
                {
                    self.hovered = None;
                }
                self.index_highlights();
            }
            ProseEvent::RevealBlock { page, .. } => self.reveal_page = Some(*page),
            ProseEvent::CaretPlaced { block, offset } => self.place_caret(*block, *offset),
            ProseEvent::BlockSplit { new_block, .. } => {
                if let Some(split) = self.splitting.as_mut() {
                    split.new_block = Some(*new_block);
                }
                self.place_caret(*new_block, DocOffset::ZERO);
            }
            ProseEvent::FindResults { matches, current } => {
                self.find_matches = matches.clone();
                self.find_current = *current;
                self.index_highlights();
                self.reveal_current_match();
            }
            ProseEvent::FindUpdated { current } => {
                self.find_current = *current;
                self.reveal_current_match();
            }
            ProseEvent::BandsUpdated { bands } => self.bands = bands.clone(),
            ProseEvent::RemoteCursors { cursors } => {
                self.remote_cursors = cursors.clone();
                self.remote_at.clear();
                for cursor in cursors {
                    self.remote_at
                        .entry(cursor.position.block_id)
                        .or_default()
                        .push(cursor.clone());
                }
            }
            // A picture arriving, a masked cell, a refusal and an error each
            // repaint or warn, and none of them touches what is being typed
            // (`PROSE-UI-027`). The history's availability is the chrome's.
            ProseEvent::MediaArrived { .. }
            | ProseEvent::MergeConflictDetected { .. }
            | ProseEvent::DocumentRefused { .. }
            | ProseEvent::ImportCompleted { .. }
            | ProseEvent::UndoState { .. }
            | ProseEvent::Error(_) => {}
        }
    }

    /// A caret the document placed: no selection, no column kept.
    fn place_caret(&mut self, block_id: BlockId, offset: DocOffset) {
        self.cursor = Some(CursorPosition {
            block_id,
            offset,
            anchor: None,
        });
        self.caret_affinity = crate::layout::run::Affinity::Downstream;
        self.x_goal = None;
    }

    /// Drop the text commands the Logic thread took. A command it took
    /// as this view drew it confirms the draft; one it did not ends the
    /// draft, whose text the change takes back.
    fn take_acked(&mut self, acked: ambre_bus::app::prose::TextAck, origin: ChangeOrigin) {
        let mut ends_draft = false;
        while self.popped < acked {
            self.popped = self.popped.next();
            let Some(edit) = self.inflight.pop_front() else {
                continue;
            };
            if origin == ChangeOrigin::Local {
                self.draft.confirm(self.popped);
            } else {
                // The document did not do what the view drew: the catch-up
                // redraws it, and the entries after it counted on it.
                let _ = edit;
                ends_draft = true;
            }
        }
        if ends_draft {
            self.draft.clear();
        }
    }

    /// What a change did to the text.
    ///
    /// This peer's own change moves nothing: the view drew it when it sent
    /// the command. Any other change is first rewritten past the commands
    /// still in flight (`PROSE-UI-182`). It then shifts the draft and both
    /// ends of the selection, but never the anchor of a drag in flight
    /// (`PROSE-UI-028`, `PROSE-UI-152`).
    /// A deleted block drops the draft anchored in it (`PROSE-UI-143`), and
    /// the caret collapses beside it (`PROSE-UI-178`).
    fn take_text_change(&mut self, origin: ChangeOrigin, edits: &[TextEdit], removed: &[BlockId]) {
        if origin == ChangeOrigin::Remote {
            let (inflight, edits) = rebase::transform_all(self.inflight.make_contiguous(), edits);
            self.inflight = inflight.into();
            self.draft.shift(&edits);
            // The slash moves with the text around it.
            if let Some(menu) = &mut self.slash {
                menu.origin.1 = rebase::through_all(menu.origin.1, menu.origin.0, &edits);
            }
            if let Some(cursor) = &mut self.cursor {
                cursor.offset = rebase::through_all(cursor.offset, cursor.block_id, &edits);
                if self.dragging.is_none()
                    && let Some((block, offset)) = &mut cursor.anchor
                {
                    *offset = rebase::through_all(*offset, *block, &edits);
                }
            }
        }
        for gone in removed {
            if self
                .slash
                .as_ref()
                .is_some_and(|menu| menu.origin.0 == *gone)
            {
                self.slash = None;
            }
            if self.draft.touches(*gone) {
                self.draft.forget(*gone);
            }
            let Some(cursor) = &mut self.cursor else {
                continue;
            };
            if cursor.anchor.is_some_and(|(block, _)| block == *gone) {
                cursor.anchor = None;
            }
            if cursor.block_id == *gone {
                // A cell is laid out inside its table: the caret goes beside
                // the deleted block the flow holds.
                self.cursor = self.nearest_confirmed(*gone).or_else(|| {
                    removed
                        .iter()
                        .find_map(|block| self.nearest_confirmed(*block))
                });
                self.caret_affinity = crate::layout::run::Affinity::Downstream;
                self.x_goal = None;
                self.dragging = None;
            }
        }
    }

    /// The confirmed position nearest a deleted block. It is the end of the
    /// text block before it, else the start of the one after it.
    fn nearest_confirmed(&self, gone: BlockId) -> Option<CursorPosition> {
        let index = self.flow_index(&crate::events::tree_id(gone))?;
        let text = |(_, block): (
            crate::layout::run::FlowIndex,
            &crate::layout::place::BlockLayout,
        )| {
            self.shaped.blocks.get(&block.block_id).map(|run| {
                let len = run.data().len();
                (
                    crate::events::block_id(block.block_id),
                    len.as_end().as_doc(),
                )
            })
        };
        let before = self
            .shaped
            .window_blocks()
            .rev()
            .filter(|(at, _)| *at < index)
            .find_map(text);
        let place = match before {
            Some(found) => Some(found),
            None => self
                .shaped
                .window_blocks()
                .filter(|(at, _)| *at > index)
                .find_map(text)
                .map(|(block, _)| (block, DocOffset::ZERO)),
        };
        place.map(|(block_id, offset)| CursorPosition {
            block_id,
            offset,
            anchor: None,
        })
    }

    /// Advance the recount clock by one frame, and answer the recount when
    /// one is due (`PROSE-FEAT-124`).
    ///
    /// A frame that saw a change restarts the wait, so a run of changes costs
    /// one recount, [`RECOUNT_DELAY`] after the last of them. The caller
    /// sends the command it gets back.
    pub fn tick(&mut self, now: ambre_core::PageTime) -> Option<ProseCommand> {
        if std::mem::take(&mut self.changed) == Changed::Changed {
            self.quiet_since = Some(now);
            return None;
        }
        let since = self.quiet_since?;
        if now - since < RECOUNT_DELAY {
            return None;
        }
        self.quiet_since = None;
        Some(ProseCommand::CountWords)
    }
}

/// The five figures as the shared status bar shows them (`PROSE-FEAT-040`),
/// each one labelled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusFigures {
    pub words: String,
    pub characters: String,
    pub with_spaces: String,
    pub paragraphs: String,
    pub pages: String,
}

impl StatusFigures {
    /// The labels in the order the status bar shows them.
    #[must_use]
    pub fn labels(&self) -> [&str; 5] {
        [
            &self.words,
            &self.characters,
            &self.with_spaces,
            &self.paragraphs,
            &self.pages,
        ]
    }
}

/// The figures `count` reads as.
///
/// Prose hands labels to the slot every application shares, and builds no bar
/// of its own (`PROSE-FEAT-123`). The host renders them in that slot.
#[must_use]
pub fn status_figures(count: &WordCount) -> StatusFigures {
    let plural = |n: u32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    StatusFigures {
        words: plural(count.words.get(), "word", "words"),
        characters: plural(count.characters.get(), "character", "characters"),
        with_spaces: format!("{} with spaces", count.characters_with_spaces),
        paragraphs: plural(count.paragraphs.get(), "paragraph", "paragraphs"),
        pages: plural(count.pages.get(), "page", "pages"),
    }
}

#[cfg(test)]
mod tests {
    /// The view's state carries the document's text in many fields, and its
    /// `Debug` prints none of it.
    #[test]
    fn the_view_state_prints_lengths_and_counts_alone() {
        use crate::events::BlockInit;
        let (mut state, _) = crate::view::tests::laid_out(1, 3);
        crate::view::tests::append(&mut state, BlockInit::Paragraph, "unseen");
        let mut ui = super::ProseUiState::default();
        let _ = ui.apply_layout(state.shaped_layout());
        ui.composing = "composed".into();
        let block = state.root_blocks()[0];
        ui.draft.push_str(
            ambre_bus::app::prose::TextAck::new(1),
            "drafted",
            &block,
            ambre_bus::app::prose::DocOffset::ZERO,
        );
        let printed = format!("{ui:?}");
        for secret in ["word", "unseen", "composed", "drafted"] {
            assert!(!printed.contains(secret), "{secret} in {printed}");
        }
        assert!(printed.contains(" 8 chars"), "{printed}");
    }

    #[test]
    fn a_draft_insertion_prints_its_text_by_length_alone() {
        let change = super::Change::Insert("typed".into());
        assert!(crate::redact::tests::hides(&change, "typed"));
    }

    /// `PROSE-IO-056`: a written file waits for the host once, with the media
    /// type the Logic thread named.
    #[test]
    fn a_written_file_is_handed_to_the_host_once() {
        let mut state = ProseUiState::default();
        let filename =
            ambre_core::ExportFilename::named("document", ambre_core::ExportExtension::Ambre);
        state.apply_event(&ProseEvent::ExportReady {
            filename: filename.clone(),
            media_type: ambre_core::MediaType::from("application/octet-stream"),
            data: ambre_core::Payload::<ambre_core::FileBytes>::new(vec![1, 2, 3]),
            report: ambre_bus::app::prose::ConversionReport::clean(
                ambre_bus::app::prose::ConversionFormat::Ambre,
            ),
        });
        assert_eq!(
            state.take_export(),
            Some(ExportFile {
                filename,
                media_type: ambre_core::MediaType::from("application/octet-stream"),
                bytes: ambre_core::Payload::<ambre_core::FileBytes>::new(vec![1, 2, 3]),
            })
        );
        assert_eq!(state.take_export(), None);
    }

    /// `PROSE-FEAT-312`: each of the five figures has its label, in the order
    /// the status bar shows them.
    #[test]
    fn the_status_bar_reads_the_five_figures() {
        let count = WordCount {
            words: ambre_core::Tally::<crate::events::Words>::new(12),
            characters: ambre_core::Tally::<crate::events::Graphemes>::new(1),
            characters_with_spaces: ambre_core::Tally::<crate::events::Graphemes>::new(70),
            paragraphs: ambre_core::Tally::<crate::events::Paragraphs>::new(3),
            pages: crate::events::PageCount::new(1),
        };
        assert_eq!(
            super::status_figures(&count).labels(),
            [
                "12 words",
                "1 character",
                "70 with spaces",
                "3 paragraphs",
                "1 page"
            ]
        );
    }

    use super::*;
    use ambre_bus::app::prose::DocSpan;

    fn a_document() -> Id<Document> {
        Id::<Document>::from_bytes([7; 16])
    }

    #[test]
    fn a_full_channel_drops_the_command_rather_than_holding_the_frame() {
        let (tx, _rx) =
            ambre_bus::conduit::local::<LogicCommand>(ambre_bus::conduit::ChannelCapacity::new(1));
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
    fn a_full_channel_holds_an_edit_rather_than_the_frame() {
        let (tx, rx) =
            ambre_bus::conduit::local::<LogicCommand>(ambre_bus::conduit::ChannelCapacity::new(1));
        let mut outbox = Outbox::new(tx);
        assert_eq!(
            send_edit(&mut outbox, a_document(), ProseCommand::Noop),
            SendResult::Sent
        );
        // The channel holds one, so the second edit waits in the outbox. It
        // answers at once, and the edit is still on its way.
        let held = send_edit(&mut outbox, a_document(), ProseCommand::Noop);
        assert_eq!(held, SendResult::Held);
        assert!(held.accepted());
        assert!(outbox.is_busy());
        assert!(rx.try_recv().is_ok());
        assert_eq!(outbox.flush(), SendOutcome::Sent);
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn a_closed_channel_is_told_apart_from_a_full_one() {
        let (tx, rx) =
            ambre_bus::conduit::local::<LogicCommand>(ambre_bus::conduit::ChannelCapacity::new(1));
        drop(rx);
        assert_eq!(
            send_command(&tx, a_document(), ProseCommand::Noop),
            SendResult::Disconnected
        );
    }

    #[test]
    fn the_draft_echo_never_reaches_the_wire() {
        let (tx, rx) =
            ambre_bus::conduit::local::<LogicCommand>(ambre_bus::conduit::ChannelCapacity::new(4));
        let block = BlockId::new(ambre_core::PeerId::new(1), ambre_core::OpCounter::new(0));
        let mut state = ProseUiState::default();
        state.draft.push_str(
            ambre_bus::app::prose::TextAck::new(1),
            "hel",
            &block,
            DocOffset::ZERO,
        );
        send_command(
            &tx,
            a_document(),
            ProseCommand::InsertText {
                block_id: block,
                pos: DocOffset::ZERO,
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
        let block = BlockId::new(ambre_core::PeerId::new(1), ambre_core::OpCounter::new(0));
        let mut state = ProseUiState::default();
        state.draft.push_str(
            ambre_bus::app::prose::TextAck::new(1),
            "hel",
            &block,
            DocOffset::ZERO,
        );
        // A change alone keeps the draft: the text is not laid out yet.
        state.apply_event(&ProseEvent::DocUpdated {
            payload: ambre_core::LoroPayload::Delta(
                ambre_core::Payload::<ambre_core::DeltaBytes>::new(vec![1]),
            ),
        });
        assert_eq!(state.draft.text_of(block, ""), "hel");
        let typed = TextEdit {
            block_id: block,
            at: DocOffset::ZERO,
            delta: DocDelta::new(3),
        };
        state.inflight.push_back(typed);
        state.apply_event(&ProseEvent::TextChanged {
            origin: crate::events::ChangeOrigin::Local,
            seq: ambre_core::Generation::ZERO,
            acked: ambre_bus::app::prose::TextAck::new(1),
            edits: vec![typed],
            removed: Vec::new(),
        });
        assert!(state.inflight.is_empty());
        assert_eq!(state.draft.text_of(block, ""), "hel");
        state.draft.laid();
        assert!(state.draft.is_empty());
    }

    #[test]
    fn an_event_carrying_no_change_leaves_the_draft_alone() {
        let block = BlockId::new(ambre_core::PeerId::new(1), ambre_core::OpCounter::new(0));
        let mut state = ProseUiState::default();
        state.draft.push_str(
            ambre_bus::app::prose::TextAck::new(1),
            "hel",
            &block,
            DocOffset::ZERO,
        );
        state.apply_event(&ProseEvent::MediaArrived {
            content_hash: ambre_core::ContentHash::parse(&"a".repeat(64)).expect("a hash"),
            blocks: Vec::new(),
        });
        assert_eq!(state.draft.text_of(block, ""), "hel");
    }

    #[test]
    fn a_split_leaves_the_caret_at_the_start_of_the_new_paragraph() {
        let source = BlockId::new(ambre_core::PeerId::new(1), ambre_core::OpCounter::new(0));
        let new_block = BlockId::new(ambre_core::PeerId::new(1), ambre_core::OpCounter::new(4));
        let mut state = ProseUiState::default();
        state.apply_event(&ProseEvent::BlockSplit { source, new_block });
        let cursor = state.cursor.expect("the caret moved");
        assert_eq!(cursor.block_id, new_block);
        assert_eq!(cursor.offset, DocOffset::ZERO);
    }

    #[test]
    fn the_draft_lays_its_entries_over_the_laid_text_in_order() {
        let block = BlockId::new(ambre_core::PeerId::new(1), ambre_core::OpCounter::new(0));
        let other = BlockId::new(ambre_core::PeerId::new(1), ambre_core::OpCounter::new(1));
        let mut draft = DraftBuffer::default();
        draft.push_str(
            ambre_bus::app::prose::TextAck::new(1),
            "he",
            &block,
            DocOffset::ZERO,
        );
        draft.push_str(
            ambre_bus::app::prose::TextAck::new(2),
            "llo",
            &block,
            DocOffset::new(2),
        );
        draft.push_delete(
            ambre_bus::app::prose::TextAck::new(3),
            &block,
            DocOffset::new(1),
            DocOffset::new(3),
        );
        assert_eq!(draft.text_of(block, "!"), "hlo!");
        assert_eq!(draft.delta(block), DocDelta::new(3));
        assert_eq!(draft.text_of(other, "ab"), "ab");
        // An offset of the laid text, and back.
        assert_eq!(draft.to_doc(block, RunOffset::new(1)), DocOffset::new(4));
        assert_eq!(draft.to_laid(block, DocOffset::new(4)), RunOffset::new(1));
    }

    #[test]
    fn a_run_of_changes_costs_one_recount_after_it_stops() {
        let mut state = ProseUiState::default();
        let mut recounts = 0;
        // A change on every frame for a second, one frame every sixteen
        // milliseconds: no recount while the run lasts.
        let mut now = 0.0;
        while now < 1000.0 {
            state.apply_event(&ProseEvent::DocUpdated {
                payload: ambre_core::LoroPayload::Delta(ambre_core::Payload::<
                    ambre_core::DeltaBytes,
                >::new(vec![1])),
            });
            recounts += usize::from(state.tick(ambre_core::PageTime::new(now)).is_some());
            now += 16.0;
        }
        assert_eq!(recounts, 0);
        // Then the text stays still for two seconds: one recount, once the
        // delay has run out, and no second one.
        let stopped = now;
        while now < stopped + 2000.0 {
            if let Some(command) = state.tick(ambre_core::PageTime::new(now)) {
                assert_eq!(command, ProseCommand::CountWords);
                assert!(now - stopped >= RECOUNT_DELAY.get() - 16.0);
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
            block_id: BlockId::new(ambre_core::PeerId::new(1), ambre_core::OpCounter::new(0)),
            range: DocSpan::from_usize_range(0..3),
        };
        ui.apply_event(&ProseEvent::FindResults {
            matches: vec![hit],
            current: Some(ambre_bus::app::find::MatchIndex::ZERO),
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
