//! SVG import (`toile-interop` §2.A).
//!
//! `usvg` parses the document into a normalized render tree; we walk it and map
//! each node to a Toile `CreateElement`. usvg flattens every primitive
//! (`<rect>`, `<circle>`, `<line>`, ...) into a `Path`, so vector nodes map to
//! `Shape(Path)` carrying a `kurbo::BezPath`; an axis-aligned rectangle contour
//! is recognized back into `Shape(Rect)` (the common, safe case). `<image>` maps
//! to an `Image` element plus a plaintext [`MediaBlob`] (§5), and `<g>` becomes
//! an [`ImportGroup`] rebuilt after apply -- see the note on groups below.
//!
//! **Text (why not usvg).** usvg only exposes `<text>` by flattening it to glyph
//! *paths*, which needs a font database -- impossible in the browser (no system
//! fonts) and destructive (it would lose the editable string). So the usvg font
//! stack is disabled and `<text>` is read directly from the SVG source with
//! `roxmltree`: the raw body plus its `x`/`y` become a `Text` element that Toile
//! renders through its own Parley stack. Phase 0 reads top-level `x`/`y` only (no
//! transform / `tspan` offset resolution), noted where it matters.
//!
//! **Groups (§2.A).** The spec maps `<g>` to a Toile `group`. A group is created
//! only via `GroupElements { ids }`, which needs the `ElementId`s the engine mints
//! *after* `CreateElement` is applied -- ids an offline converter cannot know. So
//! the converter speaks in slots: it imports the children as elements (the group's
//! transform baked into their geometry via `abs_transform` / abs bounds, so
//! geometry stays world-space) and records an [`ImportGroup`] whose members are the
//! slots (and nested groups) it contains. Groups are pushed **innermost-first**
//! (post-order) so a nested member `Group(i)` always refers to an already-resolved
//! entry. The apply pass mints the ids and issues `GroupElements` in the same undo
//! group, reconstructing the hierarchy. No group is flattened or reported as lost.
//!
//! All geometry is clamped to the world bound and the element/depth counts are
//! enforced while walking (§2.D); any breach aborts the whole import atomically
//! (§1) -- the returned `Err` carries no commands.

use kurbo::{BezPath, PathEl, Point, Rect, Shape, Size};
use usvg::tiny_skia_path::PathSegment;

use super::raster::{decode_media, pending_hash};
use super::{ImportBatch, ImportError, ImportGroup, ImportRef, Limits};
use crate::logic::command::{InitContent, ToileCommand};
use crate::logic::model::{ElementKind, ElementStyle, ShapeKind};

/// Parse SVG `bytes` and convert to an atomic [`ImportBatch`] (§2.A). Returns
/// `Err` (and no commands) on a parse failure, an exceeded safety limit, or an
/// SVG that maps to zero elements (§1 atomicity).
pub fn import_svg(bytes: &[u8], limits: &Limits) -> Result<ImportBatch, ImportError> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &opt).map_err(|e| ImportError::Parse(e.to_string()))?;

    let mut batch = ImportBatch::default();
    let mut media_counter = 0usize;
    // The document root is not itself a `<g>`; its direct-member refs are discarded
    // (only nested `<g>` become groups). Nested groups are pushed during the walk.
    walk_group(tree.root(), 0, limits, &mut batch, &mut media_counter)?;

    // Text is read from the source (see the module note), not the usvg tree.
    if let Ok(source) = std::str::from_utf8(bytes) {
        import_text_nodes(source, limits, &mut batch)?;
    }

    if batch.commands.is_empty() {
        return Err(ImportError::Empty);
    }
    batch.report.imported = batch.commands.len();
    Ok(batch)
}

/// Extract every `<text>` element from the SVG source via `roxmltree` and map it
/// to a `Text` `CreateElement`. usvg cannot give us editable text (see the module
/// note), so this direct pass preserves the string. A malformed document yields no
/// text (the vector import already succeeded); the count/limit bound still applies.
fn import_text_nodes(
    source: &str,
    limits: &Limits,
    batch: &mut ImportBatch,
) -> Result<(), ImportError> {
    let doc = match roxmltree::Document::parse(source) {
        Ok(d) => d,
        Err(_) => return Ok(()),
    };
    for node in doc.descendants().filter(|n| n.has_tag_name("text")) {
        // Concatenate only the actual text children (avoid double-counting the
        // element node's own text accessor).
        let body: String = node
            .descendants()
            .filter(|d| d.is_text())
            .filter_map(|d| d.text())
            .collect();
        let body = body.trim().to_owned();
        if body.is_empty() {
            continue;
        }
        limits.check_element_count(batch.commands.len())?;
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
        let rect = limits.clamp_rect(Rect::new(x, top, x + w, top + size * 1.4));
        push_element(
            batch,
            ElementKind::Text,
            rect,
            InitContent::Text(body),
            None,
        );
    }
    Ok(())
}

