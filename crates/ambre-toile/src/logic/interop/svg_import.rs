//! SVG import (`toile-import-export`).
//!
//! `usvg` parses the document into a normalized render tree; we walk it and map
//! each node to a Toile `CreateElement`. usvg flattens every primitive (`<rect>`,
//! `<circle>`, `<line>`, ...) into a `Path`. So vector nodes map to `Shape(Path)`
//! carrying a `kurbo::BezPath`. An axis-aligned rectangle contour is recognized
//! back into `Shape(Rect)`, the common and safe case. `<image>` maps to an `Image`
//! element plus a plaintext [`MediaBlob`], and `<g>` becomes an [`ImportGroup`]
//! rebuilt after apply -- see the note on groups below.
//!
//! **Text (why not usvg).** usvg only exposes `<text>` by flattening it to glyph
//! *paths*, which needs a font database. That is impossible in the browser (no
//! system fonts) and destructive (it would lose the editable string). So the usvg
//! font stack is disabled. The normalized tree then carries no text node at all,
//! and `<text>` is read from the SVG source with `roxmltree`. The raw body plus
//! its `x`/`y` become a `Text` element Toile renders through its own Parley
//! stack.
//!
//! **One walk over two trees.** The source and the usvg tree are stepped
//! together. A text node then lands where the drawing put it. In document
//! order (`TOILE-IO-027`), inside the group that holds it (`TOILE-IO-083`),
//! under the transforms above it (`TOILE-IO-080`). Those come from the usvg
//! group the walk is inside, which already carries them.
//!
//! Each usvg child consumes the next source child that is not a `<text>`, and
//! every `<text>` passed on the way is imported at that point. Text the reader cannot read is reported
//! rather than dropped in silence (`TOILE-IO-081`).
//!
//! **Groups.** The spec maps `<g>` to a Toile `group`. A group is created only via
//! `GroupElements { ids }`, which needs the `Id<Element>`s the engine mints *after*
//! `CreateElement` is applied -- ids an offline converter cannot know. So the
//! converter speaks in slots. It imports the children as elements, with the group's
//! transform baked into their geometry via `abs_transform` / abs bounds so geometry
//! stays world-space. It then records an [`ImportGroup`] whose members are the
//! slots (and nested groups) it contains.
//!
//! Groups are pushed **innermost-first** (post-order) so a nested member `Group(i)`
//! always refers to an already-resolved entry. The apply pass mints the ids and
//! issues `GroupElements` in the same undo group, reconstructing the hierarchy. A
//! group the walk reduced to one member, or to none, cannot be rebuilt and is
//! reported (`TOILE-IO-079`).
//!
//! All geometry is clamped to the world bound and the element/depth counts are
//! enforced while walking. Any breach aborts the whole import atomically: the
//! returned `Err` carries no commands.

use ambre_core::{ElementCount, Rgb, Rgba};
use kurbo::{BezPath, PathEl, Point, Rect, Shape, Size};
use usvg::tiny_skia_path::PathSegment;

use super::raster::decode_media;
use super::{
    ImportBatch, ImportDepth, ImportElementSlot, ImportError, ImportGroup, ImportGroupSlot,
    ImportRef, Limits,
};
use crate::logic::command::{ImportSlot, InitContent, MediaRef, ToileCommand};
use crate::logic::model::{ElementKind, ElementStyle, ShapeKind};

