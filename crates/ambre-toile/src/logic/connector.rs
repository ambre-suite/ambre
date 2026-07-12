//! Connector geometry (§5). A connector references a source and target
//! [`ElementId`] plus an [`Anchor`] on each; its path is **pure client compute**
//! -- recomputed at derivation from the current element bounds, never a CRDT
//! mutation on element move. This keeps drags to a single register write and
//! makes connectors free of merge conflicts.

use std::collections::HashMap;

use kurbo::{BezPath, Point, Rect};

use crate::logic::model::{Anchor, ConnectorId, ElementId, EndCaps, PathType};

/// A resolved connector, materialized from its CRDT `LoroMap` for routing.
#[derive(Clone, Debug)]
pub struct Connector {
    pub id: ConnectorId,
    pub from: ElementId,
    pub from_anch: Anchor,
    pub to: ElementId,
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
        Anchor::Free { u, v } => {
            Point::new(r.x0 + r.width() * u as f64, r.y0 + r.height() * v as f64)
        }
    }
}

impl Connector {
    /// Route against the current element bounds. `None` when an endpoint element
    /// was deleted -- the connector is then dropped from the `DisplayViewport`
    /// and, if the deletion was local, garbage-collected (§5.A).
    pub fn route(&self, bounds: &HashMap<ElementId, Rect>) -> Option<BezPath> {
        let a = anchor_point(bounds.get(&self.from)?, self.from_anch);
        let b = anchor_point(bounds.get(&self.to)?, self.to_anch);
        Some(match self.path_type {
            PathType::Straight => straight(a, b),
            PathType::Elbow => elbow(a, self.from_anch, b, self.to_anch),
            PathType::Curved => curved(a, self.from_anch, b, self.to_anch),
        })
    }
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

/// Cubic Bezier routing: control points pushed out along each anchor's outward
/// normal by a third of the endpoint distance, giving a smooth S / C curve.
fn curved(a: Point, from: Anchor, b: Point, to: Anchor) -> BezPath {
    let dist = (b - a).hypot();
    let reach = (dist / 3.0).max(1.0);
    let c1 = a + anchor_normal(from) * reach;
    let c2 = b + anchor_normal(to) * reach;
    let mut p = BezPath::new();
    p.move_to(a);
    p.curve_to(c1, c2, b);
    p
}

fn anchor_is_horizontal(a: Anchor) -> bool {
    matches!(a, Anchor::Left | Anchor::Right)
}

/// Outward unit normal for an anchor. `Center` / `Free` have no natural normal;
/// they fall back to no push (zero vector), which degrades `Curved` to a mild
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

    fn two_boxes() -> HashMap<ElementId, Rect> {
        let mut m = HashMap::new();
        m.insert(ElementId("a".into()), rect(0.0, 0.0, 100.0, 100.0));
        m.insert(ElementId("b".into()), rect(300.0, 0.0, 100.0, 100.0));
        m
    }

    fn conn(path_type: PathType) -> Connector {
        Connector {
            id: ConnectorId("c".into()),
            from: ElementId("a".into()),
            from_anch: Anchor::Right,
            to: ElementId("b".into()),
            to_anch: Anchor::Left,
            path_type,
            arrows: EndCaps {
                start: false,
                end: true,
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

    #[test]
    fn missing_endpoint_yields_none() {
        let mut b = two_boxes();
        b.remove(&ElementId("b".into()));
        assert!(conn(PathType::Straight).route(&b).is_none());
    }
}
