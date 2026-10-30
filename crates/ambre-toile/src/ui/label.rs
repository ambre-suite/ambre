// ambre-toile/src/ui/label.rs
// ============================================================================
// Connector captions (toile-connectors): where a caption sits on a routed
// arrow, how big its box is, and which arrow a click lands on.
//
// A caption is not stored with a position. It rides the arrow's mid-arc point,
// recomputed from the routed path like the path itself is recomputed from the
// endpoints. So moving either shape carries the caption along and writes
// nothing (`TOILE-006`).
//
// Pure geometry -- host-tested, no `cfg(wasm32)`, no GPU.
// ============================================================================

use ambre_bus::app::toile::{ConnectorId, DisplayNode};
use ambre_core::BoardLength;
use ambre_draw::{BezPath, Point, Rect};
use kurbo::{ParamCurve, ParamCurveArclen, PathSeg};

/// Arc-length accuracy for the mid-point search, in world units. A caption that
/// sits a tenth of a pixel off centre is a caption in the right place.
const ARCLEN_ACCURACY: BoardLength = BoardLength::new(0.1);

/// The caption box, in world units. Fixed rather than text-derived: the box is
/// the wrap width and the click target. Both must be knowable before the font
/// context exists (the editor opens outside the paint pass).
pub const LABEL_W: BoardLength = BoardLength::new(160.0);
/// The height of the caption box, in world units.
pub const LABEL_H: BoardLength = BoardLength::new(24.0);

/// Caption font size in world units, scaled by the camera at paint time like
/// every other body on the board.
pub const LABEL_FONT_SIZE: ambre_core::DocPx = ambre_core::DocPx::new(14.0);

/// The point half-way along `path` by arc length: the caption's anchor. `None`
/// for an empty path. A straight arrow puts it at the midpoint; an elbow or a
/// curve puts it half-way round the bend, which is where a reader looks.
pub fn label_anchor(path: &BezPath) -> Option<Point> {
    let segs: Vec<PathSeg> = path.segments().collect();
    let (first, last) = (segs.first()?, segs.last()?);
    let lens: Vec<f64> = segs
        .iter()
        .map(|s| s.arclen(ARCLEN_ACCURACY.get()))
        .collect();
    let total: f64 = lens.iter().sum();
    if total <= 0.0 || !total.is_finite() {
        // A zero-length path: both endpoints resolved to one point.
        return Some(first.eval(0.0));
    }
    let mut want = total / 2.0;
    for (seg, len) in segs.iter().zip(&lens) {
        if want <= *len {
            return Some(seg.eval(seg.inv_arclen(want, ARCLEN_ACCURACY.get())));
        }
        want -= len;
    }
    Some(last.eval(1.0))
}

/// The caption's world box: the fixed label box centred on the anchor.
pub fn label_rect(path: &BezPath) -> Option<Rect> {
    let c = label_anchor(path)?;
    let (half_w, half_h) = ((LABEL_W / 2.0).get(), (LABEL_H / 2.0).get());
    Some(Rect::new(
        c.x - half_w,
        c.y - half_h,
        c.x + half_w,
        c.y + half_h,
    ))
}

/// The top-most connector `world_pt` lands on: its outline within the pick
/// tolerance, or anywhere inside the caption box of an arrow that carries one.
/// `pick` is the world reach of the screen pick floor (`hit::pick_floor`).
/// Nodes are back-to-front, so the walk runs in reverse.
pub fn connector_at(
    nodes: &[DisplayNode],
    world_pt: Point,
    pick: super::viewport::BoardLength,
) -> Option<ConnectorId> {
    nodes
        .iter()
        .rev()
        .filter_map(|n| match n {
            DisplayNode::Connector(c) => Some(c),
            DisplayNode::Element(_) => None,
        })
        .find(|c| {
            let tol = super::hit::stroke_reach(super::hit::stroke_width(&c.style), pick);
            if super::hit::path_outline_distance(&c.path, world_pt) <= tol {
                return true;
            }
            !c.label.is_empty() && label_rect(&c.path).is_some_and(|r| r.contains(world_pt))
        })
        .map(|c| c.id.clone())
}

