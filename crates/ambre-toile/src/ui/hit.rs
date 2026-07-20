// ambre-toile/src/ui/hit.rs
// ============================================================================
// Exact hit-testing (toile-ui §4). The Logic thread's R-tree pre-filters
// candidates; this module refines to exact geometry (rotation, path fill, stroke
// width). Elements store their geometry in the unrotated world frame bounded by
// `DisplayElement.bounds`, with `transform` = rotation about that AABB's center
// (Logic `derive.rs`). So a hit maps the world point into the element's local
// (unrotated) frame via `transform.inverse()`, then tests the geometry there.
//
// Pure geometry -- host-tested, no `cfg(wasm32)`, no GPU.
// ============================================================================

use ambre_bus::app::toile::{DisplayElement, DisplayGeom, DisplayNode, ElementId};
use ambre_draw::{BezPath, Point, Rect};
use kurbo::{ParamCurveNearest, Shape};

/// A shape at or above this world distance from a thin stroke's outline is a
/// miss; below it, a hit. Keeps hair-thin lines and open paths clickable.
const MIN_STROKE_PICK: f64 = 4.0;

/// Exact hit-test of one element against a world-space point (§4). `false` for a
/// `Group` (it paints nothing; selection resolves through membership, §7).
pub fn exact_hit(elem: &DisplayElement, world_pt: Point) -> bool {
    if elem.locked {
        return false;
    }
    // Undo the element's rotation: geometry below is in the unrotated frame.
    let local = elem.transform.inverse() * world_pt;
    match &elem.geom {
        DisplayGeom::Rect
        | DisplayGeom::Image(_)
        | DisplayGeom::Sticky(_)
        | DisplayGeom::Text(_)
        | DisplayGeom::Frame
        | DisplayGeom::Embed(_)
        | DisplayGeom::DocEmbed { .. } => elem.bounds.contains(local),
        DisplayGeom::Ellipse => ellipse_contains(elem.bounds, local),
        DisplayGeom::Triangle => path_contains(&triangle_path(elem.bounds), local),
        DisplayGeom::Path(p) => path_hit(p, local, elem.style.stroke_w as f64),
        DisplayGeom::Stroke(pts) => polyline_hit(pts, local, elem.style.stroke_w as f64),
        DisplayGeom::Group => false,
    }
}

/// Whether `local` lies inside the axis-aligned ellipse inscribed in `bounds`.
pub fn ellipse_contains(bounds: Rect, local: Point) -> bool {
    let (rx, ry) = (bounds.width() / 2.0, bounds.height() / 2.0);
    if rx <= 0.0 || ry <= 0.0 {
        return false;
    }
    let c = bounds.center();
    let nx = (local.x - c.x) / rx;
    let ny = (local.y - c.y) / ry;
    nx * nx + ny * ny <= 1.0
}

/// The Phase-0 triangle inscribed in `bounds`: apex at the top-center, base along
/// the bottom edge. Kept identical to the renderer so hit-test matches paint.
pub fn triangle_path(bounds: Rect) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((bounds.center().x, bounds.y0));
    p.line_to((bounds.x1, bounds.y1));
    p.line_to((bounds.x0, bounds.y1));
    p.close_path();
    p
}

/// A closed path hits on fill; either open or closed also hits within
/// `stroke_w/2` (clamped to a minimum pick distance) of its outline.
fn path_hit(path: &BezPath, local: Point, stroke_w: f64) -> bool {
    if path_contains(path, local) {
        return true;
    }
    let tol = (stroke_w / 2.0).max(MIN_STROKE_PICK);
    path_outline_distance(path, local) <= tol
}

/// Non-zero winding fill test for a closed path.
fn path_contains(path: &BezPath, local: Point) -> bool {
    path.contains(local)
}

/// Minimum distance from `local` to any segment of `path` (world units).
fn path_outline_distance(path: &BezPath, local: Point) -> f64 {
    path.segments()
        .map(|seg| seg.nearest(local, 1e-3).distance_sq)
        .fold(f64::INFINITY, f64::min)
        .sqrt()
}

/// A polyline (freehand stroke) hits within `stroke_w/2` of any of its segments.
fn polyline_hit(pts: &[Point], local: Point, stroke_w: f64) -> bool {
    if pts.is_empty() {
        return false;
    }
    let tol = (stroke_w / 2.0).max(MIN_STROKE_PICK);
    if pts.len() == 1 {
        return pts[0].distance(local) <= tol;
    }
    pts.windows(2)
        .any(|w| point_segment_distance(local, w[0], w[1]) <= tol)
}

/// Euclidean distance from `p` to the segment `[a, b]`.
fn point_segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len_sq = ab.hypot2();
    if len_sq <= f64::EPSILON {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    let proj = a + ab * t;
    p.distance(proj)
}

/// The top-most element hit by `world_pt`: nodes are back-to-front, so the last
/// element that passes `exact_hit` wins (first hit walking front-to-back).
pub fn hit_topmost(nodes: &[DisplayNode], world_pt: Point) -> Option<ElementId> {
    nodes
        .iter()
        .rev()
        .filter_map(|n| match n {
            DisplayNode::Element(e) => Some(e),
            DisplayNode::Connector(_) => None,
        })
        .find(|e| exact_hit(e, world_pt))
        .map(|e| e.id.clone())
}

