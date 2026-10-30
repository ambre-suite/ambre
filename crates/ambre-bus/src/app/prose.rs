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
//!   style register, the presence sample, and the command / event enums.
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

use ambre_core::{CommentId, FootnoteId, Pt, ReplyId, Rgb, SuggestionId, UserId};

use super::grid::{CellPos, Col, ColCount, RowCount};

pub mod mark;
pub use mark::MarkValue;

ambre_core::define_index! {
    /// A code-point offset into the body of one block, as the CRDT counts it
    /// (`PROSE-FEAT-006`). Every offset this plane carries is one. The view
    /// lays a pending draft out in its own space, and converts at its edge.
    pub struct DocOffset(u32);
    /// A number of code points in one block's body.
    pub struct DocCount;
    /// The signed gap between two offsets of one block's body: code points
    /// inserted when positive, deleted when negative.
    pub struct DocDelta(i64);
    /// A run of code points in one block's body.
    pub struct DocSpan;
    /// A list indexed by offset in one block's body.
    pub struct DocVec;
}

impl DocOffset {
    /// The same offset, as a code-point offset of the block's text.
    #[must_use]
    pub fn as_char(self) -> ambre_core::CharOffset {
        ambre_core::CharOffset::new(self.get())
    }
}

impl DocCount {
    /// The code points `text` holds, as a block's body counts them.
    #[must_use]
    pub fn of(text: &str) -> Self {
        Self::from_len(text.chars().count())
    }
}

/// The number of a text change the view did not draw: a remote one, or a
/// local one the Logic thread rewrote. The view hands back the last it
/// applied in [`ProseCommand::Sync`].
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TextChangeSeq(pub u64);

impl TextChangeSeq {
    /// The number of the change after this one.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// How many text commands of the view the Logic thread has taken: the number
/// a draft entry waits on. Apart from [`TextChangeSeq`], which counts the
/// other direction.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TextAck(pub u64);

impl TextAck {
    /// The count after one more command is taken.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// The count `n` commands later.
    #[must_use]
    pub const fn plus(self, n: u64) -> Self {
        Self(self.0.saturating_add(n))
    }
}

// =============================================================================
// Pages (`features/ambre-spec-prose-viewport.md`)
// =============================================================================

ambre_core::define_index! {
    /// A page of one layout, counted from zero. The layout, the window and
    /// the view count pages this way. A reader sees a [`PageNumber`].
    pub struct PageIndex(u32);
    /// A number of pages.
    pub struct PageCount;
    /// The signed gap between two pages.
    pub struct PageDelta(i64);
    /// A run of pages.
    pub struct PageSpan;
    /// A list indexed by page.
    pub struct PageVec;
}

impl PageIndex {
    /// The number a reader sees for this page: its index plus one.
    ///
    /// The one conversion from a page index to a page number. It saturates
    /// at the last number the axis holds.
    #[must_use]
    pub fn number(self) -> PageNumber {
        PageNumber::from_position(self.to_usize())
    }
}

ambre_core::define_ordinal! {
    /// A page as a reader sees it: counted from one (`PROSE-FEAT-060`).
    ///
    /// A table of contents and a page-number field show it. The layout counts
    /// pages from zero in a [`PageIndex`], and [`PageIndex::number`] is the one
    /// way from that count to this one. Zero is not a page number, so "no page
    /// known" is an `Option` and never a zero.
    pub struct PageNumber;
}

impl PageNumber {
    /// The page index this number shows.
    #[must_use]
    pub const fn index(self) -> PageIndex {
        PageIndex::new(self.get() - 1)
    }
}

ambre_core::define_ordinal! {
    /// The number a footnote is called by, one for the first call in document
    /// order. It is counted, never stored (`PROSE-FEAT-013`).
    pub struct FootnoteOrdinal;
}

impl PageCount {
    /// The number of pages from the first up to and with `last`.
    #[must_use]
    pub const fn through(last: PageIndex) -> Self {
        last.next().count_from(PageIndex::ZERO)
    }

    /// The last of this many pages, `None` for no page.
    #[must_use]
    pub const fn last(self) -> Option<PageIndex> {
        self.as_end().prev()
    }

    /// Every page of this many, from the first.
    #[must_use]
    pub const fn span(self) -> PageSpan {
        PageSpan::new(PageIndex::ZERO, self.as_end())
    }
}

impl fmt::Display for PageCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

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
    pub peer: ambre_core::PeerId,
    pub counter: ambre_core::OpCounter,
}

impl BlockId {
    #[must_use]
    pub const fn new(peer: ambre_core::PeerId, counter: ambre_core::OpCounter) -> Self {
        Self { peer, counter }
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.counter.0, self.peer)
    }
}

/// The name of a paragraph style a block carries, built-in or user-defined
/// (`ambre-spec-prose-schema.md`). The tree stores the name itself, so the
/// newtype exists only to keep it apart from the other strings of a command.
/// A user may have typed the name, so `Debug` prints its length alone.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StyleId(pub String);

impl fmt::Debug for StyleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StyleId({} chars)", self.0.chars().count())
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
    Todo(TodoMark),
}

/// Whether a to-do item's checkbox is ticked.
///
/// A clipboard payload carries it, so the variant that was `false` comes
/// first and a payload written before reads the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TodoMark {
    /// Not ticked: the item is still to do.
    #[default]
    Open,
    /// Ticked: the item is done.
    Done,
}

impl TodoMark {
    /// [`TodoMark::Done`] when the stored or read checkbox is ticked.
    #[must_use]
    pub fn done_if(ticked: bool) -> Self {
        if ticked { Self::Done } else { Self::Open }
    }

    /// The checkbox as the document stores it.
    #[must_use]
    pub fn is_done(self) -> bool {
        self == Self::Done
    }
}

/// How an ordered item's ordinal is spelled. A clipboard payload carries it
/// as its bare code, and a decoded code outside the five is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
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

/// The error of a decoded ordinal style outside the five codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("an ordinal style is a code of zero to four, not {0}")]
pub struct InvalidOrderedStyle(pub u8);

impl TryFrom<u8> for OrderedStyle {
    type Error = InvalidOrderedStyle;

    fn try_from(code: u8) -> Result<Self, Self::Error> {
        Self::from_code(code).ok_or(InvalidOrderedStyle(code))
    }
}

impl From<OrderedStyle> for u8 {
    fn from(style: OrderedStyle) -> Self {
        style.code()
    }
}

/// One of the six heading levels the format declares
/// (`features/ambre-spec-prose-headings.md`).
///
/// A command carries one of these, so a level outside one to six cannot reach
/// a block through a command. A level read back from a document may still lie
/// outside, and [`HeadingLevel::clamp`] brings it to the nearer end. The type
/// serializes as the bare number, and a decoded number outside one to six is
/// refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum HeadingLevel {
    H1 = 1,
    H2 = 2,
    H3 = 3,
    H4 = 4,
    H5 = 5,
    H6 = 6,
}

impl HeadingLevel {
    /// The six levels, shallowest first.
    pub const ALL: [Self; 6] = [Self::H1, Self::H2, Self::H3, Self::H4, Self::H5, Self::H6];

    /// The deepest level.
    pub const MAX: Self = Self::H6;

    /// The level `level` names, or `None` outside one to six.
    #[must_use]
    pub const fn new(level: u8) -> Option<Self> {
        Some(match level {
            1 => Self::H1,
            2 => Self::H2,
            3 => Self::H3,
            4 => Self::H4,
            5 => Self::H5,
            6 => Self::H6,
            _ => return None,
        })
    }

    /// `level` brought inside one to six: zero reads as a level one, and
    /// anything past six as a level six.
    #[must_use]
    pub const fn clamp(level: u8) -> Self {
        match Self::new(level) {
            Some(level) => level,
            None if level == 0 => Self::H1,
            None => Self::H6,
        }
    }

    /// The level as a number, one to six.
    #[must_use]
    pub const fn get(self) -> u8 {
        self as u8
    }
}

/// The error of a decoded heading level outside one to six.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a heading level is one to six, not {0}")]
pub struct InvalidHeadingLevel(pub u8);

impl TryFrom<u8> for HeadingLevel {
    type Error = InvalidHeadingLevel;

    fn try_from(level: u8) -> Result<Self, Self::Error> {
        Self::new(level).ok_or(InvalidHeadingLevel(level))
    }
}

impl From<HeadingLevel> for u8 {
    fn from(level: HeadingLevel) -> Self {
        level.get()
    }
}

impl std::fmt::Display for HeadingLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

/// The language a code block is marked as. A tag is never empty: a block
/// with no language reads as plain and carries `None`, so the empty string
/// is never a language.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CodeLang(String);

impl CodeLang {
    /// The tag `text` names, `None` for the empty text, which reads as plain.
    pub fn new(text: impl Into<String>) -> Option<Self> {
        let text = text.into();
        (!text.is_empty()).then_some(Self(text))
    }

