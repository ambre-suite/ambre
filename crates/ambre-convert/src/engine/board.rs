//! The board reader of the boards module: a Figma or a Miro export to its
//! board (`crate::boards`).
//!
//! Each export is the JSON of a documented REST answer, read here into the
//! tree of the module's own JSON reader (`super::json`). The two tools write many optional keys. A key the
//! reader does not know is no error: the reader takes what it maps, and names
//! the rest in the board's losses. The JSON reader caps its nesting at 128,
//! so a hostile file never grows the module's stack through the parser.

use super::json::{self, Json};
use ambre_client_core::FileBytes;
use ambre_core::hash::HashMap;
use ambre_core::index::{IdxVec, Pos};
use ambre_core::{
    Board, Bounded, FileRefusal, FontSize, Opacity, Payload, Px, Radians, Rgb, Validated,
};

use crate::boards::{
    BoardDepth, BoardFormat, BoardItem, BoardKind, BoardLine, BoardLink, BoardLoss, BoardNode,
    BoardPaint, BoardRect, BoardShape, BoardStroke, BoardText, BoardTree, FrameTitleText, LinkCap,
    LinkCaption, TextStyling,
};

/// The deepest node the module walks. Toile refuses a board far shallower
/// than this (`Limits::max_depth`), so the bound only keeps a hostile file
/// from looping through a cycle of Miro parents.
const MAX_DEPTH: usize = 256;

/// The board of the export `data`, Figma or Miro, told apart by its keys.
///
/// # Errors
///
/// [`FileRefusal::Malformed`] when the bytes are no JSON, and
/// [`FileRefusal::OtherDocument`] for JSON that is neither export.
pub(super) fn read(data: &Payload<FileBytes>) -> Result<BoardTree, FileRefusal> {
    let root = json::parse(data).map_err(|refusal| match refusal {
        json::JsonRefusal::Malformed => FileRefusal::Malformed,
        json::JsonRefusal::TooDeep => FileRefusal::PastBound,
    })?;
    if let Some(document) = root.get("document").filter(|d| d.get("type").is_some()) {
        return Ok(figma(document));
    }
    if root.get("data").is_some_and(Json::is_array) {
        return Ok(miro(&root));
    }
    Err(FileRefusal::OtherDocument)
}

/// The arena under construction, shared by the two readers.
struct Build {
    nodes: IdxVec<BoardNode, BoardItem>,
    losses: Vec<BoardLoss>,
    depth: BoardDepth,
}

impl Build {
    fn new() -> Self {
        let mut nodes = IdxVec::new();
        nodes.push(BoardItem::new(BoardKind::Root, rect(None)));
        Self {
            nodes,
            losses: Vec::new(),
            depth: BoardDepth::Within,
        }
    }

    /// Append `item` under `parent`, and return its position.
    fn push(&mut self, parent: Pos<BoardNode>, item: BoardItem) -> Pos<BoardNode> {
        let at = self.nodes.push(item);
        if let Some(parent) = self.nodes.get_mut(parent) {
            parent.children.push(at);
        }
        at
    }

    fn finish(self, links: Vec<BoardLink>, format: BoardFormat) -> BoardTree {
        BoardTree::new(self.nodes, links, self.losses, format, self.depth)
    }
}

// -----------------------------------------------------------------------------
// Figma
// -----------------------------------------------------------------------------

/// The board of a Figma file answer, from its `document` node.
fn figma(document: &Json<'_>) -> BoardTree {
    let mut build = Build::new();
    let mut walk = FigmaWalk {
        build: &mut build,
        ids: HashMap::default(),
        connectors: Vec::new(),
    };
    walk.children(document, BoardTree::ROOT, 0);
    let FigmaWalk {
        ids, connectors, ..
    } = walk;
    let links = connectors.iter().map(|c| figma_link(c, &ids)).collect();
    build.finish(links, BoardFormat::Figma)
}