#[cfg(test)]
mod tests {
    use ambre_bus::app::toile::{DisplayConnector, ElementStyle, EndCaps};

    use super::*;
    use crate::ui::viewport::{BoardLength, ViewportTransform};

    /// The pointer's reach under a camera at zoom 1.
    fn pick() -> BoardLength {
        crate::ui::hit::pick_floor(&ViewportTransform::default())
    }

    fn line(a: (f64, f64), b: (f64, f64)) -> BezPath {
        let mut p = BezPath::new();
        p.move_to(a);
        p.line_to(b);
        p
    }

    fn conn(id: &str, path: BezPath, label: &str) -> DisplayNode {
        DisplayNode::Connector(DisplayConnector {
            id: ConnectorId(id.to_owned()),
            path,
            style: ElementStyle::default(),
            arrows: EndCaps {
                start: ambre_bus::app::toile::Cap::Plain,
                end: ambre_bus::app::toile::Cap::Arrow,
            },
            label: label.to_owned(),
        })
    }

    /// A straight arrow anchors its caption at the midpoint.
    #[test]
    fn a_straight_arrow_anchors_at_its_midpoint() {
        let p = label_anchor(&line((0.0, 0.0), (100.0, 0.0))).expect("anchored");
        assert!((p.x - 50.0).abs() < 0.5, "x was {}", p.x);
        assert!((p.y).abs() < 0.5, "y was {}", p.y);
    }

    /// An elbow anchors half-way along the run, not half-way between the ends:
    /// the two legs are measured, so the anchor lands on the path itself.
    #[test]
    fn an_elbow_anchors_half_way_along_the_run() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        p.line_to((100.0, 100.0));
        let a = label_anchor(&p).expect("anchored");
        // Total run is 200; the halfway point is the corner itself.
        assert!((a.x - 100.0).abs() < 0.5, "x was {}", a.x);
        assert!(a.y.abs() < 0.5, "y was {}", a.y);
    }

    /// `TOILE-UI-400`: an empty path has no caption anchor: nothing is routed,
    /// nothing is placed.
    #[test]
    fn an_empty_path_anchors_nothing() {
        assert!(label_anchor(&BezPath::new()).is_none());
    }

    /// A click on the arrow's line picks it; one a shape away does not.
    #[test]
    fn a_click_on_the_line_picks_the_arrow() {
        let nodes = vec![conn("c1", line((0.0, 0.0), (100.0, 0.0)), "")];
        assert_eq!(
            connector_at(&nodes, Point::new(50.0, 1.0), pick()),
            Some(ConnectorId("c1".into()))
        );
        assert_eq!(connector_at(&nodes, Point::new(50.0, 40.0), pick()), None);
    }

    /// A caption is part of its arrow's click target. A click on the words picks
    /// the arrow, which is how the caption is re-opened for editing.
    #[test]
    fn a_click_on_the_caption_picks_the_arrow() {
        let far = Point::new(50.0, 10.0); // clear of the line, inside the box
        let captioned = vec![conn("c1", line((0.0, 0.0), (100.0, 0.0)), "owns")];
        assert_eq!(
            connector_at(&captioned, far, pick()),
            Some(ConnectorId("c1".into()))
        );
        // The same box on an uncaptioned arrow is not a target: an empty
        // caption paints nothing, so nothing there is clickable.
        let bare = vec![conn("c1", line((0.0, 0.0), (100.0, 0.0)), "")];
        assert_eq!(connector_at(&bare, far, pick()), None);
    }

    /// Two arrows overlapping at a point: the later one in the paint order wins,
    /// like the element pick.
    #[test]
    fn the_top_most_arrow_wins() {
        let nodes = vec![
            conn("under", line((0.0, 0.0), (100.0, 0.0)), ""),
            conn("over", line((0.0, 0.0), (100.0, 0.0)), ""),
        ];
        assert_eq!(
            connector_at(&nodes, Point::new(50.0, 0.0), pick()),
            Some(ConnectorId("over".into()))
        );
    }
}
