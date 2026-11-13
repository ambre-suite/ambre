//! SVG import (`toile-import-export`).
//!
//! The images converter module reads the file (`ambre_convert::images`).
//! There `usvg` normalizes it, and a pass over the source adds the text usvg
//! does not carry. The module answers a flat drawing, and this file maps each
//! node of it to a Toile `CreateElement`.
//!
//! usvg flattens every primitive
//! (`<rect>`, `<circle>`, `<line>`, ...) into a path. So a path maps to
//! `Shape(Path)` carrying a `kurbo::BezPath`. An axis-aligned rectangle
//! contour is recognized back into `Shape(Rect)`, the common and safe case.
//! `<image>` maps to an `Image` element plus a plaintext [`MediaBlob`], and
//! `<g>` becomes an [`ImportGroup`] rebuilt after apply -- see the note on
//! groups below.
//!
//! **Text.** The drawing carries each `<text>` where the source put it:
//!
//! * in document order (`TOILE-IO-027`);
//! * inside the group that holds it (`TOILE-IO-083`);
//! * under the transform of that group (`TOILE-IO-080`).
//!
//! The raw body becomes a `Text` element Toile renders through its own Parley
//! stack. Text the module could not read is reported rather than dropped in
//! silence (`TOILE-IO-081`).
//!
//! **Groups.** The spec maps `<g>` to a Toile `group`. A group is created only via
//! `GroupElements { ids }`, which needs the `Id<Element>`s the engine mints *after*
//! `CreateElement` is applied -- ids an offline converter cannot know. So the
//! converter speaks in slots. It imports the children as elements, with the group's
//! transform baked into their geometry via the absolute transform / bounds so geometry
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

use crate::message::ToileConstruct;
use ambre_bus::interop::Outcome;
use ambre_convert::images::{
    SvgDepth, SvgImage, SvgKind, SvgNode, SvgPaint, SvgPath, SvgPicture, SvgPoint, SvgRect,
    SvgSegment, SvgSource, SvgText, SvgTransform, SvgTree,
};
use ambre_core::{Count, Elements, Rgba};
use ambre_core::{Px, StoredBoard, index::Pos};
use kurbo::{BezPath, PathEl, Point, Rect, Shape, Size};

use super::raster::decode_media;
use super::{
    ImportBatch, ImportDepth, ImportElement, ImportError, ImportGroup, ImportGroupRank, ImportRef,
    Limits,
};
use crate::logic::command::{ImportSlot, InitContent, MediaRef, ToileCommand};
use crate::logic::model::{ElementKind, ElementStyle, ShapeKind};

/// Map the drawing the images module read to an atomic [`ImportBatch`].
/// Returns `Err` (and no commands) on an exceeded safety limit, or a drawing
/// that maps to zero elements (atomicity).
///
/// # Errors
///
/// The limit errors of [`Limits`] when the drawing exceeds one, and
/// [`ImportError::Empty`] when the walk yields no element. No command survives
/// an error: the batch is atomic.
pub fn import_svg(tree: &SvgTree, limits: &Limits) -> Result<ImportBatch, ImportError> {
    let mut batch = ImportBatch::new(ambre_bus::interop::FileFormat::Svg);
    // A body that is not UTF-8, or not well-formed XML, costs the drawing its
    // text. The report says so, rather than letting the shapes land as though
    // the file had carried none (`TOILE-IO-081`).
    if tree.source == SvgSource::Unreadable {
        batch.report.record(ToileConstruct::Text, Outcome::Dropped);
    }
    // The module stops past its own bound, far below which the limits refuse.
    if tree.depth == SvgDepth::Exceeded {
        return Err(ImportError::TooDeep {
            limit: limits.max_depth,
        });
    }
    // The document root is not itself a `<g>`; its direct-member refs are dropped
    // (only nested `<g>` become groups). Nested groups are pushed during the walk.
    walk_group(
        tree,
        SvgTree::ROOT,
        Count::<ImportDepth>::ZERO,
        limits,
        &mut batch,
    )?;

    if batch.commands.is_empty() {
        return Err(ImportError::Empty);
    }
    Ok(batch)
}