struct FigmaWalk<'b, 'v> {
    build: &'b mut Build,
    /// The node each Figma id landed as, for the ends of a connector.
    ids: HashMap<&'v str, Pos<BoardNode>>,
    connectors: Vec<&'v Json<'v>>,
}

impl<'v> FigmaWalk<'_, 'v> {
    /// Walk the `children` of `node` under `at`. `depth` counts the nodes
    /// above.
    fn children(&mut self, node: &'v Json<'v>, at: Pos<BoardNode>, depth: usize) {
        if depth > MAX_DEPTH {
            self.build.depth = BoardDepth::Exceeded;
            return;
        }
        for child in node
            .get("children")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
        {
            self.node(child, at, depth);
        }
    }

    /// Map one node under `parent`. A hidden node shows nothing in Figma,
    /// and lands as nothing.
    fn node(&mut self, node: &'v Json<'v>, parent: Pos<BoardNode>, depth: usize) {
        if node.get("visible").and_then(Json::as_bool) == Some(false) {
            return;
        }
        let bounds = rect(node.get("absoluteBoundingBox"));
        let kind = match node.get("type").and_then(Json::as_str).unwrap_or_default() {
            "CANVAS" => {
                let at = self
                    .build
                    .push(parent, BoardItem::new(BoardKind::Page, bounds));
                self.children(node, at, depth + 1);
                return;
            }
            "FRAME" | "SECTION" | "COMPONENT" | "COMPONENT_SET" => BoardKind::Frame(title(node)),
            "INSTANCE" => {
                self.build.losses.push(BoardLoss::ComponentLink);
                BoardKind::Frame(title(node))
            }
            "GROUP" => BoardKind::Group,
            "RECTANGLE" => {
                if node
                    .get("cornerRadius")
                    .and_then(Json::as_f64)
                    .unwrap_or(0.0)
                    > 0.0
                {
                    BoardKind::Shape(BoardShape::RoundRect)
                } else {
                    BoardKind::Shape(BoardShape::Rect)
                }
            }
            "ELLIPSE" => BoardKind::Shape(BoardShape::Ellipse),
            "LINE" => BoardKind::Shape(BoardShape::Line),
            "REGULAR_POLYGON" | "STAR" => BoardKind::Shape(BoardShape::Other),
            "SHAPE_WITH_TEXT" => BoardKind::Shape(figma_shape(node.get("shapeType"))),
            "TEXT" => BoardKind::Text,
            "STICKY" => BoardKind::Sticky,
            "CONNECTOR" => {
                self.connectors.push(node);
                return;
            }
            "SLICE" => return,
            other => {
                self.build.losses.push(match other {
                    "VECTOR" | "BOOLEAN_OPERATION" | "WASHI_TAPE" | "HIGHLIGHT" => {
                        BoardLoss::VectorPath
                    }
                    "TABLE" => BoardLoss::Table,
                    "EMBED" | "LINK_UNFURL" => BoardLoss::Embed,
                    "WIDGET" | "STAMP" => BoardLoss::Widget,
                    _ => BoardLoss::UnknownNode,
                });
                return;
            }
        };
        let item = self.figma_item(node, kind, bounds);
        let at = self.build.push(parent, item);
        if let Some(id) = node.get("id").and_then(Json::as_str) {
            self.ids.insert(id, at);
        }
        self.children(node, at, depth + 1);
    }

    /// The paint, the words and the losses of a mapped node.
    fn figma_item(&mut self, node: &Json<'_>, kind: BoardKind, bounds: BoardRect) -> BoardItem {
        let fade = opacity(node.get("opacity"));
        let fill = self.figma_paint(node.get("fills"), fade);
        let is_text = kind == BoardKind::Text;
        let mut item = BoardItem::new(kind, bounds);
        if node
            .get("rotation")
            .and_then(Json::as_f64)
            .is_some_and(|r| r != 0.0)
        {
            self.build.losses.push(BoardLoss::Rotation);
        }
        let effects = node.get("effects").and_then(Json::as_array);
        if effects.is_some_and(|e| {
            e.iter()
                .any(|e| e.get("visible").and_then(Json::as_bool) != Some(false))
        }) {
            self.build.losses.push(BoardLoss::Effect);
        }
        let weight = length(node.get("strokeWeight")).unwrap_or(Px::new(1.0));
        item.stroke = match self.figma_paint(node.get("strokes"), fade) {
            BoardPaint::None => None,
            paint => Some(BoardStroke {
                paint,
                width: weight,
            }),
        };
        if is_text {
            // A text's fill is its ink, never a box behind it.
            let color = match fill {
                BoardPaint::Solid { color, .. } => Some(color),
                _ => None,
            };
            item.text = figma_text(node, color);
        } else {
            item.fill = fill;
            item.text = figma_text(node, None);
        }
        item
    }

    /// The first visible paint of a `fills` or `strokes` list. A picture
    /// fill names an image the file does not carry, so it is a loss.
    fn figma_paint(&mut self, paints: Option<&Json<'_>>, fade: Bounded<Opacity>) -> BoardPaint {
        let Some(paint) = paints.and_then(Json::as_array).and_then(|list| {
            list.iter()
                .find(|p| p.get("visible").and_then(Json::as_bool) != Some(false))
        }) else {
            return BoardPaint::None;
        };
        match paint.get("type").and_then(Json::as_str) {
            Some("SOLID") => {
                let color = paint.get("color");
                let alpha = opacity(color.and_then(|c| c.get("a")));
                BoardPaint::Solid {
                    color: unit_color(color),
                    opacity: fade.times(opacity(paint.get("opacity"))).times(alpha),
                }
            }
            Some("IMAGE") => {
                self.build.losses.push(BoardLoss::Picture);
                BoardPaint::None
            }
            _ => BoardPaint::Other,
        }
    }
}

