//! Prose (word processor) IPC contract (`features/ambre-spec-prose-engine.md`).
//!
//! Mirrors the Trame and Toile split. The typed command / event / display
//! data-plane types live here in `ambre-bus`, so the `ambre-logic` dispatcher
//! can match on `(DocState, AppCommand::Prose)` without a dependency cycle. The
//! engine crate `ambre-prose` depends on `ambre-bus`, never the reverse. The
//! stateful engine (`ProseState`) and its `ProseOutcome` wrapper stay in
//! `ambre-prose`, exactly as `ToileState` / `ToileOutcome` stay in
//! `ambre-toile`.
//!
//! * Pure data only: block identity, the typed block initializer, the paragraph
//!   style register, the presence frame, and the command / event enums.
//! * No `LoroDoc` handle, no `unsafe`, US-keyboard characters only.
//!
//! **Why [`BlockId`] and not `loro::TreeID`.** A block's identity is the id the
//! tree mints (`PROSE-085`), and [`BlockId`] is that id's two numbers under a
//! name this crate can carry. Naming the CRDT type here would pull `loro` into
//! `ambre-bus`, which `ambre-sync` links, and the Sync worker must compile no
//! CRDT (`SYNC-011`, `deny.toml`). The engine converts at its own boundary, and
//! the value is the same pair on both sides: it is one identity, not two.
//!
//! **The offset unit is the Unicode code point**, everywhere below
//! (`PROSE-FEAT-006`). It is what the CRDT indexes a body text and a cursor
//! with, so a command needs no conversion on the write path. A caret moves by
//! grapheme cluster, which is a rule of the view and not of this plane.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use ambre_core::{UserId, Uuid};

// =============================================================================
// Block identity (`ambre-spec-prose-schema.md section 2`)
// =============================================================================

/// One block's identity: the peer that minted it and the counter it minted.
///
/// The two numbers of the `loro::TreeID` the tree mints, carried under a type
/// that pulls no CRDT into this crate. It survives a move, an indent and a
/// regroup, so every reference to a block holds (`PROSE-085`, `PROSE-086`).
///
/// It renders as `{counter}@{peer}`, which is the CRDT's own spelling. A body
/// key and a log line therefore read the same on both sides of the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BlockId {
    pub peer: u64,
    pub counter: i32,
}

impl BlockId {
    #[must_use]
    pub const fn new(peer: u64, counter: i32) -> Self {
        Self { peer, counter }
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.counter, self.peer)
    }
}

// =============================================================================
// Block model (`ambre-spec-prose-schema.md section 3`)
// =============================================================================

/// The block types a document tree carries, and the `t` discriminant each one
/// is stored under.
///
/// A closed enum rather than a bare number, so a reader that meets an unknown
/// discriminant says so instead of guessing a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlockType {
    Paragraph = 0,
    Heading = 1,
    ListItem = 2,
    Table = 3,
    TableRow = 4,
    TableCell = 5,
    CodeBlock = 6,
    Image = 7,
    PageBreak = 8,
    HRule = 9,
    Toc = 10,
}

impl BlockType {
    /// The stored discriminant.
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }

    /// The type a stored discriminant names, or `None` for a number no build
    /// knows.
    #[must_use]
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Paragraph,
            1 => Self::Heading,
            2 => Self::ListItem,
            3 => Self::Table,
            4 => Self::TableRow,
            5 => Self::TableCell,
            6 => Self::CodeBlock,
            7 => Self::Image,
            8 => Self::PageBreak,
            9 => Self::HRule,
            10 => Self::Toc,
            _ => return None,
        })
    }

    /// Whether the type carries a body text container of its own
    /// (`ambre-spec-prose-schema.md section 3`). A leaf that holds no text, and
    /// every container type, answers `false`.
    #[must_use]
    pub fn has_body(self) -> bool {
        matches!(
            self,
            Self::Paragraph | Self::Heading | Self::ListItem | Self::CodeBlock
        )
    }

    /// Whether `child` may sit under this type, per the nesting table of
    /// `ambre-spec-prose-schema.md section 4` (`PROSE-095`).
    #[must_use]
    pub fn accepts(self, child: Self) -> bool {
        match self {
            Self::Table => child == Self::TableRow,
            Self::TableRow => child == Self::TableCell,
            Self::TableCell => {
                matches!(child, Self::Paragraph | Self::Heading | Self::ListItem)
            }
            Self::ListItem => matches!(child, Self::ListItem | Self::Paragraph),
            _ => false,
        }
    }

    /// Whether the type may sit at the root of the body tree, per the same
    /// table. `TableRow` and `TableCell` exist only under their own parent.
    #[must_use]
    pub fn accepted_at_root(self) -> bool {
        !matches!(self, Self::TableRow | Self::TableCell)
    }
}

/// How a list item is marked (`features/ambre-spec-prose-lists.md section 3`).
///
/// There is no list container: nesting is the block tree's own parent chain,
/// and the style is an attribute of the item. Two neighbours of different
/// styles are two runs, which is what breaks a numbering run
/// (`PROSE-FEAT-026`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListStyle {
    Bullet,
    Ordered(OrderedStyle),
    /// The checkbox, and whether it is ticked.
    Todo(bool),
}

/// How an ordered item's ordinal is spelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderedStyle {
    Decimal = 0,
    LowerAlpha = 1,
    UpperAlpha = 2,
    LowerRoman = 3,
    UpperRoman = 4,
}

impl OrderedStyle {
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Decimal,
            1 => Self::LowerAlpha,
            2 => Self::UpperAlpha,
            3 => Self::LowerRoman,
            4 => Self::UpperRoman,
            _ => return None,
        })
    }
}

/// The typed initializer of `InsertBlock`.
///
/// Each variant carries exactly the creation attributes of its block type. The
/// handler therefore writes the node's attribute map with no stringly-typed
/// path, and no follow-up command corrects it. A type with a richer creation surface keeps
/// a command of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockInit {
    Paragraph,
    /// The level is mandatory at creation, and the handler clamps it to 1..=6.
    Heading {
        level: u8,
    },
    /// The style is mandatory at creation, the way a heading's level is: an
    /// item minted without one would need a default and a follow-up command.
    ListItem {
        style: ListStyle,
    },
    /// An empty language reads as plain.
    CodeBlock {
        lang: String,
    },
    HRule,
    PageBreak,
}

