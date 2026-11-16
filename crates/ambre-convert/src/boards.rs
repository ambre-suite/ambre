//! The board the boards module answers for a Figma or a Miro export
//! (`3-Toile/features/ambre-spec-toile-import-export.md section 3`).
//!
//! Neither tool exports a documented board file. Each documents the JSON of
//! its public REST interface, which a user saves to a file with a personal
//! token, outside Ambre:
//!
//! | Format | The file Ambre reads |
//! | :--- | :--- |
//! | Figma | the answer of `GET /v1/files/:key`: a `document` node and its tree |
//! | Miro | the answer of `GET /v2/boards/:id/items`: a `data` list of items, and the answer of the connectors endpoint in the same list or beside it as `connectors` |
//!
//! The reader recognises the format from the top-level keys, so one drop
//! gate serves both. The board is format-level and flat. Toile maps it onto
//! its elements in the Main instance, with its own limits, clamps and report.
//! A node names its children by position, so postcard never recurses on the
//! Logic thread.

use ambre_core::index::{Axis, IdxVec, Pos};
use ambre_core::{Board, Bounded, FontSize, Opacity, Payload, Px, Radians, Rgb, Validated};
use serde::{Deserialize, Serialize};

/// The role of a board as it crosses back from the boards module: postcard
/// bytes. `Payload<BoardBytes>`.
pub enum BoardBytes {}

impl ambre_core::PayloadKind for BoardBytes {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("BoardBytes");
}

/// The bytes of `board`.
///
/// # Errors
///
/// [`crate::ConvertError::Refused`] when postcard cannot write the board.
pub fn encode_board(board: &BoardTree) -> Result<Payload<BoardBytes>, crate::ConvertError> {
    postcard::to_allocvec(board)
        .map(Payload::new)
        .map_err(|_| crate::ConvertError::Refused(ambre_core::FileRefusal::Unwritable))
}

/// The board `bytes` hold.
///
/// # Errors
///
/// [`crate::ConvertError::Malformed`] for bytes that decode to no board.
pub fn decode_board(bytes: &Payload<BoardBytes>) -> Result<BoardTree, crate::ConvertError> {
    postcard::from_bytes(bytes.as_bytes()).map_err(|_| crate::ConvertError::Malformed)
}

/// The places of a board: a [`Pos<BoardNode>`] names a node. Only the boards
/// module hands one out.
#[derive(PartialEq, Eq)]
pub enum BoardNode {}

impl Axis for BoardNode {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("BoardNode");
}

/// The tool a board file came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardFormat {
    Figma,
    Miro,
}

/// Whether every node of the file sat within the module's depth bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardDepth {
    Within,
    Exceeded,
}

/// The board of one export. Node 0 is the root, which draws nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardTree {
    nodes: IdxVec<BoardNode, BoardItem>,
    /// The connectors, each end resolved to a node or to nothing.
    pub links: Vec<BoardLink>,
    /// Each construct the reader met and could not put in the tree, once per
    /// occurrence.
    pub losses: Vec<BoardLoss>,
    pub format: BoardFormat,
    pub depth: BoardDepth,
}

impl BoardTree {
    /// The root, the board itself.
    pub const ROOT: Pos<BoardNode> = Pos::ZERO;

    /// The board `nodes` hold, the root at position 0.
    #[must_use]
    pub const fn new(
        nodes: IdxVec<BoardNode, BoardItem>,
        links: Vec<BoardLink>,
        losses: Vec<BoardLoss>,
        format: BoardFormat,
        depth: BoardDepth,
    ) -> Self {
        Self {
            nodes,
            links,
            losses,
            format,
            depth,
        }
    }

    /// The node at `pos`, or `None` past the end of the arena.
    #[must_use]
    pub fn get(&self, pos: Pos<BoardNode>) -> Option<&BoardItem> {
        self.nodes.get(pos)
    }
}

