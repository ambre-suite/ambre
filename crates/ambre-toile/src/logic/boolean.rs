//! Vector boolean-operation solver (`toile-vector-paths`).
//!
//! Combine the geometry of selected shape / path elements with a boolean rule
//! (union / subtract / intersect / exclude), producing one result path. kurbo
//! (0.13) has **no** path-vs-path boolean ops -- only `PathSeg::intersect_line`
//! -- so this cannot "reuse kurbo booleans" as the spec once claimed. Instead
//! each input `BezPath` is flattened to closed polygon contours and combined via
//! [`i_overlay`], and the polygon result is rebuilt as a line-segment `BezPath`.
//! Curves are lost (the result is a polyline); that is the accepted trade-off of
//! the flatten approach for a whiteboard.
//!
//! This module is **pure and deterministic**: given the same input paths,
//! rule and tolerance it returns a byte-identical result. It runs client-side,
//! off the render frame, within the < 5 ms typical-pair budget.
//!
//! ## Multi-input semantics (pinned; the spec leaves the fold to the solver)
//!
//! Inputs fold **left to right** in the given order, so the accumulator combines
//! with each successive path:
//! - [`BoolOp::Union`] / [`BoolOp::Intersect`] / [`BoolOp::Exclude`]: associative,
//!   so the fold is the natural n-ary union / intersection / symmetric difference.
//! - [`BoolOp::Subtract`]: the first element is the subject; every later element is
//!   removed from it. `((a - b) - c)` equals `a - (b ∪ c)`, so folding difference
//!   left to right matches "subject minus the union of the rest".
//!
//! Fills use the non-zero rule, matching how shapes are painted.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use kurbo::{BezPath, PathEl, Point};

use crate::logic::command::BoolOp;

/// One flattened polygon contour: a closed ring of points.
type Contour = Vec<[f64; 2]>;
/// A shape is a list of contours (outer boundary plus any holes).
type Shape = Vec<Contour>;

/// Flatten tolerance (world units) for turning curves into polylines before the
/// boolean runs. Sub-pixel at typical board zoom; small enough that the result
/// tracks the source outline, large enough to keep the contour point count (and
/// so the solve cost,) bounded.
pub const FLATTEN_TOL: f64 = 0.25;

/// Combine world-space `paths` with a boolean `op`. Each path is flattened
/// to closed contours at `tol`, the paths fold left to right via [`i_overlay`],
/// and the result is rebuilt as a line-segment [`BezPath`]. Returns an empty path
/// when fewer than two paths are given or the geometry fully cancels (the caller
/// treats an empty result as a no-op). Deterministic.
pub fn boolean_op(op: BoolOp, paths: &[BezPath], tol: f64) -> BezPath {
    if paths.len() < 2 {
        return BezPath::new();
    }
    let rule = overlay_rule(op);

    // Seed the accumulator with the first path's contours, then fold each
    // remaining path into it. `overlay` returns `Shapes` (a `Vec<Shape>`), which
    // is itself a valid subject for the next step.
    let mut acc: Vec<Shape> = vec![flatten_to_contours(&paths[0], tol)];
    for path in &paths[1..] {
        let clip = flatten_to_contours(path, tol);
        acc = acc.overlay(&clip, rule, FillRule::NonZero);
    }

    shapes_to_bezpath(&acc)
}

fn overlay_rule(op: BoolOp) -> OverlayRule {
    match op {
        BoolOp::Union => OverlayRule::Union,
        BoolOp::Subtract => OverlayRule::Difference,
        BoolOp::Intersect => OverlayRule::Intersect,
        BoolOp::Exclude => OverlayRule::Xor,
    }
}

/// Flatten a `BezPath` into closed polygon contours. Curves become line segments
/// at `tol`; each subpath becomes one contour. i_overlay auto-closes contours, so
/// open subpaths are combined as if closed (a whiteboard boolean over an open
/// path treats it as its filled region,).
fn flatten_to_contours(path: &BezPath, tol: f64) -> Shape {
    let mut contours: Shape = Vec::new();
    let mut current: Contour = Vec::new();
    let push = |current: &mut Contour, contours: &mut Shape| {
        if current.len() >= 3 {
            contours.push(std::mem::take(current));
        } else {
            current.clear();
        }
    };
    kurbo::flatten(path.iter(), tol, |el| match el {
        PathEl::MoveTo(p) => {
            push(&mut current, &mut contours);
            current.push([p.x, p.y]);
        }
        PathEl::LineTo(p) => current.push([p.x, p.y]),
        PathEl::ClosePath => push(&mut current, &mut contours),
        // `flatten` only yields MoveTo / LineTo / ClosePath.
        _ => {}
    });
    push(&mut current, &mut contours);
    contours
}