/// Recursively walk a usvg group, appending commands, and return the
/// [`ImportRef`]s of the elements/subgroups that are this group's direct members
/// (in child order). `depth` guards nesting; `media_counter` mints the placeholder
/// token for each `<image>`. A nested `<g>` recurses first, then pushes its
/// [`ImportGroup`] (post-order = innermost-first, so a `Group(i)` member always
/// refers to an already-pushed entry) and contributes a `Group` ref here.
fn walk_group(
    group: &usvg::Group,
    depth: usize,
    limits: &Limits,
    batch: &mut ImportBatch,
    media_counter: &mut usize,
) -> Result<Vec<ImportRef>, ImportError> {
    limits.check_depth(depth)?;
    let mut members = Vec::new();
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => {
                let child_members = walk_group(g, depth + 1, limits, batch, media_counter)?;
                // An empty `<g>` (all children skipped) produces no group.
                if !child_members.is_empty() {
                    let group_idx = batch.groups.len();
                    batch.groups.push(ImportGroup {
                        members: child_members,
                    });
                    members.push(ImportRef::Group(group_idx));
                }
            }
            usvg::Node::Path(p) => members.push(import_path(p, limits, batch)?),
            // Text is handled by the roxmltree pass (see module note); the font
            // stack is disabled so usvg never yields a Text node here.
            usvg::Node::Text(_) => {}
            usvg::Node::Image(img) => {
                if let Some(r) = import_image(img, limits, batch, media_counter)? {
                    members.push(r);
                }
            }
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
    limits.check_element_count(batch.commands.len())?;

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
/// Only raster PNG/JPEG are decoded (§2.B); nested SVG / GIF / WebP are noted and
/// skipped in Phase 0.
fn import_image(
    image: &usvg::Image,
    limits: &Limits,
    batch: &mut ImportBatch,
    media_counter: &mut usize,
) -> Result<Option<ImportRef>, ImportError> {
    limits.check_element_count(batch.commands.len())?;

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

    let hash = pending_hash(*media_counter);
    let blob = decode_media(raw, limits, hash.clone())?;
    *media_counter += 1;

    let rect = limits.clamp_rect(rect_from_usvg(image.abs_bounding_box()));
    let slot = push_element(
        batch,
        ElementKind::Image,
        rect,
        InitContent::Image(hash.0),
        None,
    );
    batch.media.push(blob);
    Ok(Some(ImportRef::Slot(slot)))
}

/// Append one `CreateElement` for an element occupying `rect`, plus its imported
/// style (index-aligned into `batch.styles`), and return the slot index the apply
/// pass will resolve to a freshly minted `ElementId`.
///
/// Style rides a separate `styles` slot rather than `CreateElement` because the
/// CRDT stores `ElementStyle` as an independent LWW register (§2.E): the apply
/// pass creates the element, then restyles it inside the same undo group. `None`
/// keeps the engine default (used for text/image, which import unstyled).
fn push_element(
    batch: &mut ImportBatch,
    kind: ElementKind,
    rect: Rect,
    content: InitContent,
    style: Option<ElementStyle>,
) -> usize {
    let slot = batch.commands.len();
    batch.commands.push(ToileCommand::CreateElement {
        kind,
        at: rect.origin(),
        size: Size::new(rect.width(), rect.height()),
        content,
    });
    batch.styles.push(style);
    slot
}

// -----------------------------------------------------------------------------
// Geometry + style helpers
// -----------------------------------------------------------------------------

/// Convert a tiny-skia path (local coords) to a world-space `kurbo::BezPath`,
/// applying the node's absolute transform and clamping every point (§2.D).
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

/// Recognize a closed, axis-aligned rectangle contour (the usvg flattening of a
/// `<rect>`): a single subpath of straight edges whose corners take exactly two
/// distinct x and two distinct y values. Returns its bounds, else `None`.
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
    if xs.len() != 2 || ys.len() != 2 {
        return None;
    }
    Some(Rect::new(xs[0], ys[0], xs[1], ys[1]))
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
/// and noted in the report (the §2.A image-fallback rasterization is deferred).
fn style_from_path(path: &usvg::Path, batch: &mut ImportBatch) -> ElementStyle {
    let mut style = ElementStyle {
        fill: None,
        stroke: None,
        stroke_w: 0.0,
        ..ElementStyle::default()
    };
    if let Some(fill) = path.fill() {
        match paint_argb(fill.paint(), fill.opacity().get()) {
            Some(argb) => style.fill = Some(argb),
            None => batch.report.skip("path fill: non-solid paint dropped"),
        }
    }
    if let Some(stroke) = path.stroke() {
        match paint_argb(stroke.paint(), stroke.opacity().get()) {
            Some(argb) => {
                style.stroke = Some(argb);
                style.stroke_w = stroke.width().get();
            }
            None => batch.report.skip("path stroke: non-solid paint dropped"),
        }
    }
    style
}

/// A solid paint as packed ARGB (`ElementStyle` color encoding), or `None` for a
/// gradient / pattern paint.
fn paint_argb(paint: &usvg::Paint, opacity: f32) -> Option<u32> {
    match paint {
        usvg::Paint::Color(c) => {
            let a = (opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
            Some(
                (a << 24)
                    | (u32::from(c.red) << 16)
                    | (u32::from(c.green) << 8)
                    | u32::from(c.blue),
            )
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