impl BlockInit {
    /// The block type this initializer creates.
    #[must_use]
    pub fn block_type(&self) -> BlockType {
        match self {
            Self::Paragraph => BlockType::Paragraph,
            Self::Heading { .. } => BlockType::Heading,
            Self::ListItem { .. } => BlockType::ListItem,
            Self::CodeBlock { .. } => BlockType::CodeBlock,
            Self::HRule => BlockType::HRule,
            Self::PageBreak => BlockType::PageBreak,
        }
    }
}

/// Where a paragraph's text sits between its margins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left = 0,
    Center = 1,
    Right = 2,
    Justify = 3,
}

impl Align {
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Left,
            1 => Self::Center,
            2 => Self::Right,
            3 => Self::Justify,
            _ => return None,
        })
    }
}

/// How a line's height is read from its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineHeightMode {
    /// A multiple of the font size.
    Multiple = 0,
    /// An exact height, in points.
    Exact = 1,
    /// A floor, in points.
    AtLeast = 2,
}

impl LineHeightMode {
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Multiple,
            1 => Self::Exact,
            2 => Self::AtLeast,
            _ => return None,
        })
    }
}

/// One line-height setting: the mode and the number it reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineHeight {
    pub mode: LineHeightMode,
    pub value: f32,
}

/// The nine paragraph properties, each one optional.
///
/// `None` is "leave this property alone", never "clear it". The command is
/// coarse and the storage is one key per property (`PROSE-030`). The handler
/// writes only what the caller named, so two peers changing two properties lose
/// neither (`PROSE-092`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParagraphStyle {
    pub align: Option<Align>,
    pub indent_first: Option<f32>,
    pub indent_left: Option<f32>,
    pub indent_right: Option<f32>,
    pub space_before: Option<f32>,
    pub space_after: Option<f32>,
    pub line_height: Option<LineHeight>,
    pub keep_together: Option<bool>,
    pub keep_with_next: Option<bool>,
}

impl ParagraphStyle {
    /// Whether the value names no property at all. Such a command writes
    /// nothing, and therefore exports no delta (`PROSE-047`).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

// =============================================================================
// Presence (`1-Foundations/ambre-spec-presence.md`)
// =============================================================================

/// Where a caret is, in the document's own coordinates.
///
/// A `BlockId` is a peer and a counter, globally unique and portable. The same
/// value therefore means the same place on every replica, and postcard
/// round-trips it with no custom logic. `offset` is a count of Unicode code
/// points into the named block's body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorPosition {
    pub block_id: BlockId,
    pub offset: u32,
    /// The other end of a selection, when the caret has one.
    pub anchor: Option<(BlockId, u32)>,
}

/// The peer-authored presence frame.
///
/// It carries a position and a self-asserted, cosmetic name, and no user id. The
/// canonical identity is the Sync-resolved one delivered out of band, so a peer
/// cannot forge another user's (`PRESENCE-002`, `PRESENCE-003`).
///
/// **The position is an encoded `loro::Cursor`, never an offset** (`PROSE-004`).
/// An offset names a different character on a peer that has applied an
/// insertion this one has not. A cursor rides the operation it was taken
/// against, so every peer resolves it to the same place once the two converge.
/// The bytes are opaque to this plane, which is why the block is named beside
/// them. A frame whose block this peer does not hold is dropped, and the cursor
/// is never decoded (`PROSE-057`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresencePayload {
    pub block_id: BlockId,
    pub cursor: Vec<u8>,
    pub anchor: Option<(BlockId, Vec<u8>)>,
    pub display_name: String,
}

/// One live collaborator's caret, as the view is handed it.
///
/// No colour: the view derives it from the `UserId` through
/// `ambre_draw::peer::peer_color`, so a collaborator keeps one colour across
/// Prose, Trame and Toile (`PRESENCE-004`). No serde: it moves by value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCursor {
    pub user_id: UserId,
    pub display_name: String,
    pub position: CursorPosition,
}

// =============================================================================
// Tables, the table of contents and the word count
// =============================================================================

/// Where a cell's content sits between its top and bottom edges
/// (`features/ambre-spec-prose-tables.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign {
    Top = 0,
    Middle = 1,
    Bottom = 2,
}

impl VAlign {
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Top,
            1 => Self::Middle,
            2 => Self::Bottom,
            _ => return None,
        })
    }
}

/// One border, applied to the four edges of a cell at once.
///
/// `color` is `0xRRGGBB`. The document stores it as the `"#rrggbb"` record the
/// mark registry uses, and a value past 24 bits is refused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellBorder {
    pub width_pt: f32,
    pub color: u32,
}

/// The five figures of the status bar (`features/ambre-spec-prose-word-count.md`).
///
/// Four are tallies of the text. The fifth is read off the layout, and each
/// half is refreshed by its own trigger.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WordCount {
    pub words: u32,
    /// Grapheme clusters that are not whitespace (`PROSE-FEAT-039`).
    pub characters: u32,
    /// The same clusters, whitespace and paragraph breaks included
    /// (`PROSE-FEAT-121`).
    pub characters_with_spaces: u32,
    pub paragraphs: u32,
    /// From the layout, never from the tally (`PROSE-FEAT-038`).
    pub pages: u32,
}

/// One formatting control's state over a selection
/// (`features/ambre-spec-prose-toolbar.md section 3`).
///
/// Three states, not two: a selection formatted in part is neither on nor
/// off (`PROSE-UI-096`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TriState {
    #[default]
    Off,
    On,
    Mixed,
}

/// The marks a selection carries, one state per control of the format bar
/// (`PROSE-UI-054`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActiveMarks {
    pub bold: TriState,
    pub italic: TriState,
    pub underline: TriState,
    pub strikethrough: TriState,
    pub code: TriState,
    /// The address, when the whole selection carries one link and the same
    /// one.
    pub link: Option<String>,
}

/// A value a band resolves when its page is drawn, and never stores
/// (`features/ambre-spec-prose-headers-footers.md`, `PROSE-FEAT-065`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderField {
    PageNumber,
    PageCount,
    Date(DateFieldFormat),
    Title,
}

/// What the date field stores: a format, never a rendered date and never a
/// locale (`PROSE-FEAT-162`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateFieldFormat {
    /// The language's short form.
    Short,
    /// The language's long form.
    Long,
    /// The calendar date, the same in every language.
    Iso,
}

/// Which of the two blocks of a side a field insertion targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HfTarget {
    Regular,
    FirstPage,
}