/// Map one `<text>` node to a `Text` element. Its transform is that of the
/// group around it, so the text lands under the transforms the drawing put
/// above it (`TOILE-IO-080`).
///
/// A node whose body reads empty creates nothing and is reported: its content
/// sits in a construct the module's pass does not follow. A drawing that
/// quietly loses its words reads as one that carried none (`TOILE-IO-081`).
fn import_text_node(
    node: &SvgText,
    limits: &Limits,
    batch: &mut ImportBatch,
) -> Result<Option<ImportRef>, ImportError> {
    if node.body.is_empty() {
        batch.report.record(ToileConstruct::Text, Outcome::Dropped);
        return Ok(None);
    }
    limits.check_element_count(Count::<Elements>::of(batch.commands.len()))?;
    let (x, y, size) = (node.x, node.y, node.size);
    // A minimal text box: width grows with content, height ~ the em box. The
    // authoritative extent is re-derived by the renderer; this seeds a sane AABB.
    let w = (node.body.chars().count() as f64) * size * 0.6;
    // SVG `y` is the text baseline; the element's top sits ~one em above it.
    let top = y - size;
    let tf = &node.transform;
    let corner = |px: f64, py: f64| -> Point { limits.clamp_point(apply(tf, px, py)) };
    let a = corner(x, top);
    let b = corner(x + w, top + size * 1.4);
    let rect = limits.clamp_rect(Rect::from_points(a, b));
    Ok(Some(ImportRef::Slot(push_element(
        batch,
        ElementKind::Text,
        rect,
        InitContent::Text(node.body.clone()),
        None,
    ))))
}

/// The point `(x, y)` under `tf`, in world space.
fn apply(tf: &SvgTransform, x: f64, y: f64) -> Point {
    Point::new(
        f64::from(tf.sx) * x + f64::from(tf.kx) * y + f64::from(tf.tx),
        f64::from(tf.ky) * x + f64::from(tf.sy) * y + f64::from(tf.ty),
    )
}

/// Recursively walk a group of the drawing, appending commands, and return the
/// [`ImportRef`]s of the elements/subgroups that are this group's direct members
/// (in child order). `depth` guards nesting. A nested `<g>` recurses first, then pushes its
/// [`ImportGroup`] and contributes a `Group` ref here. Post-order is
/// innermost-first, so a `Group(i)` member always refers to an already-pushed
/// entry.
fn walk_group(
    tree: &SvgTree,
    group: Pos<SvgNode>,
    depth: Count<ImportDepth>,
    limits: &Limits,
    batch: &mut ImportBatch,
) -> Result<Vec<ImportRef>, ImportError> {
    limits.check_depth(depth)?;
    let mut members = Vec::new();
    let children = tree
        .get(group)
        .map(|g| g.children.as_slice())
        .unwrap_or_default();
    for child in children {
        let Some(node) = tree.get(*child) else {
            continue;
        };
        match &node.kind {
            SvgKind::Group(_) => {
                let child_members = walk_group(tree, *child, depth.incremented(), limits, batch)?;
                match child_members.len() {
                    // A group of one member, or of none, is not a group the
                    // board can rebuild. Its children keep their geometry and
                    // the structure the drawing had is reported lost
                    // (`TOILE-IO-079`).
                    0 => batch.report.record(ToileConstruct::Group, Outcome::Dropped),
                    1 => {
                        // The member lands, and the group around it does not.
                        batch
                            .report
                            .record(ToileConstruct::Group, Outcome::Approximated);
                        members.extend(child_members);
                    }
                    _ => {
                        let group_idx = Pos::<ImportGroupRank>::from_usize(batch.groups.len());
                        batch.groups.push(ImportGroup {
                            members: child_members,
                        });
                        members.push(ImportRef::Group(group_idx));
                    }
                }
            }
            SvgKind::Path(p) => members.push(import_path(p, limits, batch)?),
            SvgKind::Text(t) => {
                if let Some(r) = import_text_node(t, limits, batch)? {
                    members.push(r);
                }
            }
            SvgKind::Image(img) => {
                if let Some(r) = import_image(img, limits, batch)? {
                    members.push(r);
                }
            }
        }
    }
    Ok(members)
}

