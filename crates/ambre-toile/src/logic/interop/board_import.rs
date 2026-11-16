//! Figma and Miro import (`toile-import-export`).
//!
//! The boards converter module reads the export into a flat board
//! (`ambre_convert::boards`). This file maps each node of it onto the board:
//!
//! * a frame becomes a frame, titled, holding its children (`TOILE-IO-140`);
//! * a group becomes a group, rebuilt after the creates land (`TOILE-IO-141`);
//! * a shape, a text and a sticky note become the same kind of element
//!   (`TOILE-IO-014`);
//! * a shape with words becomes the shape and a text, grouped
//!   (`TOILE-IO-142`);
//! * a connector between two mapped nodes becomes a connector
//!   (`TOILE-IO-143`).
//!
//! Every construct the module could not read, and every one this map
//! approximates, is named in the report (`TOILE-IO-091`). The export writes
//! the absolute box of each child of an auto layout. So such a frame lands
//! as a plain frame, with nothing to resolve (`TOILE-IO-016`).
//!
//! Geometry is clamped to the world, and the counts and the depth are held
//! while walking. Any breach aborts the whole import: the returned `Err`
//! carries no command (`TOILE-IO-092`, `TOILE-IO-093`).

use ambre_bus::interop::{FileFormat, Outcome};
use ambre_convert::boards::{
    BoardDepth, BoardFormat, BoardItem, BoardKind, BoardLine, BoardLoss, BoardNode, BoardPaint,
    BoardRect, BoardShape, BoardText, BoardTree, LinkCap, LinkCaption, TextStyling,
};
use ambre_core::hash::HashMap;
use ambre_core::index::Pos;
use ambre_core::{Board, Count, Elements, Px, Radians, Rgba, StoredBoard};
use kurbo::{BezPath, Point, Rect, Size};

use super::{
    ImportBatch, ImportConnector, ImportDepth, ImportElement, ImportError, ImportFrame,
    ImportGroup, ImportGroupRank, ImportRef, Limits,
};
use crate::logic::command::{InitContent, ToileCommand};
use crate::logic::model::{Anchor, ElementKind, ElementStyle, EndCaps, PathType, ShapeKind};
use crate::message::{Cap, FontSizeChoice, TEXT_FONT_SIZE_MAX, ToileConstruct};

/// The gap left between two Figma pages laid side by side, in board units.
const PAGE_GAP: Px<Board> = Px::new(200.0);

/// The share of its shorter side a rounded rectangle takes as its corner
/// radius: the file names a radius the board stores no copy of.
const ROUND_CORNER_SHARE: f64 = 0.1;

/// Map the board the boards module read to an atomic [`ImportBatch`].
///
/// # Errors
///
/// The limit errors of [`Limits`] when the board exceeds one, and
/// [`ImportError::Empty`] when the walk yields no element. No command
/// survives an error: the batch is atomic.
pub fn import_board(tree: &BoardTree, limits: &Limits) -> Result<ImportBatch, ImportError> {
    let format = match tree.format {
        BoardFormat::Figma => FileFormat::Figma,
        BoardFormat::Miro => FileFormat::Miro,
    };
    if tree.depth == BoardDepth::Exceeded {
        return Err(ImportError::TooDeep {
            limit: limits.max_depth,
        });
    }
    let mut walk = Walk {
        tree,
        limits,
        batch: ImportBatch::new(format),
        endpoints: HashMap::default(),
    };
    for loss in &tree.losses {
        let (construct, outcome) = construct_of(*loss);
        walk.batch.report.record(construct, outcome);
    }
    walk.root()?;
    walk.links();
    if walk.batch.commands.is_empty() {
        return Err(ImportError::Empty);
    }
    Ok(walk.batch)
}

