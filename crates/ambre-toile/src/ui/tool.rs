// ambre-toile/src/ui/tool.rs
// ============================================================================
// Tool state machine (toile-ui §5). Phase 0 implements the Select tool fully:
// move / resize / rotate / marquee. Each gesture consumes pointer events and, on
// release, emits exactly one `ToileCommand` (or a local selection change for a
// marquee). The other tools are declared here as `Tool` variants but their
// gesture handlers arrive in Phase 1.
//
// The machine is pure: it maps (pointer event, current selection, DisplayViewport,
// ViewportTransform) to an outcome, holding only in-flight drag state. Selection
// itself is local UI state owned by the view (§7). Host-tested with synthetic
// pointer sequences.
// ============================================================================

use ambre_bus::app::toile::{DisplayViewport, ElementId, ShapeKind, ToileCommand};
use ambre_draw::{Point, Rect};

use super::hit::{hit_topmost, marquee_hits};
use super::viewport::ViewportTransform;

/// The active tool. Phase 0 drives `Select`; the rest are placeholders whose
/// gestures land in Phase 1 (Pen, Shape, Connector, Draw, Sticky, Text).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Tool {
    #[default]
    Select,
    Pen,
    Sticky,
    Shape(ShapeKind),
    Connector,
    Draw,
    Text,
    Hand,
}

/// Screen-space pick radius (physical px) around a transform handle.
const HANDLE_PICK_PX: f64 = 9.0;
/// Rotate handle offset above the selection's top edge (physical px).
pub const ROTATE_OFFSET_PX: f64 = 26.0;
/// Drawn size of a square resize handle (physical px), used by the overlay.
pub const HANDLE_SIZE_PX: f64 = 8.0;

/// The 8 resize handles plus the rotate handle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    Nw,
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
    Rotate,
}

/// What a single pointer event produced. Not `PartialEq`: the `Command` variant
/// wraps `ToileCommand`, which is deliberately not comparable (it carries opaque
/// payloads). Callers `match` on it.
#[derive(Clone, Debug)]
pub enum SelectOutcome {
    /// Nothing observable (idle move, or a drag step the view just repaints).
    None,
    /// Local selection changed to this set (never a command / never synced, §7).
    SelectionChanged(Vec<ElementId>),
    /// A committed document mutation to forward to Logic.
    Command(ToileCommand),
    /// A marquee rectangle is being dragged (world space); the overlay previews it.
    Marquee(Rect),
}

/// In-flight Select drag.
#[derive(Clone, Debug)]
enum Drag {
    Move {
        ids: Vec<ElementId>,
        start: Point,
        last: Point,
    },
    Resize {
        id: ElementId,
        base: Rect,
        handle: Handle,
    },
    Rotate {
        id: ElementId,
        center: Point,
        start_angle: f64,
        base_rot: f64,
    },
    Marquee {
        start: Point,
        current: Point,
    },
}

/// The Select tool's gesture machine.
#[derive(Default)]
pub struct SelectTool {
    drag: Option<Drag>,
}