/// Map a path to a `Shape` element. Axis-aligned rectangles become
/// `Shape(Rect)`; everything else keeps its exact geometry as `Shape(Path)`.
fn import_path(
    path: &SvgPath,
    limits: &Limits,
    batch: &mut ImportBatch,
) -> Result<ImportRef, ImportError> {
    limits.check_element_count(Count::<Elements>::of(batch.commands.len()))?;

    let bez = to_bezpath(&path.segments, &path.transform, limits);
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

/// Map an `<image>` to an `Image` element plus a plaintext [`MediaBlob`].
/// Only raster PNG/JPEG are decoded; nested SVG / GIF / WebP are noted and
/// skipped in Phase 0.
fn import_image(
    image: &SvgImage,
    limits: &Limits,
    batch: &mut ImportBatch,
) -> Result<Option<ImportRef>, ImportError> {
    limits.check_element_count(Count::<Elements>::of(batch.commands.len()))?;

    // The blob's slot is its rank in the batch's media, pushed below.
    let media = MediaRef::Pending(Pos::<ImportSlot>::from_usize(batch.media.len()));
    // A WebP picture arrives decoded, and is written as PNG for the board. A
    // JPEG picture arrives with its thumbnail decoded by the module, since
    // Main links no JPEG decoder (`TOILE-IO-131`).
    let blob = match &image.picture {
        SvgPicture::Png(d) => decode_media(d.as_bytes(), limits, media.clone())?,
        SvgPicture::WebpPixels(pixels) => {
            let png = super::raster::png_of(pixels)?;
            decode_media(png.as_bytes(), limits, media.clone())?
        }
        SvgPicture::Jpeg(jpeg) => super::raster::jpeg_blob(
            jpeg.data.clone(),
            jpeg.size,
            &jpeg.thumb,
            limits,
            media.clone(),
        )?,
        SvgPicture::JpegRefused(words) => return Err(ImportError::Decode(words.clone())),
        SvgPicture::Gif | SvgPicture::Webp => {
            batch
                .report
                .record(ToileConstruct::UndecodedPicture, Outcome::Dropped);
            return Ok(None);
        }
        SvgPicture::Svg => {
            batch
                .report
                .record(ToileConstruct::NestedSvg, Outcome::Dropped);
            return Ok(None);
        }
    };

    let rect = limits.clamp_rect(rect_of(image.bounds));
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
) -> Pos<ImportElement> {
    let slot = Pos::<ImportElement>::from_usize(batch.commands.len());
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

/// Convert a path (local coords) to a world-space `kurbo::BezPath`, applying
/// the node's absolute transform and clamping every point.
fn to_bezpath(segments: &[SvgSegment], tf: &SvgTransform, limits: &Limits) -> BezPath {
    let map =
        |p: SvgPoint| -> Point { limits.clamp_point(apply(tf, f64::from(p.x), f64::from(p.y))) };
    let mut bez = BezPath::new();
    for seg in segments {
        match *seg {
            SvgSegment::MoveTo(p) => bez.move_to(map(p)),
            SvgSegment::LineTo(p) => bez.line_to(map(p)),
            SvgSegment::QuadTo(a, b) => bez.quad_to(map(a), map(b)),
            SvgSegment::CubicTo(a, b, c) => bez.curve_to(map(a), map(b), map(c)),
            SvgSegment::Close => bez.close_path(),
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
    ambre_core::order::sort_partial(&mut vs);
    vs
}

/// Build an [`ElementStyle`] from a path's fill/stroke. Non-solid paints
/// (gradients, patterns) are not representable in `ElementStyle`; they are dropped
/// and noted in the report (the image-fallback rasterization is deferred).
fn style_from_path(path: &SvgPath, batch: &mut ImportBatch) -> ElementStyle {
    let mut style = ElementStyle {
        fill: None,
        stroke: None,
        stroke_w: Px::<StoredBoard>::ZERO,
        ..ElementStyle::default()
    };
    if let Some(fill) = path.fill {
        match paint_rgba(fill) {
            Some(rgba) => style.fill = Some(rgba),
            None => batch
                .report
                .record(ToileConstruct::NonSolidPaint, Outcome::Dropped),
        }
    }
    if let Some(stroke) = path.stroke {
        match paint_rgba(stroke.paint) {
            Some(rgba) => {
                style.stroke = Some(rgba);
                style.stroke_w = Px::<StoredBoard>::new(stroke.width);
            }
            None => batch
                .report
                .record(ToileConstruct::NonSolidPaint, Outcome::Dropped),
        }
    }
    style
}

/// A solid paint as packed RGBA (`ElementStyle` color encoding), or `None` for a
/// gradient / pattern paint.
fn paint_rgba(paint: SvgPaint) -> Option<Rgba> {
    match paint {
        SvgPaint::Solid { color, opacity } => {
            let opacity = ambre_core::Bounded::<ambre_core::Opacity>::clamped(opacity);
            Some(color.to_rgba(opacity.fade(ambre_core::Alpha::OPAQUE)))
        }
        SvgPaint::Other => None,
    }
}

/// Convert absolute bounds to a `kurbo::Rect`.
fn rect_of(r: SvgRect) -> Rect {
    Rect::new(
        f64::from(r.left),
        f64::from(r.top),
        f64::from(r.right),
        f64::from(r.bottom),
    )
}