/// The four band blocks and their settings, as the view needs them to aim
/// the ordinary text commands at a band (`PROSE-FEAT-160`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BandSet {
    pub header: Option<BlockId>,
    pub footer: Option<BlockId>,
    pub header_first: Option<BlockId>,
    pub footer_first: Option<BlockId>,
    pub header_enabled: bool,
    pub footer_enabled: bool,
    pub different_first_page: bool,
}

/// What a search looks for (`features/ambre-spec-prose-find-replace.md`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindQuery {
    pub text: String,
    pub case_sensitive: bool,
    /// A match must not touch a letter, a digit or an underscore on either
    /// side.
    pub whole_word: bool,
    /// The regular-expression search, unbuilt: the engine refuses a query
    /// that sets it.
    pub use_regex: bool,
}

/// One match: a range of code points inside the body of one block
/// (`PROSE-FEAT-034`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindMatch {
    pub block_id: BlockId,
    pub range: (u32, u32),
}

/// One footnote as the view lists it (`features/ambre-spec-prose-footnotes.md`).
///
/// The ordinal is counted from the calls in document order and never stored
/// (`PROSE-FEAT-013`, `PROSE-FEAT-095`). `None` marks a note no call reaches,
/// kept for the author to review (`PROSE-FEAT-098`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FootnoteEntry {
    pub footnote_id: Uuid,
    pub ordinal: Option<u32>,
    pub text: String,
}

/// One reply of a comment thread (`features/ambre-spec-prose-comments.md`).
///
/// The author is the one the writing session stamped, never a field the
/// command carried (`PROSE-FEAT-077`). The stamp is display only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentReply {
    pub reply_id: Uuid,
    pub author: String,
    pub user_id: UserId,
    pub created_at: u64,
    pub text: String,
}

/// One comment thread as the view lists it.
///
/// The anchor is the live mark's span, or the last one it held when
/// `orphaned` is set (`PROSE-FEAT-202`, `PROSE-FEAT-203`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentThread {
    pub comment_id: Uuid,
    pub anchor_block: Option<BlockId>,
    pub anchor_start: u32,
    pub anchor_end: u32,
    pub orphaned: bool,
    pub resolved: bool,
    pub replies: Vec<CommentReply>,
}

/// Which edit a suggestion proposes (`features/ambre-spec-prose-track-changes.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    /// Text a reviewer typed: accepting keeps it, rejecting removes it.
    Insertion,
    /// Text a reviewer deleted: accepting removes it, rejecting keeps it.
    Deletion,
}

/// One marked span of a suggestion, in its block's own offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuggestionSpan {
    pub block_id: BlockId,
    pub start: u32,
    pub end: u32,
}

/// One run of a link mark, in its block's own offsets, and the target it
/// names (`features/ambre-spec-prose-selection.md section 2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    pub block_id: BlockId,
    pub start: u32,
    pub end: u32,
    pub url: String,
}

/// One pending suggestion as the view lists it, derived from its marks and its
/// entry (`PROSE-FEAT-261`). A change across a paragraph break holds several
/// spans and stays one suggestion (`PROSE-FEAT-259`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub suggestion_id: Uuid,
    pub kind: SuggestionKind,
    pub author: String,
    pub user_id: UserId,
    pub spans: Vec<SuggestionSpan>,
}

/// What a cross reference shows (`features/ambre-spec-prose-cross-references.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrossRefKind {
    /// The page the target starts on.
    PageNumber,
    /// The target heading's derived section number.
    SectionNumber,
    /// A label the author wrote, whatever characters it holds.
    CustomText(String),
}

/// One cross reference as drawn from the last layout. Nothing here is stored
/// (`PROSE-FEAT-129`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedCrossRef {
    /// Where the reference sits: its block and its code-point offset.
    pub block_id: BlockId,
    pub offset: u32,
    pub target: BlockId,
    /// The value drawn. `None` is the broken mark (`PROSE-FEAT-014`).
    pub shown: Option<String>,
}

/// One line of a table of contents. Nothing here is persisted
/// (`features/ambre-spec-prose-table-of-contents.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocEntry {
    pub block_id: BlockId,
    pub level: u8,
    pub text: String,
    /// Present only while the block is configured as numbered
    /// (`PROSE-FEAT-062`).
    pub section_number: Option<Vec<u32>>,
    /// The one-based page the heading starts on (`PROSE-FEAT-060`).
    pub page: u32,
}

/// The entries one table-of-contents block lists, built from the headings and
/// a layout.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableOfContents {
    pub entries: Vec<TocEntry>,
    pub max_level: u8,
    pub numbered: bool,
}

// =============================================================================
// Commands -- the content plane, UI thread -> Logic thread
// =============================================================================

/// One thing to do to a Prose document.
///
/// Values move on `flume` between the two threads, which share one linear
/// memory, so no shape here derives `serde`. Every offset is a count of Unicode
/// code points into the body of the block the command names (`PROSE-045`).
///
/// The variants are the ones the engine answers today. Comments, footnotes,
/// find, interop and the clipboard reach this enum with the Story that builds
/// each of them (`ambre-roadmap.md` E5).
#[derive(Clone, Debug, PartialEq)]
pub enum ProseCommand {
    /// Changes nothing, and is the one command a test drives the empty outcome
    /// with (`PROSE-047`).
    Noop,
    /// The view has taken every text change up to `seen`, the number the
    /// last one it applied carried. The Logic thread answers with
    /// [`ProseEvent::Synced`] once everything sent before has landed.
    Sync {
        seen: u64,
    },

    // -- Text editing (`features/ambre-spec-prose-text-editing.md`) --
    /// One committed run of text, at one position in one block.
    InsertText {
        block_id: BlockId,
        pos: u32,
        text: String,
    },
    /// A range inside one block.
    DeleteText {
        block_id: BlockId,
        range: (u32, u32),
    },
    /// One arbitrary, possibly cross-block, selection deleted in one command.
    /// The endpoints arrive in either order and the handler normalizes them. It
    /// is also the single join primitive (`PROSE-037`, `PROSE-039`).
    DeleteRange {
        anchor: (BlockId, u32),
        focus: (BlockId, u32),
    },
    /// The inverse of the join case above (`PROSE-038`).
    SplitBlock {
        block_id: BlockId,
        pos: u32,
    },

    // -- Inline marks (`features/ambre-spec-prose-inline-styles.md`) --
    /// `value` is the mark's payload, always a String: the key, its value
    /// grammar and its canonical form are the registry of
    /// `ambre-spec-prose-schema.md section 3` (`PROSE-008`). `None` is the
    /// value of a key whose presence alone is meaningful.
    ApplyMark {
        block_id: BlockId,
        range: (u32, u32),
        key: String,
        value: Option<String>,
    },
    RemoveMark {
        block_id: BlockId,
        range: (u32, u32),
        key: String,
    },
    ClearInlineMarks {
        block_id: BlockId,
        range: (u32, u32),
    },