impl SelectTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while a drag gesture is in flight (the view keeps repainting).
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Begin a gesture at a pointer press. `screen` is physical-pixel canvas
    /// coordinates; `world` is `screen` mapped through `cam.screen_to_world()`.
    pub fn pointer_down(
        &mut self,
        world: Point,
        screen: Point,
        selection: &[ElementId],
        display: &DisplayViewport,
        cam: &ViewportTransform,
    ) -> SelectOutcome {
        // A single-element selection exposes transform handles; test them first so
        // a handle press starts a resize/rotate rather than a move.
        if selection.len() == 1
            && let Some(sb) = selection_bounds(selection, display)
        {
            let screen_b = world_rect_to_screen(sb, cam);
            if let Some(h) = pick_handle(screen_b, screen) {
                let id = selection[0].clone();
                match h {
                    Handle::Rotate => {
                        let center = sb.center();
                        self.drag = Some(Drag::Rotate {
                            id: id.clone(),
                            center,
                            start_angle: (world - center).atan2(),
                            base_rot: element_rotation(display, &id),
                        });
                    }
                    _ => {
                        self.drag = Some(Drag::Resize {
                            id,
                            base: sb,
                            handle: h,
                        });
                    }
                }
                return SelectOutcome::None;
            }
        }

        // No handle: hit-test the board.
        match hit_topmost(&display.nodes, world) {
            Some(id) => {
                let changed = !selection.contains(&id);
                let ids = if changed {
                    vec![id]
                } else {
                    selection.to_vec()
                };
                self.drag = Some(Drag::Move {
                    ids: ids.clone(),
                    start: world,
                    last: world,
                });
                if changed {
                    SelectOutcome::SelectionChanged(ids)
                } else {
                    SelectOutcome::None
                }
            }
            None => {
                self.drag = Some(Drag::Marquee {
                    start: world,
                    current: world,
                });
                SelectOutcome::None
            }
        }
    }

    /// Continue the in-flight gesture as the pointer moves.
    pub fn pointer_move(&mut self, world: Point) -> SelectOutcome {
        match self.drag.as_mut() {
            Some(Drag::Move { last, .. }) => {
                *last = world;
                SelectOutcome::None
            }
            Some(Drag::Resize { .. }) | Some(Drag::Rotate { .. }) => SelectOutcome::None,
            Some(Drag::Marquee { start, current }) => {
                *current = world;
                SelectOutcome::Marquee(Rect::from_points(*start, *current))
            }
            None => SelectOutcome::None,
        }
    }

    /// End the gesture, emitting the committing command (or selection change).
    pub fn pointer_up(&mut self, world: Point, display: &DisplayViewport) -> SelectOutcome {
        let Some(drag) = self.drag.take() else {
            return SelectOutcome::None;
        };
        match drag {
            Drag::Move { ids, start, last } => {
                let delta = last - start;
                if delta.hypot2() <= f64::EPSILON {
                    SelectOutcome::None
                } else {
                    SelectOutcome::Command(ToileCommand::MoveElements { ids, delta })
                }
            }
            Drag::Resize { id, base, handle } => {
                let new = resize_rect(base, handle, world);
                SelectOutcome::Command(ToileCommand::ResizeElement { id, new })
            }
            Drag::Rotate {
                id,
                center,
                start_angle,
                base_rot,
            } => {
                let now = (world - center).atan2();
                let radians = base_rot + (now - start_angle);
                SelectOutcome::Command(ToileCommand::RotateElement { id, radians })
            }
            Drag::Marquee { start, current } => {
                let rect = Rect::from_points(start, current);
                SelectOutcome::SelectionChanged(marquee_hits(&display.nodes, rect))
            }
        }
    }

    /// Abort any in-flight gesture (Escape / focus loss). No command emitted.
    pub fn cancel(&mut self) {
        self.drag = None;
    }
}

/// World union AABB of the selected elements (`None` if empty / unresolved).
pub fn selection_bounds(selection: &[ElementId], display: &DisplayViewport) -> Option<Rect> {
    let mut acc: Option<Rect> = None;
    for node in &display.nodes {
        if let ambre_bus::app::toile::DisplayNode::Element(e) = node
            && selection.contains(&e.id)
        {
            acc = Some(match acc {
                Some(r) => r.union(e.bounds),
                None => e.bounds,
            });
        }
    }
    acc
}

/// Map a world-space axis-aligned rectangle to screen space (translate+scale
/// camera, so the result is still axis-aligned).
pub fn world_rect_to_screen(r: Rect, cam: &ViewportTransform) -> Rect {
    let t = cam.world_to_screen();
    Rect::from_points(t * Point::new(r.x0, r.y0), t * Point::new(r.x1, r.y1))
}

