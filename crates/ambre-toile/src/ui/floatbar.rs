// ambre-toile/src/ui/floatbar.rs
// ============================================================================
// Command surface + placement for the content-anchored floating selection
// toolbar (toile-ui SS5quinquies.D). The host-visible half: the pure model the
// canvas view publishes to the DOM bar each frame -- the bar's screen anchor
// (recomputed from the `ViewportTransform`, so it follows the selection as the
// board pans / zooms) plus the applicability flags that gate its buttons.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so the
// geometry + flags are host-unit-tested (SS10), exactly like `ui::menu`,
// `ui::style` and `ui::status`. The wasm-only Leptos component that renders it
// (`chrome::SelectionToolbar`) consumes this model; its command buttons route
// through the shared `on_command` path with `ui::menu` `CommandId`s, so the bar
// is a strict subset of the context menu and the two never diverge (SS5ter).
//
// The bar's transient hiding (during a move/resize/rotate gesture, a pan, an
// open inline editor, or an empty selection) is decided by the view at publish
// time (`None` == hidden), not here; this module only turns a resolved,
// visible selection into an anchored [`FloatBar`].
// ============================================================================

use ambre_bus::app::toile::{DisplayElement, DisplayNode, DisplayViewport, ElementId, ElementKind};
use ambre_draw::{Point, Rect};

use super::viewport::ViewportTransform;

/// The floating selection toolbar's published model (toile-ui SS5quinquies.D).
/// `x` / `y` are the client (viewport) CSS-pixel position of the selection's
/// top-edge midpoint -- the bar is `position: fixed` there and lifted above the
/// selection with a CSS gap ("air", like Miro). The flags gate the applicable
/// buttons, mirroring the context-menu scope (`ui::menu::CtxScope`); `fill` is
/// the selection's common fill for the quick-colour swatch (`None` when unset or
/// mixed). Published only for a resolved, visible selection (`None` == hidden).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FloatBar {
    /// Client CSS-pixel x of the selection's top-edge midpoint.
    pub x: f64,
    /// Client CSS-pixel y of the selection's top edge.
    pub y: f64,
    /// At least two elements selected -> Group applies.
    pub multi: bool,
    /// At least one selected element is a group node -> Ungroup applies.
    pub has_group: bool,
    /// At least one selected element is locked -> Unlock applies.
    pub any_locked: bool,
    /// At least one selected element is unlocked -> Lock applies.
    pub any_unlocked: bool,
    /// The selection's common fill (`ElementStyle` ARGB), or `None` when unset or
    /// diverging across the selection -- the quick-colour swatch seed.
    pub fill: Option<u32>,
}

/// A quick-colour pick the floating bar emits back (toile-ui SS5quinquies.D):
/// the chosen fill (`None` == "no fill"). The view drains it and folds it into a
/// whole-register `RestyleElements` over the live selection (SS2.E), attaching the
/// ids the DOM bar cannot name -- the same seam as the inspector's `restyle`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct QuickFill(pub Option<u32>);

/// The client CSS-pixel anchor for the bar: the selection's top-edge midpoint,
/// mapped through the camera (physical screen px), converted to CSS px by the
/// device pixel ratio, then shifted by the canvas's client offset so the
/// `position: fixed` bar lands in viewport coordinates. Pure; `dpr` is 1.0 and
/// `offset` is `(0, 0)` in host tests.
pub fn top_center_anchor(
    world_bounds: Rect,
    cam: &ViewportTransform,
    dpr: f64,
    offset: (f64, f64),
) -> (f64, f64) {
    let mid_top = Point::new((world_bounds.x0 + world_bounds.x1) / 2.0, world_bounds.y0);
    let screen = cam.world_to_screen() * mid_top;
    (offset.0 + screen.x / dpr, offset.1 + screen.y / dpr)
}