    // -- Block structure --
    /// `init` carries the attributes the block type needs at creation, so no
    /// handler invents a default a follow-up command must correct.
    InsertBlock {
        parent_id: Option<BlockId>,
        after_id: Option<BlockId>,
        init: BlockInit,
        style_id: Option<String>,
    },
    DeleteBlock {
        block_id: BlockId,
    },
    MoveBlock {
        block_id: BlockId,
        new_parent: Option<BlockId>,
        after_id: Option<BlockId>,
    },
    SetBlockStyle {
        block_id: BlockId,
        style_id: String,
    },
    /// `features/ambre-spec-prose-headings.md`; the level is clamped to 1..=6.
    /// `None` takes the level away, and the block becomes a paragraph
    /// (`PROSE-FEAT-092`). One command for the two directions of one control:
    /// the style picker offers the six levels and the paragraph beside them.
    SetHeadingLevel {
        block_id: BlockId,
        level: Option<u8>,
    },
    SetCodeLang {
        block_id: BlockId,
        lang: String,
    },

    // -- Lists (`features/ambre-spec-prose-lists.md`) --
    /// Change an item's marker, which may break one numbering run into two
    /// (`PROSE-FEAT-140`, `PROSE-FEAT-278`).
    SetListStyle {
        block_id: BlockId,
        style: ListStyle,
    },
    /// Reparent the item under its previous neighbour. The first item of a run
    /// has none, and the command is refused rather than ignored
    /// (`PROSE-FEAT-028`).
    IndentList {
        block_id: BlockId,
    },
    /// Reparent the item under its grandparent, after its former parent's
    /// subtree. A top-level item leaves the list and becomes a paragraph
    /// (`PROSE-FEAT-027`).
    UnindentList {
        block_id: BlockId,
    },
    /// The command is coarse and the storage is per property: the handler
    /// writes only what the caller named (`PROSE-030`, `PROSE-092`).
    SetParagraphStyle {
        block_id: BlockId,
        style: ParagraphStyle,
    },

    // -- Tables (`features/ambre-spec-prose-tables.md`) --
    /// A table of `rows` by `cols`, each cell holding one empty paragraph,
    /// every column at the document's default width. A table carries more
    /// than `BlockInit` holds, so it keeps a command of its own.
    InsertTable {
        parent_id: Option<BlockId>,
        after_id: Option<BlockId>,
        rows: u32,
        cols: u32,
    },
    /// A row of one cell per column. `None` puts it first.
    InsertTableRow {
        table_id: BlockId,
        after_row: Option<BlockId>,
    },
    DeleteTableRow {
        row_id: BlockId,
    },
    /// `at_col` is the index the new column takes, from zero to the column
    /// count. `None` takes the document's default width (`PROSE-FEAT-232`).
    InsertTableCol {
        table_id: BlockId,
        at_col: u32,
        width_pt: Option<f32>,
    },
    DeleteTableCol {
        table_id: BlockId,
        col_index: u32,
    },
    ResizeTableCol {
        table_id: BlockId,
        col_index: u32,
        width_pt: f32,
    },
    /// Two grid positions, `(row, column)`, in either order. The handler
    /// resolves them to two block ids before it writes (`PROSE-FEAT-241`).
    MergeTableCells {
        table_id: BlockId,
        start: (u32, u32),
        end: (u32, u32),
    },
    UnmergeTableCell {
        cell_id: BlockId,
    },
    /// Each `None` leaves that property alone.
    SetCellStyle {
        cell_id: BlockId,
        bg: Option<u32>,
        valign: Option<VAlign>,
        border: Option<CellBorder>,
        padding_pt: Option<f32>,
    },
    /// Tab and Shift-Tab: the next or the previous stop in reading order.
    /// Past the last cell, a row is appended (`PROSE-FEAT-251`).
    StepTableCell {
        cell_id: BlockId,
        backwards: bool,
    },

    // -- Table of contents (`features/ambre-spec-prose-table-of-contents.md`) --
    /// `max_level` is clamped to one to six (`PROSE-FEAT-292`).
    InsertToc {
        parent_id: Option<BlockId>,
        after_id: Option<BlockId>,
        max_level: u8,
        numbered: bool,
    },
    UpdateTocConfig {
        block_id: BlockId,
        max_level: u8,
        numbered: bool,
    },

    // -- Viewport (`features/ambre-spec-prose-viewport.md`) --
    /// Hand the view the zero-based pages `first` to `last`, both included.
    /// The view sends it when the canvas nears a page the layout in force
    /// does not carry (`PROSE-UI-211`); it writes nothing.
    ShowPages {
        first: u32,
        last: u32,
    },
    /// Hand the view the pages around the page `block_id` starts on. The view
    /// sends it to reveal a block the layout in force does not carry
    /// (`PROSE-UI-212`); it writes nothing.
    ShowBlock {
        block_id: BlockId,
    },

    // -- Word count (`features/ambre-spec-prose-word-count.md`) --
    /// Recount the four text tallies. The view sends it once a run of changes
    /// has stopped (`PROSE-FEAT-124`); it writes nothing.
    CountWords,

    // -- Format bar (`features/ambre-spec-prose-toolbar.md`) --
    /// Read the marks a selection carries. The endpoints arrive in either
    /// order; it writes nothing (`PROSE-UI-099`).
    ReadMarks {
        anchor: (BlockId, u32),
        focus: (BlockId, u32),
    },

    // -- Clipboard (`features/ambre-spec-prose-clipboard.md`) --
    /// Build the clipboard payload of a whole selection. The endpoints
    /// arrive in either order; it writes nothing (`PROSE-UI-040`,
    /// `PROSE-UI-109`).
    CopyRange {
        anchor: (BlockId, u32),
        focus: (BlockId, u32),
    },
    /// Insert a payload this application copied, at one place, marks and
    /// block structure kept (`PROSE-UI-107`). The bytes are the ones the
    /// Logic thread built; the view only carries them (`PROSE-UI-105`).
    /// Bytes another application wrote do not decode, and `text`, the
    /// plain-text mirror read beside them, is pasted instead
    /// (`PROSE-UI-042`).
    PasteRich {
        at: (BlockId, u32),
        payload: Vec<u8>,
        text: String,
    },
    /// Insert plain text of several lines at one place, one paragraph per
    /// line, the first joining the paragraph there (`PROSE-UI-042`). Text of
    /// one line travels as [`ProseCommand::InsertText`].
    PasteText {
        at: (BlockId, u32),
        text: String,
    },
    /// Apply one presence mark over a whole selection, or remove it. Every
    /// block the selection covers changes in one commit, so one undo takes
    /// the whole gesture back.
    FormatSelection {
        anchor: (BlockId, u32),
        focus: (BlockId, u32),
        key: String,
        apply: bool,
    },