/// What the report names for each loss of the module.
const fn construct_of(loss: BoardLoss) -> (ToileConstruct, Outcome) {
    match loss {
        BoardLoss::Picture => (ToileConstruct::Picture, Outcome::Dropped),
        BoardLoss::VectorPath => (ToileConstruct::VectorPath, Outcome::Dropped),
        BoardLoss::Embed => (ToileConstruct::Embed, Outcome::Dropped),
        BoardLoss::Card => (ToileConstruct::Card, Outcome::Dropped),
        BoardLoss::Table => (ToileConstruct::Table, Outcome::Dropped),
        BoardLoss::Widget => (ToileConstruct::Widget, Outcome::Dropped),
        BoardLoss::UnknownNode => (ToileConstruct::UnknownElement, Outcome::Dropped),
        BoardLoss::Effect => (ToileConstruct::Effect, Outcome::Dropped),
        BoardLoss::Rotation => (ToileConstruct::Rotation, Outcome::Approximated),
        BoardLoss::ComponentLink => (ToileConstruct::ComponentLink, Outcome::Dropped),
    }
}

struct Walk<'t> {
    tree: &'t BoardTree,
    limits: &'t Limits,
    batch: ImportBatch,
    /// The slot each node landed as, for the ends of a connector.
    endpoints: HashMap<Pos<BoardNode>, Pos<ImportElement>>,
}