/// The name of a frame.
fn title(node: &Json<'_>) -> Validated<FrameTitleText> {
    Validated::new(node.get("name").and_then(Json::as_str).unwrap_or_default())
}

/// The words of a Figma node: its characters, its size and its runs.
/// `None` when it carries no characters, or an empty string of them.
fn figma_text(node: &Json<'_>, color: Option<Rgb>) -> Option<BoardText> {
    let body = node
        .get("characters")
        .and_then(Json::as_str)
        .filter(|body| !body.is_empty())?;
    let overrides = node.get("characterStyleOverrides").and_then(Json::as_array);
    Some(BoardText {
        body: Validated::new(body),
        size: points(node.get("style").and_then(|s| s.get("fontSize"))),
        color,
        styling: if overrides.is_some_and(|o| o.iter().any(|v| v.as_f64() != Some(0.0))) {
            TextStyling::Mixed
        } else {
            TextStyling::Uniform
        },
    })
}

/// The outline a Figma `shapeType` names.
fn figma_shape(shape: Option<&Json<'_>>) -> BoardShape {
    match shape.and_then(Json::as_str) {
        Some("SQUARE") => BoardShape::Rect,
        Some("ROUNDED_RECTANGLE") => BoardShape::RoundRect,
        Some("ELLIPSE") => BoardShape::Ellipse,
        Some("DIAMOND") => BoardShape::Diamond,
        Some("TRIANGLE_UP") => BoardShape::Triangle,
        _ => BoardShape::Other,
    }
}