    // -- Headers and footers (`features/ambre-spec-prose-headers-footers.md`) --
    /// Show or hide the top band (`PROSE-FEAT-167`).
    SetHeaderEnabled {
        enabled: bool,
    },
    /// Show or hide the bottom band (`PROSE-FEAT-167`).
    SetFooterEnabled {
        enabled: bool,
    },
    /// Turn the first page's own header and footer on or off, together
    /// (`PROSE-FEAT-063`).
    SetDifferentFirstPage {
        enabled: bool,
    },
    /// A field in the header block `target` names. A position past the end
    /// is clamped to it (`PROSE-FEAT-068`).
    InsertHeaderField {
        field: HeaderField,
        target: HfTarget,
        pos: u32,
    },
    InsertFooterField {
        field: HeaderField,
        target: HfTarget,
        pos: u32,
    },
    /// Align the band whose block is `block_id`, the one the caret is in
    /// (`PROSE-FEAT-171`). Justified is not a band alignment and is refused.
    SetBandAlign {
        block_id: BlockId,
        align: Align,
    },
    /// The view's wall clock, sent once a run of local changes stops. The
    /// engine writes it as the document's last-modified stamp when this peer
    /// wrote since the last one (`PROSE-FEAT-066`).
    StampModified {
        now_ms: u64,
    },

    // -- Find and replace (`features/ambre-spec-prose-find-replace.md`) --
    /// Open a search, or replace the one open. It writes nothing
    /// (`PROSE-FEAT-151`).
    FindStart {
        query: FindQuery,
    },
    FindNext,
    FindPrev,
    /// End the session: the query, the matches and the index are dropped
    /// (`PROSE-FEAT-158`).
    FindClose,
    /// Replace the current match with `replacement`.
    ReplaceOne {
        replacement: String,
    },
    /// Replace every match, as one write and one undo item (`FIND-006`).
    ReplaceAll {
        replacement: String,
    },

    // -- Footnotes (`features/ambre-spec-prose-footnotes.md`) --
    /// A note, and one call to it at `pos` in `block_id` (`PROSE-FEAT-099`).
    InsertFootnote {
        block_id: BlockId,
        pos: u32,
        note_text: String,
    },
    /// Rewrite a note's body.
    EditFootnote {
        footnote_id: Uuid,
        new_text: String,
    },
    /// Remove a note and every call to it (`PROSE-FEAT-070`).
    DeleteFootnote {
        footnote_id: Uuid,
    },

    // -- Comments (`features/ambre-spec-prose-comments.md`) --
    /// A thread over `range` in `block_id`, opened by a reply holding `text`.
    /// `now_ms` is the display stamp: the Logic thread reads no clock.
    AddComment {
        block_id: BlockId,
        range: (u32, u32),
        text: String,
        now_ms: u64,
    },
    /// A reply at the end of the thread, which reopens it (`PROSE-FEAT-079`).
    ReplyToComment {
        comment_id: Uuid,
        text: String,
        now_ms: u64,
    },
    /// Close a thread.
    ResolveComment {
        comment_id: Uuid,
    },
    /// Reopen a thread, with no reply added.
    ReopenComment {
        comment_id: Uuid,
    },
    /// Remove a thread and its mark, and leave the text.
    DeleteComment {
        comment_id: Uuid,
    },
    /// Remove one of the caller's replies, or the thread when it is the head
    /// (`PROSE-FEAT-081`).
    DeleteReply {
        comment_id: Uuid,
        reply_id: Uuid,
    },
    /// Rewrite one of the caller's replies (`PROSE-FEAT-080`).
    EditCommentReply {
        comment_id: Uuid,
        reply_id: Uuid,
        new_text: String,
    },

    // -- Track changes (`features/ambre-spec-prose-track-changes.md`) --
    /// Turn suggestion mode on or off for this session. Nothing pending is
    /// applied or cancelled (`PROSE-FEAT-089`).
    EnableTrackChanges {
        enabled: bool,
    },
    /// Apply every span of one suggestion (`PROSE-FEAT-083`).
    AcceptSuggestion {
        suggestion_id: Uuid,
    },
    /// Cancel every span of one suggestion (`PROSE-FEAT-273`).
    RejectSuggestion {
        suggestion_id: Uuid,
    },
    /// Apply every pending suggestion.
    AcceptAllSuggestions,
    /// Cancel every pending suggestion.
    RejectAllSuggestions,

    // -- Cross references (`features/ambre-spec-prose-cross-references.md`) --
    /// A reference at `pos` in `block_id`, naming `target_block_id`
    /// (`PROSE-FEAT-134`).
    InsertCrossRef {
        block_id: BlockId,
        pos: u32,
        target_block_id: BlockId,
        kind: CrossRefKind,
    },
    /// Bring the target of the reference at `pos` in `block_id` into view. It
    /// writes nothing (`PROSE-UI-086`).
    FollowCrossRef {
        block_id: BlockId,
        pos: u32,
    },

    // -- Presence (`1-Foundations/ambre-spec-presence.md`) --
    /// The handler builds the wire frame and returns it as the outcome's
    /// ephemeral half. No user id is ever put on the wire.
    UpdateCursor {
        pos: CursorPosition,
    },

    // -- Local history (`features/ambre-spec-prose-undo.md`) --
    /// Applied to the document by the undo manager; the delta it produces is
    /// broadcast like any other mutation.
    Undo,
    Redo,

    // -- Import and export (`features/ambre-spec-prose-import-export.md`) --
    /// Write the whole document out in `format`, and answer
    /// [`ProseEvent::ExportReady`]. Read-only: an export emits no delta
    /// (`PROSE-IO-017`). The Logic dispatcher serves it, because the native
    /// header needs the identity its registry holds.
    ExportDocument {
        format: ExportFormat,
    },
}

/// The formats a document leaves the editor in
/// (`features/ambre-spec-prose-import-export.md section 3`). A writer adds its
/// variant on the day it lands, so the view never offers a format no code
/// writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// The native file: the shared header over the document's own snapshot.
    Ambre,
}