    /// The tag as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The text a block stores for `lang`: the tag, or the empty text of a
    /// plain block.
    pub fn stored(lang: Option<&Self>) -> &str {
        lang.map_or("", Self::as_str)
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
    /// The level is mandatory at creation.
    Heading {
        level: HeadingLevel,
    },
    /// The style is mandatory at creation, the way a heading's level is: an
    /// item minted without one would need a default and a follow-up command.
    ListItem {
        style: ListStyle,
    },
    /// `None` reads as plain.
    CodeBlock {
        lang: Option<CodeLang>,
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

/// A line height as a multiple of the font size: a bare ratio, finite and
/// above zero. A zero or negative multiple would stack every line on one.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct LineSpacingMultiple(f32);

impl LineSpacingMultiple {
    /// Single spacing.
    pub const SINGLE: Self = Self(1.0);
    /// One and a half lines, the seeded default.
    pub const ONE_AND_HALF: Self = Self(1.5);

    /// The multiple `ratio`, `None` unless it is finite and above zero.
    #[must_use]
    pub fn new(ratio: f32) -> Option<Self> {
        (ratio.is_finite() && ratio > 0.0).then_some(Self(ratio))
    }

    /// The bare ratio a layout engine reads.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl fmt::Display for LineSpacingMultiple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// The error of a line-height multiple that is not finite and above zero.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("a line-height multiple is finite and above zero, not {0}")]
pub struct InvalidLineSpacingMultiple(pub f32);

impl TryFrom<f32> for LineSpacingMultiple {
    type Error = InvalidLineSpacingMultiple;

    fn try_from(ratio: f32) -> Result<Self, Self::Error> {
        Self::new(ratio).ok_or(InvalidLineSpacingMultiple(ratio))
    }
}

impl From<LineSpacingMultiple> for f32 {
    fn from(multiple: LineSpacingMultiple) -> Self {
        multiple.0
    }
}

/// One line-height setting. The mode decides the unit of its number, so each
/// mode carries its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    /// A multiple of the font size.
    Multiple(LineSpacingMultiple),
    /// An exact height.
    Exact(Pt),
    /// A floor on the height.
    AtLeast(Pt),
}

impl LineHeight {
    /// The setting a stored mode and number read as. `None` for a multiple
    /// that is not finite and above zero, and for a height that is not finite.
    #[must_use]
    pub fn from_stored(mode: LineHeightMode, value: f32) -> Option<Self> {
        let height = || Some(Pt::new(value)).filter(|points| points.is_finite());
        Some(match mode {
            LineHeightMode::Multiple => Self::Multiple(LineSpacingMultiple::new(value)?),
            LineHeightMode::Exact => Self::Exact(height()?),
            LineHeightMode::AtLeast => Self::AtLeast(height()?),
        })
    }

    /// The mode the store writes.
    #[must_use]
    pub fn mode(self) -> LineHeightMode {
        match self {
            Self::Multiple(_) => LineHeightMode::Multiple,
            Self::Exact(_) => LineHeightMode::Exact,
            Self::AtLeast(_) => LineHeightMode::AtLeast,
        }
    }

    /// The number the store writes beside the mode.
    #[must_use]
    pub fn stored_value(self) -> f32 {
        match self {
            Self::Multiple(factor) => factor.get(),
            Self::Exact(height) | Self::AtLeast(height) => height.get(),
        }
    }
}

ambre_core::define_flag! {
    /// Whether a paragraph may split across a page break
    /// (`features/ambre-spec-prose-pagination.md`).
    pub enum KeepTogether {
        /// The paragraph may split across a page break.
        Splittable,
        /// The paragraph moves whole to the next page rather than split.
        Together,
    }

    /// Whether a paragraph stays on the page of the next one
    /// (`features/ambre-spec-prose-pagination.md`).
    pub enum KeepWithNext {
        /// A page break may fall between the paragraph and the next one.
        Free,
        /// The paragraph moves with the next one to its page.
        WithNext,
    }
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
    pub indent_first: Option<Pt>,
    pub indent_left: Option<Pt>,
    pub indent_right: Option<Pt>,
    pub space_before: Option<Pt>,
    pub space_after: Option<Pt>,
    pub line_height: Option<LineHeight>,
    pub keep_together: Option<KeepTogether>,
    pub keep_with_next: Option<KeepWithNext>,
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

/// Where a caret is on this replica: the block, and the offset resolved
/// against the document this peer holds.
///
/// `offset` is a count of Unicode code points into the named block's body. A
/// command carries the position and the view is handed it. No frame carries
/// it: an offset names another character on a peer that has not applied the
/// same insertions. So the presence sample carries an [`EncodedCursor`]
/// instead (`PROSE-004`). The type derives `serde`, and nothing serializes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorPosition {
    pub block_id: BlockId,
    pub offset: DocOffset,
    /// The other end of a selection, when the caret has one.
    pub anchor: Option<(BlockId, DocOffset)>,
}

/// The peer-authored presence sample.
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
/// them. A sample whose block this peer does not hold is dropped, and the cursor
/// is never decoded (`PROSE-057`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresencePayload {
    pub block_id: BlockId,
    pub cursor: EncodedCursor,
    pub anchor: Option<(BlockId, EncodedCursor)>,
    pub display_name: Option<ambre_core::DisplayName>,
}

/// A `loro::Cursor` as `Cursor::encode` writes it (`PROSE-004`).
///
/// The bytes are opaque to this plane. Only the engine that holds the document
/// decodes them, and bytes it cannot decode resolve to no caret
/// (`PROSE-057`). Serde sees the bare bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EncodedCursor(Vec<u8>);

impl EncodedCursor {
    /// Wraps the bytes `Cursor::encode` wrote.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The bytes, for `Cursor::decode`.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// One live collaborator's caret, as the view is handed it.
///
/// No colour: the view derives it from the `UserId` through
/// `ambre_draw::peer::peer_color`, so a collaborator keeps one colour across
/// Prose, Trame and Toile (`PRESENCE-004`). No serde: it moves by value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCursor {
    pub user_id: UserId,
    pub display_name: Option<ambre_core::DisplayName>,
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
/// The document stores `color` as the `"#rrggbb"` record the mark registry
/// uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellBorder {
    pub width: Pt,
    pub color: Rgb,
}

ambre_core::define_tally! {
    /// A number of words, as Unicode word boundaries split the text.
    pub struct WordTotal;
    /// A number of grapheme clusters: the characters a reader counts.
    pub struct GraphemeCount;
    /// A number of paragraphs that hold a visible character.
    pub struct ParagraphCount;
}

/// The five figures of the status bar (`features/ambre-spec-prose-word-count.md`).
///
/// Four are tallies of the text. The fifth is read off the layout, and each
/// half is refreshed by its own trigger.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WordCount {
    pub words: WordTotal,
    /// Grapheme clusters that are not whitespace (`PROSE-FEAT-039`).
    pub characters: GraphemeCount,
    /// The same clusters, whitespace and paragraph breaks included
    /// (`PROSE-FEAT-121`).
    pub characters_with_spaces: GraphemeCount,
    pub paragraphs: ParagraphCount,
    /// From the layout, never from the tally (`PROSE-FEAT-038`).
    pub pages: PageCount,
}

/// What a format command does to one presence mark over a selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkAction {
    Remove,
    Apply,
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
    pub link: Option<ambre_core::LinkUrl>,
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
    pub header_shown: BandState,
    pub footer_shown: BandState,
    pub first_page: FirstPage,
}

/// Whose change of the text an event answers: the view's own, which it drew
/// when it sent the command, or a change it never drew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ChangeOrigin {
    #[default]
    Remote,
    Local,
}

/// Whether a session's edits land or wait as suggestions
/// (`features/ambre-spec-prose-track-changes.md`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EditMode {
    /// An edit lands in the text.
    #[default]
    Editing,
    /// An edit is marked as a suggestion, for another to accept or reject.
    Suggesting,
}

/// Whether one side, the header or the footer, is drawn (`PROSE-FEAT-167`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BandState {
    #[default]
    Off,
    On,
}

impl BandState {
    /// [`BandState::On`] when the document's setting says the side shows.
    #[must_use]
    pub fn on_if(shown: bool) -> Self {
        if shown { Self::On } else { Self::Off }
    }

    /// The setting as the document stores it.
    #[must_use]
    pub fn is_on(self) -> bool {
        self == Self::On
    }
}

/// Whether the first page draws a header and a footer of its own
/// (`PROSE-FEAT-063`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FirstPage {
    /// The first page draws the regular pair.
    #[default]
    SameAsRest,
    /// The first page draws its own pair.
    OwnBands,
}

impl FirstPage {
    /// [`FirstPage::OwnBands`] when the document's setting says so.
    #[must_use]
    pub fn own_if(own: bool) -> Self {
        if own {
            Self::OwnBands
        } else {
            Self::SameAsRest
        }
    }

    /// The setting as the document stores it.
    #[must_use]
    pub fn has_own(self) -> bool {
        self == Self::OwnBands
    }
}

/// What a search looks for (`features/ambre-spec-prose-find-replace.md`):
/// the shared shape over Prose's own options.
pub type FindQuery = super::find::FindQuery<FindOptions>;

/// The refinements of a Prose search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FindOptions {
    pub case: ambre_core::CaseMatch,
    pub word: ambre_core::WordMatch,
    /// [`PatternKind::Regex`] asks for the regular-expression search, unbuilt:
    /// the engine refuses a query that sets it.
    pub pattern: PatternKind,
}

/// How a find reads its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PatternKind {
    /// The text is looked for as it is written.
    #[default]
    Literal,
    /// The text is a regular expression.
    Regex,
}

/// One match: a range of code points inside the body of one block
/// (`PROSE-FEAT-034`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindMatch {
    pub block_id: BlockId,
    pub range: DocSpan,
}

/// One footnote as the view lists it (`features/ambre-spec-prose-footnotes.md`).
///
/// The ordinal is counted from the calls in document order and never stored
/// (`PROSE-FEAT-013`, `PROSE-FEAT-095`). `None` marks a note no call reaches,
/// kept for the author to review (`PROSE-FEAT-098`).
#[derive(Clone, PartialEq, Eq)]
pub struct FootnoteEntry {
    pub footnote_id: FootnoteId,
    pub ordinal: Option<FootnoteOrdinal>,
    pub text: String,
}

// The note's body is the author's: a log carries its length alone.
impl fmt::Debug for FootnoteEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FootnoteEntry")
            .field("footnote_id", &self.footnote_id)
            .field("ordinal", &self.ordinal)
            .field("text", &ambre_core::RedactedChars::of(&self.text))
            .finish()
    }
}