impl Walk<'_> {
    /// Walk the root. A Figma page holds its own plane. So each page after
    /// the first moves right of the one before it, and the report says so.
    fn root(&mut self) -> Result<(), ImportError> {
        let children = self.children_of(BoardTree::ROOT);
        let pages = children
            .iter()
            .filter(|c| {
                self.tree
                    .get(**c)
                    .is_some_and(|n| n.kind == BoardKind::Page)
            })
            .count();
        let mut right: Option<Px<Board>> = None;
        let tree = self.tree;
        for child in children {
            let Some(node) = tree.get(child) else {
                continue;
            };
            if node.kind == BoardKind::Page {
                let content = self.content_bounds(child);
                let shift = match (right, content) {
                    (Some(right), Some(content)) => right + PAGE_GAP - Px::new(content.x0),
                    _ => Px::ZERO,
                };
                if let Some(content) = content {
                    right = Some(Px::<Board>::new(content.x1) + shift);
                }
                if pages > 1 {
                    self.batch
                        .report
                        .record(ToileConstruct::Page, Outcome::Approximated);
                }
                self.members(child, Count::<ImportDepth>::ZERO, shift)?;
            } else {
                self.node(child, Count::<ImportDepth>::ZERO, Px::ZERO)?;
            }
        }
        Ok(())
    }

    fn children_of(&self, at: Pos<BoardNode>) -> Vec<Pos<BoardNode>> {
        self.tree
            .get(at)
            .map(|n| n.children.clone())
            .unwrap_or_default()
    }

    /// The union of the boxes under `at`, or `None` for a page with nothing.
    fn content_bounds(&self, at: Pos<BoardNode>) -> Option<Rect> {
        let mut stack = self.children_of(at);
        let mut union: Option<Rect> = None;
        // The module bounds the depth, and each node is pushed once, as the
        // arena is a tree.
        while let Some(pos) = stack.pop() {
            let Some(node) = self.tree.get(pos) else {
                continue;
            };
            let r = rect_of(node.bounds);
            union = Some(union.map_or(r, |u| u.union(r)));
            stack.extend(node.children.iter().copied());
        }
        union
    }

    /// Map the children of `at`, and return the refs of those that landed.
    fn members(
        &mut self,
        at: Pos<BoardNode>,
        depth: Count<ImportDepth>,
        shift: Px<Board>,
    ) -> Result<Vec<ImportRef>, ImportError> {
        self.limits.check_depth(depth)?;
        let mut members = Vec::new();
        for child in self.children_of(at) {
            if let Some(r) = self.node(child, depth.incremented(), shift)? {
                members.push(r);
            }
        }
        Ok(members)
    }

    /// Map one node, and return the ref of what it landed as.
    fn node(
        &mut self,
        pos: Pos<BoardNode>,
        depth: Count<ImportDepth>,
        shift: Px<Board>,
    ) -> Result<Option<ImportRef>, ImportError> {
        let tree = self.tree;
        let Some(node) = tree.get(pos) else {
            return Ok(None);
        };
        let rect = self.place(node.bounds, shift);
        match &node.kind {
            // A page or a root nested below the root is no structure the
            // board holds: its members land where they are.
            BoardKind::Root | BoardKind::Page => {
                let members = self.members(pos, depth, shift)?;
                Ok(self.group_of(members))
            }
            BoardKind::Frame(title) => {
                let style = self.style(node, ElementKind::Frame);
                let slot = self.push(ElementKind::Frame, rect, InitContent::None, style, node)?;
                self.endpoints.insert(pos, slot);
                let members = self.members(pos, depth, shift)?;
                self.batch.frames.push(ImportFrame {
                    frame: slot,
                    name: title.clone(),
                    members,
                });
                Ok(Some(ImportRef::Slot(slot)))
            }
            BoardKind::Group => {
                let members = self.members(pos, depth, shift)?;
                Ok(self.group_of(members))
            }
            BoardKind::Shape(shape) => self.shape(pos, node, *shape, rect),
            BoardKind::Text => {
                let Some(text) = node.text.as_ref().filter(|t| !t.body.as_str().is_empty()) else {
                    self.batch
                        .report
                        .record(ToileConstruct::Text, Outcome::Dropped);
                    return Ok(None);
                };
                let slot = self.text(text, rect, node)?;
                self.endpoints.insert(pos, slot);
                Ok(Some(ImportRef::Slot(slot)))
            }
            BoardKind::Sticky => {
                let mut style = self.style(node, ElementKind::Sticky);
                let body = node.text.as_ref().map(|t| {
                    self.ink(t, &mut style);
                    t.body.as_str().to_owned()
                });
                let slot = self.push(
                    ElementKind::Sticky,
                    rect,
                    InitContent::Text(body.unwrap_or_default()),
                    style,
                    node,
                )?;
                self.endpoints.insert(pos, slot);
                Ok(Some(ImportRef::Slot(slot)))
            }
        }
    }

    /// A shape, and the text over it when it carries words. The two land
    /// grouped, so they move as the one node the file drew.
    fn shape(
        &mut self,
        pos: Pos<BoardNode>,
        node: &BoardItem,
        shape: BoardShape,
        rect: Rect,
    ) -> Result<Option<ImportRef>, ImportError> {
        let (kind, content) = match shape {
            BoardShape::Rect | BoardShape::RoundRect => (ShapeKind::Rect, InitContent::None),
            BoardShape::Ellipse => (ShapeKind::Ellipse, InitContent::None),
            BoardShape::Triangle => (ShapeKind::Triangle, InitContent::None),
            BoardShape::Line => (ShapeKind::Line, InitContent::None),
            BoardShape::Diamond => (ShapeKind::Path, InitContent::Path(diamond(rect))),
            BoardShape::Other => {
                self.batch
                    .report
                    .record(ToileConstruct::ForeignShape, Outcome::Approximated);
                (ShapeKind::Rect, InitContent::None)
            }
        };
        let mut style = self.style(node, ElementKind::Shape(kind));
        if shape == BoardShape::RoundRect {
            style.corner = stored(Px::new(
                rect.width().min(rect.height()) * ROUND_CORNER_SHARE,
            ));
        }
        let slot = self.push(ElementKind::Shape(kind), rect, content, style, node)?;
        self.endpoints.insert(pos, slot);
        let Some(text) = node.text.as_ref().filter(|t| !t.body.as_str().is_empty()) else {
            return Ok(Some(ImportRef::Slot(slot)));
        };
        let label = self.text(text, rect, node)?;
        Ok(self.group_of(vec![ImportRef::Slot(slot), ImportRef::Slot(label)]))
    }

    /// A text element over `rect`, at the size and in the ink the file
    /// names.
    fn text(
        &mut self,
        text: &BoardText,
        rect: Rect,
        node: &BoardItem,
    ) -> Result<Pos<ImportElement>, ImportError> {
        let mut style = ElementStyle {
            fill: None,
            stroke: None,
            stroke_w: Px::<StoredBoard>::ZERO,
            ..ElementStyle::default()
        };
        self.ink(text, &mut style);
        if let Some(size) = text.size {
            style.font.size = FontSizeChoice::picked(Some(size.at_most(TEXT_FONT_SIZE_MAX)));
        }
        self.push(
            ElementKind::Text,
            rect,
            InitContent::Text(text.body.as_str().to_owned()),
            style,
            node,
        )
    }

    /// The ink of a text, and the report of runs styled apart.
    fn ink(&mut self, text: &BoardText, style: &mut ElementStyle) {
        if let Some(color) = text.color {
            style.text_color = Some(color.to_rgba(ambre_core::Alpha::OPAQUE));
        }
        if text.styling == TextStyling::Mixed {
            self.batch
                .report
                .record(ToileConstruct::TextFormatting, Outcome::Approximated);
        }
    }

    /// The style of a node: its fill and its stroke over the kind's default.
    fn style(&mut self, node: &BoardItem, kind: ElementKind) -> ElementStyle {
        let mut style = ElementStyle::for_kind(kind);
        style.fill = match node.fill {
            BoardPaint::None => None,
            BoardPaint::Solid { .. } | BoardPaint::Other => self.paint(node.fill),
        };
        // A sticky note the file gave no colour keeps the board's own.
        if kind == ElementKind::Sticky && style.fill.is_none() {
            style.fill = ElementStyle::for_kind(kind).fill;
        }
        match node.stroke {
            Some(stroke) => {
                style.stroke = self.paint(stroke.paint);
                style.stroke_w = if style.stroke.is_some() {
                    stored(stroke.width)
                } else {
                    Px::<StoredBoard>::ZERO
                };
            }
            None => {
                style.stroke = None;
                style.stroke_w = Px::<StoredBoard>::ZERO;
            }
        }
        style
    }

    /// A solid paint as packed RGBA. A gradient or a picture fill is
    /// reported and lands as no paint.
    fn paint(&mut self, paint: BoardPaint) -> Option<Rgba> {
        match paint {
            BoardPaint::None => None,
            BoardPaint::Solid { color, opacity } => {
                Some(color.to_rgba(opacity.fade(ambre_core::Alpha::OPAQUE)))
            }
            BoardPaint::Other => {
                self.batch
                    .report
                    .record(ToileConstruct::NonSolidPaint, Outcome::Dropped);
                None
            }
        }
    }

    /// A group of `members`: none lands nothing, one passes through, and two
    /// or more push a group rebuilt after the creates.
    fn group_of(&mut self, members: Vec<ImportRef>) -> Option<ImportRef> {
        match members.as_slice() {
            [] => None,
            [only] => Some(*only),
            _ => {
                let rank = Pos::<ImportGroupRank>::from_usize(self.batch.groups.len());
                self.batch.groups.push(ImportGroup { members });
                Some(ImportRef::Group(rank))
            }
        }
    }

    /// The board box of `bounds`, moved right by `shift` and clamped. A side
    /// of zero, a Figma line, keeps one unit so the create accepts it.
    fn place(&self, bounds: BoardRect, shift: Px<Board>) -> Rect {
        let r = rect_of(bounds);
        let shift = shift.get();
        let r = Rect::new(
            r.x0 + shift,
            r.y0,
            r.x0 + shift + r.width().max(1.0),
            r.y0 + r.height().max(1.0),
        );
        self.limits.clamp_rect(r)
    }

    /// Append one create, its style and its rotation, and return its slot.
    fn push(
        &mut self,
        kind: ElementKind,
        rect: Rect,
        content: InitContent,
        style: ElementStyle,
        node: &BoardItem,
    ) -> Result<Pos<ImportElement>, ImportError> {
        self.limits
            .check_element_count(Count::<Elements>::of(self.batch.commands.len()))?;
        let slot = Pos::<ImportElement>::from_usize(self.batch.commands.len());
        self.batch.commands.push(ToileCommand::CreateElement {
            kind,
            at: rect.origin(),
            size: Size::new(rect.width(), rect.height()),
            content,
            fill: None,
            correlate: None,
        });
        self.batch.styles.push(Some(style));
        let rotation = if node.rotation.is_finite() {
            node.rotation
        } else {
            Radians::ZERO
        };
        if rotation != Radians::ZERO {
            self.batch
                .rotations
                .resize(self.batch.commands.len() - 1, Radians::ZERO);
            self.batch.rotations.push(rotation);
        }
        Ok(slot)
    }

    /// The connectors whose two ends landed. Any other is reported, as is a
    /// caption, which the board's connector does not carry.
    fn links(&mut self) {
        let tree = self.tree;
        for link in &tree.links {
            if link.caption == LinkCaption::Present {
                self.batch
                    .report
                    .record(ToileConstruct::ConnectorLabel, Outcome::Dropped);
            }
            let end = |pos: Option<Pos<BoardNode>>| pos.and_then(|p| self.endpoints.get(&p));
            let (Some(from), Some(to)) = (end(link.from), end(link.to)) else {
                self.batch
                    .report
                    .record(ToileConstruct::Connector, Outcome::Dropped);
                continue;
            };
            let cap = |c: LinkCap| match c {
                LinkCap::Plain => Cap::Plain,
                LinkCap::Arrow => Cap::Arrow,
            };
            let connector = ImportConnector {
                from: ImportRef::Slot(*from),
                from_anch: Anchor::Center,
                to: ImportRef::Slot(*to),
                to_anch: Anchor::Center,
                path_type: match link.line {
                    BoardLine::Straight => PathType::Straight,
                    BoardLine::Elbow => PathType::Elbow,
                    BoardLine::Curved => PathType::Curved,
                },
                arrows: EndCaps {
                    start: cap(link.start),
                    end: cap(link.end),
                },
            };
            self.batch.connectors.push(connector);
        }
    }
}