impl ExportFormat {
    /// The media type of the bytes this format writes (`PROSE-IO-020`).
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Ambre => "application/octet-stream",
        }
    }
}

impl ProseCommand {
    /// Whether this command can change the document's shared CRDT state.
    ///
    /// Used by the Logic dispatcher to gate a read-only caller
    /// (`ambre-spec-architecture.md section 2`). Presence is *not* mutating: a
    /// viewer still shows a cursor.
    #[must_use]
    pub fn is_mutating(&self) -> bool {
        !matches!(
            self,
            Self::Noop
                | Self::Sync { .. }
                | Self::UpdateCursor { .. }
                | Self::CountWords
                | Self::ShowPages { .. }
                | Self::ShowBlock { .. }
                | Self::ReadMarks { .. }
                | Self::FindStart { .. }
                | Self::FindNext
                | Self::FindPrev
                | Self::FindClose
                | Self::FollowCrossRef { .. }
                | Self::ExportDocument { .. }
        )
    }

    /// What a refusal calls this command when it tells the author
    /// (`PROSE-105`). The match is exhaustive, so a command added to the enum
    /// does not compile until it has a name a reader understands.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Noop => "nothing",
            Self::Sync { .. } => "catching up",
            Self::InsertText { .. } => "typing",
            Self::CopyRange { .. } => "copying",
            Self::PasteRich { .. } | Self::PasteText { .. } => "pasting",
            Self::DeleteText { .. } => "deleting text",
            Self::DeleteRange { .. } => "deleting the selection",
            Self::SplitBlock { .. } => "splitting the block",
            Self::ApplyMark { .. } => "applying the style",
            Self::RemoveMark { .. } => "removing the style",
            Self::ClearInlineMarks { .. } => "clearing the styles",
            Self::InsertBlock { .. } => "inserting a block",
            Self::DeleteBlock { .. } => "deleting the block",
            Self::MoveBlock { .. } => "moving the block",
            Self::SetBlockStyle { .. } => "setting the block style",
            Self::SetHeadingLevel { .. } => "setting the heading level",
            Self::SetListStyle { .. } => "setting the list style",
            Self::IndentList { .. } => "indenting a list item",
            Self::UnindentList { .. } => "outdenting a list item",
            Self::SetCodeLang { .. } => "setting the code language",
            Self::SetParagraphStyle { .. } => "setting the paragraph style",
            Self::InsertTable { .. } => "inserting a table",
            Self::InsertTableRow { .. } => "inserting a row",
            Self::DeleteTableRow { .. } => "deleting the row",
            Self::InsertTableCol { .. } => "inserting a column",
            Self::DeleteTableCol { .. } => "deleting the column",
            Self::ResizeTableCol { .. } => "resizing the column",
            Self::MergeTableCells { .. } => "merging the cells",
            Self::UnmergeTableCell { .. } => "unmerging the cells",
            Self::SetCellStyle { .. } => "styling the cell",
            Self::StepTableCell { .. } => "moving to another cell",
            Self::InsertToc { .. } => "inserting a table of contents",
            Self::UpdateTocConfig { .. } => "changing the table of contents",
            Self::CountWords => "counting the words",
            Self::ShowPages { .. } => "showing the pages",
            Self::ShowBlock { .. } => "showing the block",
            Self::ReadMarks { .. } => "reading the styles",
            Self::FormatSelection { .. } => "formatting the selection",
            Self::SetHeaderEnabled { .. } => "showing or hiding the header",
            Self::SetFooterEnabled { .. } => "showing or hiding the footer",
            Self::SetDifferentFirstPage { .. } => "changing the first page's bands",
            Self::InsertHeaderField { .. } => "inserting a header field",
            Self::InsertFooterField { .. } => "inserting a footer field",
            Self::SetBandAlign { .. } => "aligning the band",
            Self::StampModified { .. } => "stamping the document",
            Self::FindStart { .. } => "searching",
            Self::FindNext => "going to the next match",
            Self::FindPrev => "going to the previous match",
            Self::FindClose => "closing the search",
            Self::ReplaceOne { .. } => "replacing the match",
            Self::ReplaceAll { .. } => "replacing every match",
            Self::InsertFootnote { .. } => "inserting a footnote",
            Self::EditFootnote { .. } => "editing the footnote",
            Self::DeleteFootnote { .. } => "deleting the footnote",
            Self::AddComment { .. } => "adding a comment",
            Self::ReplyToComment { .. } => "replying to the comment",
            Self::ResolveComment { .. } => "resolving the comment",
            Self::ReopenComment { .. } => "reopening the comment",
            Self::DeleteComment { .. } => "deleting the comment",
            Self::DeleteReply { .. } => "deleting the reply",
            Self::EditCommentReply { .. } => "editing the reply",
            Self::EnableTrackChanges { .. } => "switching suggestion mode",
            Self::AcceptSuggestion { .. } => "accepting the suggestion",
            Self::RejectSuggestion { .. } => "rejecting the suggestion",
            Self::AcceptAllSuggestions => "accepting every suggestion",
            Self::RejectAllSuggestions => "rejecting every suggestion",
            Self::InsertCrossRef { .. } => "inserting a cross reference",
            Self::FollowCrossRef { .. } => "following the cross reference",
            Self::UpdateCursor { .. } => "moving the cursor",
            Self::Undo => "undoing",
            Self::Redo => "redoing",
            Self::ExportDocument { .. } => "saving to a file",
        }
    }
}

// =============================================================================
// Events -- the content plane, Logic thread -> UI thread
// =============================================================================