/// A Figma connector, each end resolved through the nodes the walk mapped.
fn figma_link(node: &Json<'_>, ids: &HashMap<&str, Pos<BoardNode>>) -> BoardLink {
    let end = |key: &str| {
        node.get(key)
            .and_then(|e| e.get("endpointNodeId"))
            .and_then(Json::as_str)
            .and_then(|id| ids.get(id).copied())
    };
    let cap = |key: &str| match node.get(key).and_then(Json::as_str) {
        None | Some("NONE") => LinkCap::Plain,
        Some(_) => LinkCap::Arrow,
    };
    let caption = node
        .get("text")
        .and_then(|t| t.get("characters"))
        .or_else(|| node.get("characters"))
        .and_then(Json::as_str);
    BoardLink {
        from: end("connectorStart"),
        to: end("connectorEnd"),
        line: match node.get("connectorLineType").and_then(Json::as_str) {
            Some("STRAIGHT") => BoardLine::Straight,
            Some("CURVED") => BoardLine::Curved,
            _ => BoardLine::Elbow,
        },
        start: cap("connectorStartStrokeCap"),
        end: cap("connectorEndStrokeCap"),
        caption: if caption.is_some_and(|c| !c.is_empty()) {
            LinkCaption::Present
        } else {
            LinkCaption::Absent
        },
    }
}

// -----------------------------------------------------------------------------
// Miro
// -----------------------------------------------------------------------------

/// The board of a Miro items answer: the `data` list, and the connectors in
/// it or beside it.
fn miro(root: &Json<'_>) -> BoardTree {
    let mut build = Build::new();
    let items: Vec<&Json<'_>> = root
        .get("data")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .collect();
    let beside = root.get("connectors");
    let beside = beside
        .and_then(Json::as_array)
        .or_else(|| beside.and_then(|c| c.get("data")).and_then(Json::as_array));
    let mut connectors: Vec<&Json<'_>> = beside.into_iter().flatten().collect();
    let index: HashMap<&str, usize> = items
        .iter()
        .enumerate()
        .filter_map(|(rank, item)| item.get("id").and_then(Json::as_str).map(|id| (id, rank)))
        .collect();
    let parents: Vec<Option<usize>> = items
        .iter()
        .map(|item| {
            item.get("parent")
                .and_then(|p| p.get("id"))
                .and_then(Json::as_str)
                .and_then(|id| index.get(id).copied())
        })
        .collect();
    let mut corners: Vec<Option<Corner>> = vec![None; items.len()];
    // Every node first, so a child finds its frame whatever the list order.
    let mut placed: Vec<Option<Pos<BoardNode>>> = Vec::with_capacity(items.len());
    for (rank, item) in items.iter().enumerate() {
        let corner = corner_of(rank, &items, &parents, &mut corners, &mut build.depth);
        let kind = match item.get("type").and_then(Json::as_str).unwrap_or_default() {
            "frame" => Some(BoardKind::Frame(Validated::new(
                item.get("data")
                    .and_then(|d| d.get("title"))
                    .and_then(Json::as_str)
                    .map(plain_text)
                    .unwrap_or_default(),
            ))),
            "shape" => Some(BoardKind::Shape(miro_shape(
                item.get("data").and_then(|d| d.get("shape")),
            ))),
            "sticky_note" => Some(BoardKind::Sticky),
            "text" => Some(BoardKind::Text),
            "connector" => {
                connectors.push(item);
                None
            }
            other => {
                build.losses.push(match other {
                    "image" => BoardLoss::Picture,
                    "embed" | "preview" | "document" | "app_card" => BoardLoss::Embed,
                    "card" => BoardLoss::Card,
                    "table" | "data_table_format" => BoardLoss::Table,
                    _ => BoardLoss::UnknownNode,
                });
                None
            }
        };
        placed.push(kind.map(|kind| {
            let item = miro_item(item, kind, corner);
            build.nodes.push(item)
        }));
    }
    // Then the tree: a child of a frame the reader mapped joins it, any other
    // item joins the root, in list order.
    for (rank, at) in placed.iter().enumerate() {
        let Some(at) = *at else { continue };
        let parent = parents
            .get(rank)
            .copied()
            .flatten()
            .filter(|_| !in_cycle(rank, &parents))
            .and_then(|p| placed.get(p).copied().flatten())
            .filter(|p| {
                build
                    .nodes
                    .get(*p)
                    .is_some_and(|n| matches!(n.kind, BoardKind::Frame(_)))
            })
            .unwrap_or(BoardTree::ROOT);
        if let Some(node) = build.nodes.get_mut(parent) {
            node.children.push(at);
        }
    }
    let links = connectors
        .iter()
        .map(|c| miro_link(c, &index, &placed))
        .collect();
    build.finish(links, BoardFormat::Miro)
}