/// Build the floating-bar model from the UI selection and the culled viewport, or
/// `None` when nothing selectable resolves (the bar hides). Pure: reads only the
/// `DisplayViewport`, the selection ids and the camera. The union of the selected
/// elements' bounds anchors the bar; the per-element kind / lock state set the
/// button flags; the common fill seeds the quick-colour swatch. Connectors are not
/// addressed by the bar, so only `DisplayElement`s contribute.
pub fn build_floatbar(
    selection: &[ElementId],
    display: &DisplayViewport,
    cam: &ViewportTransform,
    dpr: f64,
    offset: (f64, f64),
) -> Option<FloatBar> {
    if selection.is_empty() {
        return None;
    }

    let mut union: Option<Rect> = None;
    let mut count = 0usize;
    let mut has_group = false;
    let mut any_locked = false;
    let mut any_unlocked = false;
    // Common-fill fold: track the first fill and whether any diverges (mixed).
    let mut fill: Option<u32> = None;
    let mut fill_seen = false;
    let mut fill_mixed = false;

    for id in selection {
        let Some(e) = find_element(display, id) else {
            continue;
        };
        count += 1;
        if e.kind == ElementKind::Group {
            has_group = true;
        }
        if e.locked {
            any_locked = true;
        } else {
            any_unlocked = true;
        }
        union = Some(match union {
            Some(u) => u.union(e.bounds),
            None => e.bounds,
        });
        if !fill_seen {
            fill = e.style.fill;
            fill_seen = true;
        } else if fill != e.style.fill {
            fill_mixed = true;
        }
    }

    // Nothing in the selection is present in the culled set: hide the bar.
    let union = union?;
    let (x, y) = top_center_anchor(union, cam, dpr, offset);
    Some(FloatBar {
        x,
        y,
        multi: count >= 2,
        has_group,
        any_locked,
        any_unlocked,
        fill: if fill_mixed { None } else { fill },
    })
}

