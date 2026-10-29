// ambre-toile/src/ui/hit.rs
// ============================================================================
// Exact hit-testing (toile-selection). The Logic thread's R-tree pre-filters
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

use ambre_core::ViewPx;

use super::viewport::{BoardLength, MIN_ZOOM, ViewportTransform};

/// The screen distance within which a pointer reaches a thin stroke's
/// outline. Keeps hair-thin lines and open paths clickable (`TOILE-UI-225`).
pub(crate) const PICK_REACH: ViewPx = ViewPx::new(4.0);

/// [`PICK_REACH`] in world units under `cam`: the reach of a pointer at this
/// zoom. Every pointer hit test takes it, so the reach holds on screen. A zoom
/// below the camera's floor reads as the floor.
pub fn pick_floor(cam: &ViewportTransform) -> BoardLength {
    let floored = ViewportTransform {
        zoom: if cam.zoom < MIN_ZOOM {
            MIN_ZOOM
        } else {
            cam.zoom
        },
        ..*cam
    };
    floored.to_board(PICK_REACH)
}

/// Exact hit-test of one element against a world-space point. `pick` is the
/// floor of a stroke's reach, in world units (see [`pick_floor`]). `false`
/// for a `Group` (it paints nothing; selection resolves through membership,).
pub fn exact_hit(elem: &DisplayElement, world_pt: Point, pick: BoardLength) -> bool {
    if elem.locked == ambre_bus::app::toile::LockState::Locked {
        return false;
    }
    // Undo the element's rotation: geometry below is in the unrotated frame.
    let local = elem.transform.inverse() * world_pt;
    match &elem.geom {
        DisplayGeom::Rect
        | DisplayGeom::Image(_)
        | DisplayGeom::Sticky(_)
        | DisplayGeom::Text(_)
        | DisplayGeom::Frame(_)
        | DisplayGeom::Embed(_)
        | DisplayGeom::DocEmbed { .. } => elem.bounds.contains(local),
        DisplayGeom::Ellipse => ellipse_contains(elem.bounds, local),
        DisplayGeom::Triangle => path_contains(&triangle_path(elem.bounds), local),
        DisplayGeom::Path(p) => path_hit(p, local, stroke_reach(stroke_width(&elem.style), pick)),
        DisplayGeom::Stroke(pts) => {
            polyline_hit(pts, local, stroke_reach(stroke_width(&elem.style), pick))
        }
        DisplayGeom::Group => false,
    }
}