/// One reply of a comment thread (`features/ambre-spec-prose-comments.md`).
///
/// The author is the one the writing session stamped, never a field the
/// command carried (`PROSE-FEAT-077`). The stamp is display only.
#[derive(Clone, PartialEq, Eq)]
pub struct CommentReply {
    pub reply_id: ReplyId,
    pub author: Option<ambre_core::DisplayName>,
    pub user_id: UserId,
    pub created_at: crate::Timestamp,
    pub text: String,
}

// The reply's body is the author's: a log carries its length alone.
impl fmt::Debug for CommentReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommentReply")
            .field("reply_id", &self.reply_id)
            .field("author", &self.author)
            .field("user_id", &self.user_id)
            .field("created_at", &self.created_at)
            .field("text", &ambre_core::RedactedChars::of(&self.text))
            .finish()
    }
}

ambre_core::define_flag! {
    /// Whether a comment's mark still spans text (`PROSE-FEAT-202`,
    /// `PROSE-FEAT-203`).
    pub enum AnchorState {
        /// The mark spans text, so the anchor is its live span.
        Anchored,
        /// The text under the mark is gone, so the anchor is the last span
        /// the mark held.
        Orphaned,
    }
}

/// One comment thread as the view lists it.
///
/// The anchor is the live mark's span, or the last one it held when
/// `orphaned` reads [`AnchorState::Orphaned`] (`PROSE-FEAT-202`,
/// `PROSE-FEAT-203`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentThread {
    pub comment_id: CommentId,
    pub anchor_block: Option<BlockId>,
    pub anchor_start: DocOffset,
    pub anchor_end: DocOffset,
    pub orphaned: AnchorState,
    pub resolved: ambre_core::ThreadState,
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
    pub start: DocOffset,
    pub end: DocOffset,
}

/// The key an inline mark is stored under in a block's text
/// (`ambre-spec-prose-schema.md section 3`).
///
/// A key is a registered root, such as `b` or `fg`, or a root and an id
/// joined by a colon, such as `cm:<id>`. A document written by a newer client
/// may carry a key this build does not know, so the type holds any text. The
/// prose registry reads the root and rules on it. The spelling is the one the
/// document stores. `Debug` prints the bare text, as the `String` it replaced
/// did.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MarkKey(String);

impl MarkKey {
    /// The key spelled as `text`.
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The key as the document stores it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part before the first colon, which names the registry entry. A key
    /// with no colon is its own root.
    #[must_use]
    pub fn root(&self) -> &str {
        self.0.split(':').next().unwrap_or(&self.0)
    }
}

impl From<&str> for MarkKey {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl std::borrow::Borrow<str> for MarkKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for MarkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for MarkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One run of a link mark, in its block's own offsets, and the target it
/// names (`features/ambre-spec-prose-selection.md section 2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    pub block_id: BlockId,
    pub start: DocOffset,
    pub end: DocOffset,
    pub url: ambre_core::LinkUrl,
}

/// One pending suggestion as the view lists it, derived from its marks and its
/// entry (`PROSE-FEAT-261`). A change across a paragraph break holds several
/// spans and stays one suggestion (`PROSE-FEAT-259`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub suggestion_id: SuggestionId,
    pub kind: SuggestionKind,
    pub author: Option<ambre_core::DisplayName>,
    pub user_id: UserId,
    pub spans: Vec<SuggestionSpan>,
}

/// What a cross reference shows (`features/ambre-spec-prose-cross-references.md`).
///
/// The reference record stores it, and the variant's position is its stored
/// code: a new kind is appended, never inserted.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossRefKind {
    /// The page the target starts on.
    PageNumber,
    /// The derived number of the target heading.
    HeadingNumber,
    /// A label the author wrote, whatever characters it holds.
    CustomText(String),
}

// The label is the author's: a log carries its length alone.
impl fmt::Debug for CrossRefKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PageNumber => f.write_str("PageNumber"),
            Self::HeadingNumber => f.write_str("HeadingNumber"),
            Self::CustomText(text) => f
                .debug_tuple("CustomText")
                .field(&ambre_core::RedactedChars::of(text))
                .finish(),
        }
    }
}

/// One cross reference as drawn from the last layout. Nothing here is stored
/// (`PROSE-FEAT-129`).
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedCrossRef {
    /// Where the reference sits: its block and its code-point offset.
    pub block_id: BlockId,
    pub offset: DocOffset,
    pub target: BlockId,
    /// The value drawn. `None` is the broken mark (`PROSE-FEAT-014`).
    pub shown: Option<String>,
}

// The value drawn may be a label the author wrote: a log carries its length.
impl fmt::Debug for ResolvedCrossRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedCrossRef")
            .field("block_id", &self.block_id)
            .field("offset", &self.offset)
            .field("target", &self.target)
            .field(
                "shown",
                &self.shown.as_deref().map(ambre_core::RedactedChars::of),
            )
            .finish()
    }
}

/// The number drawn in front of a heading, one counter per level from the
/// first (`PROSE-FEAT-030`). The first second-level heading under the second
/// first-level one is `2.1`. It is derived, stored nowhere, and never empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadingNumber(Vec<u32>);

impl HeadingNumber {
    /// The number made of `parts`, outermost level first. `None` for no part:
    /// a heading always has a counter at its own level.
    #[must_use]
    pub fn new(parts: Vec<u32>) -> Option<Self> {
        (!parts.is_empty()).then_some(Self(parts))
    }

    /// The counters, outermost level first.
    #[must_use]
    pub fn parts(&self) -> &[u32] {
        &self.0
    }
}

impl fmt::Display for HeadingNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, part) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(".")?;
            }
            write!(f, "{part}")?;
        }
        Ok(())
    }
}

/// One line of a table of contents. Nothing here is persisted
/// (`features/ambre-spec-prose-table-of-contents.md`).
#[derive(Clone, PartialEq, Eq)]
pub struct TocEntry {
    pub block_id: BlockId,
    pub level: HeadingLevel,
    pub text: String,
    /// Present only while the block is configured as numbered
    /// (`PROSE-FEAT-062`).
    pub heading_number: Option<HeadingNumber>,
    /// The page the heading starts on (`PROSE-FEAT-060`). `None` while the
    /// layout places no block that holds the heading.
    pub page: Option<PageNumber>,
}

// The heading's text is the author's: a log carries its length alone.
impl fmt::Debug for TocEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TocEntry")
            .field("block_id", &self.block_id)
            .field("level", &self.level)
            .field("text", &ambre_core::RedactedChars::of(&self.text))
            .field("heading_number", &self.heading_number)
            .field("page", &self.page)
            .finish()
    }
}

/// The entries one table-of-contents block lists, built from the headings and
/// a layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableOfContents {
    pub entries: Vec<TocEntry>,
    pub max_level: HeadingLevel,
    pub numbered: TocNumbering,
}

/// Whether a table of contents prints the number of each heading
/// (`PROSE-FEAT-062`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TocNumbering {
    /// The entries show no heading number.
    #[default]
    Plain,
    /// Each entry shows the number of its heading.
    Numbered,
}

impl TocNumbering {
    /// [`TocNumbering::Numbered`] when the block's stored switch is on.
    #[must_use]
    pub fn numbered_if(numbered: bool) -> Self {
        if numbered {
            Self::Numbered
        } else {
            Self::Plain
        }
    }

    /// The switch as the block stores it.
    #[must_use]
    pub fn is_numbered(self) -> bool {
        self == Self::Numbered
    }
}

// =============================================================================
// Commands -- the content plane, UI thread -> Logic thread
// =============================================================================

/// One thing to do to a Prose document.
///
/// Values move on a `conduit` channel between the two threads, which share one linear
/// memory, so no shape here derives `serde`. Every offset is a count of Unicode
/// code points into the body of the block the command names (`PROSE-045`).
///
/// The variants are the ones the engine answers today. The pictures, the
/// spell checker and the Word, HTML and PDF filters reach this enum with the
/// Story that builds each of them (`ambre-roadmap.md`).
#[derive(Clone, PartialEq)]
pub enum ProseCommand {
    /// Changes nothing, and is the one command a test drives the empty outcome
    /// with (`PROSE-047`).
    Noop,
    /// The view has taken every text change up to `seen`, the number the
    /// last one it applied carried. The Logic thread answers with
    /// [`ProseEvent::Synced`] once everything sent before has landed.
    Sync {
        seen: TextChangeSeq,
    },

    // -- Text editing (`features/ambre-spec-prose-text-editing.md`) --
    /// One committed run of text, at one position in one block.
    InsertText {
        block_id: BlockId,
        pos: DocOffset,
        text: String,
    },
    /// A range inside one block.
    DeleteText {
        block_id: BlockId,
        range: DocSpan,
    },
    /// One arbitrary, possibly cross-block, selection deleted in one command.
    /// The endpoints arrive in either order and the handler normalizes them. It
    /// is also the single join primitive (`PROSE-037`, `PROSE-039`).
    DeleteRange {
        anchor: (BlockId, DocOffset),
        focus: (BlockId, DocOffset),
    },
    /// The inverse of the join case above (`PROSE-038`).
    SplitBlock {
        block_id: BlockId,
        pos: DocOffset,
    },