/// Parse SVG `bytes` and convert to an atomic [`ImportBatch`]. Returns
/// `Err` (and no commands) on a parse failure, an exceeded safety limit, or an
/// SVG that maps to zero elements (atomicity).
///
/// # Errors
///
/// [`ImportError::Parse`] when usvg refuses `bytes`, the limit errors of
/// [`Limits`] when the tree exceeds one, and [`ImportError::Empty`] when the
/// walk yields no element. No command survives an error: the batch is atomic.
pub fn import_svg(
    file: &ambre_core::Payload<ambre_core::FileBytes>,
    limits: &Limits,
) -> Result<ImportBatch, ImportError> {
    let bytes = file.as_bytes();
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &opt).map_err(|e| ImportError::Parse(e.to_string()))?;

    let mut batch = ImportBatch::default();
    // The source tree, for the text nodes usvg refuses to carry. A body that is
    // not UTF-8, or not well-formed XML, costs the drawing its text. The report
    // says so, rather than letting the shapes land as though the file had
    // carried none (`TOILE-IO-081`).
    let source = std::str::from_utf8(bytes).ok();
    let doc = source.and_then(|src| match roxmltree::Document::parse(src) {
        Ok(d) => Some(d),
        Err(e) => {
            batch
                .report
                .skip(format!("text: the source is not well-formed XML ({e})"));
            None
        }
    });
    if source.is_none() {
        batch.report.skip("text: the source is not UTF-8");
    }
    let root_xml = doc.as_ref().map(|d| d.root_element());
    // The document root is not itself a `<g>`; its direct-member refs are dropped
    // (only nested `<g>` become groups). Nested groups are pushed during the walk.
    walk_group(tree.root(), root_xml, ImportDepth::ZERO, limits, &mut batch)?;

    if batch.commands.is_empty() {
        return Err(ImportError::Empty);
    }
    batch.report.imported = ambre_bus::app::toile::ElementCount::of_len(batch.commands.len());
    Ok(batch)
}

/// Map one `<text>` node of the source to a `Text` element, inside the group
/// the walk is currently in. `tf` is that group's absolute transform, so the
/// text lands under the transforms the drawing put above it (`TOILE-IO-080`).
///
/// A node whose body reads empty creates nothing and is reported: its content
/// sits in a construct this pass does not follow. A drawing that quietly loses
/// its words reads as one that carried none (`TOILE-IO-081`). usvg cannot give
/// us editable text (see the module note), so the string is taken from the
/// source.
fn import_text_node(
    node: roxmltree::Node,
    tf: &usvg::Transform,
    limits: &Limits,
    batch: &mut ImportBatch,
) -> Result<Option<ImportRef>, ImportError> {
    // Concatenate only the actual text children (avoid double-counting the
    // element node's own text accessor).
    let body: String = node
        .descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect();
    let body = body.trim().to_owned();
    if body.is_empty() {
        batch
            .report
            .skip("text: a node the reader could not read a body from");
        return Ok(None);
    }
    limits.check_element_count(ElementCount::of_len(batch.commands.len()))?;
    let x = node
        .attribute("x")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    let y = node
        .attribute("y")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    let size = node
        .attribute("font-size")
        .and_then(|v| v.trim_end_matches("px").parse::<f64>().ok())
        .unwrap_or(16.0);
    // A minimal text box: width grows with content, height ~ the em box. The
    // authoritative extent is re-derived by the renderer; this seeds a sane AABB.
    let w = (body.chars().count() as f64) * size * 0.6;
    // SVG `y` is the text baseline; the element's top sits ~one em above it.
    let top = y - size;
    let corner = |px: f64, py: f64| -> Point {
        let wx = f64::from(tf.sx) * px + f64::from(tf.kx) * py + f64::from(tf.tx);
        let wy = f64::from(tf.ky) * px + f64::from(tf.sy) * py + f64::from(tf.ty);
        limits.clamp_point(Point::new(wx, wy))
    };
    let a = corner(x, top);
    let b = corner(x + w, top + size * 1.4);
    let rect = limits.clamp_rect(Rect::from_points(a, b));
    Ok(Some(ImportRef::Slot(push_element(
        batch,
        ElementKind::Text,
        rect,
        InitContent::Text(body),
        None,
    ))))
}

/// Whether a source element carries drawn content, and therefore pairs with a
/// node of the usvg tree. `<defs>` and the metadata elements beside it describe
/// the drawing without being in it.
fn is_drawn(node: roxmltree::Node) -> bool {
    node.is_element()
        && !matches!(
            node.tag_name().name(),
            "defs" | "title" | "desc" | "metadata" | "style"
        )
}