/// Whether the parents of the item at `rank` lead back to it, or run deeper
/// than [`MAX_DEPTH`]. Such an item joins the root, so the tree stays a tree.
fn in_cycle(rank: usize, parents: &[Option<usize>]) -> bool {
    let mut at = rank;
    for _ in 0..=MAX_DEPTH {
        match parents.get(at).copied().flatten() {
            None => return false,
            Some(parent) if parent == rank => return true,
            Some(parent) => at = parent,
        }
    }
    true
}

/// The absolute top-left corner of an item, in board units.
#[derive(Debug, Clone, Copy)]
struct Corner {
    left: Px<Board>,
    top: Px<Board>,
}

/// The box of a Miro item: its `geometry`, where an absent side reads as
/// the other one.
fn miro_size(item: &Json<'_>) -> (Px<Board>, Px<Board>) {
    let geometry = item.get("geometry");
    let width = length(geometry.and_then(|g| g.get("width")));
    let height = length(geometry.and_then(|g| g.get("height")));
    let fallback = Px::<Board>::new(100.0);
    (
        width.or(height).unwrap_or(fallback),
        height.or(width).unwrap_or(fallback),
    )
}

/// The absolute corner of the item at `rank`. A position relative to its
/// parent's corner adds that corner, read first. A chain of parents deeper
/// than [`MAX_DEPTH`], a cycle included, reads from the canvas and marks the
/// board.
fn corner_of(
    rank: usize,
    items: &[&Json<'_>],
    parents: &[Option<usize>],
    corners: &mut [Option<Corner>],
    depth: &mut BoardDepth,
) -> Corner {
    // The chain from the item up to the first ancestor already placed.
    let mut chain = vec![rank];
    let mut at = rank;
    while let Some(parent) = parents.get(at).copied().flatten() {
        if corners.get(parent).copied().flatten().is_some() {
            break;
        }
        if chain.len() > MAX_DEPTH || chain.contains(&parent) {
            *depth = BoardDepth::Exceeded;
            break;
        }
        chain.push(parent);
        at = parent;
    }
    let mut corner = Corner {
        left: Px::ZERO,
        top: Px::ZERO,
    };
    for &rank in chain.iter().rev() {
        let Some(item) = items.get(rank) else {
            continue;
        };
        let position = item.get("position");
        let relative = position
            .and_then(|p| p.get("relativeTo"))
            .and_then(Json::as_str)
            == Some("parent_top_left");
        let base = parents
            .get(rank)
            .copied()
            .flatten()
            .and_then(|p| corners.get(p).copied().flatten())
            .filter(|_| relative)
            .unwrap_or(Corner {
                left: Px::ZERO,
                top: Px::ZERO,
            });
        let x = length(position.and_then(|p| p.get("x"))).unwrap_or(Px::ZERO);
        let y = length(position.and_then(|p| p.get("y"))).unwrap_or(Px::ZERO);
        let (width, height) = miro_size(item);
        let centred = position
            .and_then(|p| p.get("origin"))
            .and_then(Json::as_str)
            != Some("top_left");
        let (dx, dy) = if centred {
            (width * 0.5, height * 0.5)
        } else {
            (Px::ZERO, Px::ZERO)
        };
        corner = Corner {
            left: base.left + x - dx,
            top: base.top + y - dy,
        };
        if let Some(slot) = corners.get_mut(rank) {
            *slot = Some(corner);
        }
    }
    corner
}

/// The paint, the words and the rotation of a mapped Miro item.
fn miro_item(item: &Json<'_>, kind: BoardKind, corner: Corner) -> BoardItem {
    let (width, height) = miro_size(item);
    let style = item.get("style");
    let field = |key: &str| style.and_then(|s| s.get(key));
    let is_sticky = kind == BoardKind::Sticky;
    let is_text = kind == BoardKind::Text;
    let mut node = BoardItem::new(
        kind,
        BoardRect {
            left: corner.left,
            top: corner.top,
            width,
            height,
        },
    );
    let degrees = item
        .get("geometry")
        .and_then(|g| g.get("rotation"))
        .and_then(Json::as_f64)
        .filter(|d| d.is_finite())
        .unwrap_or(0.0);
    node.rotation = Radians::new(degrees.to_radians());
    let fill = field("fillColor");
    node.fill = if is_sticky {
        sticky_color(fill).map_or(BoardPaint::None, |color| BoardPaint::Solid {
            color,
            opacity: Bounded::<Opacity>::OPAQUE,
        })
    } else if is_text {
        BoardPaint::None
    } else {
        hex_color(fill).map_or(BoardPaint::None, |color| BoardPaint::Solid {
            color,
            opacity: opacity(field("fillOpacity")),
        })
    };
    node.stroke = hex_color(field("borderColor")).map(|color| BoardStroke {
        paint: BoardPaint::Solid {
            color,
            opacity: opacity(field("borderOpacity")),
        },
        width: length(field("borderWidth")).unwrap_or(Px::new(1.0)),
    });
    let content = item
        .get("data")
        .and_then(|d| d.get("content"))
        .and_then(Json::as_str)
        .filter(|c| !c.is_empty());
    node.text = content.map(|html| BoardText {
        body: Validated::new(plain_text(html)),
        size: points(field("fontSize")),
        color: hex_color(field("color")),
        styling: if has_inline_markup(html) {
            TextStyling::Mixed
        } else {
            TextStyling::Uniform
        },
    });
    node
}

/// The outline a Miro `shape` names.
fn miro_shape(shape: Option<&Json<'_>>) -> BoardShape {
    match shape.and_then(Json::as_str) {
        Some("rectangle" | "square") => BoardShape::Rect,
        Some("round_rectangle") => BoardShape::RoundRect,
        Some("circle" | "ellipse") => BoardShape::Ellipse,
        Some("triangle") => BoardShape::Triangle,
        Some("rhombus") => BoardShape::Diamond,
        _ => BoardShape::Other,
    }
}

/// A Miro connector, each end resolved through the items mapped.
fn miro_link(
    node: &Json<'_>,
    index: &HashMap<&str, usize>,
    placed: &[Option<Pos<BoardNode>>],
) -> BoardLink {
    let end = |key: &str| {
        node.get(key)
            .and_then(|e| e.get("id"))
            .and_then(Json::as_str)
            .and_then(|id| index.get(id))
            .and_then(|rank| placed.get(*rank).copied().flatten())
    };
    let style = node.get("style");
    let cap =
        |key: &str, absent: LinkCap| match style.and_then(|s| s.get(key)).and_then(Json::as_str) {
            None => absent,
            Some("none") => LinkCap::Plain,
            Some(_) => LinkCap::Arrow,
        };
    let captions = node.get("captions").and_then(Json::as_array);
    BoardLink {
        from: end("startItem"),
        to: end("endItem"),
        line: match node.get("shape").and_then(Json::as_str) {
            Some("straight") => BoardLine::Straight,
            Some("elbowed") => BoardLine::Elbow,
            _ => BoardLine::Curved,
        },
        start: cap("startStrokeCap", LinkCap::Plain),
        end: cap("endStrokeCap", LinkCap::Arrow),
        caption: if captions.is_some_and(|c| !c.is_empty()) {
            LinkCaption::Present
        } else {
            LinkCaption::Absent
        },
    }
}

/// The colour a Miro sticky note names: a word of its palette, or a hex
/// colour. The words map to the colours of Miro's palette.
fn sticky_color(value: Option<&Json<'_>>) -> Option<Rgb> {
    let bits: u32 = match value.and_then(Json::as_str)? {
        "gray" => 0x00E6_E6E6,
        "light_yellow" => 0x00FF_F9B1,
        "yellow" => 0x00F5_D128,
        "orange" => 0x00FF_9D48,
        "light_green" => 0x00D5_F692,
        "green" => 0x00C9_DF56,
        "dark_green" => 0x0093_D275,
        "cyan" => 0x0067_C6C0,
        "light_pink" => 0x00FF_CEE0,
        "pink" => 0x00EA_94BB,
        "violet" => 0x00BE_88C7,
        "red" => 0x00F2_4726,
        "light_blue" => 0x00A6_CCF5,
        "blue" => 0x006C_D8FA,
        "dark_blue" => 0x009E_A9FF,
        "black" => 0x0000_0000,
        _ => return hex_color(value),
    };
    let [_, r, g, b] = bits.to_be_bytes();
    Some(Rgb::from_channels(r, g, b))
}

// -----------------------------------------------------------------------------
// Shared readers
// -----------------------------------------------------------------------------

/// A Figma `absoluteBoundingBox`, or the empty box when it is absent.
fn rect(value: Option<&Json<'_>>) -> BoardRect {
    let side = |key: &str| length(value.and_then(|v| v.get(key))).unwrap_or(Px::ZERO);
    BoardRect {
        left: side("x"),
        top: side("y"),
        width: side("width"),
        height: side("height"),
    }
}

/// A length in board units: a number, or a string that spells one, as Miro
/// writes a border width. A value that is no finite number reads as absent.
fn length(value: Option<&Json<'_>>) -> Option<Px<Board>> {
    let number = match value? {
        Json::Number(n) => Some(*n),
        Json::Text(s) => s.trim().parse().ok(),
        _ => None,
    }?;
    number.is_finite().then(|| Px::new(number))
}

/// A font size in points, read as [`length`] reads a length.
fn points(value: Option<&Json<'_>>) -> Option<FontSize> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a font size past f32 reads as the cap of FontSize"
    )]
    length(value).and_then(|px| FontSize::new(px.get() as f32))
}