    // -- Inline marks (`features/ambre-spec-prose-inline-styles.md`) --
    /// `value` is the mark's payload, checked by the grammar of the
    /// registry of `ambre-spec-prose-schema.md section 3` (`PROSE-008`).
    /// `None` is the value of a key whose presence alone is meaningful. The
    /// Logic thread checks the value again under `key`, since a sender may
    /// have checked it under another key.
    ApplyMark {
        block_id: BlockId,
        range: DocSpan,
        key: MarkKey,
        value: Option<MarkValue>,
    },
    RemoveMark {
        block_id: BlockId,
        range: DocSpan,
        key: MarkKey,
    },
    ClearInlineMarks {
        block_id: BlockId,
        range: DocSpan,
    },

    // -- Block structure --
    /// `init` carries the attributes the block type needs at creation, so no
    /// handler invents a default a follow-up command must correct.
    InsertBlock {
        parent_id: Option<BlockId>,
        after_id: Option<BlockId>,
        init: BlockInit,
        style_id: Option<StyleId>,
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
        style_id: StyleId,
    },
    /// `features/ambre-spec-prose-headings.md`. `None` takes the level away,
    /// and the block becomes a paragraph (`PROSE-FEAT-092`). One command for the two directions of one control:
    /// the style picker offers the six levels and the paragraph beside them.
    SetHeadingLevel {
        block_id: BlockId,
        level: Option<HeadingLevel>,
    },
    /// `None` sets the block back to plain.
    SetCodeLang {
        block_id: BlockId,
        lang: Option<CodeLang>,
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
        rows: RowCount,
        cols: ColCount,
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
        at_col: Col,
        width: Option<Pt>,
    },
    DeleteTableCol {
        table_id: BlockId,
        col_index: Col,
    },
    ResizeTableCol {
        table_id: BlockId,
        col_index: Col,
        width: Pt,
    },
    /// Two grid positions in either order. The handler resolves them to two
    /// block ids before it writes (`PROSE-FEAT-241`).
    MergeTableCells {
        table_id: BlockId,
        start: CellPos,
        end: CellPos,
    },
    UnmergeTableCell {
        cell_id: BlockId,
    },
    /// Each `None` leaves that property alone.
    SetCellStyle {
        cell_id: BlockId,
        bg: Option<Rgb>,
        valign: Option<VAlign>,
        border: Option<CellBorder>,
        padding: Option<Pt>,
    },
    /// Tab and Shift-Tab: the next or the previous stop in reading order.
    /// Past the last cell, a row is appended (`PROSE-FEAT-251`).
    StepTableCell {
        cell_id: BlockId,
        direction: ambre_core::StepDirection,
    },

    // -- Table of contents (`features/ambre-spec-prose-table-of-contents.md`) --
    /// `max_level` is one of the six levels (`PROSE-FEAT-292`).
    InsertToc {
        parent_id: Option<BlockId>,
        after_id: Option<BlockId>,
        max_level: HeadingLevel,
        numbered: TocNumbering,
    },
    UpdateTocConfig {
        block_id: BlockId,
        max_level: HeadingLevel,
        numbered: TocNumbering,
    },

    // -- Viewport (`features/ambre-spec-prose-viewport.md`) --
    /// Hand the view the zero-based pages `first` to `last`, both included.
    /// The view sends it when the canvas nears a page the layout in force
    /// does not carry (`PROSE-UI-211`); it writes nothing.
    ShowPages {
        first: PageIndex,
        last: PageIndex,
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
        anchor: (BlockId, DocOffset),
        focus: (BlockId, DocOffset),
    },

    // -- Clipboard (`features/ambre-spec-prose-clipboard.md`) --
    /// Build the clipboard payload of a whole selection. The endpoints
    /// arrive in either order; it writes nothing (`PROSE-UI-040`,
    /// `PROSE-UI-109`).
    CopyRange {
        anchor: (BlockId, DocOffset),
        focus: (BlockId, DocOffset),
    },
    /// Insert what the clipboard held at one place, in one commit. The
    /// Logic thread takes the richest half it can read (`PROSE-UI-042`).
    /// This application's own bytes come first, marks and block structure
    /// kept (`PROSE-UI-107`). The HTML another application wrote comes next,
    /// then the plain text, one paragraph per line. The view only carries the halves
    /// (`PROSE-UI-105`).
    Paste {
        at: (BlockId, DocOffset),
        text: String,
        html: Option<String>,
        payload: Option<ambre_core::ClipboardBytes>,
    },
    /// Apply one presence mark over a whole selection, or remove it. Every
    /// block the selection covers changes in one commit, so one undo takes
    /// the whole gesture back.
    FormatSelection {
        anchor: (BlockId, DocOffset),
        focus: (BlockId, DocOffset),
        key: MarkKey,
        action: MarkAction,
    },

    // -- Headers and footers (`features/ambre-spec-prose-headers-footers.md`) --
    /// Show or hide the top band (`PROSE-FEAT-167`).
    SetHeaderEnabled {
        shown: BandState,
    },
    /// Show or hide the bottom band (`PROSE-FEAT-167`).
    SetFooterEnabled {
        shown: BandState,
    },
    /// Turn the first page's own header and footer on or off, together
    /// (`PROSE-FEAT-063`).
    SetDifferentFirstPage {
        first_page: FirstPage,
    },
    /// A field in the header block `target` names. A position past the end
    /// is clamped to it (`PROSE-FEAT-068`).
    InsertHeaderField {
        field: HeaderField,
        target: HfTarget,
        pos: DocOffset,
    },
    InsertFooterField {
        field: HeaderField,
        target: HfTarget,
        pos: DocOffset,
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
        now: ambre_core::Timestamp,
    },