/// Recursively walk a usvg group, appending commands, and return the
/// [`ImportRef`]s of the elements/subgroups that are this group's direct members
/// (in child order). `depth` guards nesting. A nested `<g>` recurses first, then pushes its
/// [`ImportGroup`] and contributes a `Group` ref here. Post-order is
/// innermost-first, so a `Group(i)` member always refers to an already-pushed
/// entry.
fn walk_group(
    group: &usvg::Group,
    xml: Option<roxmltree::Node>,
    depth: ImportDepth,
    limits: &Limits,
    batch: &mut ImportBatch,
) -> Result<Vec<ImportRef>, ImportError> {
    limits.check_depth(depth)?;
    let mut members = Vec::new();
    let tf = group.abs_transform();
    // The source children this group's usvg children are paired against, in
    // document order. Empty when the source could not be read, which leaves the
    // walk exactly as it was before the text pass joined it.
    let mut source: Vec<roxmltree::Node> = xml
        .map(|n| n.children().filter(|c| is_drawn(*c)).collect())
        .unwrap_or_default();
    // Popped from the back, so the order walked is the document's.
    source.reverse();
    // Every `<text>` up to the next drawn source child, imported where the
    // drawing put it (`TOILE-IO-027`).
    macro_rules! take_text_before_next {
        () => {{
            let mut paired = None;
            while let Some(node) = source.pop() {
                if node.has_tag_name("text") {
                    if let Some(r) = import_text_node(node, &tf, limits, batch)? {
                        members.push(r);
                    }
                } else {
                    paired = Some(node);
                    break;
                }
            }
            paired
        }};
    }
    for node in group.children() {
        let paired = take_text_before_next!();
        match node {
            usvg::Node::Group(g) => {
                let child_members = walk_group(g, paired, depth.incremented(), limits, batch)?;
                match child_members.len() {
                    // A group of one member, or of none, is not a group the
                    // board can rebuild. Its children keep their geometry and
                    // the structure the drawing had is reported lost
                    // (`TOILE-IO-079`).
                    0 => batch.report.skip("group: every child was skipped"),
                    1 => {
                        batch
                            .report
                            .skip("group: one member left, so no group was rebuilt");
                        members.extend(child_members);
                    }
                    _ => {
                        let group_idx = ImportGroupSlot::from_usize(batch.groups.len());
                        batch.groups.push(ImportGroup {
                            members: child_members,
                        });
                        members.push(ImportRef::Group(group_idx));
                    }
                }
            }
            usvg::Node::Path(p) => members.push(import_path(p, limits, batch)?),
            // The font stack is disabled, so usvg yields no text node: the
            // source walk above carries `<text>` instead (see module note).
            usvg::Node::Text(_) => {}
            usvg::Node::Image(img) => {
                if let Some(r) = import_image(img, limits, batch)? {
                    members.push(r);
                }
            }
        }
    }
    // The text after the last drawn node of this group.
    let _ = take_text_before_next!();
    while let Some(node) = source.pop() {
        if node.has_tag_name("text")
            && let Some(r) = import_text_node(node, &tf, limits, batch)?
        {
            members.push(r);
        }
    }
    Ok(members)
}

/// Map a usvg path to a `Shape` element. Axis-aligned rectangles become
/// `Shape(Rect)`; everything else keeps its exact geometry as `Shape(Path)`.
fn import_path(
    path: &usvg::Path,
    limits: &Limits,
    batch: &mut ImportBatch,
) -> Result<ImportRef, ImportError> {
    limits.check_element_count(ElementCount::of_len(batch.commands.len()))?;

    let bez = to_bezpath(path.data(), &path.abs_transform(), limits);
    let bounds = bez.bounding_box();
    let rect = limits.clamp_rect(bounds);
    let style = style_from_path(path, batch);

    let (kind, content) = match as_axis_aligned_rect(&bez) {
        Some(_) => (ElementKind::Shape(ShapeKind::Rect), InitContent::None),
        None => (ElementKind::Shape(ShapeKind::Path), InitContent::Path(bez)),
    };

    Ok(ImportRef::Slot(push_element(
        batch,
        kind,
        rect,
        content,
        Some(style),
    )))
}

