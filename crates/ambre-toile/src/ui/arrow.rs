// ambre-toile/src/ui/arrow.rs
// ============================================================================
// Selecting an arrow (toile-connectors, toile-selection). An arrow is not an
// element: it has no box, no rotation and no body. So it is not a member of the
// element selection but a selection of its own, and the two are exclusive.
//
// What lives here is the geometry a selected arrow needs. That is where its two
// endpoint handles sit, which one a press grabs, and what a release over a shape
// means. It is kept pure so the host tier can run it.
// ============================================================================

use ambre_bus::app::toile::{DisplayNode, DisplayViewport, End, NodeId, ToileCommand, ZTarget};
use ambre_commands::CommandId;
use ambre_core::ViewPx;
use ambre_core::id::{Connector, Element, Id};
use ambre_draw::{BezPath, Point};
use kurbo::ParamCurve;

use super::viewport::BoardLength;

/// Endpoint handle radius in screen pixels. The grab tolerance is this radius
/// taken back into world units, so a handle stays equally easy to hit at every
/// zoom.
pub const HANDLE_R: ViewPx = ViewPx::new(5.0);

/// The two ends of a routed arrow in world space: where it starts, where it
/// ends. `None` for an empty path.
pub fn endpoints(path: &BezPath) -> Option<(Point, Point)> {
    let mut segs = path.segments();
    let first = segs.next()?;
    let last = path.segments().last().unwrap_or(first);
    Some((first.eval(0.0), last.eval(1.0)))
}

/// Which endpoint handle `world` grabs, within `tol` world units. The nearer
/// handle wins when both are in range, so a zero-length arrow still answers.
pub fn handle_at(path: &BezPath, world: Point, tol: BoardLength) -> Option<End> {
    let (from, to) = endpoints(path)?;
    let (df, dt) = (from.distance(world), to.distance(world));
    if BoardLength::new(df.min(dt)) > tol {
        return None;
    }
    Some(if df <= dt { End::From } else { End::To })
}

/// The command a re-aim release produces: the dragged endpoint moves to the
/// element under the pointer, anchored on that element's nearest side. `None`
/// when the release lands on empty board or on an element the display has
/// dropped. A re-aim needs somewhere to land, and a dropped endpoint would
/// dangle the arrow the user was repairing.
pub fn retarget_on_release(
    id: &Id<Connector>,
    endpoint: End,
    display: &DisplayViewport,
    world: Point,
    pick: BoardLength,
) -> Option<ToileCommand> {
    let to = super::hit::hit_topmost(&display.nodes, world, pick)?;
    let bounds = super::tool::element_bounds(display, &to)?;
    Some(ToileCommand::RetargetConnector {
        id: id.clone(),
        endpoint,
        to,
        anch: super::tool::nearest_anchor(bounds, world),
    })
}

/// What a board command means when the selection is one arrow. An arrow is not
/// an element, so most entries mean nothing on it. It cannot be grouped,
/// locked, reacted to, put in a frame, or copied (its two endpoints would not
/// travel with it). What is left is the four the story names, minus the restyle
/// the inspector drives through its own path.
pub fn arrow_command_for(id: CommandId, arrow: &Id<Connector>) -> Vec<ToileCommand> {
    use super::menu::{BACKWARD, BRING_FRONT, DELETE, FORWARD, SEND_BACK};
    let reorder = |to| {
        vec![ToileCommand::Reorder {
            node: NodeId::Connector(arrow.clone()),
            to,
        }]
    };
    match id {
        DELETE => vec![ToileCommand::DeleteConnector { id: arrow.clone() }],
        BRING_FRONT => reorder(ZTarget::Front),
        SEND_BACK => reorder(ZTarget::Back),
        FORWARD => reorder(ZTarget::Forward),
        BACKWARD => reorder(ZTarget::Backward),
        _ => Vec::new(),
    }
}

/// The routed path of one connector in the current display, if it is there.
pub fn path_of<'a>(display: &'a DisplayViewport, id: &Id<Connector>) -> Option<&'a BezPath> {
    display.nodes.iter().find_map(|n| match n {
        DisplayNode::Connector(c) if &c.id == id => Some(&c.path),
        _ => None,
    })
}

/// Whether an element id is still in the culled display. A selected arrow whose
/// endpoint left the board is answered by [`path_of`] instead: the deriver drops
/// the whole connector.
pub fn holds_element(display: &DisplayViewport, id: &Id<Element>) -> bool {
    display
        .nodes
        .iter()
        .any(|n| matches!(n, DisplayNode::Element(e) if &e.id == id))
}

#[cfg(test)]
mod tests {
    use ambre_bus::app::toile::{
        Anchor, DisplayConnector, DisplayElement, DisplayGeom, ElementKind, ElementStyle, EndCaps,
        ShapeKind, Viewport,
    };
    use ambre_draw::{Affine, Rect};

    use super::*;

    fn line(a: (f64, f64), b: (f64, f64)) -> BezPath {
        let mut p = BezPath::new();
        p.move_to(a);
        p.line_to(b);
        p
    }