    // -- Find and replace (`features/ambre-spec-prose-find-replace.md`) --
    /// Open a search, or replace the one open. It writes nothing
    /// (`PROSE-FEAT-151`). `None` is the emptied field: it closes the open
    /// search and the highlight clears (`FIND-005`).
    FindStart {
        query: Option<FindQuery>,
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
    /// Replace every match, as one write and one undo item (`FIND-013`).
    ReplaceAll {
        replacement: String,
    },

    // -- Footnotes (`features/ambre-spec-prose-footnotes.md`) --
    /// A note, and one call to it at `pos` in `block_id` (`PROSE-FEAT-099`).
    InsertFootnote {
        block_id: BlockId,
        pos: DocOffset,
        note_text: String,
    },
    /// Rewrite a note's body.
    EditFootnote {
        footnote_id: FootnoteId,
        new_text: String,
    },
    /// Remove a note and every call to it (`PROSE-FEAT-070`).
    DeleteFootnote {
        footnote_id: FootnoteId,
    },

    // -- Comments (`features/ambre-spec-prose-comments.md`) --
    /// A thread over `range` in `block_id`, opened by a reply holding `text`.
    /// `now` is the display stamp: the Logic thread reads no clock.
    AddComment {
        block_id: BlockId,
        range: DocSpan,
        text: String,
        now: ambre_core::Timestamp,
    },
    /// A reply at the end of the thread, which reopens it (`PROSE-FEAT-079`).
    ReplyToComment {
        comment_id: CommentId,
        text: String,
        now: ambre_core::Timestamp,
    },
    /// Close a thread.
    ResolveComment {
        comment_id: CommentId,
    },
    /// Reopen a thread, with no reply added.
    ReopenComment {
        comment_id: CommentId,
    },
    /// Remove a thread and its mark, and leave the text.
    DeleteComment {
        comment_id: CommentId,
    },
    /// Remove one of the caller's replies, or the thread when it is the head
    /// (`PROSE-FEAT-081`).
    DeleteReply {
        comment_id: CommentId,
        reply_id: ReplyId,
    },
    /// Rewrite one of the caller's replies (`PROSE-FEAT-080`).
    EditCommentReply {
        comment_id: CommentId,
        reply_id: ReplyId,
        new_text: String,
    },

    // -- Track changes (`features/ambre-spec-prose-track-changes.md`) --
    /// Turn suggestion mode on or off for this session. Nothing pending is
    /// applied or cancelled (`PROSE-FEAT-089`).
    EnableTrackChanges {
        mode: EditMode,
    },
    /// Apply every span of one suggestion (`PROSE-FEAT-083`).
    AcceptSuggestion {
        suggestion_id: SuggestionId,
    },
    /// Cancel every span of one suggestion (`PROSE-FEAT-273`).
    RejectSuggestion {
        suggestion_id: SuggestionId,
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
        pos: DocOffset,
        target_block_id: BlockId,
        kind: CrossRefKind,
    },
    /// Bring the target of the reference at `pos` in `block_id` into view. It
    /// writes nothing (`PROSE-UI-086`).
    FollowCrossRef {
        block_id: BlockId,
        pos: DocOffset,
    },

    // -- Presence (`1-Foundations/ambre-spec-presence.md`) --
    /// The handler builds the presence sample and returns it as the outcome's
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
    /// Bring a file into the document, after its last block, as one batch
    /// (`PROSE-IO-027`, `PROSE-IO-051`). Answers
    /// [`ProseEvent::ImportCompleted`], or a refusal carrying an
    /// [`InteropError`] when the file produced nothing (`PROSE-IO-047`).
    ImportDocument {
        format: ImportFormat,
        data: ambre_core::FileBytes,
    },
}

// A text a user typed prints its length alone, and every other field its own
// `Debug`. A log of a command carries none of the document's text.
impl fmt::Debug for ProseCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        macro_rules! fields {
            ($name:literal $(, $field:ident)*) => {
                f.debug_struct($name)$(.field(stringify!($field), $field))*.finish()
            };
        }
        macro_rules! texts {
            ($name:literal $(, $field:ident)* ; $text:ident $(, $tail:ident)*) => {
                f.debug_struct($name)
                    $(.field(stringify!($field), $field))*
                    .field(stringify!($text), &ambre_core::RedactedChars::of($text))
                    $(.field(stringify!($tail), $tail))*
                    .finish()
            };
        }
        match self {
            Self::Noop => f.write_str("Noop"),
            Self::Sync { seen } => fields!("Sync", seen),
            Self::InsertText {
                block_id,
                pos,
                text,
            } => texts!("InsertText", block_id, pos; text),
            Self::DeleteText { block_id, range } => fields!("DeleteText", block_id, range),
            Self::DeleteRange { anchor, focus } => fields!("DeleteRange", anchor, focus),
            Self::SplitBlock { block_id, pos } => fields!("SplitBlock", block_id, pos),
            Self::ApplyMark {
                block_id,
                range,
                key,
                value,
            } => fields!("ApplyMark", block_id, range, key, value),
            Self::RemoveMark {
                block_id,
                range,
                key,
            } => fields!("RemoveMark", block_id, range, key),
            Self::ClearInlineMarks { block_id, range } => {
                fields!("ClearInlineMarks", block_id, range)
            }
            Self::InsertBlock {
                parent_id,
                after_id,
                init,
                style_id,
            } => fields!("InsertBlock", parent_id, after_id, init, style_id),
            Self::DeleteBlock { block_id } => fields!("DeleteBlock", block_id),
            Self::MoveBlock {
                block_id,
                new_parent,
                after_id,
            } => fields!("MoveBlock", block_id, new_parent, after_id),
            Self::SetBlockStyle { block_id, style_id } => {
                fields!("SetBlockStyle", block_id, style_id)
            }
            Self::SetHeadingLevel { block_id, level } => {
                fields!("SetHeadingLevel", block_id, level)
            }
            Self::SetCodeLang { block_id, lang } => fields!("SetCodeLang", block_id, lang),
            Self::SetListStyle { block_id, style } => fields!("SetListStyle", block_id, style),
            Self::IndentList { block_id } => fields!("IndentList", block_id),
            Self::UnindentList { block_id } => fields!("UnindentList", block_id),
            Self::SetParagraphStyle { block_id, style } => {
                fields!("SetParagraphStyle", block_id, style)
            }
            Self::InsertTable {
                parent_id,
                after_id,
                rows,
                cols,
            } => fields!("InsertTable", parent_id, after_id, rows, cols),
            Self::InsertTableRow {
                table_id,
                after_row,
            } => fields!("InsertTableRow", table_id, after_row),
            Self::DeleteTableRow { row_id } => fields!("DeleteTableRow", row_id),
            Self::InsertTableCol {
                table_id,
                at_col,
                width,
            } => fields!("InsertTableCol", table_id, at_col, width),
            Self::DeleteTableCol {
                table_id,
                col_index,
            } => fields!("DeleteTableCol", table_id, col_index),
            Self::ResizeTableCol {
                table_id,
                col_index,
                width,
            } => fields!("ResizeTableCol", table_id, col_index, width),
            Self::MergeTableCells {
                table_id,
                start,
                end,
            } => fields!("MergeTableCells", table_id, start, end),
            Self::UnmergeTableCell { cell_id } => fields!("UnmergeTableCell", cell_id),
            Self::SetCellStyle {
                cell_id,
                bg,
                valign,
                border,
                padding,
            } => fields!("SetCellStyle", cell_id, bg, valign, border, padding),
            Self::StepTableCell { cell_id, direction } => {
                fields!("StepTableCell", cell_id, direction)
            }
            Self::InsertToc {
                parent_id,
                after_id,
                max_level,
                numbered,
            } => fields!("InsertToc", parent_id, after_id, max_level, numbered),
            Self::UpdateTocConfig {
                block_id,
                max_level,
                numbered,
            } => fields!("UpdateTocConfig", block_id, max_level, numbered),
            Self::ShowPages { first, last } => fields!("ShowPages", first, last),
            Self::ShowBlock { block_id } => fields!("ShowBlock", block_id),
            Self::CountWords => f.write_str("CountWords"),
            Self::ReadMarks { anchor, focus } => fields!("ReadMarks", anchor, focus),
            Self::CopyRange { anchor, focus } => fields!("CopyRange", anchor, focus),
            Self::Paste {
                at,
                text,
                html,
                payload,
            } => f
                .debug_struct("Paste")
                .field("at", at)
                .field("text", &ambre_core::RedactedChars::of(text))
                .field("html", &html.as_deref().map(ambre_core::RedactedChars::of))
                .field("payload", payload)
                .finish(),
            Self::FormatSelection {
                anchor,
                focus,
                key,
                action,
            } => fields!("FormatSelection", anchor, focus, key, action),
            Self::SetHeaderEnabled { shown } => fields!("SetHeaderEnabled", shown),
            Self::SetFooterEnabled { shown } => fields!("SetFooterEnabled", shown),
            Self::SetDifferentFirstPage { first_page } => {
                fields!("SetDifferentFirstPage", first_page)
            }
            Self::InsertHeaderField { field, target, pos } => {
                fields!("InsertHeaderField", field, target, pos)
            }
            Self::InsertFooterField { field, target, pos } => {
                fields!("InsertFooterField", field, target, pos)
            }
            Self::SetBandAlign { block_id, align } => fields!("SetBandAlign", block_id, align),
            Self::StampModified { now } => fields!("StampModified", now),
            Self::FindStart { query } => fields!("FindStart", query),
            Self::FindNext => f.write_str("FindNext"),
            Self::FindPrev => f.write_str("FindPrev"),
            Self::FindClose => f.write_str("FindClose"),
            Self::ReplaceOne { replacement } => texts!("ReplaceOne"; replacement),
            Self::ReplaceAll { replacement } => texts!("ReplaceAll"; replacement),
            Self::InsertFootnote {
                block_id,
                pos,
                note_text,
            } => texts!("InsertFootnote", block_id, pos; note_text),
            Self::EditFootnote {
                footnote_id,
                new_text,
            } => texts!("EditFootnote", footnote_id; new_text),
            Self::DeleteFootnote { footnote_id } => fields!("DeleteFootnote", footnote_id),
            Self::AddComment {
                block_id,
                range,
                text,
                now,
            } => texts!("AddComment", block_id, range; text, now),
            Self::ReplyToComment {
                comment_id,
                text,
                now,
            } => texts!("ReplyToComment", comment_id; text, now),
            Self::ResolveComment { comment_id } => fields!("ResolveComment", comment_id),
            Self::ReopenComment { comment_id } => fields!("ReopenComment", comment_id),
            Self::DeleteComment { comment_id } => fields!("DeleteComment", comment_id),
            Self::DeleteReply {
                comment_id,
                reply_id,
            } => fields!("DeleteReply", comment_id, reply_id),
            Self::EditCommentReply {
                comment_id,
                reply_id,
                new_text,
            } => texts!("EditCommentReply", comment_id, reply_id; new_text),
            Self::EnableTrackChanges { mode } => fields!("EnableTrackChanges", mode),
            Self::AcceptSuggestion { suggestion_id } => {
                fields!("AcceptSuggestion", suggestion_id)
            }
            Self::RejectSuggestion { suggestion_id } => {
                fields!("RejectSuggestion", suggestion_id)
            }
            Self::AcceptAllSuggestions => f.write_str("AcceptAllSuggestions"),
            Self::RejectAllSuggestions => f.write_str("RejectAllSuggestions"),
            Self::InsertCrossRef {
                block_id,
                pos,
                target_block_id,
                kind,
            } => fields!("InsertCrossRef", block_id, pos, target_block_id, kind),
            Self::FollowCrossRef { block_id, pos } => fields!("FollowCrossRef", block_id, pos),
            Self::UpdateCursor { pos } => fields!("UpdateCursor", pos),
            Self::Undo => f.write_str("Undo"),
            Self::Redo => f.write_str("Redo"),
            Self::ExportDocument { format } => fields!("ExportDocument", format),
            Self::ImportDocument { format, data } => fields!("ImportDocument", format, data),
        }
    }
}

/// The formats a file enters the editor in
/// (`features/ambre-spec-prose-import-export.md section 3`). A reader adds its
/// variant on the day it lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportFormat {
    Markdown,
}

/// The formats a document leaves the editor in
/// (`features/ambre-spec-prose-import-export.md section 3`). A writer adds its
/// variant on the day it lands, so the view never offers a format no code
/// writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// The native file: the shared header over the document's own snapshot.
    Ambre,
    /// Markdown text, in the flavour named.
    Markdown(MarkdownFlavor),
}

/// The Markdown dialects a writer targets. GFM adds the table, the to-do item
/// and the footnote that CommonMark has no syntax for (`PROSE-IO-061`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownFlavor {
    Gfm,
    CommonMark,
}

/// The format a report speaks of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversionFormat {
    Ambre,
    Markdown,
}

/// What a finished conversion lost, for the user to read (`PROSE-IO-004`,
/// `PROSE-IO-053`). A conversion that produced nothing answers an
/// [`InteropError`] instead: the two are never one type (`PROSE-IO-047`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionReport {
    pub format: ConversionFormat,
    pub warnings: Vec<ConversionWarning>,
}

impl ConversionReport {
    /// A report of nothing lost.
    #[must_use]
    pub fn clean(format: ConversionFormat) -> Self {
        Self {
            format,
            warnings: Vec::new(),
        }
    }
}

ambre_core::define_tally! {
    /// The times one construct was lost in a conversion, the figure a
    /// warning carries in place of a repeat.
    pub struct WarningCount;
}

/// One fidelity loss. The same construct lost twice is one warning with its
/// count, so a long file does not bury the user in repeats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversionWarning {
    /// A construct the format has no syntax for, dropped or approximated.
    FeatureNotSupported {
        feature: UnsupportedFeature,
        format: ConversionFormat,
        count: WarningCount,
    },
    /// An element the reader does not recognise, dropped with its markup
    /// and its text kept where it has any (`PROSE-IO-005`).
    ElementDropped {
        element: DroppedElement,
        count: WarningCount,
    },
    /// A block the tree refuses where the file put it, moved to the nearest
    /// place it allows (`PROSE-IO-026`, `PROSE-IO-052`).
    BlockLifted {
        block: LiftedBlock,
        count: WarningCount,
    },
}

