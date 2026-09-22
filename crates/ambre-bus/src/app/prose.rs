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

use std::fmt;

use serde::{Deserialize, Serialize};

use ambre_core::UserId;

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
    pub fn new(peer: u64, counter: i32) -> Self {
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
// Commands -- the content plane, UI thread -> Logic thread
// =============================================================================

/// One thing to do to a Prose document.
///
/// Values move on `flume` between the two threads, which share one linear
/// memory, so no shape here derives `serde`. Every offset is a count of Unicode
/// code points into the body of the block the command names (`PROSE-045`).
///
/// The variants are the ones the engine core answers. Tables, comments,
/// footnotes, find, interop and the clipboard reach this enum with the Story
/// that builds each of them (`ambre-roadmap.md` E5).
#[derive(Clone, Debug, PartialEq)]
pub enum ProseCommand {
    /// Changes nothing, and is the one command a test drives the empty outcome
    /// with (`PROSE-047`).
    Noop,

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
    SetHeadingLevel {
        block_id: BlockId,
        level: u8,
    },
    SetCodeLang {
        block_id: BlockId,
        lang: String,
    },
    /// The command is coarse and the storage is per property: the handler
    /// writes only what the caller named (`PROSE-030`, `PROSE-092`).
    SetParagraphStyle {
        block_id: BlockId,
        style: ParagraphStyle,
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
}

impl ProseCommand {
    /// Whether this command can change the document's shared CRDT state.
    ///
    /// Used by the Logic dispatcher to gate a read-only caller
    /// (`ambre-spec-architecture.md section 2`). Presence is *not* mutating: a
    /// viewer still shows a cursor.
    #[must_use]
    pub fn is_mutating(&self) -> bool {
        !matches!(self, Self::Noop | Self::UpdateCursor { .. })
    }

    /// What a refusal calls this command when it tells the author
    /// (`PROSE-105`). The match is exhaustive, so a command added to the enum
    /// does not compile until it has a name a reader understands.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Noop => "nothing",
            Self::InsertText { .. } => "typing",
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
            Self::SetCodeLang { .. } => "setting the code language",
            Self::SetParagraphStyle { .. } => "setting the paragraph style",
            Self::UpdateCursor { .. } => "moving the cursor",
            Self::Undo => "undoing",
            Self::Redo => "redoing",
        }
    }
}

// =============================================================================
// Events -- the content plane, Logic thread -> UI thread
// =============================================================================

/// One thing the view is told. A second enum would put a document fact in two
/// places and leave the dispatch with a hole (`PROSE-UI-001`).
///
/// The layout, the shaped runs, the table of contents and the word count reach
/// this enum later. So do the find results and the interop reports. Each arrives
/// with the Story that builds it (`ambre-roadmap.md` E5).
#[derive(Clone, Debug, PartialEq)]
pub enum ProseEvent {
    /// The confirmed change, local or imported. The bytes are the ones applied,
    /// never a re-export (`PROSE-023`, `PROSE-048`).
    DocUpdated { delta: Vec<u8> },
    /// The paragraph that parted, and the one that carries its tail. The view
    /// needs the new block's identity to leave the caret at its start
    /// (`PROSE-UI-141`): it cannot mint the id and cannot read the document.
    BlockSplit { source: BlockId, new_block: BlockId },
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
    /// A mark value the key's own grammar refuses (`PROSE-008`).
    #[error("{value} is not a value the {key} style takes")]
    BadMarkValue { key: String, value: String },
    /// A collaborator's change this peer could not apply.
    #[error("a collaborator's change could not be applied; the document may be behind")]
    RemoteImportFailed,
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