/// An opacity from 0 to 1, opaque when absent, clamped into the range.
fn opacity(value: Option<&Json<'_>>) -> Bounded<Opacity> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the clamp keeps the value between 0 and 1"
    )]
    length(value).map_or(Bounded::<Opacity>::OPAQUE, |px| {
        Bounded::<Opacity>::clamped(px.get() as f32)
    })
}

/// A Figma colour, channels from 0 to 1. A missing channel reads as 0.
fn unit_color(value: Option<&Json<'_>>) -> Rgb {
    let channel = |key: &str| {
        let unit = value
            .and_then(|v| v.get(key))
            .and_then(Json::as_f64)
            .filter(|c| c.is_finite())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the clamp keeps the channel between 0 and 255"
        )]
        let byte = (unit * 255.0).round() as u8;
        byte
    };
    Rgb::from_channels(channel("r"), channel("g"), channel("b"))
}

/// A `#rrggbb` or `#rgb` colour. `transparent` and any other word read as
/// no colour.
fn hex_color(value: Option<&Json<'_>>) -> Option<Rgb> {
    let hex = value.and_then(Json::as_str)?.trim().strip_prefix('#')?;
    let digits: Vec<u8> = hex
        .chars()
        .map(|c| c.to_digit(16).and_then(|d| u8::try_from(d).ok()))
        .collect::<Option<_>>()?;
    match *digits.as_slice() {
        [r, g, b] => Some(Rgb::from_channels(r * 17, g * 17, b * 17)),
        [r1, r2, g1, g2, b1, b2] => {
            Some(Rgb::from_channels(r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2))
        }
        _ => None,
    }
}