/// One thing the view is told. A second enum would put a document fact in two
/// places and leave the dispatch with a hole (`PROSE-UI-001`).
///
/// The interop reports reach this enum later, with the Story that builds them
/// (`ambre-roadmap.md` E5).
#[derive(Clone, Debug, PartialEq)]
pub enum ProseEvent {
    /// A layout the Logic thread computed and shaped, whole. The view draws it
    /// and never lays text out itself (`PROSE-UI-006`).
    LayoutShaped { layout: SharedLayout },
    /// The confirmed change, local or imported. The bytes are the ones applied,
    /// never a re-export (`PROSE-023`, `PROSE-048`).
    DocUpdated { delta: Vec<u8> },
    /// What that change did to the text, in code points, for a view that
    /// holds no document (`PROSE-UI-001`). The draft and the caret follow it
    /// (`PROSE-UI-005`, `PROSE-UI-028`, `PROSE-UI-143`).
    TextChanged {
        /// The change is what a text command of the view did, as the view
        /// already drew it. Any other change, remote or not, is `false`.
        local: bool,
        /// The count of changes the view did not draw, this one included when
        /// `local` is `false`. The view hands it back in
        /// [`ProseCommand::Sync`].
        seq: u64,
        /// How many insertions and deletions of text the Logic thread has
        /// taken from the view so far.
        acked: u64,
        /// One run after another, each in the text the previous one left.
        edits: Vec<TextEdit>,
        /// The blocks the change deleted.
        removed: Vec<BlockId>,
    },
    /// The paragraph that parted, and the one that carries its tail. The view
    /// needs the new block's identity to leave the caret at its start
    /// (`PROSE-UI-141`): it cannot mint the id and cannot read the document.
    BlockSplit { source: BlockId, new_block: BlockId },
    /// Every command sent before the [`ProseCommand::Sync`] it answers has
    /// landed, and `acked` counts the text commands among them.
    Synced { acked: u64 },
    /// The whole live peer set, replaced rather than merged (`PRESENCE-009`).
    RemoteCursors { cursors: Vec<RemoteCursor> },
    /// A picture finished decrypting, and the blocks that show it are stale
    /// (`PROSE-059`, `PROSE-060`). The bytes stay in the Logic thread's cache;
    /// the view asks for them when it paints.
    MediaArrived {
        content_hash: String,
        blocks: Vec<BlockId>,
    },
    /// A full-state read the schema gate refused (`SCHEMA-008`).
    DocumentRefused { reason: String },
    /// A finished export: the bytes, the name to save them under, and their
    /// media type, so one download path serves every format (`PROSE-IO-020`).
    /// The view hands them to the browser (`PROSE-IO-056`).
    ExportReady {
        filename: String,
        media_type: &'static str,
        data: Vec<u8>,
    },
    /// The entries of one table-of-contents block, rebuilt from a layout
    /// (`PROSE-FEAT-003`).
    TocUpdated {
        block_id: BlockId,
        toc: TableOfContents,
    },
    /// The five figures, after a recount or a new layout.
    WordCountUpdated { count: WordCount },
    /// The marks of the selection a [`ProseCommand::ReadMarks`] named, with
    /// that selection as it was sent (`PROSE-UI-057`).
    SelectionMarks {
        anchor: (BlockId, u32),
        focus: (BlockId, u32),
        marks: ActiveMarks,
    },
    /// The clipboard payload of the selection a [`ProseCommand::CopyRange`]
    /// named: its plain-text mirror, and this application's own bytes
    /// (`PROSE-UI-108`).
    Clipboard { text: String, payload: Vec<u8> },
    /// A cell a merge covers still holds text: masked, not destroyed. A hint,
    /// and it blocks nothing (`PROSE-FEAT-255`).
    MergeConflictDetected { cell_id: BlockId },
    /// Where the caret lands after a step between cells: the first text block
    /// of `cell`, at its start (`PROSE-FEAT-250`, `PROSE-FEAT-251`).
    CellEntered { cell: BlockId, block: BlockId },
    /// Where the caret lands after an undo or a redo: the edit taken back or
    /// put back (`PROSE-UI-045`). After a paste, it lands after the text
    /// pasted. The view collapses any selection to it
    /// (`PROSE-UI-073`) and writes nothing back (`PROSE-UI-074`).
    CaretPlaced { block: BlockId, offset: u32 },
    /// Every footnote, in the order of its calls, orphans last
    /// (`PROSE-FEAT-013`, `PROSE-FEAT-098`).
    FootnotesUpdated { notes: Vec<FootnoteEntry> },
    /// Every comment thread, in document order of its anchor, orphans last.
    CommentsUpdated { threads: Vec<CommentThread> },
    /// Every pending suggestion, in document order of its first span, and
    /// whether this session is suggesting.
    SuggestionsUpdated {
        suggesting: bool,
        suggestions: Vec<Suggestion>,
    },
    /// Every run of a link mark, in document order.
    LinksUpdated { links: Vec<LinkSpan> },
    /// Every cross reference, resolved against the last layout.
    CrossRefsUpdated { refs: Vec<ResolvedCrossRef> },
    /// Bring a block into view, from its first page (`PROSE-UI-085`).
    RevealBlock { block: BlockId, page: u32 },
    /// The four band blocks and their settings, after a band command
    /// (`PROSE-FEAT-160`).
    BandsUpdated { bands: BandSet },
    /// Every match of the open search, in document order, and the index of
    /// the current one. An empty list clears the highlight (`PROSE-FEAT-289`).
    /// It goes to the peer that searched and to no other (`PROSE-FEAT-148`).
    FindResults {
        matches: Vec<FindMatch>,
        current: Option<u32>,
    },
    /// The current match moved; the list did not (`PROSE-FEAT-152`).
    FindUpdated { current: Option<u32> },
    /// A command the engine could not honour. The value names what was
    /// refused and why (`PROSE-105`).
    Error(ProseError),
}

// =============================================================================
// Errors
// =============================================================================

/// One refusal, as the author is told about it.
///
/// A number and a message named neither the command nor a reason, so a view
/// could only print it. Both halves here are readable by code. The view picks
/// where to put the message from `refused`, and the message itself from
/// `reason` (`PROSE-105`, `PROSE-106`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{refused} refused: {reason}")]
pub struct ProseError {
    /// What the engine turned down: a command, under the name
    /// [`ProseCommand::name`] gives it. A collaborator's update names itself,
    /// entering the engine the same way and failing on the same terms.
    pub refused: &'static str,
    /// Why.
    pub reason: ProseRefusal,
}

impl ProseError {
    #[must_use]
    pub fn new(refused: &'static str, reason: ProseRefusal) -> Self {
        Self { refused, reason }
    }
}