/// The frame whose title band holds the screen point `screen_pt`, top-most
/// first (`TOILE-254`).
///
/// A frame's title sits outside its box, so the ordinary hit test cannot see
/// it: this is what makes a title clickable. Screen space, because that is
/// where the band is drawn.
pub fn frame_title_at(
    nodes: &[DisplayNode],
    screen_pt: Point,
    cam: &ViewportTransform,
) -> Option<ElementId> {
    // Reverse paint order: the band drawn last is the one on top, so it takes
    // the press where two overlap.
    super::render::frame_bands(nodes, cam)
        .filter(|(_, band, _)| band.contains(screen_pt))
        .last()
        .map(|(e, _, _)| e.id.clone())
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

/// The triangle the walk paints, so hit-test matches paint.
pub use super::walk::triangle_path;

/// How far from its outline a stroke of width `stroke_w` is hit: half its
/// width, and never less than the pointer's reach `pick`.
pub(crate) fn stroke_reach(stroke_w: BoardLength, pick: BoardLength) -> BoardLength {
    (stroke_w / 2.0).max(pick)
}

/// The stroke width of a style, as a board length. The style keeps the `f32`
/// the bus and the document store carry.
pub(crate) fn stroke_width(style: &ambre_bus::app::toile::ElementStyle) -> BoardLength {
    BoardLength::from(style.stroke_w)
}

/// A closed path hits on fill; either open or closed also hits within `tol`
/// of its outline.
fn path_hit(path: &BezPath, local: Point, tol: BoardLength) -> bool {
    if path_contains(path, local) {
        return true;
    }
    path_outline_distance(path, local) <= tol
}

/// Non-zero winding fill test for a closed path.
fn path_contains(path: &BezPath, local: Point) -> bool {
    path.contains(local)
}

/// Minimum distance from `local` to any segment of `path`.
pub(crate) fn path_outline_distance(path: &BezPath, local: Point) -> BoardLength {
    BoardLength::new(
        path.segments()
            .map(|seg| seg.nearest(local, 1e-3).distance_sq)
            .fold(f64::INFINITY, f64::min)
            .sqrt(),
    )
}

/// A polyline (freehand stroke) hits within `tol` of any of its segments.
fn polyline_hit(pts: &[Point], local: Point, tol: BoardLength) -> bool {
    match pts {
        [] => false,
        [only] => BoardLength::new(only.distance(local)) <= tol,
        _ => pts
            .windows(2)
            .any(|w| matches!(w, &[a, b] if point_segment_distance(local, a, b) <= tol)),
    }
}

/// Euclidean distance from `p` to the segment `[a, b]`, in the element's
/// local board units.
fn point_segment_distance(p: Point, a: Point, b: Point) -> BoardLength {
    let ab = b - a;
    let len_sq = ab.hypot2();
    if len_sq <= f64::EPSILON {
        return BoardLength::new(p.distance(a));
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    let proj = a + ab * t;
    BoardLength::new(p.distance(proj))
}

/// The top-most element hit by `world_pt`: nodes are back-to-front, so the last
/// element that passes `exact_hit` wins (first hit walking front-to-back).
/// `pick` is the pointer's reach in world units (see [`pick_floor`]).
pub fn hit_topmost(nodes: &[DisplayNode], world_pt: Point, pick: BoardLength) -> Option<ElementId> {
    nodes
        .iter()
        .rev()
        .filter_map(|n| match n {
            DisplayNode::Element(e) => Some(e),
            DisplayNode::Connector(_) => None,
        })
        .find(|e| exact_hit(e, world_pt, pick))
        .map(|e| e.id.clone())
}

/// The world-space upright box of an element's **rotated** shape, for marquee
/// containment (`TOILE-UI-237`). `bounds` is the unrotated rect and `transform`
/// rotates it about its center; the box the user sees is the rotated one.
fn rotated_aabb(e: &DisplayElement) -> Rect {
    let b = e.bounds;
    let corners = [
        Point::new(b.x0, b.y0),
        Point::new(b.x1, b.y0),
        Point::new(b.x1, b.y1),
        Point::new(b.x0, b.y1),
    ];
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for c in corners {
        let p = e.transform * c;
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    Rect::new(min_x, min_y, max_x, max_y)
}

/// Marquee (rubber-band) selection: every element whose world AABB intersects
/// `rect`, refined to exact geometry for the partially-covered ones. An element
/// fully inside `rect` is kept without an exact test; one straddling the border
/// must actually intersect the geometry. Locked elements and groups are skipped.
/// Containment reads the **rotated** box, the shape the user sees
/// (`TOILE-UI-237`), not the unrotated `bounds`.
pub fn marquee_hits(nodes: &[DisplayNode], rect: Rect) -> Vec<ElementId> {
    nodes
        .iter()
        .filter_map(|n| match n {
            DisplayNode::Element(e) => Some(e),
            DisplayNode::Connector(_) => None,
        })
        .filter(|e| {
            e.locked == ambre_bus::app::toile::LockState::Unlocked
                && !matches!(e.geom, DisplayGeom::Group)
        })
        .filter(|e| {
            let b = rotated_aabb(e);
            if rect.union(b) == rect {
                // Fully contained.
                true
            } else if rect.intersect(b).area() > 0.0 {
                // Partial cover: refine. Cheap approximation adequate for Phase 0:
                // any corner of the AABB-overlap region that lies in the shape
                // counts as a touch. So does the element center inside the rect.
                let c = b.center();
                // The box's own geometry decides, with no pointer reach.
                rect.contains(c) || exact_hit(e, c, BoardLength::ZERO)
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

    /// A hit under the camera at zoom 1.
    fn hit(e: &DisplayElement, p: Point) -> bool {
        exact_hit(e, p, pick_floor(&ViewportTransform::default()))
    }

    fn elem(geom: DisplayGeom, bounds: Rect, transform: Affine) -> DisplayElement {
        DisplayElement {
            id: ElementId("p_1".to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform,
            bounds,
            locked: ambre_bus::app::toile::LockState::Unlocked,
            style: ElementStyle::default(),
            geom,
            link: None,
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
        assert!(hit(&e, Point::new(50.0, 25.0)));
        assert!(!hit(&e, Point::new(150.0, 25.0)));
    }

    #[test]
    fn rotated_rect_uses_local_frame() {
        let bounds = Rect::new(-50.0, -10.0, 50.0, 10.0);
        // Rotate 90 degrees about the origin (the AABB center).
        let t = Affine::rotate(std::f64::consts::FRAC_PI_2);
        let e = elem(DisplayGeom::Rect, bounds, t);
        // A point far along +y in world space maps back into the wide local rect.
        assert!(hit(&e, Point::new(0.0, 40.0)));
        assert!(!hit(&e, Point::new(40.0, 0.0)));
    }

    #[test]
    fn ellipse_corner_is_a_miss() {
        let e = elem(
            DisplayGeom::Ellipse,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Affine::IDENTITY,
        );
        assert!(hit(&e, Point::new(50.0, 50.0))); // center
        assert!(!hit(&e, Point::new(5.0, 5.0))); // corner outside the disc
    }

    #[test]
    fn triangle_fill() {
        let e = elem(
            DisplayGeom::Triangle,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Affine::IDENTITY,
        );
        assert!(hit(&e, Point::new(50.0, 80.0))); // near the base center
        assert!(!hit(&e, Point::new(5.0, 5.0))); // top corner, outside apex
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
        e.style.stroke_w = ambre_bus::app::toile::StyleLength::new(2.0);
        assert!(hit(&e, Point::new(50.0, 1.0))); // on the line
        assert!(!hit(&e, Point::new(50.0, 40.0))); // far from it
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
        assert!(hit(&e, Point::new(25.0, 25.0)));
        assert!(!hit(&e, Point::new(90.0, 45.0)));
    }

    /// `TOILE-UI-225`: a hairline is hit three screen pixels off its line and
    /// missed six pixels off, at every zoom the camera allows.
    #[test]
    fn a_hairline_is_hit_within_the_same_screen_reach_at_every_zoom() {
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((1000.0, 0.0));
        let mut e = elem(
            DisplayGeom::Path(line),
            Rect::new(0.0, 0.0, 1000.0, 0.0),
            Affine::IDENTITY,
        );
        e.style.stroke_w = ambre_bus::app::toile::StyleLength::new(0.5);
        let mut cam = ViewportTransform::default();
        use super::super::viewport::{MAX_ZOOM, MIN_ZOOM};
        for zoom in [MIN_ZOOM.get(), 0.5, 1.0, 4.0, MAX_ZOOM.get()] {
            cam.zoom = ambre_core::ZoomFactor::positive(zoom);
            let pick = pick_floor(&cam);
            // `px` screen pixels off the line, in world units at this zoom.
            let off = |px: f64| Point::new(500.0, px / zoom);
            assert!(exact_hit(&e, off(3.0), pick), "3 px off at zoom {zoom}");
            assert!(!exact_hit(&e, off(6.0), pick), "6 px off at zoom {zoom}");
        }
    }

    #[test]
    fn group_never_hits() {
        let e = elem(
            DisplayGeom::Group,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Affine::IDENTITY,
        );
        assert!(!hit(&e, Point::new(50.0, 50.0)));
    }

    #[test]
    fn locked_never_hits() {
        let mut e = elem(
            DisplayGeom::Rect,
            Rect::new(0.0, 0.0, 100.0, 50.0),
            Affine::IDENTITY,
        );
        e.locked = ambre_bus::app::toile::LockState::Locked;
        assert!(!hit(&e, Point::new(50.0, 25.0)));
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
            hit_topmost(
                &nodes,
                Point::new(50.0, 50.0),
                pick_floor(&ViewportTransform::default())
            ),
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

    /// `TOILE-UI-238`: a marquee takes neither a locked element nor a group
    /// node, even when it covers both whole. The free element beside them is
    /// still taken.
    #[test]
    fn a_marquee_skips_a_locked_element_and_a_group_node() {
        let node = |id: &str, geom: DisplayGeom, locked: ambre_bus::app::toile::LockState| {
            let mut e = elem(geom, Rect::new(10.0, 10.0, 40.0, 40.0), Affine::IDENTITY);
            e.id = ElementId(id.to_owned());
            e.locked = locked;
            DisplayNode::Element(e)
        };
        let nodes = [
            node(
                "locked",
                DisplayGeom::Rect,
                ambre_bus::app::toile::LockState::Locked,
            ),
            node(
                "group",
                DisplayGeom::Group,
                ambre_bus::app::toile::LockState::Unlocked,
            ),
            node(
                "free",
                DisplayGeom::Rect,
                ambre_bus::app::toile::LockState::Unlocked,
            ),
        ];
        let hits = marquee_hits(&nodes, Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(hits, vec![ElementId("free".to_owned())]);
    }

    /// `TOILE-UI-237`: a rotated element is judged by the shape the user sees,
    /// not by the unrotated bounds. A long rect rotated 90 degrees is tall and
    /// thin, so a marquee over the unrotated tip must miss it.
    #[test]
    fn marquee_judges_a_rotated_element_by_its_rotated_box() {
        let tilted = DisplayNode::Element({
            let mut e = elem(
                DisplayGeom::Rect,
                Rect::new(-50.0, -5.0, 50.0, 5.0),
                Affine::rotate(std::f64::consts::FRAC_PI_2),
            );
            e.id = ElementId("tilted".to_owned());
            e
        });
        // Over the unrotated tip (x 40..55): off the rotated shape, a miss.
        let miss = marquee_hits(
            std::slice::from_ref(&tilted),
            Rect::new(40.0, -1.0, 55.0, 1.0),
        );
        assert!(miss.is_empty(), "the rotated shape is not there");
        // Covering the rotated shape (now tall) takes it.
        let hit = marquee_hits(
            std::slice::from_ref(&tilted),
            Rect::new(-10.0, -60.0, 10.0, 60.0),
        );
        assert_eq!(hit, vec![ElementId("tilted".to_owned())]);
    }
}