/// Find a displayed element by id (linear over the culled set, which is small).
fn find_element<'a>(display: &'a DisplayViewport, id: &ElementId) -> Option<&'a DisplayElement> {
    display.nodes.iter().find_map(|n| match n {
        DisplayNode::Element(e) if &e.id == id => Some(e),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{DisplayElement, DisplayGeom, ElementStyle, ShapeKind, Viewport};
    use ambre_draw::Size;
    use kurbo::Affine;

    fn eid(s: &str) -> ElementId {
        ElementId(s.to_owned())
    }

    fn elem(
        id: &str,
        kind: ElementKind,
        bounds: Rect,
        locked: bool,
        fill: Option<u32>,
    ) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: eid(id),
            kind,
            transform: Affine::IDENTITY,
            bounds,
            locked,
            style: ElementStyle {
                fill,
                ..ElementStyle::default()
            },
            geom: match kind {
                ElementKind::Group => DisplayGeom::Group,
                _ => DisplayGeom::Rect,
            },
            group: None,
        })
    }

    fn viewport(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            nodes,
            viewport: Viewport::default(),
        }
    }

    /// An identity camera (zoom 1, centered on origin) with a known size, so
    /// world->screen is a pure translate by half the size.
    fn cam() -> ViewportTransform {
        ViewportTransform {
            center: Point::ZERO,
            zoom: 1.0,
            size: Size::new(1000.0, 800.0),
        }
    }

    #[test]
    fn empty_selection_hides_bar() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            false,
            None,
        )]);
        assert_eq!(build_floatbar(&[], &dv, &cam(), 1.0, (0.0, 0.0)), None);
    }

    #[test]
    fn unresolved_selection_hides_bar() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            false,
            None,
        )]);
        assert_eq!(
            build_floatbar(&[eid("ghost")], &dv, &cam(), 1.0, (0.0, 0.0)),
            None
        );
    }

    #[test]
    fn anchor_is_top_edge_midpoint_in_screen_space() {
        // A 20-wide rect from (10,20)->(30,60): top-edge midpoint is (20, 20) in
        // world. With zoom 1 centered on origin, screen = world + size/2 =
        // (20+500, 20+400) = (520, 420). dpr 1, no offset.
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            Rect::new(10.0, 20.0, 30.0, 60.0),
            false,
            Some(0xff00_ff00),
        )]);
        let bar = build_floatbar(&[eid("a")], &dv, &cam(), 1.0, (0.0, 0.0)).expect("bar");
        assert!((bar.x - 520.0).abs() < 1e-9);
        assert!((bar.y - 420.0).abs() < 1e-9);
        assert!(!bar.multi);
        assert_eq!(bar.fill, Some(0xff00_ff00));
    }

    #[test]
    fn dpr_and_offset_apply_to_the_anchor() {
        // Same rect, dpr 2 halves the physical px, then the canvas client offset
        // shifts into viewport coords: ((520/... )). Screen physical = (520, 420);
        // /2 = (260, 210); + offset (40, 96) = (300, 306).
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            Rect::new(10.0, 20.0, 30.0, 60.0),
            false,
            None,
        )]);
        let bar = build_floatbar(&[eid("a")], &dv, &cam(), 2.0, (40.0, 96.0)).expect("bar");
        assert!((bar.x - 300.0).abs() < 1e-9);
        assert!((bar.y - 306.0).abs() < 1e-9);
    }

    #[test]
    fn union_bounds_anchor_a_multi_selection() {
        // Two rects; union top edge is the smaller y0 (10), midpoint x is the
        // union center x = (0 + 80)/2 = 40. Screen = (40+500, 10+400).
        let dv = viewport(vec![
            elem(
                "a",
                ElementKind::Sticky,
                Rect::new(0.0, 10.0, 20.0, 30.0),
                false,
                None,
            ),
            elem(
                "b",
                ElementKind::Sticky,
                Rect::new(60.0, 40.0, 80.0, 70.0),
                false,
                None,
            ),
        ]);
        let bar = build_floatbar(&[eid("a"), eid("b")], &dv, &cam(), 1.0, (0.0, 0.0)).expect("bar");
        assert!((bar.x - 540.0).abs() < 1e-9);
        assert!((bar.y - 410.0).abs() < 1e-9);
        assert!(bar.multi);
    }

    #[test]
    fn flags_track_group_and_lock_state() {
        let dv = viewport(vec![
            elem(
                "g",
                ElementKind::Group,
                Rect::new(0.0, 0.0, 10.0, 10.0),
                false,
                None,
            ),
            elem(
                "l",
                ElementKind::Sticky,
                Rect::new(0.0, 0.0, 5.0, 5.0),
                true,
                None,
            ),
        ]);
        let bar = build_floatbar(&[eid("g"), eid("l")], &dv, &cam(), 1.0, (0.0, 0.0)).expect("bar");
        assert!(bar.has_group);
        assert!(bar.any_locked);
        assert!(bar.any_unlocked);
        assert!(bar.multi);
    }

    #[test]
    fn common_fill_is_none_when_mixed() {
        let dv = viewport(vec![
            elem(
                "a",
                ElementKind::Sticky,
                Rect::new(0.0, 0.0, 5.0, 5.0),
                false,
                Some(0xffff_0000),
            ),
            elem(
                "b",
                ElementKind::Sticky,
                Rect::new(6.0, 0.0, 10.0, 5.0),
                false,
                Some(0xff00_00ff),
            ),
        ]);
        let bar = build_floatbar(&[eid("a"), eid("b")], &dv, &cam(), 1.0, (0.0, 0.0)).expect("bar");
        assert_eq!(bar.fill, None);
    }

    #[test]
    fn common_fill_survives_when_shared() {
        let dv = viewport(vec![
            elem(
                "a",
                ElementKind::Shape(ShapeKind::Rect),
                Rect::new(0.0, 0.0, 5.0, 5.0),
                false,
                Some(0xff11_2233),
            ),
            elem(
                "b",
                ElementKind::Shape(ShapeKind::Rect),
                Rect::new(6.0, 0.0, 10.0, 5.0),
                false,
                Some(0xff11_2233),
            ),
        ]);
        let bar = build_floatbar(&[eid("a"), eid("b")], &dv, &cam(), 1.0, (0.0, 0.0)).expect("bar");
        assert_eq!(bar.fill, Some(0xff11_2233));
    }
}