/// Why the engine refused.
///
/// One variant per reason the engine can state, never one per call site. Two
/// handlers refusing a range out of bounds carry the same variant. They differ
/// by the command they name.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProseRefusal {
    /// A block id that resolves to no live block.
    #[error("block {0} is not in the document")]
    NoSuchBlock(BlockId),
    /// An offset or a range outside the body text it addresses
    /// (`PROSE-045`).
    #[error("the range leaves the body of the block the command names")]
    BadRange,
    /// A move or an insert the nesting table refuses (`PROSE-040`,
    /// `PROSE-095`).
    #[error("the document refuses that block under that parent")]
    NestingRefused,
    /// A mark key the registry of `ambre-spec-prose-schema.md section 3` does
    /// not declare (`PROSE-008`).
    #[error("the mark registry declares no key {0}")]
    UnknownMark(String),
    /// A style command aimed at an anchor key, which only its own command
    /// writes (`PROSE-FEAT-298`, `PROSE-FEAT-299`).
    #[error("{0} is not a style, and only its own command writes it")]
    AnchorKey(String),
    /// A mark value the key's own grammar refuses (`PROSE-008`).
    #[error("{value} is not a value the {key} style takes")]
    BadMarkValue { key: String, value: String },
    /// A collaborator's change this peer could not apply.
    #[error("a collaborator's change could not be applied; the document may be behind")]
    RemoteImportFailed,
    /// A row or a column index outside the grid the table has now
    /// (`PROSE-FEAT-233`).
    #[error("that row or column is not in the table")]
    OutsideTheGrid,
    /// A merge whose covered cells do not form a solid rectangle
    /// (`PROSE-FEAT-047`).
    #[error("the cells to merge do not form a rectangle")]
    MergeNotRectangular,
    /// A merge over a cell that still holds text (`PROSE-FEAT-046`).
    #[error("a cell to merge still holds text")]
    MergeOverText,
    /// A table dimension, a width or a colour the format cannot hold.
    #[error("{0} is not a value a table takes")]
    BadTableValue(String),
    /// A field aimed at the first page's own block while the setting is off
    /// (`PROSE-FEAT-067`).
    #[error("the first page has no band of its own while the setting is off")]
    FirstPageBandOff,
    /// A search or a replacement with no search open, or a query this build
    /// cannot run.
    #[error("{0}")]
    FindRefused(&'static str),
    /// A footnote asked for in a header or a footer (`PROSE-FEAT-071`).
    #[error("a header or a footer cannot hold a footnote")]
    FootnoteInBand,
    /// A footnote asked for in a note's own text (`PROSE-FEAT-306`).
    #[error("a footnote cannot hold a footnote")]
    FootnoteInNote,
    /// The command names a footnote the document does not hold.
    #[error("footnote {0} is not in the document")]
    NoSuchFootnote(Uuid),
    /// A comment asked for in a header or a footer, drawn on every page.
    #[error("a header or a footer cannot hold a comment")]
    CommentInBand,
    /// A comment asked for in a note's text, which the comment panel does not
    /// place (`PROSE-FEAT-307`).
    #[error("a footnote cannot hold a comment")]
    CommentInNote,
    /// The command names a comment thread the document does not hold.
    #[error("comment {0} is not in the document")]
    NoSuchComment(Uuid),
    /// The command names a reply the thread does not hold.
    #[error("reply {0} is not in the thread")]
    NoSuchReply(Uuid),
    /// A reply edited or deleted by someone other than its author
    /// (`PROSE-FEAT-080`, `PROSE-FEAT-198`).
    #[error("only its author can change a reply")]
    NotYourReply,
    /// A comment written before the session named its user.
    #[error("the session has no user to sign the comment")]
    NoIdentity,
    /// A command suggestion mode does not intercept: only typing and deleting
    /// become suggestions, so structure and formatting wait for the mode to
    /// be off.
    #[error("suggestion mode records typing and deleting only")]
    NotWhileSuggesting,
    /// The position holds no cross reference to follow.
    #[error("no cross reference sits there")]
    NoCrossRef,
    /// A write the CRDT engine itself refused.
    #[error("the document engine could not write the change: {0}")]
    WriteFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_block_discriminant_round_trips() {
        for code in 0u8..=10 {
            let block_type = BlockType::from_code(code).expect("a declared discriminant");
            assert_eq!(block_type.code(), code);
        }
        assert_eq!(BlockType::from_code(11), None);
    }

    #[test]
    fn the_nesting_table_refuses_a_table_inside_a_cell() {
        assert!(BlockType::TableCell.accepts(BlockType::Paragraph));
        assert!(!BlockType::TableCell.accepts(BlockType::Table));
        assert!(BlockType::Table.accepts(BlockType::TableRow));
        assert!(!BlockType::Table.accepts(BlockType::Paragraph));
        assert!(!BlockType::Paragraph.accepts(BlockType::Paragraph));
    }

    #[test]
    fn a_row_and_a_cell_are_refused_at_the_root() {
        assert!(BlockType::Paragraph.accepted_at_root());
        assert!(BlockType::Table.accepted_at_root());
        assert!(!BlockType::TableRow.accepted_at_root());
        assert!(!BlockType::TableCell.accepted_at_root());
    }

    #[test]
    fn presence_is_not_a_mutation() {
        let pos = CursorPosition {
            block_id: BlockId::new(1, 0),
            offset: 0,
            anchor: None,
        };
        assert!(!ProseCommand::UpdateCursor { pos }.is_mutating());
        assert!(!ProseCommand::Noop.is_mutating());
        assert!(
            ProseCommand::SplitBlock {
                block_id: BlockId::new(1, 0),
                pos: 0
            }
            .is_mutating()
        );
    }
}

/// One run of text inserted or deleted in one block's body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub block_id: BlockId,
    /// Where the run starts, in the text as it stood before this edit.
    pub at: u32,
    /// The code points inserted, positive, or deleted, negative.
    pub delta: i64,
}

/// A value handed from the Logic thread to the UI thread over the shared heap,
/// opaque to this crate.
///
/// The layout type belongs to the engine crate, which depends on this one, so
/// the envelope cannot name it. Both threads share one heap (`ARCH-001`), so the
/// value moves as a pointer and is never copied or serialized. This envelope
/// never crosses to the Sync worker.
#[derive(Clone)]
pub struct SharedLayout(Arc<dyn Any + Send + Sync>);

impl SharedLayout {
    pub fn new<T: Any + Send + Sync>(value: T) -> Self {
        Self(Arc::new(value))
    }

    /// A value the sender keeps a handle on, so the last handle dropped is
    /// its own and the receiver never frees the value.
    pub fn from_arc<T: Any + Send + Sync>(value: Arc<T>) -> Self {
        Self(value)
    }

    /// The value as a handle of its own, when it is of the type asked for.
    #[must_use]
    pub fn shared<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        Arc::clone(&self.0).downcast().ok()
    }

    /// The value, when it is of the type asked for.
    #[must_use]
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

/// Two handles are equal when they point at the same value: a layout is a
/// whole value, never compared field by field.
impl PartialEq for SharedLayout {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for SharedLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedLayout(..)")
    }
}