    fn rect(id: &str, b: Rect) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: Id::<Element>::from_text(id.to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds: b,
            locked: ambre_bus::app::toile::LockState::Unlocked,
            style: ElementStyle::default(),
            geom: DisplayGeom::Rect,
            link: None,
            group: None,
        })
    }

    fn arrow(id: &str, path: BezPath) -> DisplayNode {
        DisplayNode::Connector(DisplayConnector {
            id: Id::<Connector>::from_text(id.to_owned()),
            path,
            style: ElementStyle::default(),
            arrows: EndCaps {
                start: ambre_bus::app::toile::Cap::Plain,
                end: ambre_bus::app::toile::Cap::Arrow,
            },
            label: String::new(),
        })
    }

    fn viewport(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: ambre_core::BoardLength::new(20.0),
            viewport: Viewport::default(),
            reactions: Default::default(),
            pins: Vec::new(),
            nodes,
        }
    }

    /// `TOILE-UI-351`: the handles sit at the two ends of the routed path,
    /// whatever runs between them.
    #[test]
    fn the_handles_sit_at_the_two_ends() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        p.line_to((100.0, 100.0));
        let (from, to) = endpoints(&p).expect("routed");
        assert_eq!(from, Point::new(0.0, 0.0));
        assert_eq!(to, Point::new(100.0, 100.0));
    }

    /// A press near an end grabs that end. One in the middle of the arrow grabs
    /// neither, so a click on the line selects instead of re-aiming.
    #[test]
    fn a_press_grabs_the_nearer_end() {
        let p = line((0.0, 0.0), (100.0, 0.0));
        assert_eq!(
            handle_at(&p, Point::new(2.0, 1.0), BoardLength::new(6.0)),
            Some(End::From)
        );
        assert_eq!(
            handle_at(&p, Point::new(98.0, 1.0), BoardLength::new(6.0)),
            Some(End::To)
        );
        assert_eq!(
            handle_at(&p, Point::new(50.0, 0.0), BoardLength::new(6.0)),
            None
        );
    }

    /// A re-aim released over a shape moves the dragged end there, on the side
    /// the user aimed at.
    #[test]
    fn a_release_over_a_shape_re_aims_that_end() {
        let display = viewport(vec![
            rect("a", Rect::new(0.0, 0.0, 100.0, 100.0)),
            arrow("c", line((100.0, 50.0), (300.0, 50.0))),
        ]);
        let cmd = retarget_on_release(
            &Id::<Connector>::from_text("c".into()),
            End::To,
            &display,
            Point::new(50.0, 98.0),
            crate::ui::hit::pick_floor(&crate::ui::viewport::ViewportTransform::default()),
        );
        match cmd {
            Some(ToileCommand::RetargetConnector {
                id,
                endpoint,
                to,
                anch,
            }) => {
                assert_eq!(id, Id::<Connector>::from_text("c".into()));
                assert!(matches!(endpoint, End::To));
                assert_eq!(to, Id::<Element>::from_text("a".into()));
                assert!(matches!(anch, Anchor::Bottom), "aimed at the bottom edge");
            }
            other => panic!("expected a retarget, got {other:?}"),
        }
    }

    /// The four board commands an arrow answers, and the silence on the rest.
    #[test]
    fn an_arrow_answers_delete_and_the_z_order() {
        use super::super::menu::{BACKWARD, BRING_FRONT, COPY, DELETE, FORWARD, GROUP, SEND_BACK};
        let c = Id::<Connector>::from_text("c".into());
        assert!(matches!(
            arrow_command_for(DELETE, &c).as_slice(),
            [ToileCommand::DeleteConnector { .. }]
        ));
        for (cmd, want) in [
            (BRING_FRONT, ZTarget::Front),
            (SEND_BACK, ZTarget::Back),
            (FORWARD, ZTarget::Forward),
            (BACKWARD, ZTarget::Backward),
        ] {
            match arrow_command_for(cmd, &c).as_slice() {
                [ToileCommand::Reorder { node, to }] => {
                    assert_eq!(node, &NodeId::Connector(c.clone()));
                    assert_eq!(
                        std::mem::discriminant(to),
                        std::mem::discriminant(&want),
                        "wrong z-order target"
                    );
                }
                other => panic!("expected one reorder, got {other:?}"),
            }
        }
        // Grouping and copying an arrow are not offered: an arrow alone carries
        // neither geometry nor its endpoints.
        assert!(arrow_command_for(GROUP, &c).is_empty());
        assert!(arrow_command_for(COPY, &c).is_empty());
    }

    /// `TOILE-UI-372`: a re-aim released over empty board sends nothing: an
    /// arrow re-aimed at nothing is an arrow the user just broke.
    #[test]
    fn a_release_over_empty_board_re_aims_nothing() {
        let display = viewport(vec![arrow("c", line((0.0, 0.0), (100.0, 0.0)))]);
        assert!(
            retarget_on_release(
                &Id::<Connector>::from_text("c".into()),
                End::From,
                &display,
                Point::new(500.0, 500.0),
                crate::ui::hit::pick_floor(&crate::ui::viewport::ViewportTransform::default()),
            )
            .is_none()
        );
    }
}