/// Map a usvg `<image>` to an `Image` element plus a plaintext [`MediaBlob`].
/// Only raster PNG/JPEG are decoded; nested SVG / GIF / WebP are noted and
/// skipped in Phase 0.
fn import_image(
    image: &usvg::Image,
    limits: &Limits,
    batch: &mut ImportBatch,
) -> Result<Option<ImportRef>, ImportError> {
    limits.check_element_count(ElementCount::of_len(batch.commands.len()))?;

    let raw: &[u8] = match image.kind() {
        usvg::ImageKind::PNG(d) | usvg::ImageKind::JPEG(d) => d.as_slice(),
        usvg::ImageKind::GIF(_) | usvg::ImageKind::WEBP(_) => {
            batch.report.skip("image: GIF/WebP not decoded in Phase 0");
            return Ok(None);
        }
        usvg::ImageKind::SVG(_) => {
            batch
                .report
                .skip("image: nested SVG not inlined in Phase 0");
            return Ok(None);
        }
    };

    // The blob's slot is its rank in the batch's media, pushed below.
    let media = MediaRef::Pending(ImportSlot::from_usize(batch.media.len()));
    let blob = decode_media(raw, limits, media.clone())?;

    let rect = limits.clamp_rect(rect_from_usvg(image.abs_bounding_box()));
    let slot = push_element(
        batch,
        ElementKind::Image,
        rect,
        InitContent::Image(media),
        None,
    );
    batch.media.push(blob);
    Ok(Some(ImportRef::Slot(slot)))
}

/// Append one `CreateElement` for an element occupying `rect`, plus its imported
/// style (index-aligned into `batch.styles`). Return the slot index the apply
/// pass will resolve to a freshly minted `Id<Element>`.
///
/// Style rides a separate `styles` slot rather than `CreateElement` because the
/// CRDT stores `ElementStyle` as an independent LWW register. The apply pass
/// creates the element, then restyles it inside the same undo group. `None` keeps
/// the engine default (used for text/image, which import unstyled).
fn push_element(
    batch: &mut ImportBatch,
    kind: ElementKind,
    rect: Rect,
    content: InitContent,
    style: Option<ElementStyle>,
) -> ImportElementSlot {
    let slot = ImportElementSlot::from_usize(batch.commands.len());
    batch.commands.push(ToileCommand::CreateElement {
        kind,
        at: rect.origin(),
        size: Size::new(rect.width(), rect.height()),
        content,
        fill: None,
        correlate: None,
    });
    batch.styles.push(style);
    slot
}

// -----------------------------------------------------------------------------
// Geometry + style helpers
// -----------------------------------------------------------------------------

/// Convert a tiny-skia path (local coords) to a world-space `kurbo::BezPath`,
/// applying the node's absolute transform and clamping every point.
fn to_bezpath(path: &usvg::tiny_skia_path::Path, tf: &usvg::Transform, limits: &Limits) -> BezPath {
    let map = |x: f32, y: f32| -> Point {
        // tiny-skia Transform: [sx kx tx; ky sy ty]. Row-major point mapping.
        let wx =
            f64::from(tf.sx) * f64::from(x) + f64::from(tf.kx) * f64::from(y) + f64::from(tf.tx);
        let wy =
            f64::from(tf.ky) * f64::from(x) + f64::from(tf.sy) * f64::from(y) + f64::from(tf.ty);
        limits.clamp_point(Point::new(wx, wy))
    };
    let mut bez = BezPath::new();
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => bez.move_to(map(p.x, p.y)),
            PathSegment::LineTo(p) => bez.line_to(map(p.x, p.y)),
            PathSegment::QuadTo(a, b) => bez.quad_to(map(a.x, a.y), map(b.x, b.y)),
            PathSegment::CubicTo(a, b, c) => {
                bez.curve_to(map(a.x, a.y), map(b.x, b.y), map(c.x, c.y))
            }
            PathSegment::Close => bez.close_path(),
        }
    }
    bez
}