/// The plain text of the HTML Miro writes in a content field. A paragraph,
/// a list item and a break end a line; every other tag is dropped and an
/// entity reads as its character.
fn plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(c) = rest.chars().next() {
        match c {
            '<' => {
                let end = rest.find('>').map_or(rest.len(), |e| e + 1);
                let tag = rest.get(1..end.saturating_sub(1)).unwrap_or_default();
                let name: String = tag
                    .trim_start_matches('/')
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect::<String>()
                    .to_ascii_lowercase();
                let closes = tag.starts_with('/');
                let breaks =
                    name == "br" || (closes && matches!(name.as_str(), "p" | "li" | "div"));
                if breaks && !out.is_empty() {
                    out.push('\n');
                }
                rest = rest.get(end..).unwrap_or_default();
            }
            '&' => {
                let end = rest.find(';').filter(|e| *e <= 8);
                let decoded = end
                    .and_then(|e| rest.get(1..e))
                    .and_then(|name| match name {
                        "amp" => Some('&'),
                        "lt" => Some('<'),
                        "gt" => Some('>'),
                        "quot" => Some('"'),
                        "apos" | "#39" => Some('\''),
                        "nbsp" => Some(' '),
                        _ => None,
                    });
                match (decoded, end) {
                    (Some(ch), Some(e)) => {
                        out.push(ch);
                        rest = rest.get(e + 1..).unwrap_or_default();
                    }
                    _ => {
                        out.push('&');
                        rest = rest.get(1..).unwrap_or_default();
                    }
                }
            }
            _ => {
                out.push(c);
                rest = rest.get(c.len_utf8()..).unwrap_or_default();
            }
        }
    }
    out.trim_end_matches('\n').to_owned()
}