/// One node of the board and its children, in paint order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardItem {
    pub children: Vec<Pos<BoardNode>>,
    pub kind: BoardKind,
    /// The box of the node before its rotation, in absolute board units.
    pub bounds: BoardRect,
    /// The rotation about the centre of the box.
    pub rotation: Radians,
    pub fill: BoardPaint,
    pub stroke: Option<BoardStroke>,
    /// The words the node carries: a text, a sticky note, or a shape with a
    /// label.
    pub text: Option<BoardText>,
}

impl BoardItem {
    /// A node of `kind` at `bounds`, upright, with no paint and no text.
    #[must_use]
    pub const fn new(kind: BoardKind, bounds: BoardRect) -> Self {
        Self {
            children: Vec::new(),
            kind,
            bounds,
            rotation: Radians::ZERO,
            fill: BoardPaint::None,
            stroke: None,
            text: None,
        }
    }
}

/// What a node is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BoardKind {
    /// The root of the tree.
    Root,
    /// A Figma page. Each page holds its own plane of coordinates.
    Page,
    /// A frame, with its title.
    Frame(Validated<FrameTitleText>),
    Group,
    Shape(BoardShape),
    Text,
    Sticky,
}

/// The title of a frame, as the file spells it: a
/// `Validated<FrameTitleText>`. No rule constrains it, and a log never
/// prints it.
pub enum FrameTitleText {}

impl ambre_core::Free for FrameTitleText {}

/// The words of a node, as plain text: a `Validated<BoardWords>`. No rule
/// constrains them, and a log never prints them.
pub enum BoardWords {}

impl ambre_core::Free for BoardWords {}

/// The outline of a shape, as the two tools name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardShape {
    Rect,
    RoundRect,
    Ellipse,
    Triangle,
    Line,
    Diamond,
    /// An outline the board has no counterpart for: a cloud, a callout, a
    /// flowchart symbol.
    Other,
}

/// A rectangle in absolute board units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoardRect {
    pub left: Px<Board>,
    pub top: Px<Board>,
    pub width: Px<Board>,
    pub height: Px<Board>,
}

/// How a fill or a stroke paints.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BoardPaint {
    None,
    /// One colour, at an opacity.
    Solid {
        color: Rgb,
        opacity: Bounded<Opacity>,
    },
    /// A gradient, a picture fill or a pattern.
    Other,
}

/// The stroke of a node: its paint and its width.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoardStroke {
    pub paint: BoardPaint,
    pub width: Px<Board>,
}

/// The words of a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardText {
    /// The plain text, one line per paragraph.
    pub body: Validated<BoardWords>,
    /// The size the file names, or `None` when it names none.
    pub size: Option<FontSize>,
    /// The ink the file names, or `None` when it names none.
    pub color: Option<Rgb>,
    pub styling: TextStyling,
}

/// Whether a text styles its runs apart: a bold word, a link, a second size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextStyling {
    Uniform,
    Mixed,
}

/// A connector between two nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardLink {
    /// The node each end attaches to, or `None` for an end the reader could
    /// not resolve: a free point, or a node it dropped.
    pub from: Option<Pos<BoardNode>>,
    pub to: Option<Pos<BoardNode>>,
    pub line: BoardLine,
    pub start: LinkCap,
    pub end: LinkCap,
    pub caption: LinkCaption,
}

/// How a connector routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardLine {
    Straight,
    Elbow,
    Curved,
}

/// What one end of a connector draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkCap {
    Plain,
    Arrow,
}

/// Whether a connector carries words on its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkCaption {
    Absent,
    Present,
}

/// A construct the reader met and could not put in the tree, or put there
/// changed. Toile names each one in its report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardLoss {
    /// A picture: its pixels sit behind an address the reader never fetches.
    Picture,
    /// A vector network or a boolean operation, whose outline the file holds
    /// only on request.
    VectorPath,
    /// A web embed, a link preview, a document or an app card.
    Embed,
    /// A Miro card.
    Card,
    /// A table.
    Table,
    /// A Figma widget or stamp.
    Widget,
    /// A node of a type the reader does not know.
    UnknownNode,
    /// A shadow or a blur.
    Effect,
    /// A Figma rotation: the node lands upright in its bounding box.
    Rotation,
    /// A Figma component instance, which lands as a plain frame.
    ComponentLink,
}
