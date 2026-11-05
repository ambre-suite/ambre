//! Connector geometry. A connector references a source and target [`Id<Element>`]
//! plus an [`Anchor`] on each. Its path is **pure client compute**, recomputed
//! at derivation from the current element bounds, never a CRDT mutation on
//! element move. This keeps drags to a single register write and makes
//! connectors free of merge conflicts.

use std::collections::HashMap;

use ambre_core::{Board, Px};
use kurbo::{BezPath, Point, Rect};

use ambre_core::id::{self, Element, Id};

use crate::logic::model::{Anchor, EndCaps, PathType};

/// A resolved connector, materialized from its CRDT `LoroMap` for routing.
#[derive(Clone, Debug, PartialEq)]
pub struct Connector {
    pub id: Id<id::Connector>,
    pub from: Id<Element>,
    pub from_anch: Anchor,
    pub to: Id<Element>,
    pub to_anch: Anchor,
    pub path_type: PathType,
    pub arrows: EndCaps,
}

/// The world-space point where `anchor` sits on `r`'s AABB.
pub fn anchor_point(r: &Rect, anchor: Anchor) -> Point {
    match anchor {
        Anchor::Top => Point::new(r.x0 + r.width() / 2.0, r.y0),
        Anchor::Bottom => Point::new(r.x0 + r.width() / 2.0, r.y1),
        Anchor::Left => Point::new(r.x0, r.y0 + r.height() / 2.0),
        Anchor::Right => Point::new(r.x1, r.y0 + r.height() / 2.0),
        Anchor::Center => Point::new(r.x0 + r.width() / 2.0, r.y0 + r.height() / 2.0),
        Anchor::Free { u, v } => Point::new(
            u.along(Px::<Board>::new(r.x0), Px::<Board>::new(r.x1))
                .get(),
            v.along(Px::<Board>::new(r.y0), Px::<Board>::new(r.y1))
                .get(),
        ),
    }
}

impl Connector {
    /// Route against the current element bounds. `None` when an endpoint element
    /// was deleted -- the connector is then dropped from the `DisplayViewport`
    /// and, if the deletion was local, garbage-collected.
    pub fn route(&self, bounds: &HashMap<Id<Element>, Rect>) -> Option<BezPath> {
        let a = anchor_point(bounds.get(&self.from)?, self.from_anch);
        let b = anchor_point(bounds.get(&self.to)?, self.to_anch);
        Some(match self.path_type {
            PathType::Straight => straight(a, b),
            PathType::Elbow => elbow(a, self.from_anch, b, self.to_anch),
            PathType::Curved => curved(a, self.from_anch, b, self.to_anch),
        })
    }
}

impl Connector {
    /// A box the routed path never leaves, without routing it: the endpoints,
    /// and for a curve its two control points too. `None` when an endpoint
    /// element was deleted, like [`route`](Self::route).
    pub fn hull(&self, bounds: &HashMap<Id<Element>, Rect>) -> Option<Rect> {
        let a = anchor_point(bounds.get(&self.from)?, self.from_anch);
        let b = anchor_point(bounds.get(&self.to)?, self.to_anch);
        let ends = Rect::from_points(a, b);
        Some(match self.path_type {
            // The elbow's two breaks sit on the endpoints' own coordinates.
            PathType::Straight | PathType::Elbow => ends,
            PathType::Curved => {
                let [c1, c2] = control_points(a, self.from_anch, b, self.to_anch);
                ends.union_pt(c1).union_pt(c2)
            }
        })
    }
}

/// Whether a routed outline enters `r`. The box of a diagonal arrow can meet a
/// frame its line never crosses, so the cull asks this once the box has.
pub fn outline_meets(path: &BezPath, r: Rect) -> bool {
    let mut from: Option<Point> = None;
    let mut hit = false;
    // A quarter of a world unit: a curve flattened that finely strays from
    // its outline by less than any stroke paints.
    kurbo::flatten(path.iter(), 0.25, |el| {
        if hit {
            return;
        }
        match el {
            kurbo::PathEl::MoveTo(p) => {
                hit = r.contains(p);
                from = Some(p);
            }
            kurbo::PathEl::LineTo(p) => {
                if let Some(a) = from {
                    hit = segment_meets(a, p, r);
                }
                from = Some(p);
            }
            _ => {}
        }
    });
    hit
}

/// Whether the segment `a`-`b` enters `r`: Liang-Barsky clipping, which keeps
/// the part of the segment inside each of the four half-planes in turn.
fn segment_meets(a: Point, b: Point, r: Rect) -> bool {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [
        (-d.x, a.x - r.x0),
        (d.x, r.x1 - a.x),
        (-d.y, a.y - r.y0),
        (d.y, r.y1 - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false; // parallel to this edge, and outside it
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
        if t0 > t1 {
            return false;
        }
    }
    true
}

fn straight(a: Point, b: Point) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(a);
    p.line_to(b);
    p
}