/// Rebuild an i_overlay result (`Vec<Shape>`) into a single [`BezPath`]: one
/// closed subpath per contour, across every shape. Deterministic in the order
/// i_overlay returns.
fn shapes_to_bezpath(shapes: &[Shape]) -> BezPath {
    let mut out = BezPath::new();
    for shape in shapes {
        for contour in shape {
            let mut pts = contour.iter();
            let Some(first) = pts.next() else {
                continue;
            };
            out.move_to(Point::new(first[0], first[1]));
            for p in pts {
                out.line_to(Point::new(p[0], p[1]));
            }
            out.close_path();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Rect;

    /// Total signed area magnitude of a flattened path (shoelace per subpath),
    /// used to compare boolean results without depending on vertex order.
    fn path_area(path: &BezPath) -> f64 {
        let mut total = 0.0;
        let mut start = Point::ZERO;
        let mut prev = Point::ZERO;
        let mut acc = 0.0;
        let close = |acc: &mut f64, total: &mut f64| {
            *total += (*acc / 2.0).abs();
            *acc = 0.0;
        };
        for el in path.elements() {
            match el {
                PathEl::MoveTo(p) => {
                    start = *p;
                    prev = *p;
                }
                PathEl::LineTo(p) => {
                    acc += prev.x * p.y - p.x * prev.y;
                    prev = *p;
                }
                PathEl::ClosePath => {
                    acc += prev.x * start.y - start.x * prev.y;
                    close(&mut acc, &mut total);
                    prev = start;
                }
                _ => {}
            }
        }
        close(&mut acc, &mut total);
        total
    }

    fn rect_path(r: Rect) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((r.x0, r.y0));
        p.line_to((r.x1, r.y0));
        p.line_to((r.x1, r.y1));
        p.line_to((r.x0, r.y1));
        p.close_path();
        p
    }

    // Two unit squares overlapping in a 5x10 strip: a=[0,10]x[0,10], b=[5,15]x[0,10].
    fn overlapping() -> (BezPath, BezPath) {
        (
            rect_path(Rect::new(0.0, 0.0, 10.0, 10.0)),
            rect_path(Rect::new(5.0, 0.0, 15.0, 10.0)),
        )
    }

    #[test]
    fn union_area_is_combined_minus_overlap() {
        let (a, b) = overlapping();
        let out = boolean_op(BoolOp::Union, &[a, b], FLATTEN_TOL);
        // 100 + 100 - 50 overlap = 150.
        assert!(
            (path_area(&out) - 150.0).abs() < 1e-6,
            "{}",
            path_area(&out)
        );
    }

    #[test]
    fn intersect_area_is_overlap() {
        let (a, b) = overlapping();
        let out = boolean_op(BoolOp::Intersect, &[a, b], FLATTEN_TOL);
        // The 5x10 overlap strip.
        assert!((path_area(&out) - 50.0).abs() < 1e-6, "{}", path_area(&out));
    }

    #[test]
    fn subtract_removes_the_clip() {
        let (a, b) = overlapping();
        let out = boolean_op(BoolOp::Subtract, &[a, b], FLATTEN_TOL);
        // a (100) minus the 50 overlap = 50.
        assert!((path_area(&out) - 50.0).abs() < 1e-6, "{}", path_area(&out));
    }

    #[test]
    fn exclude_is_symmetric_difference() {
        let (a, b) = overlapping();
        let out = boolean_op(BoolOp::Exclude, &[a, b], FLATTEN_TOL);
        // 150 union minus 50 overlap = 100 (two disjoint 5x10 wings).
        assert!(
            (path_area(&out) - 100.0).abs() < 1e-6,
            "{}",
            path_area(&out)
        );
    }

    #[test]
    fn union_of_disjoint_keeps_both() {
        let a = rect_path(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rect_path(Rect::new(20.0, 0.0, 30.0, 10.0));
        let out = boolean_op(BoolOp::Union, &[a, b], FLATTEN_TOL);
        assert!(
            (path_area(&out) - 200.0).abs() < 1e-6,
            "{}",
            path_area(&out)
        );
    }

    #[test]
    fn intersect_of_disjoint_is_empty() {
        let a = rect_path(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rect_path(Rect::new(20.0, 0.0, 30.0, 10.0));
        let out = boolean_op(BoolOp::Intersect, &[a, b], FLATTEN_TOL);
        assert!(out.elements().is_empty(), "expected empty result");
    }

    #[test]
    fn subtract_folds_over_multiple_clips() {
        // a minus b minus c: a=[0,30]x[0,10], b removes [0,10], c removes [20,30].
        let a = rect_path(Rect::new(0.0, 0.0, 30.0, 10.0));
        let b = rect_path(Rect::new(0.0, 0.0, 10.0, 10.0));
        let c = rect_path(Rect::new(20.0, 0.0, 30.0, 10.0));
        let out = boolean_op(BoolOp::Subtract, &[a, b, c], FLATTEN_TOL);
        // Only the middle [10,20]x[0,10] strip survives = 100.
        assert!(
            (path_area(&out) - 100.0).abs() < 1e-6,
            "{}",
            path_area(&out)
        );
    }

    #[test]
    fn fewer_than_two_paths_is_empty() {
        let a = rect_path(Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(
            boolean_op(BoolOp::Union, &[a], FLATTEN_TOL)
                .elements()
                .is_empty()
        );
        assert!(
            boolean_op(BoolOp::Union, &[], FLATTEN_TOL)
                .elements()
                .is_empty()
        );
    }

    #[test]
    fn deterministic_across_runs() {
        let (a, b) = overlapping();
        let first = boolean_op(BoolOp::Union, &[a.clone(), b.clone()], FLATTEN_TOL);
        let second = boolean_op(BoolOp::Union, &[a, b], FLATTEN_TOL);
        assert_eq!(first.to_svg(), second.to_svg());
    }
}