/// A box of the board.
fn rect_of(bounds: BoardRect) -> Rect {
    let (left, top) = (bounds.left.get(), bounds.top.get());
    Rect::new(
        left,
        top,
        left + bounds.width.get().max(0.0),
        top + bounds.height.get().max(0.0),
    )
}

/// A length stored in a style.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a style keeps its lengths in f32"
)]
fn stored(length: Px<Board>) -> Px<StoredBoard> {
    Px::<StoredBoard>::new(length.get() as f32)
}

/// The diamond inscribed in `rect`.
fn diamond(rect: Rect) -> BezPath {
    let c = rect.center();
    let mut path = BezPath::new();
    path.move_to(Point::new(c.x, rect.y0));
    path.line_to(Point::new(rect.x1, c.y));
    path.line_to(Point::new(c.x, rect.y1));
    path.line_to(Point::new(rect.x0, c.y));
    path.close_path();
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ToileReport;

    const FIGMA: &str = include_str!("fixtures/figma_board.json");
    const MIRO: &str = include_str!("fixtures/miro_board.json");

    /// The board the boards engine the host links reads from `json`.
    fn tree_of(json: &str) -> BoardTree {
        let converter = ambre_convert::Converter::load(
            ambre_core::ConverterFamily::Boards,
            &ambre_core::Payload::new(Vec::new()),
        )
        .expect("the host links the boards engine");
        let data = ambre_core::Payload::<ambre_client_core::FileBytes>::new(json.as_bytes().to_vec());
        match converter.convert(&ambre_convert::Conversion::ReadBoard { data }) {
            Ok(ambre_convert::Converted::Board(bytes)) => {
                ambre_convert::boards::decode_board(&bytes).expect("a board decodes")
            }
            other => panic!("the boards engine answers a board: {other:?}"),
        }
    }

    fn batch_of(json: &str) -> ImportBatch {
        import_board(&tree_of(json), &Limits::default()).expect("the fixture maps")
    }

    /// The kind and the box of the create at `slot`.
    fn create(batch: &ImportBatch, slot: usize) -> (ElementKind, Rect) {
        match batch.commands.get(slot) {
            Some(ToileCommand::CreateElement { kind, at, size, .. }) => {
                (*kind, Rect::from_origin_size(*at, *size))
            }
            other => panic!("slot {slot} is a create: {other:?}"),
        }
    }

    fn text_of(batch: &ImportBatch, slot: usize) -> String {
        match batch.commands.get(slot) {
            Some(ToileCommand::CreateElement {
                content: InitContent::Text(body),
                ..
            }) => body.clone(),
            other => panic!("slot {slot} carries a text: {other:?}"),
        }
    }

    fn count(report: &ToileReport, construct: ToileConstruct, outcome: Outcome) -> u32 {
        report.count(construct, outcome).get()
    }

    /// Covers `TOILE-IO-014` for Figma: each node lands as the same kind of
    /// element, with its words, its size and its paint.
    #[test]
    fn each_figma_node_lands_as_its_own_kind() {
        let batch = batch_of(FIGMA);
        let kinds: Vec<ElementKind> = (0..batch.commands.len())
            .map(|slot| create(&batch, slot).0)
            .collect();
        assert_eq!(
            kinds,
            [
                ElementKind::Frame,
                ElementKind::Sticky,
                ElementKind::Text,
                ElementKind::Shape(ShapeKind::Path),
                ElementKind::Text,
                ElementKind::Shape(ShapeKind::Rect),
                ElementKind::Shape(ShapeKind::Ellipse),
                ElementKind::Shape(ShapeKind::Rect),
                ElementKind::Shape(ShapeKind::Ellipse),
                ElementKind::Frame,
                ElementKind::Shape(ShapeKind::Rect),
                ElementKind::Shape(ShapeKind::Rect),
            ]
        );
        assert_eq!(text_of(&batch, 1), "Ship the import");
        assert_eq!(text_of(&batch, 4), "Decide");
        let owner = batch.styles[2].as_ref().expect("a style");
        assert_eq!(owner.text_color, Some(Rgba::from_bits(0x0000_00FF)));
        assert_eq!(owner.font.size.as_fixed().map(|s| s.points()), Some(24.0));
        let banner = batch.styles[5].as_ref().expect("a style");
        assert_eq!(banner.fill, None, "the gradient is dropped");
        assert_eq!(banner.stroke, Some(Rgba::from_bits(0xFF00_00FF)));
        assert!(
            banner.corner.get() > 0.0,
            "a rounded rectangle keeps a corner"
        );
        let tilted = batch.styles[6].as_ref().expect("a style");
        assert_eq!(
            tilted.fill,
            Some(Rgba::from_bits(0x0080_0080)),
            "half opaque"
        );
    }

    /// Covers `TOILE-IO-016`: a frame under an auto layout lands as a plain
    /// frame, each child at the absolute box the file carries.
    #[test]
    fn an_auto_layout_frame_lands_with_its_children_where_figma_put_them() {
        let batch = batch_of(FIGMA);
        assert_eq!(create(&batch, 0).1, Rect::new(0.0, 0.0, 800.0, 600.0));
        assert_eq!(create(&batch, 1).1, Rect::new(40.0, 40.0, 240.0, 240.0));
        assert_eq!(create(&batch, 2).1, Rect::new(40.0, 260.0, 340.0, 300.0));
    }

    /// Covers `TOILE-IO-140`, `TOILE-IO-141` and `TOILE-IO-142`: a frame names its title and
    /// its members, and a shape with words lands grouped with its text.
    #[test]
    fn a_frame_holds_its_members_and_a_labelled_shape_is_grouped() {
        let batch = batch_of(FIGMA);
        let sprint = batch
            .frames
            .iter()
            .find(|f| f.name.as_str() == "Sprint plan")
            .expect("the frame");
        assert_eq!(sprint.frame, Pos::<ImportElement>::from_usize(0));
        assert_eq!(sprint.members.len(), 5);
        assert!(batch.groups.iter().any(|g| g.members
            == [
                ImportRef::Slot(Pos::from_usize(3)),
                ImportRef::Slot(Pos::from_usize(4))
            ]));
        assert!(batch.groups.iter().any(|g| g.members
            == [
                ImportRef::Slot(Pos::from_usize(7)),
                ImportRef::Slot(Pos::from_usize(8))
            ]));
    }

    /// Covers `TOILE-IO-145`: each Figma page after the first lands right of
    /// the one before it, and the report says the pages were laid out.
    #[test]
    fn a_second_figma_page_lands_beside_the_first() {
        let batch = batch_of(FIGMA);
        let (_, alone) = create(&batch, 11);
        assert_eq!(alone.x0, 1500.0 + PAGE_GAP.get());
        assert_eq!(
            count(&batch.report, ToileConstruct::Page, Outcome::Approximated),
            2
        );
    }

    /// Covers `TOILE-IO-143`: a connector between two mapped nodes lands, and
    /// one with a free end is reported, as its caption is.
    #[test]
    fn a_connector_lands_between_mapped_nodes_and_a_loose_one_is_reported() {
        let batch = batch_of(FIGMA);
        assert_eq!(
            batch.connectors,
            [ImportConnector {
                from: ImportRef::Slot(Pos::from_usize(1)),
                from_anch: Anchor::Center,
                to: ImportRef::Slot(Pos::from_usize(3)),
                to_anch: Anchor::Center,
                path_type: PathType::Elbow,
                arrows: EndCaps {
                    start: Cap::Plain,
                    end: Cap::Arrow,
                },
            }]
        );
        let report = &batch.report;
        assert_eq!(
            count(report, ToileConstruct::Connector, Outcome::Dropped),
            1
        );
        assert_eq!(
            count(report, ToileConstruct::ConnectorLabel, Outcome::Dropped),
            1
        );
    }

    /// Covers `TOILE-IO-091` for Figma: each construct the import drops or
    /// changes is named, once per occurrence, and a hidden node is not.
    #[test]
    fn every_figma_construct_dropped_is_named() {
        let report = batch_of(FIGMA).report;
        for (construct, outcome) in [
            (ToileConstruct::VectorPath, Outcome::Dropped),
            (ToileConstruct::ComponentLink, Outcome::Dropped),
            (ToileConstruct::Embed, Outcome::Dropped),
            (ToileConstruct::Table, Outcome::Dropped),
            (ToileConstruct::Picture, Outcome::Dropped),
            (ToileConstruct::Effect, Outcome::Dropped),
            (ToileConstruct::NonSolidPaint, Outcome::Dropped),
            (ToileConstruct::Rotation, Outcome::Approximated),
            (ToileConstruct::TextFormatting, Outcome::Approximated),
        ] {
            assert_eq!(count(&report, construct, outcome), 1, "{construct:?}");
        }
    }

    /// Covers `TOILE-IO-014`, `TOILE-IO-144` and `TOILE-IO-091` for Miro.
    /// Each item lands as its kind, and a child of a frame at its parent's
    /// corner. A rotation is kept, and each item dropped is named.
    #[test]
    fn each_miro_item_lands_as_its_kind_and_each_loss_is_named() {
        let batch = batch_of(MIRO);
        assert_eq!(
            create(&batch, 0),
            (ElementKind::Frame, Rect::new(0.0, 0.0, 1000.0, 800.0))
        );
        assert_eq!(
            create(&batch, 1),
            (ElementKind::Sticky, Rect::new(0.0, 0.0, 200.0, 200.0))
        );
        assert_eq!(
            create(&batch, 2),
            (
                ElementKind::Shape(ShapeKind::Rect),
                Rect::new(320.0, 60.0, 480.0, 140.0)
            )
        );
        assert_eq!(text_of(&batch, 1), "Went well");
        assert_eq!(text_of(&batch, 3), "Action");
        assert_eq!(text_of(&batch, 4), "Notes & ideas");
        assert_eq!(create(&batch, 4).1, Rect::new(1200.0, 100.0, 1500.0, 150.0));
        let quarter = Radians::new(std::f64::consts::FRAC_PI_4);
        assert_eq!(batch.rotations.get(2), Some(&quarter));
        assert_eq!(batch.rotations.get(3), Some(&quarter));
        assert_eq!(batch.frames.len(), 1);
        assert_eq!(batch.frames[0].name.as_str(), "Retro");
        assert_eq!(batch.frames[0].members.len(), 2);
        assert_eq!(batch.connectors.len(), 1);
        let shape = batch.styles[2].as_ref().expect("a style");
        assert_eq!(shape.fill, Some(Rgba::from_bits(0xFF88_00FF)));
        assert_eq!(shape.stroke_w.get(), 2.0);
        let report = &batch.report;
        for (construct, outcome, n) in [
            (ToileConstruct::Picture, Outcome::Dropped, 1),
            (ToileConstruct::Card, Outcome::Dropped, 1),
            (ToileConstruct::Embed, Outcome::Dropped, 1),
            (ToileConstruct::UnknownElement, Outcome::Dropped, 1),
            (ToileConstruct::Connector, Outcome::Dropped, 1),
            (ToileConstruct::ConnectorLabel, Outcome::Dropped, 1),
            (ToileConstruct::ForeignShape, Outcome::Approximated, 1),
            (ToileConstruct::TextFormatting, Outcome::Approximated, 1),
        ] {
            assert_eq!(count(report, construct, outcome), n, "{construct:?}");
        }
    }

    /// Covers `TOILE-IO-092`: a board past the element bound is refused whole,
    /// and so is one nested past the module's depth.
    #[test]
    fn a_board_past_a_bound_is_refused_whole() {
        let tight = Limits {
            max_elements: Count::<Elements>::new(3),
            ..Limits::default()
        };
        assert!(matches!(
            import_board(&tree_of(FIGMA), &tight),
            Err(ImportError::TooManyElements { .. })
        ));
        let mut deep = String::from(
            r#"{"document":{"type":"DOCUMENT","children":[{"type":"CANVAS","children":["#,
        );
        for _ in 0..40 {
            deep.push_str(r#"{"type":"GROUP","children":["#);
        }
        deep.push_str(
            r#"{"type":"RECTANGLE","absoluteBoundingBox":{"x":0,"y":0,"width":1,"height":1}}"#,
        );
        for _ in 0..40 {
            deep.push_str("]}");
        }
        deep.push_str("]}]}}");
        let shallow = Limits {
            max_depth: Count::<ImportDepth>::new(8),
            ..Limits::default()
        };
        assert!(matches!(
            import_board(&tree_of(&deep), &shallow),
            Err(ImportError::TooDeep { .. })
        ));
    }

    /// Covers `TOILE-IO-084` for a board: an export whose nodes all drop maps
    /// to nothing and is refused, never applied as an empty import.
    #[test]
    fn a_board_of_nothing_the_board_holds_is_refused() {
        let tree = tree_of(r#"{"data":[{"id":"1","type":"image"}]}"#);
        assert!(matches!(
            import_board(&tree, &Limits::default()),
            Err(ImportError::Empty)
        ));
    }
}