/// Orthogonal (elbow) routing: a single mid-axis break. Chooses a horizontal-
/// then-vertical or vertical-then-horizontal dogleg from the source anchor's
/// dominant axis, so the first segment leaves the element along its anchor.
fn elbow(a: Point, from: Anchor, b: Point, _to: Anchor) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(a);
    let mid = if anchor_is_horizontal(from) {
        // Leave horizontally: break at the source x's midpoint to the target.
        let mx = (a.x + b.x) / 2.0;
        [Point::new(mx, a.y), Point::new(mx, b.y)]
    } else {
        let my = (a.y + b.y) / 2.0;
        [Point::new(a.x, my), Point::new(b.x, my)]
    };
    p.line_to(mid[0]);
    p.line_to(mid[1]);
    p.line_to(b);
    p
}

/// Cubic Bezier routing. Control points are pushed out along each anchor's
/// outward normal by a third of the endpoint distance, giving a smooth S / C
/// curve.
fn curved(a: Point, from: Anchor, b: Point, to: Anchor) -> BezPath {
    let [c1, c2] = control_points(a, from, b, to);
    let mut p = BezPath::new();
    p.move_to(a);
    p.curve_to(c1, c2, b);
    p
}

/// The curve's two control points: each endpoint pushed out along its anchor's
/// normal by a third of the endpoint distance. A cubic stays inside the hull of
/// its endpoints and these.
fn control_points(a: Point, from: Anchor, b: Point, to: Anchor) -> [Point; 2] {
    let dist = (b - a).hypot();
    let reach = (dist / 3.0).max(1.0);
    [
        a + anchor_normal(from) * reach,
        b + anchor_normal(to) * reach,
    ]
}

fn anchor_is_horizontal(a: Anchor) -> bool {
    matches!(a, Anchor::Left | Anchor::Right)
}

/// Outward unit normal for an anchor. `Center` / `Free` have no natural normal.
/// They fall back to no push (zero vector), which degrades `Curved` to a mild
/// bow driven only by the opposite anchor.
fn anchor_normal(a: Anchor) -> kurbo::Vec2 {
    match a {
        Anchor::Top => kurbo::Vec2::new(0.0, -1.0),
        Anchor::Bottom => kurbo::Vec2::new(0.0, 1.0),
        Anchor::Left => kurbo::Vec2::new(-1.0, 0.0),
        Anchor::Right => kurbo::Vec2::new(1.0, 0.0),
        Anchor::Center | Anchor::Free { .. } => kurbo::Vec2::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect::new(x, y, x + w, y + h)
    }

    fn two_boxes() -> HashMap<Id<Element>, Rect> {
        let mut m = HashMap::new();
        m.insert(
            Id::<Element>::from_text("a".into()),
            rect(0.0, 0.0, 100.0, 100.0),
        );
        m.insert(
            Id::<Element>::from_text("b".into()),
            rect(300.0, 0.0, 100.0, 100.0),
        );
        m
    }

    fn conn(path_type: PathType) -> Connector {
        Connector {
            id: Id::<id::Connector>::from_text("c".into()),
            from: Id::<Element>::from_text("a".into()),
            from_anch: Anchor::Right,
            to: Id::<Element>::from_text("b".into()),
            to_anch: Anchor::Left,
            path_type,
            arrows: EndCaps {
                start: crate::message::Cap::Plain,
                end: crate::message::Cap::Arrow,
            },
        }
    }

    #[test]
    fn straight_endpoints_are_the_anchors() {
        let b = two_boxes();
        let path = conn(PathType::Straight).route(&b).unwrap();
        // Starts at the right-center of A, ends at the left-center of B.
        let els: Vec<_> = path.elements().to_vec();
        assert!(matches!(els[0], kurbo::PathEl::MoveTo(p) if p == Point::new(100.0, 50.0)));
        assert!(matches!(els[1], kurbo::PathEl::LineTo(p) if p == Point::new(300.0, 50.0)));
    }

    #[test]
    fn every_path_type_routes() {
        let b = two_boxes();
        for pt in [PathType::Straight, PathType::Elbow, PathType::Curved] {
            assert!(conn(pt).route(&b).is_some());
        }
    }

    /// `TOILE-045`: the box the cull tests holds the whole routed outline, for
    /// every path kind and every pair of sides, the bulging curve included.
    #[test]
    fn the_hull_holds_the_routed_path() {
        use kurbo::Shape;
        let mut b = two_boxes();
        b.insert(
            Id::<Element>::from_text("b".into()),
            rect(300.0, 250.0, 100.0, 100.0),
        );
        let sides = [
            Anchor::Top,
            Anchor::Bottom,
            Anchor::Left,
            Anchor::Right,
            Anchor::Center,
        ];
        for pt in [PathType::Straight, PathType::Elbow, PathType::Curved] {
            for from_anch in sides {
                for to_anch in sides {
                    let c = Connector {
                        from_anch,
                        to_anch,
                        ..conn(pt)
                    };
                    let hull = c.hull(&b).unwrap().inflate(1e-9, 1e-9);
                    let outline = c.route(&b).unwrap().bounding_box();
                    assert_eq!(
                        hull.union(outline),
                        hull,
                        "{pt:?} {from_anch:?} {to_anch:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn missing_endpoint_yields_none() {
        let mut b = two_boxes();
        b.remove(&Id::<Element>::from_text("b".into()));
        assert!(conn(PathType::Straight).route(&b).is_none());
    }
}