/// Whether the HTML styles a run apart from the rest. A bold word, an italic
/// word, a link, an underline, a strike and a span each count.
fn has_inline_markup(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    [
        "<strong", "<b>", "<em", "<i>", "<u>", "<s>", "<a ", "<span", "<code",
    ]
    .iter()
    .any(|tag| lower.contains(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_str(text: &str) -> Result<BoardTree, FileRefusal> {
        read(&Payload::new(text.as_bytes().to_vec()))
    }

    #[test]
    fn json_of_neither_tool_is_another_document() {
        assert_eq!(read_str("{\"name\":1}"), Err(FileRefusal::OtherDocument));
        assert_eq!(read_str("not json"), Err(FileRefusal::Malformed));
    }

    #[test]
    fn miro_html_reads_as_lines_of_plain_text() {
        assert_eq!(
            plain_text("<p>One &amp; <strong>two</strong></p><p>three<br>four</p>"),
            "One & two\nthree\nfour"
        );
        assert!(has_inline_markup("<p>a <strong>b</strong></p>"));
        assert!(!has_inline_markup("<p>a</p>"));
    }

    #[test]
    fn a_cycle_of_miro_parents_reads_from_the_canvas() {
        let board = read_str(
            r#"{"data":[
                {"id":"1","type":"frame","parent":{"id":"2"},"position":{"x":0,"y":0,"relativeTo":"parent_top_left"},"geometry":{"width":10,"height":10}},
                {"id":"2","type":"frame","parent":{"id":"1"},"position":{"x":0,"y":0,"relativeTo":"parent_top_left"},"geometry":{"width":10,"height":10}}
            ]}"#,
        )
        .expect("a board");
        assert_eq!(board.depth, BoardDepth::Exceeded);
    }

    #[test]
    fn hex_colours_read_in_both_lengths() {
        let v = |s: &'static str| Json::Text(std::borrow::Cow::Borrowed(s));
        assert_eq!(
            hex_color(Some(&v("#ff8000"))),
            Some(Rgb::from_channels(255, 128, 0))
        );
        assert_eq!(
            hex_color(Some(&v("#f80"))),
            Some(Rgb::from_channels(255, 136, 0))
        );
        assert_eq!(hex_color(Some(&v("transparent"))), None);
    }
}