/// A construct a format has no syntax for, the subject of a [`ConversionWarning::FeatureNotSupported`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnsupportedFeature {
    PageFormat,
    HeaderAndFooter,
    ParagraphFormatting,
    HeadingLevel,
    MergedCells,
    Image,
    PageBreak,
    TableOfContents,
    TodoItem,
    ListNumbering,
    CellFormatting,
    Table,
    ColumnWidths,
    HeadingInCell,
    ListInCell,
    BlockInCell,
    ParagraphsInCell,
    Footnote,
    Underline,
    Superscript,
    Subscript,
    Strikethrough,
    Styling,
    Comment,
    Suggestion,
    CrossReference,
}

impl UnsupportedFeature {
    /// The words the user reads for it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PageFormat => "page format",
            Self::HeaderAndFooter => "header and footer",
            Self::ParagraphFormatting => "paragraph formatting",
            Self::HeadingLevel => "heading level outside 1 to 6",
            Self::MergedCells => "merged cells",
            Self::Image => "image",
            Self::PageBreak => "page break",
            Self::TableOfContents => "table of contents",
            Self::TodoItem => "to-do item",
            Self::ListNumbering => "list numbering style",
            Self::CellFormatting => "cell formatting",
            Self::Table => "table",
            Self::ColumnWidths => "column widths",
            Self::HeadingInCell => "heading in a table cell",
            Self::ListInCell => "list in a table cell",
            Self::BlockInCell => "block in a table cell",
            Self::ParagraphsInCell => "several paragraphs in a table cell",
            Self::Footnote => "footnote",
            Self::Underline => "underline",
            Self::Superscript => "superscript",
            Self::Subscript => "subscript",
            Self::Strikethrough => "strikethrough",
            Self::Styling => "colour, size or font",
            Self::Comment => "comment",
            Self::Suggestion => "suggestion",
            Self::CrossReference => "cross reference",
        }
    }
}

impl fmt::Display for UnsupportedFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// An element a reader does not recognise, the subject of a [`ConversionWarning::ElementDropped`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DroppedElement {
    Footnote,
    Html,
    Math,
    BlockQuote,
    Image,
    UnknownBlock,
    DeepList,
    TableOfContents,
}

impl DroppedElement {
    /// The words the user reads for it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Footnote => "footnote",
            Self::Html => "HTML",
            Self::Math => "math",
            Self::BlockQuote => "block quote",
            Self::Image => "image",
            Self::UnknownBlock => "unknown block",
            Self::DeepList => "list nesting deeper than 16",
            Self::TableOfContents => "table of contents",
        }
    }
}

impl fmt::Display for DroppedElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The kind of block a [`ConversionWarning::BlockLifted`] moved. The three table types read as one table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LiftedBlock {
    Paragraph,
    Heading,
    ListItem,
    Table,
    CodeBlock,
    Image,
    PageBreak,
    Rule,
    TableOfContents,
}

impl LiftedBlock {
    /// The words the user reads for it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Paragraph => "paragraph",
            Self::Heading => "heading",
            Self::ListItem => "list item",
            Self::Table => "table",
            Self::CodeBlock => "code block",
            Self::Image => "image",
            Self::PageBreak => "page break",
            Self::Rule => "rule",
            Self::TableOfContents => "table of contents",
        }
    }
}

impl fmt::Display for LiftedBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Why a find was refused, the reason a [`ProseRefusal::FindRefused`] carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FindRefusal {
    RegexUnbuilt,
    NoSearch,
}

impl FindRefusal {
    /// The words the user reads for it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RegexUnbuilt => "the regular-expression search is not built",
            Self::NoSearch => "no search is open",
        }
    }
}

impl fmt::Display for FindRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Why a conversion produced nothing (`PROSE-IO-010`, `PROSE-IO-047`).
///
/// The reason of a parse failure may quote the foreign file, so `Debug`
/// prints its length alone.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum InteropError {
    /// The bytes do not read as the format named.
    #[error("the file could not be read: {0}")]
    ParseFailed(String),
    /// The file is larger than a conversion accepts.
    #[error("the file is too large to import ({} bytes)", .0.0)]
    TooLarge(ambre_core::ByteSize),
}

impl std::fmt::Debug for InteropError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParseFailed(reason) => f
                .debug_tuple("ParseFailed")
                .field(&ambre_core::RedactedChars::of(reason))
                .finish(),
            Self::TooLarge(size) => f.debug_tuple("TooLarge").field(size).finish(),
        }
    }
}

impl ExportFormat {
    /// The media type of the bytes this format writes (`PROSE-IO-020`).
    #[must_use]
    pub fn media_type(self) -> ambre_core::MediaType {
        ambre_core::MediaType::from(match self {
            Self::Ambre => "application/octet-stream",
            Self::Markdown(_) => "text/markdown; charset=utf-8",
        })
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
    pub fn name(&self) -> CommandName {
        CommandName(match self {
            Self::Noop => "nothing",
            Self::Sync { .. } => "catching up",
            Self::InsertText { .. } => "typing",
            Self::CopyRange { .. } => "copying",
            Self::Paste { .. } => "pasting",
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
            Self::ReplyToComment { .. } => CommandName::REPLY_TO_COMMENT.0,
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
            Self::ImportDocument { .. } => "importing a file",
        })
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
#[derive(Clone, PartialEq)]
pub enum ProseEvent {
    /// A layout the Logic thread computed and shaped, whole. The view draws it
    /// and never lays text out itself (`PROSE-UI-006`).
    LayoutShaped { layout: SharedLayout },
    /// The confirmed change, local or imported. The bytes are the ones applied,
    /// never a re-export (`PROSE-023`, `PROSE-048`). A delta, or the snapshot
    /// an import adopted whole.
    DocUpdated { payload: ambre_core::LoroPayload },
    /// What that change did to the text, in code points, for a view that
    /// holds no document (`PROSE-UI-001`). The draft and the caret follow it
    /// (`PROSE-UI-005`, `PROSE-UI-028`, `PROSE-UI-143`).
    TextChanged {
        /// [`ChangeOrigin::Local`] when the change is what a text command of
        /// the view did, as the view already drew it. Any other change, remote
        /// or not, is [`ChangeOrigin::Remote`].
        origin: ChangeOrigin,
        /// The count of changes the view did not draw, this one included when
        /// `origin` is [`ChangeOrigin::Remote`]. The view hands it back in
        /// [`ProseCommand::Sync`].
        seq: TextChangeSeq,
        /// How many insertions and deletions of text the Logic thread has
        /// taken from the view so far.
        acked: TextAck,
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
    Synced { acked: TextAck },
    /// The whole live peer set, replaced rather than merged (`PRESENCE-009`).
    RemoteCursors { cursors: Vec<RemoteCursor> },
    /// A picture finished decrypting, and the blocks that show it are stale
    /// (`PROSE-059`, `PROSE-060`). The bytes stay in the Logic thread's cache;
    /// the view asks for them when it paints.
    MediaArrived {
        content_hash: ambre_core::ContentHash,
        blocks: Vec<BlockId>,
    },
    /// A full-state read the schema gate refused (`SCHEMA-008`).
    DocumentRefused { reason: String },
    /// A finished export: the bytes, the name to save them under, and their
    /// media type, so one download path serves every format (`PROSE-IO-020`).
    /// The view hands them to the browser (`PROSE-IO-056`).
    ExportReady {
        filename: ambre_core::ExportFilename,
        media_type: ambre_core::MediaType,
        data: ambre_core::FileBytes,
        /// What the format could not carry (`PROSE-IO-018`).
        report: ConversionReport,
    },
    /// An import landed, and what the file lost on the way in
    /// (`PROSE-IO-053`).
    ImportCompleted { report: ConversionReport },
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
        anchor: (BlockId, DocOffset),
        focus: (BlockId, DocOffset),
        marks: ActiveMarks,
    },
    /// The clipboard payload of the selection a [`ProseCommand::CopyRange`]
    /// named: its plain-text mirror (`PROSE-UI-108`), its HTML for another
    /// editor, and this application's own bytes.
    Clipboard {
        text: String,
        html: String,
        payload: ambre_core::ClipboardBytes,
    },
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
    CaretPlaced { block: BlockId, offset: DocOffset },
    /// Every footnote, in the order of its calls, orphans last
    /// (`PROSE-FEAT-013`, `PROSE-FEAT-098`).
    FootnotesUpdated { notes: Vec<FootnoteEntry> },
    /// Every comment thread, in document order of its anchor, orphans last.
    CommentsUpdated { threads: Vec<CommentThread> },
    /// Every pending suggestion, in document order of its first span, and
    /// whether this session is suggesting.
    SuggestionsUpdated {
        mode: EditMode,
        suggestions: Vec<Suggestion>,
    },
    /// Every run of a link mark, in document order.
    LinksUpdated { links: Vec<LinkSpan> },
    /// Every cross reference, resolved against the last layout.
    CrossRefsUpdated { refs: Vec<ResolvedCrossRef> },
    /// Bring a block into view, from its first page (`PROSE-UI-085`).
    RevealBlock { block: BlockId, page: PageIndex },
    /// The four band blocks and their settings, after a band command
    /// (`PROSE-FEAT-160`).
    BandsUpdated { bands: BandSet },
    /// Every match of the open search, in document order, and the index of
    /// the current one. An empty list clears the highlight (`PROSE-FEAT-289`).
    /// It goes to the peer that searched and to no other (`PROSE-FEAT-148`).
    FindResults {
        matches: Vec<FindMatch>,
        current: Option<super::find::MatchIndex>,
    },
    /// The current match moved; the list did not (`PROSE-FEAT-152`).
    FindUpdated {
        current: Option<super::find::MatchIndex>,
    },
    /// What the local history holds after a command that wrote. The chrome
    /// disables its undo and redo affordances from this alone (`UNDO-006`),
    /// and a command that wrote nothing sends none (`UNDO-009`).
    UndoState {
        undo: ambre_core::Availability,
        redo: ambre_core::Availability,
    },
    /// A command the engine could not honour. The value names what was
    /// refused and why (`PROSE-105`).
    Error(ProseError),
}

// The clipboard text is the author's and prints its length alone; every other
// field prints its own `Debug`, which holds no text of the document.
impl fmt::Debug for ProseEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        macro_rules! fields {
            ($name:literal $(, $field:ident)*) => {
                f.debug_struct($name)$(.field(stringify!($field), $field))*.finish()
            };
        }
        match self {
            Self::LayoutShaped { layout } => fields!("LayoutShaped", layout),
            Self::DocUpdated { payload } => fields!("DocUpdated", payload),
            Self::TextChanged {
                origin,
                seq,
                acked,
                edits,
                removed,
            } => fields!("TextChanged", origin, seq, acked, edits, removed),
            Self::BlockSplit { source, new_block } => fields!("BlockSplit", source, new_block),
            Self::Synced { acked } => fields!("Synced", acked),
            Self::RemoteCursors { cursors } => fields!("RemoteCursors", cursors),
            Self::MediaArrived {
                content_hash,
                blocks,
            } => fields!("MediaArrived", content_hash, blocks),
            Self::DocumentRefused { reason } => fields!("DocumentRefused", reason),
            Self::ExportReady {
                filename,
                media_type,
                data,
                report,
            } => fields!("ExportReady", filename, media_type, data, report),
            Self::ImportCompleted { report } => fields!("ImportCompleted", report),
            Self::TocUpdated { block_id, toc } => fields!("TocUpdated", block_id, toc),
            Self::WordCountUpdated { count } => fields!("WordCountUpdated", count),
            Self::SelectionMarks {
                anchor,
                focus,
                marks,
            } => fields!("SelectionMarks", anchor, focus, marks),
            Self::Clipboard {
                text,
                html,
                payload,
            } => f
                .debug_struct("Clipboard")
                .field("text", &ambre_core::RedactedChars::of(text))
                .field("html", &ambre_core::RedactedChars::of(html))
                .field("payload", payload)
                .finish(),
            Self::MergeConflictDetected { cell_id } => fields!("MergeConflictDetected", cell_id),
            Self::CellEntered { cell, block } => fields!("CellEntered", cell, block),
            Self::CaretPlaced { block, offset } => fields!("CaretPlaced", block, offset),
            Self::FootnotesUpdated { notes } => fields!("FootnotesUpdated", notes),
            Self::CommentsUpdated { threads } => fields!("CommentsUpdated", threads),
            Self::SuggestionsUpdated { mode, suggestions } => {
                fields!("SuggestionsUpdated", mode, suggestions)
            }
            Self::LinksUpdated { links } => fields!("LinksUpdated", links),
            Self::CrossRefsUpdated { refs } => fields!("CrossRefsUpdated", refs),
            Self::RevealBlock { block, page } => fields!("RevealBlock", block, page),
            Self::BandsUpdated { bands } => fields!("BandsUpdated", bands),
            Self::FindResults { matches, current } => fields!("FindResults", matches, current),
            Self::FindUpdated { current } => fields!("FindUpdated", current),
            Self::UndoState { undo, redo } => fields!("UndoState", undo, redo),
            Self::Error(error) => f.debug_tuple("Error").field(error).finish(),
        }
    }
}

// =============================================================================
// Errors
// =============================================================================

/// The words a refusal uses for a command (`PROSE-105`). Only
/// [`ProseCommand::name`] and the constants below build one, so a refusal
/// names only a command that exists. A test that reads it compares two names,
/// never two strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandName(&'static str);

impl CommandName {
    /// The name of [`ProseCommand::ReplyToComment`], for a reader that must
    /// recognise that refusal without a command to ask.
    pub const REPLY_TO_COMMENT: Self = Self("replying to the comment");

