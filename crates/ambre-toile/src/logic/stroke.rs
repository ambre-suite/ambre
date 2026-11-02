//! Freehand stroke smoothing. The CRDT stores only the raw sampled
//! [`StrokePoint`]s. The smoothed polyline is recomputed here at derivation
//! time, and again on the GPU path in `toile-board-rendering`, never persisted.

use kurbo::Point;

use crate::logic::model::StrokePoint;

/// Number of interpolated segments per Catmull-Rom span at derivation time. The
/// GPU path in `toile-board-rendering` picks its own subdivision from the zoom; the Logic
/// thread only needs a polyline dense enough for hit-testing and bounds.
const SEGMENTS_PER_SPAN: usize = 8;

/// Catmull-Rom smoothing through the sampled points. Pure and deterministic;
/// endpoints are duplicated for the boundary tangents. Reuses `out` (cleared
/// first) so the hot path allocates only the output buffer.
///
/// With fewer than two points the polyline is the points themselves (a single
/// dot or an empty stroke), so a degenerate stroke still renders.
pub fn smooth_stroke(points: &[StrokePoint], out: &mut Vec<Point>) {
    out.clear();
    let point = |s: &StrokePoint| Point::new(s.x.widened().get(), s.y.widened().get());
    match points {
        [] => {}
        [only] => out.push(point(only)),
        _ => {
            // The clamp keeps the index inside `points`, so the fallback to
            // the origin is never taken.
            let p = |i: isize| -> Point {
                let idx = i.clamp(0, points.len() as isize - 1) as usize;
                points.get(idx).map_or(Point::ORIGIN, point)
            };
            out.push(p(0));
            for i in 0..points.len() as isize - 1 {
                let p0 = p(i - 1);
                let p1 = p(i);
                let p2 = p(i + 1);
                let p3 = p(i + 2);
                for s in 1..=SEGMENTS_PER_SPAN {
                    let t = s as f64 / SEGMENTS_PER_SPAN as f64;
                    out.push(catmull_rom(p0, p1, p2, p3, t));
                }
            }
        }
    }
}

/// Standard uniform Catmull-Rom interpolation at parameter `t` in `[0, 1]`
/// between `p1` and `p2`, using `p0` / `p3` for the tangents.
fn catmull_rom(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> Point {
    let t2 = t * t;
    let t3 = t2 * t;
    let f = |a: f64, b: f64, c: f64, d: f64| {
        0.5 * ((2.0 * b)
            + (-a + c) * t
            + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
            + (-a + 3.0 * b - 3.0 * c + d) * t3)
    };
    Point::new(f(p0.x, p1.x, p2.x, p3.x), f(p0.y, p1.y, p2.y, p3.y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{Px, StoredBoard};

    fn sp(x: f32, y: f32) -> StrokePoint {
        StrokePoint::full_pressure(Px::<StoredBoard>::new(x), Px::<StoredBoard>::new(y))
    }

    #[test]
    fn empty_and_single_point() {
        let mut out = Vec::new();
        smooth_stroke(&[], &mut out);
        assert!(out.is_empty());
        smooth_stroke(&[sp(3.0, 4.0)], &mut out);
        assert_eq!(out, vec![Point::new(3.0, 4.0)]);
    }

    #[test]
    fn passes_through_samples_and_is_denser() {
        let pts = [sp(0.0, 0.0), sp(10.0, 0.0), sp(20.0, 0.0)];
        let mut out = Vec::new();
        smooth_stroke(&pts, &mut out);
        // First and last output points coincide with the first and last samples.
        assert_eq!(out.first().copied(), Some(Point::new(0.0, 0.0)));
        assert_eq!(out.last().copied(), Some(Point::new(20.0, 0.0)));
        // Denser than the raw samples.
        assert!(out.len() > pts.len());
    }
    /// `TOILE-039`: the outline passes through every sample, the interior
    /// ones included. Each span ends exactly on its sample.
    #[test]
    fn the_outline_passes_through_every_sample() {
        let pts = [
            sp(0.0, 0.0),
            sp(10.0, 30.0),
            sp(25.0, -5.0),
            sp(40.0, 20.0),
            sp(45.0, 0.0),
        ];
        let mut out = Vec::new();
        smooth_stroke(&pts, &mut out);
        assert_eq!(out.len(), 1 + (pts.len() - 1) * SEGMENTS_PER_SPAN);
        for (i, s) in pts.iter().enumerate() {
            let at = out[i * SEGMENTS_PER_SPAN];
            let want = Point::new(s.x.widened().get(), s.y.widened().get());
            assert!(
                at.distance(want) < 1e-9,
                "sample {i}: {at:?} is not {want:?}"
            );
        }
    }
}