/// Recognize a closed, axis-aligned rectangle contour, the usvg flattening of a
/// `<rect>`. It is a single subpath of straight edges whose corners take
/// exactly two distinct x and two distinct y values. Returns its bounds, else
/// `None`.
fn as_axis_aligned_rect(bez: &BezPath) -> Option<Rect> {
    let mut pts: Vec<Point> = Vec::new();
    for el in bez.elements() {
        match el {
            PathEl::MoveTo(p) => {
                if !pts.is_empty() {
                    return None; // more than one subpath -> not a simple rect
                }
                pts.push(*p);
            }
            PathEl::LineTo(p) => pts.push(*p),
            PathEl::ClosePath => {}
            // Any curve disqualifies a rectangle.
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => return None,
        }
    }
    // Drop a trailing point that duplicates the start (explicit close-to-origin).
    if pts.len() >= 2 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() != 4 {
        return None;
    }
    let xs: Vec<f64> = distinct(pts.iter().map(|p| p.x));
    let ys: Vec<f64> = distinct(pts.iter().map(|p| p.y));
    match (xs.as_slice(), ys.as_slice()) {
        (&[x0, x1], &[y0, y1]) => Some(Rect::new(x0, y0, x1, y1)),
        _ => None,
    }
}

/// The distinct values of an iterator of coordinates, within a small epsilon,
/// returned sorted ascending. Used to test the two-x / two-y rectangle property.
fn distinct(it: impl Iterator<Item = f64>) -> Vec<f64> {
    const EPS: f64 = 1e-6;
    let mut vs: Vec<f64> = Vec::new();
    for v in it {
        if !vs.iter().any(|u| (u - v).abs() < EPS) {
            vs.push(v);
        }
    }
    vs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    vs
}

/// Build an [`ElementStyle`] from a usvg path's fill/stroke. Non-solid paints
/// (gradients, patterns) are not representable in `ElementStyle`; they are dropped
/// and noted in the report (the image-fallback rasterization is deferred).
fn style_from_path(path: &usvg::Path, batch: &mut ImportBatch) -> ElementStyle {
    let mut style = ElementStyle {
        fill: None,
        stroke: None,
        stroke_w: ambre_core::StoredBoardLength::ZERO,
        ..ElementStyle::default()
    };
    if let Some(fill) = path.fill() {
        match paint_rgba(
            fill.paint(),
            ambre_core::Opacity::clamped(fill.opacity().get()),
        ) {
            Some(rgba) => style.fill = Some(rgba),
            None => batch.report.skip("path fill: non-solid paint dropped"),
        }
    }
    if let Some(stroke) = path.stroke() {
        match paint_rgba(
            stroke.paint(),
            ambre_core::Opacity::clamped(stroke.opacity().get()),
        ) {
            Some(rgba) => {
                style.stroke = Some(rgba);
                style.stroke_w = ambre_core::StoredBoardLength::new(stroke.width().get());
            }
            None => batch.report.skip("path stroke: non-solid paint dropped"),
        }
    }
    style
}

/// A solid paint as packed RGBA (`ElementStyle` color encoding), or `None` for a
/// gradient / pattern paint.
fn paint_rgba(paint: &usvg::Paint, opacity: ambre_core::Opacity) -> Option<Rgba> {
    match paint {
        usvg::Paint::Color(c) => {
            let a = opacity.fade(ambre_core::Alpha::OPAQUE);
            Some(Rgb::from_channels(c.red, c.green, c.blue).to_rgba(a))
        }
        _ => None,
    }
}

/// Convert a tiny-skia `Rect` (absolute bounds) to a `kurbo::Rect`.
fn rect_from_usvg(r: usvg::tiny_skia_path::Rect) -> Rect {
    Rect::new(
        f64::from(r.left()),
        f64::from(r.top()),
        f64::from(r.right()),
        f64::from(r.bottom()),
    )
}