/// The 8 resize-handle centers (screen space) for a selection's screen bounds.
pub fn resize_handle_points(b: Rect) -> [(Handle, Point); 8] {
    let (cx, cy) = (b.center().x, b.center().y);
    [
        (Handle::Nw, Point::new(b.x0, b.y0)),
        (Handle::N, Point::new(cx, b.y0)),
        (Handle::Ne, Point::new(b.x1, b.y0)),
        (Handle::E, Point::new(b.x1, cy)),
        (Handle::Se, Point::new(b.x1, b.y1)),
        (Handle::S, Point::new(cx, b.y1)),
        (Handle::Sw, Point::new(b.x0, b.y1)),
        (Handle::W, Point::new(b.x0, cy)),
    ]
}

/// The rotate-handle center (screen space): above the top-center edge.
pub fn rotate_handle_point(b: Rect) -> Point {
    Point::new(b.center().x, b.y0 - ROTATE_OFFSET_PX)
}

/// Which handle (if any) a screen point presses, within the pick radius.
fn pick_handle(screen_b: Rect, screen: Point) -> Option<Handle> {
    if rotate_handle_point(screen_b).distance(screen) <= HANDLE_PICK_PX {
        return Some(Handle::Rotate);
    }
    resize_handle_points(screen_b)
        .into_iter()
        .find(|(_, p)| p.distance(screen) <= HANDLE_PICK_PX)
        .map(|(h, _)| h)
}

/// Resize `base` by dragging `handle` to world point `to`; the opposite edge(s)
/// stay put. Corners move both axes, edge handles move one.
fn resize_rect(base: Rect, handle: Handle, to: Point) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (base.x0, base.y0, base.x1, base.y1);
    match handle {
        Handle::Nw => {
            x0 = to.x;
            y0 = to.y;
        }
        Handle::N => y0 = to.y,
        Handle::Ne => {
            x1 = to.x;
            y0 = to.y;
        }
        Handle::E => x1 = to.x,
        Handle::Se => {
            x1 = to.x;
            y1 = to.y;
        }
        Handle::S => y1 = to.y,
        Handle::Sw => {
            x0 = to.x;
            y1 = to.y;
        }
        Handle::W => x0 = to.x,
        Handle::Rotate => {}
    }
    // Normalize so x0<=x1, y0<=y1 even if the drag crossed the opposite edge.
    Rect::new(x0, y0, x1, y1).abs()
}