    /// The words a refusal prints for this command.
    #[must_use]
    pub fn label(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for CommandName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// What the engine turned down, as a refusal names it (`PROSE-105`).
///
/// A command names itself through [`ProseCommand::name`]. The other variants
/// name what the engine refused when no command of the author was the cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusedAction {
    /// A refusal built deep in a handler, before the entry point names the
    /// command it came from. It reaches a screen only if a handler refuses
    /// where no command drives the engine, and no such path exists today.
    Unnamed,
    /// A command, under the name [`ProseCommand::name`] gives it.
    Command(CommandName),
    /// Growing a block back into a document that a command left empty.
    KeepingBlock,
    /// A collaborator's update. It enters the engine the way a command does
    /// and fails on the same terms.
    RemoteUpdate,
    /// Writing the document out to a file.
    Exporting,
}

impl RefusedAction {
    /// The words a refusal prints for what it refused.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Unnamed => "that",
            Self::Command(name) => name.label(),
            Self::KeepingBlock => "keeping a block",
            Self::RemoteUpdate => "a collaborator's update",
            Self::Exporting => "exporting",
        }
    }
}

impl fmt::Display for RefusedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One refusal, as the author is told about it.
///
/// A number and a message named neither the command nor a reason, so a view
/// could only print it. Both halves here are readable by code. The view picks
/// where to put the message from `refused`, and the message itself from
/// `reason` (`PROSE-105`, `PROSE-106`).
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{refused} refused: {reason}")]
pub struct ProseError {
    /// What the engine turned down.
    pub refused: RefusedAction,
    /// Why.
    pub reason: ProseRefusal,
}

impl ProseError {
    #[must_use]
    pub fn new(refused: RefusedAction, reason: ProseRefusal) -> Self {
        Self { refused, reason }
    }
}

/// A table value the format cannot hold, as a table command asked for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TableValue {
    /// A table of no row or no column (`PROSE-FEAT-228`).
    Size { rows: RowCount, cols: ColCount },
    /// A column width.
    Width(Pt),
    /// A cell border's width.
    Border(Pt),
    /// A cell's padding.
    Padding(Pt),
}

impl fmt::Display for TableValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size { rows, cols } => write!(f, "{} by {}", rows.get(), cols.get()),
            Self::Width(width) => write!(f, "a width of {} pt", width.get()),
            Self::Border(width) => write!(f, "a border of {} pt", width.get()),
            Self::Padding(padding) => write!(f, "a padding of {} pt", padding.get()),
        }
    }
}

/// Why the engine refused.
///
/// One variant per reason the engine can state, never one per call site. Two
/// handlers refusing a range out of bounds carry the same variant. They differ
/// by the command they name.
///
/// `Debug` prints a refused mark value by its length: the author typed it.
/// `Display` keeps it, because the author reads the refusal.
#[derive(Clone, PartialEq, thiserror::Error)]
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
    UnknownMark(MarkKey),
    /// A style command aimed at an anchor key, which only its own command
    /// writes (`PROSE-FEAT-298`, `PROSE-FEAT-299`).
    #[error("{0} is not a style, and only its own command writes it")]
    AnchorKey(MarkKey),
    /// A mark value the key's own grammar refuses (`PROSE-008`).
    #[error("{value} is not a value the {key} style takes")]
    BadMarkValue { key: MarkKey, value: String },
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
    /// A table dimension or a width the format cannot hold.
    #[error("{0} is not a value a table takes")]
    BadTableValue(TableValue),
    /// A justified header or footer: a band is aligned left, centred or
    /// right, and nothing else (`PROSE-FEAT-171`).
    #[error("a header or a footer cannot be justified")]
    BandJustified,
    /// A field aimed at the first page's own block while the setting is off
    /// (`PROSE-FEAT-067`).
    #[error("the first page has no band of its own while the setting is off")]
    FirstPageBandOff,
    /// A search or a replacement with no search open, or a query this build
    /// cannot run.
    #[error("{0}")]
    FindRefused(FindRefusal),
    /// A footnote asked for in a header or a footer (`PROSE-FEAT-071`).
    #[error("a header or a footer cannot hold a footnote")]
    FootnoteInBand,
    /// A footnote asked for in a note's own text (`PROSE-FEAT-306`).
    #[error("a footnote cannot hold a footnote")]
    FootnoteInNote,
    /// The command names a footnote the document does not hold.
    #[error("footnote {0} is not in the document")]
    NoSuchFootnote(FootnoteId),
    /// A comment asked for in a header or a footer, drawn on every page.
    #[error("a header or a footer cannot hold a comment")]
    CommentInBand,
    /// A comment asked for in a note's text, which the comment panel does not
    /// place (`PROSE-FEAT-307`).
    #[error("a footnote cannot hold a comment")]
    CommentInNote,
    /// The command names a comment thread the document does not hold.
    #[error("comment {0} is not in the document")]
    NoSuchComment(CommentId),
    /// The command names a reply the thread does not hold.
    #[error("reply {0} is not in the thread")]
    NoSuchReply(ReplyId),
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
    /// A paragraph length or line height that is not a finite number. The
    /// store never receives one.
    #[error("a paragraph length must be a finite number")]
    NonFiniteLength,
    /// A write the CRDT engine itself refused.
    #[error("the document engine could not write the change: {0}")]
    WriteFailed(String),
    /// A file that produced nothing (`PROSE-IO-047`).
    #[error(transparent)]
    Interop(#[from] InteropError),
}