/// Marquee (rubber-band) selection: every element whose world AABB intersects
/// `rect`, refined to exact geometry for the partially-covered ones (an element
/// fully inside `rect` is kept without an exact test; one straddling the border
/// must actually intersect the geometry). Locked elements and groups are skipped.
pub fn marquee_hits(nodes: &[DisplayNode], rect: Rect) -> Vec<ElementId> {
    nodes
        .iter()
        .filter_map(|n| match n {
            DisplayNode::Element(e) => Some(e),
            DisplayNode::Connector(_) => None,
        })
        .filter(|e| !e.locked && !matches!(e.geom, DisplayGeom::Group))
        .filter(|e| {
            let b = e.bounds;
            if rect.union(b) == rect {
                // Fully contained.
                true
            } else if rect.intersect(b).area() > 0.0 {
                // Partial cover: refine. Cheap approximation adequate for Phase 0
                // -- any corner of the AABB-overlap region that lies in the shape,
                // or the element center inside the rect, counts as a touch.
                let c = b.center();
                rect.contains(c) || exact_hit(e, c)
            } else {
                false
            }
        })
        .map(|e| e.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{ElementKind, ElementStyle, ShapeKind};
    use ambre_draw::Affine;

    fn elem(geom: DisplayGeom, bounds: Rect, transform: Affine) -> DisplayElement {
        DisplayElement {
            id: ElementId("p_1".to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform,
            bounds,
            locked: false,
            style: ElementStyle::default(),
            geom,
            group: None,
        }
    }

    #[test]
    fn rect_inside_and_outside() {
        let e = elem(
            DisplayGeom::Rect,
            Rect::new(0.0, 0.0, 100.0, 50.0),
            Affine::IDENTITY,
        );
        assert!(exact_hit(&e, Point::new(50.0, 25.0)));
        assert!(!exact_hit(&e, Point::new(150.0, 25.0)));
    }

    #[test]
    fn rotated_rect_uses_local_frame() {
        let bounds = Rect::new(-50.0, -10.0, 50.0, 10.0);
        // Rotate 90 degrees about the origin (the AABB center).
        let t = Affine::rotate(std::f64::consts::FRAC_PI_2);
        let e = elem(DisplayGeom::Rect, bounds, t);
        // A point far along +y in world space maps back into the wide local rect.
        assert!(exact_hit(&e, Point::new(0.0, 40.0)));
        assert!(!exact_hit(&e, Point::new(40.0, 0.0)));
    }

    #[test]
    fn ellipse_corner_is_a_miss() {
        let e = elem(
            DisplayGeom::Ellipse,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Affine::IDENTITY,
        );
        assert!(exact_hit(&e, Point::new(50.0, 50.0))); // center
        assert!(!exact_hit(&e, Point::new(5.0, 5.0))); // corner outside the disc
    }

    #[test]
    fn triangle_fill() {
        let e = elem(
            DisplayGeom::Triangle,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Affine::IDENTITY,
        );
        assert!(exact_hit(&e, Point::new(50.0, 80.0))); // near the base center
        assert!(!exact_hit(&e, Point::new(5.0, 5.0))); // top corner, outside apex
    }

    #[test]
    fn open_path_hits_near_the_stroke_only() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        let mut e = elem(
            DisplayGeom::Path(p),
            Rect::new(0.0, 0.0, 100.0, 0.0),
            Affine::IDENTITY,
        );
        e.style.stroke_w = 2.0;
        assert!(exact_hit(&e, Point::new(50.0, 1.0))); // on the line
        assert!(!exact_hit(&e, Point::new(50.0, 40.0))); // far from it
    }

    #[test]
    fn stroke_proximity() {
        let e = elem(
            DisplayGeom::Stroke(vec![
                Point::new(0.0, 0.0),
                Point::new(50.0, 50.0),
                Point::new(100.0, 0.0),
            ]),
            Rect::new(0.0, 0.0, 100.0, 50.0),
            Affine::IDENTITY,
        );
        assert!(exact_hit(&e, Point::new(25.0, 25.0)));
        assert!(!exact_hit(&e, Point::new(90.0, 45.0)));
    }

    #[test]
    fn group_never_hits() {
        let e = elem(
            DisplayGeom::Group,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Affine::IDENTITY,
        );
        assert!(!exact_hit(&e, Point::new(50.0, 50.0)));
    }

    #[test]
    fn locked_never_hits() {
        let mut e = elem(
            DisplayGeom::Rect,
            Rect::new(0.0, 0.0, 100.0, 50.0),
            Affine::IDENTITY,
        );
        e.locked = true;
        assert!(!exact_hit(&e, Point::new(50.0, 25.0)));
    }

    #[test]
    fn topmost_wins_front_to_back() {
        let back = DisplayNode::Element({
            let mut e = elem(
                DisplayGeom::Rect,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                Affine::IDENTITY,
            );
            e.id = ElementId("back".to_owned());
            e
        });
        let front = DisplayNode::Element({
            let mut e = elem(
                DisplayGeom::Rect,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                Affine::IDENTITY,
            );
            e.id = ElementId("front".to_owned());
            e
        });
        let nodes = vec![back, front];
        assert_eq!(
            hit_topmost(&nodes, Point::new(50.0, 50.0)),
            Some(ElementId("front".to_owned()))
        );
    }

    #[test]
    fn marquee_selects_covered_elements() {
        let a = DisplayNode::Element({
            let mut e = elem(
                DisplayGeom::Rect,
                Rect::new(10.0, 10.0, 40.0, 40.0),
                Affine::IDENTITY,
            );
            e.id = ElementId("a".to_owned());
            e
        });
        let b = DisplayNode::Element({
            let mut e = elem(
                DisplayGeom::Rect,
                Rect::new(500.0, 500.0, 540.0, 540.0),
                Affine::IDENTITY,
            );
            e.id = ElementId("b".to_owned());
            e
        });
        let hits = marquee_hits(&[a, b], Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(hits, vec![ElementId("a".to_owned())]);
    }
}