/// Extract the rotation angle (radians) baked into an element's transform. The
/// Logic derive stores it as a rotation about the AABB center, so the linear part
/// is a pure rotation and `atan2(b, a)` recovers the angle.
fn element_rotation(display: &DisplayViewport, id: &ElementId) -> f64 {
    for node in &display.nodes {
        if let ambre_bus::app::toile::DisplayNode::Element(e) = node
            && &e.id == id
        {
            let c = e.transform.as_coeffs();
            return c[1].atan2(c[0]);
        }
    }
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{
        DisplayElement, DisplayGeom, DisplayNode, ElementKind, ElementStyle,
    };
    use ambre_draw::{Affine, Vec2};

    fn rect_elem(id: &str, bounds: Rect) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: ElementId(id.to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds,
            locked: false,
            style: ElementStyle::default(),
            geom: DisplayGeom::Rect,
        })
    }

    fn viewport(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            viewport: Default::default(),
            nodes,
        }
    }

    // Camera with identity-ish mapping: at zoom 1 and origin-centered, world (0,0)
    // maps to screen center. Use a size so screen == world + size/2.
    fn cam() -> ViewportTransform {
        ViewportTransform {
            center: Point::ZERO,
            zoom: 1.0,
            size: ambre_draw::Size::new(1000.0, 1000.0),
        }
    }

    fn to_screen(cam: &ViewportTransform, w: Point) -> Point {
        cam.world_to_screen() * w
    }

    #[test]
    fn click_selects_then_drag_emits_move() {
        let disp = viewport(vec![rect_elem("a", Rect::new(0.0, 0.0, 100.0, 100.0))]);
        let cam = cam();
        let mut tool = SelectTool::new();
        let w0 = Point::new(50.0, 50.0);
        let out = tool.pointer_down(w0, to_screen(&cam, w0), &[], &disp, &cam);
        assert!(
            matches!(out, SelectOutcome::SelectionChanged(ref ids) if ids == &[ElementId("a".into())])
        );
        tool.pointer_move(Point::new(70.0, 60.0));
        let up = tool.pointer_up(Point::new(70.0, 60.0), &disp);
        match up {
            SelectOutcome::Command(ToileCommand::MoveElements { ids, delta }) => {
                assert_eq!(ids, vec![ElementId("a".into())]);
                assert_eq!(delta, Vec2::new(20.0, 10.0));
            }
            other => panic!("expected MoveElements, got {other:?}"),
        }
    }

    #[test]
    fn click_without_movement_emits_no_command() {
        let disp = viewport(vec![rect_elem("a", Rect::new(0.0, 0.0, 100.0, 100.0))]);
        let cam = cam();
        let mut tool = SelectTool::new();
        let w = Point::new(50.0, 50.0);
        tool.pointer_down(w, to_screen(&cam, w), &[], &disp, &cam);
        assert!(matches!(tool.pointer_up(w, &disp), SelectOutcome::None));
    }

    #[test]
    fn drag_empty_makes_a_marquee_then_selects() {
        let disp = viewport(vec![rect_elem("a", Rect::new(0.0, 0.0, 100.0, 100.0))]);
        let cam = cam();
        let mut tool = SelectTool::new();
        // Press on empty space.
        let w0 = Point::new(-200.0, -200.0);
        let out = tool.pointer_down(w0, to_screen(&cam, w0), &[], &disp, &cam);
        assert!(matches!(out, SelectOutcome::None));
        // Drag to enclose element "a".
        let m = tool.pointer_move(Point::new(200.0, 200.0));
        assert!(matches!(m, SelectOutcome::Marquee(_)));
        let up = tool.pointer_up(Point::new(200.0, 200.0), &disp);
        assert!(
            matches!(up, SelectOutcome::SelectionChanged(ref ids) if ids == &[ElementId("a".into())])
        );
    }

    #[test]
    fn drag_corner_handle_emits_resize() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let mut tool = SelectTool::new();
        let sel = vec![ElementId("a".into())];
        // Press the SE handle (world corner 100,100 -> its screen position).
        let se_world = Point::new(100.0, 100.0);
        let se_screen = to_screen(&cam, se_world);
        let out = tool.pointer_down(se_world, se_screen, &sel, &disp, &cam);
        assert!(matches!(out, SelectOutcome::None));
        assert!(tool.is_dragging());
        // Drag it out to (150,140).
        let up = tool.pointer_up(Point::new(150.0, 140.0), &disp);
        match up {
            SelectOutcome::Command(ToileCommand::ResizeElement { id, new }) => {
                assert_eq!(id, ElementId("a".into()));
                assert_eq!(new, Rect::new(0.0, 0.0, 150.0, 140.0));
            }
            other => panic!("expected ResizeElement, got {other:?}"),
        }
    }

    #[test]
    fn drag_rotate_handle_emits_rotate() {
        let bounds = Rect::new(-50.0, -50.0, 50.0, 50.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let mut tool = SelectTool::new();
        let sel = vec![ElementId("a".into())];
        // Rotate handle sits above the top-center in screen space.
        let screen_b = world_rect_to_screen(bounds, &cam);
        let rot_screen = rotate_handle_point(screen_b);
        // The world point under that screen position (start angle reference).
        let rot_world = cam.screen_to_world() * rot_screen;
        let out = tool.pointer_down(rot_world, rot_screen, &sel, &disp, &cam);
        assert!(matches!(out, SelectOutcome::None));
        // Move the pointer to the element's right (world +x from center) -> the
        // vector angle changes from "straight up" to "straight right": +90 deg.
        let up = tool.pointer_up(Point::new(60.0, 0.0), &disp);
        match up {
            SelectOutcome::Command(ToileCommand::RotateElement { id, radians }) => {
                assert_eq!(id, ElementId("a".into()));
                assert!((radians - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
            }
            other => panic!("expected RotateElement, got {other:?}"),
        }
    }
}