impl fmt::Debug for ProseRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        macro_rules! tuple {
            ($name:literal, $value:expr) => {
                f.debug_tuple($name).field($value).finish()
            };
        }
        match self {
            Self::NoSuchBlock(block) => tuple!("NoSuchBlock", block),
            Self::BadRange => f.write_str("BadRange"),
            Self::NestingRefused => f.write_str("NestingRefused"),
            Self::UnknownMark(key) => tuple!("UnknownMark", key),
            Self::AnchorKey(key) => tuple!("AnchorKey", key),
            Self::BadMarkValue { key, value } => f
                .debug_struct("BadMarkValue")
                .field("key", key)
                .field("value", &ambre_core::RedactedChars::of(value))
                .finish(),
            Self::RemoteImportFailed => f.write_str("RemoteImportFailed"),
            Self::OutsideTheGrid => f.write_str("OutsideTheGrid"),
            Self::MergeNotRectangular => f.write_str("MergeNotRectangular"),
            Self::MergeOverText => f.write_str("MergeOverText"),
            Self::BadTableValue(value) => tuple!("BadTableValue", value),
            Self::BandJustified => f.write_str("BandJustified"),
            Self::FirstPageBandOff => f.write_str("FirstPageBandOff"),
            Self::FindRefused(refusal) => tuple!("FindRefused", refusal),
            Self::FootnoteInBand => f.write_str("FootnoteInBand"),
            Self::FootnoteInNote => f.write_str("FootnoteInNote"),
            Self::NoSuchFootnote(footnote) => tuple!("NoSuchFootnote", footnote),
            Self::CommentInBand => f.write_str("CommentInBand"),
            Self::CommentInNote => f.write_str("CommentInNote"),
            Self::NoSuchComment(comment) => tuple!("NoSuchComment", comment),
            Self::NoSuchReply(reply) => tuple!("NoSuchReply", reply),
            Self::NotYourReply => f.write_str("NotYourReply"),
            Self::NoIdentity => f.write_str("NoIdentity"),
            Self::NotWhileSuggesting => f.write_str("NotWhileSuggesting"),
            Self::NoCrossRef => f.write_str("NoCrossRef"),
            Self::NonFiniteLength => f.write_str("NonFiniteLength"),
            Self::WriteFailed(why) => tuple!("WriteFailed", why),
            Self::Interop(error) => tuple!("Interop", error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A style name and a mark value print their length alone in `Debug`,
    /// and none of the text a user may have typed.
    #[test]
    fn a_style_name_and_a_mark_value_print_their_length_alone() {
        let style = StyleId("My notes".to_owned());
        assert_eq!(format!("{style:?}"), "StyleId(8 chars)");
        let link = mark::MarkRoot::Link.key();
        let value = mark::validate_value(&link, Some("https://a.example")).unwrap();
        assert_eq!(format!("{value:?}"), "MarkValue(17 chars)");
        let refused = ProseRefusal::BadMarkValue {
            key: link,
            value: "Q3 layoffs".to_owned(),
        };
        let printed = format!("{refused:?}");
        assert!(printed.contains("10 chars"), "{printed}");
        assert!(!printed.contains("Q3 layoffs"), "{printed}");
        assert!(refused.to_string().contains("Q3 layoffs"));
    }

    /// The reason of a parse failure may quote the foreign file, so `Debug`
    /// prints its length alone.
    #[test]
    fn a_parse_failure_prints_the_length_of_its_reason_alone() {
        let failed = InteropError::ParseFailed("near 'Q3 layoffs'".to_owned());
        assert_eq!(format!("{failed:?}"), "ParseFailed(17 chars)");
        let large = InteropError::TooLarge(ambre_core::ByteSize(9));
        assert_eq!(
            format!("{large:?}"),
            format!("TooLarge({:?})", ambre_core::ByteSize(9))
        );
    }

    /// A heading number reads as its counters joined by dots, and there is
    /// none without a counter.
    #[test]
    fn a_heading_number_reads_dotted_and_is_never_empty() {
        let number = HeadingNumber::new(vec![2, 1, 3]).expect("three counters");
        assert_eq!(number.to_string(), "2.1.3");
        assert_eq!(number.parts(), [2, 1, 3]);
        assert_eq!(HeadingNumber::new(vec![]), None);
    }

    /// Each refused action prints the words a refusal printed before it was
    /// typed (`PROSE-105`).
    #[test]
    fn each_refused_action_keeps_its_label() {
        for (action, label) in [
            (RefusedAction::Unnamed, "that"),
            (RefusedAction::Command(ProseCommand::Undo.name()), "undoing"),
            (RefusedAction::KeepingBlock, "keeping a block"),
            (RefusedAction::RemoteUpdate, "a collaborator's update"),
            (RefusedAction::Exporting, "exporting"),
        ] {
            assert_eq!(action.label(), label);
            let error = ProseError::new(action, ProseRefusal::NestingRefused);
            assert_eq!(
                error.to_string(),
                format!("{label} refused: {}", ProseRefusal::NestingRefused)
            );
        }
    }

    #[test]
    fn a_command_name_prints_the_words_it_was_given() {
        for (command, label) in [
            (ProseCommand::Undo, "undoing"),
            (ProseCommand::Redo, "redoing"),
            (ProseCommand::CountWords, "counting the words"),
            (
                ProseCommand::AcceptAllSuggestions,
                "accepting every suggestion",
            ),
        ] {
            let name = command.name();
            assert_eq!(name.label(), label);
            assert_eq!(name.to_string(), label);
            assert_eq!(RefusedAction::Command(name).to_string(), label);
        }
        assert_ne!(ProseCommand::Undo.name(), ProseCommand::Redo.name());
    }

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

    /// Every command, event and listed record that holds a text the author
    /// wrote prints its length in `Debug`, and none of the text.
    #[test]
    fn an_authored_text_prints_its_length_alone() {
        let secret = "Q3 layoffs".to_owned();
        let block_id = BlockId::new(ambre_core::PeerId(1), ambre_core::OpCounter(0));
        let pos = DocOffset::ZERO;
        let now = ambre_core::Timestamp::default();
        let comment_id = CommentId::new();
        let reply_id = ReplyId::new();
        let footnote_id = FootnoteId::new();
        let printed = [
            format!(
                "{:?}",
                ProseCommand::InsertText {
                    block_id,
                    pos,
                    text: secret.clone(),
                }
            ),
            format!(
                "{:?}",
                ProseCommand::Paste {
                    at: (block_id, pos),
                    text: secret.clone(),
                    html: Some(secret.clone()),
                    payload: None,
                }
            ),
            format!(
                "{:?}",
                ProseCommand::ReplaceOne {
                    replacement: secret.clone(),
                }
            ),
            format!(
                "{:?}",
                ProseCommand::ReplaceAll {
                    replacement: secret.clone(),
                }
            ),
            format!(
                "{:?}",
                ProseCommand::InsertFootnote {
                    block_id,
                    pos,
                    note_text: secret.clone(),
                }
            ),
            format!(
                "{:?}",
                ProseCommand::EditFootnote {
                    footnote_id,
                    new_text: secret.clone(),
                }
            ),
            format!(
                "{:?}",
                ProseCommand::AddComment {
                    block_id,
                    range: DocSpan::new(pos, pos),
                    text: secret.clone(),
                    now,
                }
            ),
            format!(
                "{:?}",
                ProseCommand::ReplyToComment {
                    comment_id,
                    text: secret.clone(),
                    now,
                }
            ),
            format!(
                "{:?}",
                ProseCommand::EditCommentReply {
                    comment_id,
                    reply_id,
                    new_text: secret.clone(),
                }
            ),
            format!(
                "{:?}",
                ProseCommand::InsertCrossRef {
                    block_id,
                    pos,
                    target_block_id: block_id,
                    kind: CrossRefKind::CustomText(secret.clone()),
                }
            ),
            format!(
                "{:?}",
                ProseEvent::Clipboard {
                    text: secret.clone(),
                    html: secret.clone(),
                    payload: ambre_core::ClipboardBytes::default(),
                }
            ),
            format!(
                "{:?}",
                FootnoteEntry {
                    footnote_id,
                    ordinal: None,
                    text: secret.clone(),
                }
            ),
            format!(
                "{:?}",
                CommentReply {
                    reply_id,
                    author: None,
                    user_id: UserId::default(),
                    created_at: now,
                    text: secret.clone(),
                }
            ),
            format!(
                "{:?}",
                TocEntry {
                    block_id,
                    level: HeadingLevel::new(1).expect("a level"),
                    text: secret.clone(),
                    heading_number: None,
                    page: None,
                }
            ),
            format!(
                "{:?}",
                ResolvedCrossRef {
                    block_id,
                    offset: pos,
                    target: block_id,
                    shown: Some(secret.clone()),
                }
            ),
        ];
        for line in printed {
            assert!(line.contains("10 chars"), "{line}");
            assert!(!line.contains("Q3 layoffs"), "{line}");
        }
    }

    #[test]
    fn presence_is_not_a_mutation() {
        let pos = CursorPosition {
            block_id: BlockId::new(ambre_core::PeerId(1), ambre_core::OpCounter(0)),
            offset: DocOffset::ZERO,
            anchor: None,
        };
        assert!(!ProseCommand::UpdateCursor { pos }.is_mutating());
        assert!(!ProseCommand::Noop.is_mutating());
        assert!(
            ProseCommand::SplitBlock {
                block_id: BlockId::new(ambre_core::PeerId(1), ambre_core::OpCounter(0)),
                pos: DocOffset::ZERO
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
    pub at: DocOffset,
    /// The code points inserted, positive, or deleted, negative.
    pub delta: DocDelta,
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
