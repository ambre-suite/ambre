// ambre-toile/src/ui/tool.rs
// ============================================================================
// Tool state machine (toile-tools). `SelectTool` implements the Select tool
// fully (move / resize / rotate / marquee). `ToolMachine` wraps it and the
// creation tools (Shape / Connector / Draw / Pen / Sticky / Text). It
// dispatches each pointer event to the active tool and emits exactly one
// `ToileCommand` on gesture end (or a local selection change for a marquee).
//
// The machine is pure: it maps (pointer event, current selection, DisplayViewport,
// ViewportTransform) to an outcome, holding only in-flight draft state. Selection
// itself is local UI state owned by the view. Host-tested with synthetic
// pointer sequences.
// ============================================================================

use crate::message::{
    Anchor, CommentAnchor, DisplayGeom, DisplayNode, DisplayViewport, ElementKind, EndCaps,
    InitContent, PathType, ShapeKind, StrokePoint, ToileCommand,
};
use ambre_core::id::{Connector, Element, Id};
use ambre_core::{Board, Px, Rgba, StoredBoard};
use ambre_draw::{Affine, BezPath, Point, Rect, Size};
use kurbo::Shape;

use super::hit::{hit_topmost, marquee_hits};
use super::viewport::ViewportTransform;

/// The active tool. `Select` is the Phase-0 default; the creation tools (Pen,
/// Shape, Connector, Draw, Sticky, Text) and `Hand` (pan-only) are driven by
/// `ToolMachine`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Tool {
    #[default]
    Select,
    Pen,
    /// Sticky note. Carries the card colour (RGBA) picked from the palette so a
    /// click places the note in that colour atomically (folded into
    /// `CreateElement.fill`, toile-tools). `aria-pressed` compares by
    /// discriminant, so any `Sticky(_)` reads active regardless of colour.
    Sticky(Rgba),
    Shape(ShapeKind),
    Connector,
    Draw,
    Text,
    /// Drop a comment thread where it clicks, on an element or on empty board
    /// (`TOILE-UI-042`, `toile-comments`). The click opens a composer rather
    /// than a thread. A thread is born with its first message, so nothing is
    /// written until the user posts (`TOILE-UI-307`).
    Comment,
    /// The highlighter (`TOILE-UI-074`, `toile-presence`): a drag streams a
    /// laser point to the other peers and leaves no element behind. It writes
    /// nothing at all -- the trail is presence, relayed and forgotten.
    Laser,
    Hand,
}

/// Screen-space pick radius (logical px) around a corner / rotate handle.
const HANDLE_PICK: ambre_core::Px<ambre_core::View> = ambre_core::Px::<ambre_core::View>::new(9.0);
/// Screen-space grab band (logical px) around a selection edge. The whole side
/// resizes (with a directional cursor), even though only the corners draw a
/// visual square (toile-tools).
const EDGE_GRAB: ambre_core::Px<ambre_core::View> = ambre_core::Px::<ambre_core::View>::new(6.0);
/// Rotate handle offset above the selection's top edge (logical px).
pub const ROTATE_OFFSET: ambre_core::Px<ambre_core::View> =
    ambre_core::Px::<ambre_core::View>::new(26.0);
/// Drawn size of a square resize handle (logical px), used by the overlay.
pub const HANDLE_SIZE: ambre_core::Px<ambre_core::View> =
    ambre_core::Px::<ambre_core::View>::new(8.0);

/// The cursor an in-flight Select gesture calls for: a directional
/// resize / grab on a transform `Handle`, or a closed-hand move. Mapped to a CSS
/// cursor string by the view (via `palette::resize_cursor`) so the cursor stays
/// stable for the whole drag.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GestureCursor {
    Handle(Handle),
    Move,
}

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
    /// Local selection changed to this set (never a command / never synced,).
    SelectionChanged(Vec<Id<Element>>),
    /// A committed document mutation to forward to Logic.
    Command(ToileCommand),
    /// A marquee rectangle is being dragged (world space); the overlay previews it.
    Marquee(Rect),
}

/// In-flight Select drag.
#[derive(Clone, Debug)]
enum Drag {
    Move {
        ids: Vec<Id<Element>>,
        start: Point,
        last: Point,
    },
    Resize {
        id: Id<Element>,
        base: Rect,
        handle: Handle,
        /// Pointer at press (world). The release compares against it: a press
        /// that moved nothing emits no command (`TOILE-UI-252`).
        start: Point,
        /// The element's rotation (radians about `base.center()`), captured at
        /// press so the resize math can keep the opposite corner world-fixed on a
        /// rotated element.
        rot: ambre_core::Radians,
        /// Live pointer (world), updated each move so the view can preview the
        /// resize without a Logic round-trip.
        last: Point,
    },
    Rotate {
        id: Id<Element>,
        center: Point,
        /// Pointer at press (world), read by the release like the resize's own
        /// (`TOILE-UI-252`).
        start: Point,
        start_angle: ambre_core::Radians,
        base_rot: ambre_core::Radians,
        /// Live pointer angle (world radians about `center`), updated each move
        /// for the live preview.
        last_angle: ambre_core::Radians,
    },
    Marquee {
        start: Point,
        current: Point,
        /// The group scope at press time, so release resolves each hit the way a
        /// click would (`TOILE-UI-236`).
        scope: Option<Id<Element>>,
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

    /// True when the last press landed on empty board and started a marquee.
    /// No element, handle or frame title took it.
    pub fn pressed_empty_board(&self) -> bool {
        matches!(self.drag, Some(Drag::Marquee { .. }))
    }

    /// Begin a gesture at a pointer press. `screen` is logical-pixel canvas
    /// coordinates; `world` is `screen` mapped through `cam.screen_to_world()`.
    pub fn pointer_down(
        &mut self,
        world: Point,
        screen: Point,
        selection: &[Id<Element>],
        display: &DisplayViewport,
        cam: &ViewportTransform,
        scope: Option<&Id<Element>>,
    ) -> SelectOutcome {
        // A single-element selection exposes transform handles; test them first so
        // a handle press starts a resize/rotate rather than a move.
        if let [only] = selection
            && let Some(sb) = selection_bounds(selection, display)
        {
            let screen_b = world_rect_to_screen(sb, cam);
            let rot = element_rotation(display, only);
            // The drawn box is `screen_b` rotated by `rot` about its center. A
            // point hits a handle iff its de-rotated image hits the axis-aligned
            // box (toile-selection). Press and hover share this de-rotation so they
            // always agree.
            let pick = unrotate_point(screen, screen_b.center(), rot);
            if let Some(h) = selection_handle_at(only, display, screen_b, pick) {
                let id = only.clone();
                match h {
                    Handle::Rotate => {
                        let center = sb.center();
                        let start_angle = ambre_core::Radians::new((world - center).atan2());
                        self.drag = Some(Drag::Rotate {
                            id: id.clone(),
                            center,
                            start: world,
                            start_angle,
                            base_rot: element_rotation(display, &id),
                            last_angle: start_angle,
                        });
                    }
                    _ => {
                        self.drag = Some(Drag::Resize {
                            id,
                            base: sb,
                            handle: h,
                            start: world,
                            rot,
                            last: world,
                        });
                    }
                }
                return SelectOutcome::None;
            }
        }

        // A frame's title sits outside its box, so the board hit test below
        // cannot see it. A press on one selects that frame: it is how a user
        // grabs a frame without clicking through what is inside it
        // (`TOILE-254`).
        let title_hit = super::hit::frame_title_at(&display.nodes, screen, cam);

        // No handle: hit-test the board, then resolve the raw hit up its group
        // chain to the selection target. That is the top-most enclosing group,
        // unless a scope has been entered, so a grouped element selects as its
        // group.
        let pick = super::hit::pick_floor(cam);
        match title_hit.or_else(|| hit_topmost(&display.nodes, world, pick)) {
            Some(hit) => {
                let id = resolve_group_target(hit, display, scope);
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
                    scope: scope.cloned(),
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
            Some(Drag::Resize { last, .. }) => {
                *last = world;
                SelectOutcome::None
            }
            Some(Drag::Rotate {
                center, last_angle, ..
            }) => {
                *last_angle = ambre_core::Radians::new((world - *center).atan2());
                SelectOutcome::None
            }
            Some(Drag::Marquee { start, current, .. }) => {
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
            Drag::Resize {
                id,
                base,
                handle,
                start,
                rot,
                ..
            } => {
                if (world - start).hypot2() <= f64::EPSILON {
                    return SelectOutcome::None;
                }
                let new = resize_rect_rotated(base, handle, world, rot);
                SelectOutcome::Command(ToileCommand::ResizeElement { id, new })
            }
            Drag::Rotate {
                id,
                center,
                start,
                start_angle,
                base_rot,
                ..
            } => {
                if (world - start).hypot2() <= f64::EPSILON {
                    return SelectOutcome::None;
                }
                let now = ambre_core::Radians::new((world - center).atan2());
                let radians = base_rot + (now - start_angle);
                SelectOutcome::Command(ToileCommand::RotateElement { id, radians })
            }
            Drag::Marquee {
                start,
                current,
                scope,
            } => {
                let rect = Rect::from_points(start, current);
                // Each covered element resolves to the target a click would
                // return: the top-most group, or itself (`TOILE-UI-236`).
                let mut targets = Vec::new();
                for hit in marquee_hits(&display.nodes, rect) {
                    let target = resolve_group_target(hit, display, scope.as_ref());
                    if !targets.contains(&target) {
                        targets.push(target);
                    }
                }
                SelectOutcome::SelectionChanged(targets)
            }
        }
    }

    /// Abort any in-flight gesture (Escape / focus loss). No command emitted.
    pub fn cancel(&mut self) {
        self.drag = None;
    }

    /// The live marquee rectangle (world space) if a rubber-band drag is in
    /// flight. The machine can surface it as an overlay preview without an
    /// outcome round-trip.
    pub fn marquee(&self) -> Option<Rect> {
        match &self.drag {
            Some(Drag::Marquee { start, current, .. }) => Some(Rect::from_points(*start, *current)),
            _ => None,
        }
    }

    /// The handle a resting hover targets, for the directional cursor. Same
    /// hit-test as `pointer_down`, but pure: no drag is started. Returns `None`
    /// unless the selection is exactly one element and the pointer is on its edge /
    /// corner / rotate handle. Never called while a drag is in flight.
    pub fn hover_handle(
        &self,
        selection: &[Id<Element>],
        display: &DisplayViewport,
        cam: &ViewportTransform,
        screen: Point,
    ) -> Option<Handle> {
        let [only] = selection else {
            return None;
        };
        let sb = selection_bounds(selection, display)?;
        let screen_b = world_rect_to_screen(sb, cam);
        let rot = element_rotation(display, only);
        let pick = unrotate_point(screen, screen_b.center(), rot);
        selection_handle_at(only, display, screen_b, pick)
    }

    /// The cursor the in-flight drag calls for, so it stays stable
    /// for the whole gesture regardless of where the pointer wanders. `None` at
    /// rest or during a marquee.
    pub fn gesture_cursor(&self) -> Option<GestureCursor> {
        match &self.drag {
            Some(Drag::Resize { handle, .. }) => Some(GestureCursor::Handle(*handle)),
            Some(Drag::Rotate { .. }) => Some(GestureCursor::Handle(Handle::Rotate)),
            Some(Drag::Move { .. }) => Some(GestureCursor::Move),
            _ => None,
        }
    }

    /// Per-element live transform for the in-flight gesture: `(id, bounds,
    /// rotation)` in world space. The view can paint the real object being resized
    /// / rotated / moved before the command commits on release. Empty when no
    /// gesture (or a marquee) is in flight.
    pub fn gesture_overrides(
        &self,
        display: &DisplayViewport,
    ) -> Vec<(Id<Element>, Rect, ambre_core::Radians)> {
        match &self.drag {
            Some(Drag::Resize {
                id,
                base,
                handle,
                rot,
                last,
                ..
            }) => vec![(
                id.clone(),
                resize_rect_rotated(*base, *handle, *last, *rot),
                *rot,
            )],
            Some(Drag::Rotate {
                id,
                start_angle,
                base_rot,
                last_angle,
                ..
            }) => {
                let bounds = element_bounds(display, id).unwrap_or_default();
                vec![(id.clone(), bounds, *base_rot + (*last_angle - *start_angle))]
            }
            // A press that has not moved yet paints where it committed, so it
            // overrides nothing and costs no content repaint (`TOILE-UI-271`).
            Some(Drag::Move { start, last, .. }) if last == start => Vec::new(),
            Some(Drag::Move { ids, start, last }) => {
                let delta = *last - *start;
                ids.iter()
                    .filter_map(|id| {
                        element_bounds(display, id)
                            .map(|b| (id.clone(), b + delta, element_rotation(display, id)))
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }
}

// =============================================================================
// Tool machine: the canvas-side consumer of the active tool. Owns the Select
// gesture machine plus the in-flight state of the creation tools (Shape /
// Connector / Draw / Pen / Sticky / Text). It maps a pointer sequence to
// exactly one `ToileCommand` on gesture end. Pure: it holds only in-flight
// draft state, never document data. Host-tested with synthetic pointer
// sequences.
// =============================================================================

/// Default sizes for click-placed (not drag-sized) elements, world units.
// A sticky note is square (Miro-style): same height as before, narrower width.
const STICKY_SIZE: Size = Size::new(120.0, 120.0);
const TEXT_SIZE: Size = Size::new(200.0, 40.0);
const CLICK_SHAPE_SIZE: Size = Size::new(120.0, 80.0);
/// Below this drag extent (world units) a shape gesture counts as a click.
const MIN_DRAG: Px<Board> = Px::<Board>::new(3.0);
/// Trailing pen nodes closer than this (world units) are the closing
/// double-click's duplicate press; dropped before the path commits.
const PEN_DEDUP: Px<Board> = Px::<Board>::new(2.0);

/// What a pointer / keyboard event produced across all tools. Overlay previews
/// (marquee, rubber-band, pen path, live stroke) are queried separately via
/// [`ToolMachine::preview`], so this enum carries only observable results.
#[derive(Debug)]
pub enum ToolOutcome {
    /// Nothing to act on (idle move, or a draft step the view just repaints).
    None,
    /// Local selection changed to this set (never a command, never synced,).
    SelectionChanged(Vec<Id<Element>>),
    /// A committed document mutation to forward to Logic.
    Command(ToileCommand),
    /// The comment tool dropped a thread here (`TOILE-UI-042`). The view opens
    /// the composer at the click, and the thread is created only once the first
    /// message is posted (`TOILE-UI-307`).
    Comment(CommentAnchor),
}

/// A local, unsynced overlay to paint while a gesture is in flight.
#[derive(Clone, Debug)]
pub enum Preview {
    /// Select rubber-band on empty canvas (world space).
    Marquee(Rect),
    /// Shape drag-rect, previewed as the shape it will become.
    Shape { kind: ShapeKind, rect: Rect },
    /// Connector rubber-band from the source anchor to the cursor.
    Connector { from: Point, to: Point },
    /// Vector pen: the committed nodes plus a live segment to the cursor.
    Pen(BezPath),
    /// In-flight freehand stroke (world-space samples).
    Stroke(Vec<Point>),
}

/// One vertex of the vector pen path: an on-curve point and its optional outgoing
/// control handle. The incoming handle is the reflection of `out` about `pt`
/// (smooth node), or `pt` itself when there is no handle (a corner / line node).
#[derive(Clone, Copy, Debug)]
struct PenNode {
    pt: Point,
    out: Option<Point>,
}

impl PenNode {
    fn in_handle(&self) -> Point {
        match self.out {
            Some(o) => Point::new(2.0 * self.pt.x - o.x, 2.0 * self.pt.y - o.y),
            None => self.pt,
        }
    }
}

/// In-flight gesture for the single-commit creation tools (Pen builds across
/// clicks and lives outside this enum).
#[derive(Clone, Debug)]
enum Draft {
    /// Shape drag-rect (rect / ellipse / triangle / line).
    Shape {
        kind: ShapeKind,
        start: Point,
        current: Point,
    },
    /// Connector drag from a source element anchor toward the cursor.
    Connector {
        from: Id<Element>,
        from_anch: Anchor,
        origin: Point,
        current: Point,
    },
    /// Freehand stroke samples accumulated between press and release.
    Draw { points: Vec<StrokePoint> },
    /// A click-placed sticky / text element, committed on release. `fill` carries
    /// the sticky's picked card colour (`None` for a text element, which keeps its
    /// per-kind default,).
    Place {
        kind: ElementKind,
        at: Point,
        fill: Option<Rgba>,
    },
    /// A click-placed comment thread, resolved to its anchor on release
    /// (`TOILE-UI-042`).
    Comment { at: Point },
}

/// The active-tool state machine. Replaces the bare `SelectTool` the view drove
/// in Phase 0. It dispatches each pointer event to the active tool and exposes a
/// single `ToolOutcome` plus a queryable `preview`.
pub struct ToolMachine {
    tool: Tool,
    select: SelectTool,
    /// Single-commit creation gesture in flight (Shape / Connector / Draw / Place).
    draft: Option<Draft>,
    /// Vector pen nodes committed so far (built across clicks;).
    pen: Vec<PenNode>,
    /// True while a pen node's control handle is being dragged.
    pen_dragging: bool,
    /// The highlighter's live point while it is held down (`TOILE-UI-074`).
    /// Read by the presence sampler each frame; it never becomes an element.
    laser: Option<Point>,
    /// Last world cursor, for the pen's live segment preview.
    cursor: Point,
    /// The pick floor in world units at the press (`hit::pick_floor`). The
    /// release reuses it, so a gesture picks under one reach.
    pick: Px<Board>,
}

impl Default for ToolMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolMachine {
    pub fn new() -> Self {
        Self {
            tool: Tool::Select,
            select: SelectTool::new(),
            draft: None,
            pen: Vec::new(),
            pen_dragging: false,
            laser: None,
            cursor: Point::ZERO,
            pick: super::hit::pick_floor(&ViewportTransform::default()),
        }
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Switch tools, abandoning any in-flight gesture on the previous tool.
    pub fn set_tool(&mut self, tool: Tool) {
        if tool != self.tool {
            self.cancel();
            self.tool = tool;
        }
    }

    /// True while a gesture is in flight (drag, stroke, or a pen path being
    /// built): the view keeps repainting and routes moves here.
    pub fn is_active(&self) -> bool {
        self.select.is_dragging()
            || self.draft.is_some()
            || !self.pen.is_empty()
            || self.laser.is_some()
    }

    /// True when the Select tool's last press landed on empty board, so an
    /// arrow under it may take the click (`TOILE-UI-350`).
    pub fn pressed_empty_board(&self) -> bool {
        self.tool == Tool::Select && self.select.pressed_empty_board()
    }

    /// The highlighter's live point, or `None` when it is not held down
    /// (`TOILE-UI-074`). The presence sampler reads it each frame; nothing else
    /// does, because the trail is never board state.
    pub fn laser(&self) -> Option<Point> {
        self.laser
    }

    /// The in-flight freehand samples, so a peer sees the stroke before it
    /// commits (`TOILE-UI-217`). `None` when no stroke is being drawn.
    pub fn live_stroke(&self) -> Option<&[StrokePoint]> {
        match &self.draft {
            Some(Draft::Draw { points }) => Some(points),
            _ => None,
        }
    }

    /// Abort any in-flight gesture (Escape / focus loss / tool switch).
    pub fn cancel(&mut self) {
        self.select.cancel();
        self.draft = None;
        self.pen.clear();
        self.pen_dragging = false;
        self.laser = None;
    }

    /// Handle Escape at the tool layer: cancel any in-flight gesture and, if a
    /// creation tool is armed, revert to Select. It then stops silently placing
    /// elements on the next click. Returns `true` when the active tool changed, so
    /// the caller can republish the shared active-tool signal.
    pub fn escape(&mut self) -> bool {
        self.cancel();
        if self.tool != Tool::Select {
            self.tool = Tool::Select;
            true
        } else {
            false
        }
    }

    pub fn pointer_down(
        &mut self,
        world: Point,
        screen: Point,
        selection: &[Id<Element>],
        display: &DisplayViewport,
        cam: &ViewportTransform,
        scope: Option<&Id<Element>>,
    ) -> ToolOutcome {
        self.cursor = world;
        self.pick = super::hit::pick_floor(cam);
        match self.tool {
            Tool::Select => map_select(
                self.select
                    .pointer_down(world, screen, selection, display, cam, scope),
            ),
            Tool::Hand => ToolOutcome::None,
            Tool::Shape(kind) => {
                self.draft = Some(Draft::Shape {
                    kind,
                    start: world,
                    current: world,
                });
                ToolOutcome::None
            }
            Tool::Draw => {
                self.draft = Some(Draft::Draw {
                    points: vec![stroke_point(world)],
                });
                ToolOutcome::None
            }
            Tool::Sticky(color) => {
                self.draft = Some(Draft::Place {
                    kind: ElementKind::Sticky,
                    at: world,
                    fill: Some(color),
                });
                ToolOutcome::None
            }
            Tool::Text => {
                self.draft = Some(Draft::Place {
                    kind: ElementKind::Text,
                    at: world,
                    fill: None,
                });
                ToolOutcome::None
            }
            Tool::Comment => {
                self.draft = Some(Draft::Comment { at: world });
                ToolOutcome::None
            }
            Tool::Laser => {
                self.laser = Some(world);
                ToolOutcome::None
            }
            Tool::Connector => {
                if let Some(id) = hit_topmost(&display.nodes, world, self.pick) {
                    let b = element_bounds(display, &id).unwrap_or_default();
                    let anch = nearest_anchor(b, world);
                    self.draft = Some(Draft::Connector {
                        from: id,
                        from_anch: anch,
                        origin: anchor_point(b, anch),
                        current: world,
                    });
                }
                ToolOutcome::None
            }
            Tool::Pen => {
                self.pen_dragging = true;
                self.pen.push(PenNode {
                    pt: world,
                    out: None,
                });
                ToolOutcome::None
            }
        }
    }

    pub fn pointer_move(&mut self, world: Point) -> ToolOutcome {
        self.cursor = world;
        match self.draft.as_mut() {
            Some(Draft::Shape { current, .. }) | Some(Draft::Connector { current, .. }) => {
                *current = world;
                ToolOutcome::None
            }
            Some(Draft::Draw { points }) => {
                points.push(stroke_point(world));
                ToolOutcome::None
            }
            Some(Draft::Place { .. }) | Some(Draft::Comment { .. }) => ToolOutcome::None,
            None => {
                // The highlighter streams while it is held down and writes
                // nothing (`TOILE-UI-074`). The point rides the next presence
                // sample, and the board never hears about it.
                if self.laser.is_some() {
                    self.laser = Some(world);
                    return ToolOutcome::None;
                }
                if self.tool == Tool::Pen {
                    if self.pen_dragging
                        && let Some(node) = self.pen.last_mut()
                    {
                        node.out = Some(world);
                    }
                    ToolOutcome::None
                } else {
                    map_select(self.select.pointer_move(world))
                }
            }
        }
    }

    pub fn pointer_up(&mut self, world: Point, display: &DisplayViewport) -> ToolOutcome {
        // Lifting the highlighter clears the point, which is what tells the
        // other peers to drop the trail (`TOILE-UI-222`).
        if self.laser.take().is_some() {
            return ToolOutcome::None;
        }
        if let Some(draft) = self.draft.take() {
            return match draft {
                Draft::Shape {
                    kind,
                    start,
                    current,
                } => {
                    let mut rect = Rect::from_points(start, current).abs();
                    if rect.width() < MIN_DRAG.get() || rect.height() < MIN_DRAG.get() {
                        rect = Rect::from_origin_size(start, CLICK_SHAPE_SIZE);
                    }
                    ToolOutcome::Command(ToileCommand::CreateElement {
                        kind: ElementKind::Shape(kind),
                        at: Point::new(rect.x0, rect.y0),
                        size: rect.size(),
                        content: InitContent::None,
                        fill: None,
                        correlate: None,
                    })
                }
                Draft::Connector {
                    from, from_anch, ..
                } => match hit_topmost(&display.nodes, world, self.pick) {
                    Some(to) if to != from => {
                        let tb = element_bounds(display, &to).unwrap_or_default();
                        let to_anch = nearest_anchor(tb, world);
                        ToolOutcome::Command(ToileCommand::CreateConnector {
                            from,
                            from_anch,
                            to,
                            to_anch,
                            path_type: PathType::Straight,
                            arrows: EndCaps {
                                start: crate::message::Cap::Plain,
                                end: crate::message::Cap::Arrow,
                            },
                        })
                    }
                    _ => ToolOutcome::None,
                },
                Draft::Draw { mut points } => {
                    points.push(stroke_point(world));
                    if points.len() < 2 {
                        ToolOutcome::None
                    } else {
                        ToolOutcome::Command(ToileCommand::CommitStroke {
                            points,
                            style: draw_style(),
                        })
                    }
                }
                Draft::Comment { at } => {
                    // The element under the release owns the pin, so it follows
                    // that element through every move (`TOILE-024`); an empty
                    // board keeps a free world point.
                    let anchor = match hit_topmost(&display.nodes, world, self.pick) {
                        Some(id) => CommentAnchor::Element(id),
                        None => CommentAnchor::Point(at),
                    };
                    ToolOutcome::Comment(anchor)
                }
                Draft::Place { kind, at, fill } => {
                    let size = if kind == ElementKind::Sticky {
                        STICKY_SIZE
                    } else {
                        TEXT_SIZE
                    };
                    ToolOutcome::Command(ToileCommand::CreateElement {
                        kind,
                        at,
                        size,
                        content: InitContent::Text(String::new()),
                        fill,
                        correlate: None,
                    })
                }
            };
        }
        if self.tool == Tool::Pen {
            self.pen_dragging = false;
            return ToolOutcome::None;
        }
        map_select(self.select.pointer_up(world, display))
    }

    /// Double-click. Finishes the vector pen path; a no-op otherwise.
    pub fn double_click(&mut self, _world: Point) -> ToolOutcome {
        if self.tool == Tool::Pen {
            self.finish_pen()
        } else {
            ToolOutcome::None
        }
    }

    /// Build the pen path and emit it as a single `CreateElement`.
    fn finish_pen(&mut self) -> ToolOutcome {
        self.pen_dragging = false;
        let mut nodes = std::mem::take(&mut self.pen);
        // The finishing double-click lands a duplicate press on the last node;
        // drop trailing near-duplicates so the path is not degenerate.
        while let [.., before, last] = nodes.as_slice()
            && last.pt.distance(before.pt) < PEN_DEDUP.get()
        {
            nodes.pop();
        }
        if nodes.len() < 2 {
            return ToolOutcome::None;
        }
        let path = pen_bezpath(&nodes);
        let bb = path.bounding_box();
        ToolOutcome::Command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Path),
            at: Point::new(bb.x0, bb.y0),
            size: bb.size(),
            content: InitContent::Path(path),
            fill: None,
            correlate: None,
        })
    }

    /// The current overlay preview, derived from in-flight state.
    pub fn preview(&self) -> Option<Preview> {
        if let Some(d) = &self.draft {
            return match d {
                Draft::Shape {
                    kind,
                    start,
                    current,
                } => Some(Preview::Shape {
                    kind: *kind,
                    rect: Rect::from_points(*start, *current).abs(),
                }),
                Draft::Connector {
                    origin, current, ..
                } => Some(Preview::Connector {
                    from: *origin,
                    to: *current,
                }),
                Draft::Draw { points } => Some(Preview::Stroke(
                    points
                        .iter()
                        .map(|p| Point::new(p.x.widened().get(), p.y.widened().get()))
                        .collect(),
                )),
                Draft::Place { .. } | Draft::Comment { .. } => None,
            };
        }
        if !self.pen.is_empty() {
            // Committed nodes plus a rubber segment to the live cursor.
            let mut nodes = self.pen.clone();
            nodes.push(PenNode {
                pt: self.cursor,
                out: None,
            });
            return Some(Preview::Pen(pen_bezpath(&nodes)));
        }
        self.select.marquee().map(Preview::Marquee)
    }

    /// The handle a resting hover targets, for the directional resize
    /// cursor. `None` unless the Select tool is armed, no gesture is in
    /// flight, and the pointer is on a single selection's edge / corner.
    pub fn hover_handle(
        &self,
        selection: &[Id<Element>],
        display: &DisplayViewport,
        cam: &ViewportTransform,
        screen: Point,
    ) -> Option<Handle> {
        if self.tool != Tool::Select || self.is_active() {
            return None;
        }
        self.select.hover_handle(selection, display, cam, screen)
    }

    /// Per-element live transform for the in-flight Select gesture, forwarded
    /// so the view can preview the real object during resize / rotate / move.
    pub fn gesture_overrides(
        &self,
        display: &DisplayViewport,
    ) -> Vec<(Id<Element>, Rect, ambre_core::Radians)> {
        self.select.gesture_overrides(display)
    }

    /// The cursor the in-flight Select drag calls for; `None`
    /// unless a resize / rotate / move is in flight on the Select tool.
    pub fn gesture_cursor(&self) -> Option<GestureCursor> {
        if self.tool != Tool::Select {
            return None;
        }
        self.select.gesture_cursor()
    }
}

/// Map a `SelectOutcome` into the unified `ToolOutcome`. The marquee rectangle is
/// surfaced through [`ToolMachine::preview`], not as an outcome.
fn map_select(out: SelectOutcome) -> ToolOutcome {
    match out {
        SelectOutcome::None | SelectOutcome::Marquee(_) => ToolOutcome::None,
        SelectOutcome::SelectionChanged(ids) => ToolOutcome::SelectionChanged(ids),
        SelectOutcome::Command(cmd) => ToolOutcome::Command(cmd),
    }
}

/// A world point as a full-pressure stroke sample.
fn stroke_point(world: Point) -> StrokePoint {
    StrokePoint::full_pressure(
        Px::<StoredBoard>::new(world.x as f32),
        Px::<StoredBoard>::new(world.y as f32),
    )
}

/// Default freehand style: a solid stroke, no fill.
fn draw_style() -> crate::message::ElementStyle {
    crate::message::ElementStyle {
        fill: None,
        stroke: Some(Rgba::from_bits(0x3333_33FF)),
        stroke_w: Px::<StoredBoard>::new(2.0),
        ..Default::default()
    }
}

/// World AABB of a single element (`None` if not in the current viewport).
pub(crate) fn element_bounds(display: &DisplayViewport, id: &Id<Element>) -> Option<Rect> {
    selection_bounds(std::slice::from_ref(id), display)
}

/// Whether `id` names a group node in the current display (its geometry is the
/// derived member union; it paints nothing,).
pub(crate) fn is_group(id: &Id<Element>, display: &DisplayViewport) -> bool {
    display.nodes.iter().any(|n| {
        matches!(n, DisplayNode::Element(e) if &e.id == id
            && matches!(e.geom, DisplayGeom::Group))
    })
}

/// Resolve a raw hit element to the selection target under the current group
/// `scope`. Walk the enclosing-group chain from the hit upward:
/// - no scope -> the top-most enclosing group (the whole group selects/transforms
///   as one), or the element itself if it is ungrouped.
/// - inside a scope `s` -> the chain member one level below `s`, so a scoped
///   click selects a member or an immediate sub-group. A click outside the
///   scope falls back to the top-most group.
pub(crate) fn resolve_group_target(
    hit: Id<Element>,
    display: &DisplayViewport,
    scope: Option<&Id<Element>>,
) -> Id<Element> {
    // The group of each element, read once. A peer writes the nesting, and
    // nothing bounds its depth. So each step of the walk costs one lookup,
    // never a scan of the display or of the chain (`TOILE-306`).
    let group_of: ambre_core::hash::HashMap<&Id<Element>, &Id<Element>> = display
        .nodes
        .iter()
        .filter_map(|n| match n {
            DisplayNode::Element(e) => e.group.as_ref().map(|g| (&e.id, g)),
            _ => None,
        })
        .collect();
    let mut chain = vec![hit.clone()];
    let mut seen: ambre_core::hash::HashSet<Id<Element>> = ambre_core::hash::HashSet::default();
    seen.insert(hit.clone());
    let mut cur = hit;
    // A peer can write a group into its own subtree. The walk stops at the
    // first group already in the chain, so a cycle ends it (`TOILE-UI-398`).
    while let Some(g) = group_of.get(&cur).map(|g| (*g).clone()) {
        if !seen.insert(g.clone()) {
            break;
        }
        chain.push(g.clone());
        cur = g;
    }
    // `chain` starts with `hit`, so every lookup below finds an entry. The
    // fallback, the top-most group the walk ended on, is never taken.
    let picked = match scope.and_then(|s| chain.iter().position(|c| c == s)) {
        Some(pos) => chain.get(pos.saturating_sub(1)),
        // Clicked outside the entered scope: select the whole top-most group.
        None => chain.last(),
    };
    picked.cloned().unwrap_or(cur)
}

/// The world point of an anchor on an element's AABB.
fn anchor_point(b: Rect, a: Anchor) -> Point {
    let c = b.center();
    match a {
        Anchor::Top => Point::new(c.x, b.y0),
        Anchor::Bottom => Point::new(c.x, b.y1),
        Anchor::Left => Point::new(b.x0, c.y),
        Anchor::Right => Point::new(b.x1, c.y),
        Anchor::Center => c,
        Anchor::Free { u, v } => Point::new(
            u.along(Px::<Board>::new(b.x0), Px::<Board>::new(b.x1))
                .get(),
            v.along(Px::<Board>::new(b.y0), Px::<Board>::new(b.y1))
                .get(),
        ),
    }
}

/// The cardinal anchor of `b` closest to world point `p`.
pub(crate) fn nearest_anchor(b: Rect, p: Point) -> Anchor {
    [Anchor::Top, Anchor::Bottom, Anchor::Left, Anchor::Right]
        .into_iter()
        .min_by(|a1, a2| {
            anchor_point(b, *a1)
                .distance(p)
                .total_cmp(&anchor_point(b, *a2).distance(p))
        })
        .unwrap_or(Anchor::Center)
}

/// Build a `BezPath` from pen nodes: a cubic segment when either endpoint has a
/// control handle, a straight line otherwise.
fn pen_bezpath(nodes: &[PenNode]) -> BezPath {
    let mut p = BezPath::new();
    let Some(first) = nodes.first() else {
        return p;
    };
    p.move_to(first.pt);
    for (a, b) in nodes.iter().zip(nodes.iter().skip(1)) {
        if a.out.is_none() && b.out.is_none() {
            p.line_to(b.pt);
        } else {
            p.curve_to(a.out.unwrap_or(a.pt), b.in_handle(), b.pt);
        }
    }
    p
}

/// World union AABB of the selected elements (`None` if empty / unresolved).
/// A locked element is skipped, so it never wears the selection box
/// (`TOILE-UI-231`) even when select-all or a stale id put it in the list.
pub fn selection_bounds(selection: &[Id<Element>], display: &DisplayViewport) -> Option<Rect> {
    let mut acc: Option<Rect> = None;
    for node in &display.nodes {
        if let crate::message::DisplayNode::Element(e) = node
            && selection.contains(&e.id)
            && e.locked == crate::message::LockState::Unlocked
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

/// The 4 corner resize-handle centers (screen space) for a selection's screen
/// bounds -- the only handles drawn as a **visual square**. The sides are still
/// grabbable / resizable (see `edge_handle_at`), they just show no square, which
/// keeps the box light (toile-selection).
pub fn resize_handle_points(b: Rect) -> [(Handle, Point); 4] {
    [
        (Handle::Nw, Point::new(b.x0, b.y0)),
        (Handle::Ne, Point::new(b.x1, b.y0)),
        (Handle::Se, Point::new(b.x1, b.y1)),
        (Handle::Sw, Point::new(b.x0, b.y1)),
    ]
}

/// The rotate-handle center (screen space): above the top-center edge.
pub fn rotate_handle_point(b: Rect) -> Point {
    Point::new(b.center().x, b.y0 - f64::from(ROTATE_OFFSET.get()))
}

/// Which handle (if any) a screen point targets on a selection's screen bounds.
/// The whole edge is grabbable (Miro). The rotate knob and the 4 corners test by
/// proximity to their point (`HANDLE_PICK`). The 4 sides test by a grab band
/// (`EDGE_GRAB`) along the full length of the side. So a side resizes (with a
/// directional cursor) even though only the corners draw a visual square
/// (toile-selection).
///
/// Corners win over sides (tested first), so a corner press always resizes
/// diagonally. Used by both `pointer_down` (start a resize/rotate) and
/// `hover_handle` (pick the cursor), so press and hover always agree.
fn edge_handle_at(screen_b: Rect, screen: Point) -> Option<Handle> {
    if rotate_handle_point(screen_b).distance(screen) <= f64::from(HANDLE_PICK.get()) {
        return Some(Handle::Rotate);
    }
    // Corners first: a diagonal resize takes precedence over the two sides that
    // meet there.
    for (h, p) in [
        (Handle::Nw, Point::new(screen_b.x0, screen_b.y0)),
        (Handle::Ne, Point::new(screen_b.x1, screen_b.y0)),
        (Handle::Se, Point::new(screen_b.x1, screen_b.y1)),
        (Handle::Sw, Point::new(screen_b.x0, screen_b.y1)),
    ] {
        if p.distance(screen) <= f64::from(HANDLE_PICK.get()) {
            return Some(h);
        }
    }
    // Sides: within the grab band of the edge line AND projecting inside the
    // side's own extent. The band must not stick out past the corners.
    let grab = f64::from(EDGE_GRAB.get());
    let in_x = screen.x >= screen_b.x0 - grab && screen.x <= screen_b.x1 + grab;
    let in_y = screen.y >= screen_b.y0 - grab && screen.y <= screen_b.y1 + grab;
    if in_x && (screen.y - screen_b.y0).abs() <= grab {
        return Some(Handle::N);
    }
    if in_x && (screen.y - screen_b.y1).abs() <= grab {
        return Some(Handle::S);
    }
    if in_y && (screen.x - screen_b.x1).abs() <= grab {
        return Some(Handle::E);
    }
    if in_y && (screen.x - screen_b.x0).abs() <= grab {
        return Some(Handle::W);
    }
    None
}

/// The handle a de-rotated screen point targets on the single selection `id`.
/// A group offers its rotate handle alone: its box is the union of its members,
/// so there is nothing to stretch (`TOILE-UI-392`, `TOILE-155`). A press on its
/// edge falls through to the board hit test instead.
fn selection_handle_at(
    id: &Id<Element>,
    display: &DisplayViewport,
    screen_b: Rect,
    pick: Point,
) -> Option<Handle> {
    let h = edge_handle_at(screen_b, pick)?;
    (h == Handle::Rotate || !is_group(id, display)).then_some(h)
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

/// A rotation of `theta` about `center`.
fn rotate_about(center: Point, theta: ambre_core::Radians) -> Affine {
    Affine::translate(center.to_vec2())
        * Affine::rotate(theta.get())
        * Affine::translate(-center.to_vec2())
}

/// De-rotate a screen point into a rotated selection box's local (axis-aligned)
/// frame. The box is `screen_b` rotated by `theta` about its center, so a point
/// hits a handle iff `unrotate_point(p, center, theta)` hits the axis-aligned box.
fn unrotate_point(p: Point, center: Point, theta: ambre_core::Radians) -> Point {
    if theta == ambre_core::Radians::ZERO {
        return p;
    }
    rotate_about(center, -theta) * p
}

/// The point that stays fixed while `handle` resizes `base`. That is the opposite
/// corner for a corner handle, the midpoint of the opposite edge for a side
/// handle. Used to pin the anchor in world space when resizing a rotated element.
fn resize_anchor(base: Rect, handle: Handle) -> Point {
    let (cx, cy) = (base.center().x, base.center().y);
    match handle {
        Handle::Nw => Point::new(base.x1, base.y1),
        Handle::Ne => Point::new(base.x0, base.y1),
        Handle::Se => Point::new(base.x0, base.y0),
        Handle::Sw => Point::new(base.x1, base.y0),
        Handle::N => Point::new(cx, base.y1),
        Handle::S => Point::new(cx, base.y0),
        Handle::E => Point::new(base.x0, cy),
        Handle::W => Point::new(base.x1, cy),
        Handle::Rotate => base.center(),
    }
}

/// Resize a rotated element, keeping the anchor (opposite corner / edge) fixed in
/// world space. `base` is the element's unrotated local AABB and `rot` its angle
/// (about `base.center()`). The result is the new unrotated AABB. The caller then
/// re-applies `rot` about the NEW center (Logic derive), so we translate the rect
/// to compensate the center shift and pin the anchor. `rot == 0` degenerates to
/// the plain axis-aligned `resize_rect`.
///
/// Rotation about a point is translation-equivariant, so pinning the anchor is a
/// single closed-form shift despite the new center depending on that shift. The
/// shift is `shift = R_rot(c0) * anchor - R_rot(c1) * anchor`, with `c0`/`c1` the
/// old / new centers and `anchor` invariant under the resize.
fn resize_rect_rotated(base: Rect, handle: Handle, to: Point, rot: ambre_core::Radians) -> Rect {
    if rot == ambre_core::Radians::ZERO {
        return resize_rect(base, handle, to);
    }
    let c0 = base.center();
    // Pointer into the element's unrotated local frame (rotation about c0).
    let to_local = rotate_about(c0, -rot) * to;
    let new = resize_rect(base, handle, to_local);
    let anchor = resize_anchor(base, handle);
    let shift = (rotate_about(c0, rot) * anchor) - (rotate_about(new.center(), rot) * anchor);
    new + shift
}

/// Extract the rotation angle (radians) baked into an element's transform. The
/// Logic derive stores it as a rotation about the AABB center, so the linear part
/// is a pure rotation and `atan2(b, a)` recovers the angle.
pub fn element_rotation(display: &DisplayViewport, id: &Id<Element>) -> ambre_core::Radians {
    for node in &display.nodes {
        if let crate::message::DisplayNode::Element(e) = node
            && &e.id == id
        {
            let c = e.transform.as_coeffs();
            return ambre_core::Radians::new(c[1].atan2(c[0]));
        }
    }
    ambre_core::Radians::ZERO
}

/// What a double-click with the select tool opens an editor on.
#[derive(Debug, PartialEq)]
pub enum DoubleClickEdit {
    /// A frame's title, which sits outside the frame's own box.
    FrameTitle(Id<Element>),
    /// The body of an element that holds text.
    Body(Id<Element>),
    /// An arrow's caption.
    Caption(Id<Connector>),
}

/// The editor a double-click at `screen` opens, or `None` when it opens none
/// and the gesture falls through to the tool. `editable` answers whether an
/// element's body is text.
///
/// A frame's title comes first: the band sits outside the frame, where the
/// board hit test cannot see it (`TOILE-261`). An arrow is edited only where no
/// element is, so entering a group keeps working over a connector behind it.
pub fn double_click_edit(
    nodes: &[DisplayNode],
    screen: Point,
    cam: &ViewportTransform,
    editable: impl Fn(&Id<Element>) -> bool,
) -> Option<DoubleClickEdit> {
    if let Some(frame) = super::hit::frame_title_at(nodes, screen, cam) {
        return Some(DoubleClickEdit::FrameTitle(frame));
    }
    let world = cam.screen_to_world() * screen;
    let pick = super::hit::pick_floor(cam);
    match hit_topmost(nodes, world, pick) {
        Some(hit) if editable(&hit) => Some(DoubleClickEdit::Body(hit)),
        Some(_) => None,
        None => super::label::connector_at(nodes, world, pick).map(DoubleClickEdit::Caption),
    }
}

/// Drop from `selection` the ids the Logic thread reports the board no longer
/// holds (`TOILE-UI-385`). A peer's deletion reaches the view as a board
/// change, and the selection's data the view then refetches names them. Ids
/// are never reused, so a reply to an older selection drops only ids that are
/// gone for good. Returns whether anything was dropped.
pub fn drop_missing(selection: &mut Vec<Id<Element>>, missing: &[Id<Element>]) -> bool {
    if missing.is_empty() {
        return false;
    }
    let before = selection.len();
    selection.retain(|id| !missing.contains(id));
    selection.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{DisplayElement, DisplayGeom, DisplayNode, ElementKind, ElementStyle};
    use ambre_draw::{Affine, Vec2};

    /// `TOILE-UI-385`: the ids the board lost leave the selection, the others
    /// keep their order, and an answer naming none changes nothing.
    #[test]
    fn a_selection_drops_the_ids_the_board_lost() {
        let id = |s: &str| Id::<Element>::from_text(s.to_owned());
        let mut sel = vec![id("a"), id("b"), id("c")];
        assert!(drop_missing(&mut sel, &[id("b"), id("z")]));
        assert_eq!(sel, vec![id("a"), id("c")]);
        assert!(!drop_missing(&mut sel, &[]));
        assert!(!drop_missing(&mut sel, &[id("z")]));
        assert_eq!(sel, vec![id("a"), id("c")]);
    }

    fn rect_elem(id: &str, bounds: Rect) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: Id::<Element>::from_text(id.to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds,
            locked: crate::message::LockState::Unlocked,
            style: ElementStyle::default(),
            geom: DisplayGeom::Rect,
            link: None,
            group: None,
        })
    }

    /// A named frame: the one element whose title is clickable outside its own
    /// box (`TOILE-254`).
    fn frame_elem(id: &str, bounds: Rect, title: &str) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: Id::<Element>::from_text(id.to_owned()),
            kind: ElementKind::Frame,
            transform: Affine::IDENTITY,
            bounds,
            locked: crate::message::LockState::Unlocked,
            style: ElementStyle::default(),
            geom: DisplayGeom::Frame(title.to_owned()),
            link: None,
            group: None,
        })
    }

    /// A rectangle that is a member of group `group`.
    fn member_elem(id: &str, bounds: Rect, group: &str) -> DisplayNode {
        let DisplayNode::Element(mut e) = rect_elem(id, bounds) else {
            panic!("rect_elem builds an element")
        };
        e.group = Some(Id::<Element>::from_text(group.to_owned()));
        DisplayNode::Element(e)
    }

    /// A group node (paints nothing): `parent` is its own enclosing group, if any.
    fn group_node(id: &str, bounds: Rect, parent: Option<&str>) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: Id::<Element>::from_text(id.to_owned()),
            kind: ElementKind::Group,
            transform: Affine::IDENTITY,
            bounds,
            locked: crate::message::LockState::Unlocked,
            style: ElementStyle::default(),
            geom: DisplayGeom::Group,
            link: None,
            group: parent.map(|p| Id::<Element>::from_text(p.to_owned())),
        })
    }

    fn viewport(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            version: ambre_core::Seq::ZERO,
            grid_unit: Px::<Board>::new(20.0),
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes,
        }
    }

    #[test]
    fn click_on_group_member_selects_top_most_group() {
        // member `m` in group `g`, `g` itself nested in `outer`.
        let disp = viewport(vec![
            member_elem("m", Rect::new(0.0, 0.0, 100.0, 100.0), "g"),
            group_node("g", Rect::new(0.0, 0.0, 100.0, 100.0), Some("outer")),
            group_node("outer", Rect::new(0.0, 0.0, 100.0, 100.0), None),
        ]);
        // No scope: a click resolves to the outermost enclosing group.
        let target = resolve_group_target(Id::<Element>::from_text("m".into()), &disp, None);
        assert_eq!(target, Id::<Element>::from_text("outer".into()));
    }

    #[test]
    fn scoped_click_selects_the_member_inside_the_scope() {
        let disp = viewport(vec![
            member_elem("m", Rect::new(0.0, 0.0, 100.0, 100.0), "g"),
            group_node("g", Rect::new(0.0, 0.0, 100.0, 100.0), Some("outer")),
            group_node("outer", Rect::new(0.0, 0.0, 100.0, 100.0), None),
        ]);
        // Scope = outer: the click peels one level and selects the sub-group `g`.
        let scope = Id::<Element>::from_text("outer".into());
        let target =
            resolve_group_target(Id::<Element>::from_text("m".into()), &disp, Some(&scope));
        assert_eq!(target, Id::<Element>::from_text("g".into()));
        // Scope = g: the click selects the member itself.
        let scope = Id::<Element>::from_text("g".into());
        let target =
            resolve_group_target(Id::<Element>::from_text("m".into()), &disp, Some(&scope));
        assert_eq!(target, Id::<Element>::from_text("m".into()));
    }

    /// `TOILE-UI-398`: a peer wrote `g1` into `g2` and `g2` into `g1`. A click
    /// on a member ends, and picks a group of the cycle.
    #[test]
    fn a_click_into_a_group_cycle_ends() {
        let disp = viewport(vec![
            member_elem("m", Rect::new(0.0, 0.0, 100.0, 100.0), "g1"),
            group_node("g1", Rect::new(0.0, 0.0, 100.0, 100.0), Some("g2")),
            group_node("g2", Rect::new(0.0, 0.0, 100.0, 100.0), Some("g1")),
        ]);
        let target = resolve_group_target(Id::<Element>::from_text("m".into()), &disp, None);
        assert_eq!(target, Id::<Element>::from_text("g2".into()));
        let scope = Id::<Element>::from_text("g1".into());
        let target =
            resolve_group_target(Id::<Element>::from_text("m".into()), &disp, Some(&scope));
        assert_eq!(target, Id::<Element>::from_text("m".into()));
    }

    /// `TOILE-306`: a click on the leaf of fifty thousand nested groups picks
    /// the top group, and a scoped click the level below the scope. Each step
    /// once scanned the whole display and the whole chain.
    #[test]
    fn a_click_into_a_deep_nesting_picks_the_top_group() {
        let depth = 50_000;
        let unit = Rect::new(0.0, 0.0, 100.0, 100.0);
        let name = |level: usize| format!("g{level}");
        let mut nodes = vec![member_elem("m", unit, &name(0))];
        for level in 0..depth {
            let parent = (level + 1 < depth).then(|| name(level + 1));
            nodes.push(group_node(&name(level), unit, parent.as_deref()));
        }
        let disp = viewport(nodes);
        let leaf = Id::<Element>::from_text("m".into());
        let target = resolve_group_target(leaf.clone(), &disp, None);
        assert_eq!(target, Id::<Element>::from_text(name(depth - 1)));
        let scope = Id::<Element>::from_text(name(10));
        let target = resolve_group_target(leaf, &disp, Some(&scope));
        assert_eq!(target, Id::<Element>::from_text(name(9)));
    }

    #[test]
    fn ungrouped_element_resolves_to_itself() {
        let disp = viewport(vec![rect_elem("a", Rect::new(0.0, 0.0, 100.0, 100.0))]);
        let target = resolve_group_target(Id::<Element>::from_text("a".into()), &disp, None);
        assert_eq!(target, Id::<Element>::from_text("a".into()));
        assert!(!is_group(&Id::<Element>::from_text("a".into()), &disp));
    }

    #[test]
    fn pointer_down_on_a_member_selects_the_group() {
        let disp = viewport(vec![
            member_elem("m", Rect::new(0.0, 0.0, 100.0, 100.0), "g"),
            group_node("g", Rect::new(0.0, 0.0, 100.0, 100.0), None),
        ]);
        let cam = cam();
        let mut tool = SelectTool::new();
        let w = Point::new(50.0, 50.0);
        let out = tool.pointer_down(w, to_screen(&cam, w), &[], &disp, &cam, None);
        match out {
            SelectOutcome::SelectionChanged(ids) => {
                assert_eq!(ids, vec![Id::<Element>::from_text("g".into())]);
            }
            other => panic!("expected the group to be selected, got {other:?}"),
        }
    }

    // Camera with identity-ish mapping: at zoom 1 and origin-centered, world (0,0)
    // maps to screen center. Use a size so screen == world + size/2.
    fn cam() -> ViewportTransform {
        ViewportTransform {
            center: Point::ZERO,
            zoom: ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(1.0),
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
        let out = tool.pointer_down(w0, to_screen(&cam, w0), &[], &disp, &cam, None);
        assert!(
            matches!(out, SelectOutcome::SelectionChanged(ref ids) if ids == &[Id::<Element>::from_text("a".into())])
        );
        tool.pointer_move(Point::new(70.0, 60.0));
        let up = tool.pointer_up(Point::new(70.0, 60.0), &disp);
        match up {
            SelectOutcome::Command(ToileCommand::MoveElements { ids, delta }) => {
                assert_eq!(ids, vec![Id::<Element>::from_text("a".into())]);
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
        tool.pointer_down(w, to_screen(&cam, w), &[], &disp, &cam, None);
        assert!(matches!(tool.pointer_up(w, &disp), SelectOutcome::None));
    }

    /// `TOILE-254`: a press on a frame's title selects that frame. The title
    /// sits outside the frame's box, so the board hit test cannot see it. It
    /// is how a user grabs a frame without clicking through what is inside.
    #[test]
    fn a_press_on_a_frames_title_selects_the_frame() {
        let bounds = Rect::new(0.0, 0.0, 300.0, 300.0);
        let disp = viewport(vec![frame_elem("f", bounds, "Agenda")]);
        let cam = cam();
        let band = super::super::render::frame_title_box(
            world_rect_to_screen(bounds, &cam),
            super::super::render::FrameTitle::Named("Agenda"),
        )
        .expect("a named frame has a title band");
        let screen = band.center();
        let world = cam.screen_to_world() * screen;
        // The press point is above the frame, so the board holds nothing there.
        let pick = super::super::hit::pick_floor(&cam);
        assert!(super::super::hit::hit_topmost(&disp.nodes, world, pick).is_none());

        let mut tool = SelectTool::new();
        let out = tool.pointer_down(world, screen, &[], &disp, &cam, None);
        assert!(
            matches!(out, SelectOutcome::SelectionChanged(ref ids) if ids == &[Id::<Element>::from_text("f".into())]),
            "the title did not select its frame: {out:?}"
        );
    }

    /// `TOILE-261`: a double-click on a frame's title opens the editor on that
    /// title, and one inside the frame's box does not.
    #[test]
    fn a_double_click_on_a_frames_title_edits_that_title() {
        let bounds = Rect::new(0.0, 0.0, 300.0, 300.0);
        let disp = viewport(vec![frame_elem("f", bounds, "Agenda")]);
        let cam = cam();
        let band = super::super::render::frame_bands(&disp.nodes, &cam)
            .next()
            .expect("a named frame has a band")
            .1;
        assert_eq!(
            double_click_edit(&disp.nodes, band.center(), &cam, |_| true),
            Some(DoubleClickEdit::FrameTitle(Id::<Element>::from_text(
                "f".into()
            )))
        );
        let inside = to_screen(&cam, Point::new(150.0, 150.0));
        assert_ne!(
            double_click_edit(&disp.nodes, inside, &cam, |_| false),
            Some(DoubleClickEdit::FrameTitle(Id::<Element>::from_text(
                "f".into()
            )))
        );
    }

    /// `TOILE-253`: a frame nobody named carries a placeholder band, so it can
    /// be pressed and named like any other. The board would otherwise offer no
    /// way to give a frame its first name.
    #[test]
    fn an_unnamed_frame_can_still_be_pressed_by_its_band() {
        let bounds = Rect::new(0.0, 0.0, 300.0, 300.0);
        let disp = viewport(vec![frame_elem("f", bounds, "")]);
        let cam = cam();
        let band = super::super::render::frame_bands(&disp.nodes, &cam)
            .next()
            .expect("an unnamed frame still has a band")
            .1;
        assert_eq!(
            super::super::hit::frame_title_at(&disp.nodes, band.center(), &cam),
            Some(Id::<Element>::from_text("f".into()))
        );
    }

    #[test]
    fn drag_empty_makes_a_marquee_then_selects() {
        let disp = viewport(vec![rect_elem("a", Rect::new(0.0, 0.0, 100.0, 100.0))]);
        let cam = cam();
        let mut tool = SelectTool::new();
        // Press on empty space.
        let w0 = Point::new(-200.0, -200.0);
        let out = tool.pointer_down(w0, to_screen(&cam, w0), &[], &disp, &cam, None);
        assert!(matches!(out, SelectOutcome::None));
        // Drag to enclose element "a".
        let m = tool.pointer_move(Point::new(200.0, 200.0));
        assert!(matches!(m, SelectOutcome::Marquee(_)));
        let up = tool.pointer_up(Point::new(200.0, 200.0), &disp);
        assert!(
            matches!(up, SelectOutcome::SelectionChanged(ref ids) if ids == &[Id::<Element>::from_text("a".into())])
        );
    }

    /// `TOILE-UI-236`: a marquee resolves a covered member to its top-most group,
    /// the target a click on it would return.
    #[test]
    fn marquee_resolves_a_member_to_its_group() {
        let disp = viewport(vec![
            member_elem("m", Rect::new(0.0, 0.0, 100.0, 100.0), "g"),
            group_node("g", Rect::new(0.0, 0.0, 100.0, 100.0), None),
        ]);
        let cam = cam();
        let mut tool = SelectTool::new();
        let w0 = Point::new(-200.0, -200.0);
        assert!(matches!(
            tool.pointer_down(w0, to_screen(&cam, w0), &[], &disp, &cam, None),
            SelectOutcome::None
        ));
        let _ = tool.pointer_move(Point::new(200.0, 200.0));
        let up = tool.pointer_up(Point::new(200.0, 200.0), &disp);
        assert!(
            matches!(up, SelectOutcome::SelectionChanged(ref ids) if ids == &[Id::<Element>::from_text("g".into())])
        );
    }

    #[test]
    fn drag_corner_handle_emits_resize() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let mut tool = SelectTool::new();
        let sel = vec![Id::<Element>::from_text("a".into())];
        // Press the SE handle (world corner 100,100 -> its screen position).
        let se_world = Point::new(100.0, 100.0);
        let se_screen = to_screen(&cam, se_world);
        let out = tool.pointer_down(se_world, se_screen, &sel, &disp, &cam, None);
        assert!(matches!(out, SelectOutcome::None));
        assert!(tool.is_dragging());
        // Drag it out to (150,140).
        let up = tool.pointer_up(Point::new(150.0, 140.0), &disp);
        match up {
            SelectOutcome::Command(ToileCommand::ResizeElement { id, new }) => {
                assert_eq!(id, Id::<Element>::from_text("a".into()));
                assert_eq!(new, Rect::new(0.0, 0.0, 150.0, 140.0));
            }
            other => panic!("expected ResizeElement, got {other:?}"),
        }
    }

    #[test]
    fn edge_handle_at_picks_sides_corners_and_misses() {
        // Screen bounds 100x100 at origin (0,0)-(100,100).
        let b = Rect::new(0.0, 0.0, 100.0, 100.0);
        // Mid-side points -> the cardinal side handle (whole edge grabbable, even
        // though sides draw no visual square).
        assert_eq!(edge_handle_at(b, Point::new(50.0, 0.0)), Some(Handle::N));
        assert_eq!(edge_handle_at(b, Point::new(100.0, 50.0)), Some(Handle::E));
        assert_eq!(edge_handle_at(b, Point::new(50.0, 100.0)), Some(Handle::S));
        assert_eq!(edge_handle_at(b, Point::new(0.0, 50.0)), Some(Handle::W));
        // A point anywhere along a side (not the midpoint) still grabs it.
        assert_eq!(edge_handle_at(b, Point::new(20.0, 0.0)), Some(Handle::N));
        // Corners -> diagonal handles, taking precedence over the two sides.
        assert_eq!(edge_handle_at(b, Point::new(0.0, 0.0)), Some(Handle::Nw));
        assert_eq!(edge_handle_at(b, Point::new(100.0, 0.0)), Some(Handle::Ne));
        assert_eq!(
            edge_handle_at(b, Point::new(100.0, 100.0)),
            Some(Handle::Se)
        );
        assert_eq!(edge_handle_at(b, Point::new(0.0, 100.0)), Some(Handle::Sw));
        // The rotate handle sits above the top-center edge.
        let rot = rotate_handle_point(b);
        assert_eq!(edge_handle_at(b, rot), Some(Handle::Rotate));
        // Interior and far-outside points miss.
        assert_eq!(edge_handle_at(b, Point::new(50.0, 50.0)), None);
        assert_eq!(edge_handle_at(b, Point::new(200.0, 200.0)), None);
    }

    /// A rectangle rotated `theta` radians about its AABB center (mirrors the
    /// Logic derive), for the rotated hit-test / resize tests.
    fn rotated_rect_elem(id: &str, bounds: Rect, theta: f64) -> DisplayNode {
        let DisplayNode::Element(mut e) = rect_elem(id, bounds) else {
            panic!("rect_elem builds an element")
        };
        e.transform = rotate_about(bounds.center(), ambre_core::Radians::new(theta));
        DisplayNode::Element(e)
    }

    #[test]
    fn hover_handle_follows_the_element_rotation() {
        use std::f64::consts::FRAC_PI_2;
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![rotated_rect_elem("a", bounds, FRAC_PI_2)]);
        let cam = cam();
        let m = ToolMachine::new();
        let sel = vec![Id::<Element>::from_text("a".into())];
        let center = to_screen(&cam, bounds.center());
        let se_screen = to_screen(&cam, Point::new(100.0, 100.0)); // axis-aligned SE
        // The SE corner is drawn a quarter turn round the box center; the cursor
        // hits it there, not at the axis-aligned position.
        let rotated_se = rotate_about(center, ambre_core::Radians::new(FRAC_PI_2)) * se_screen;
        assert_eq!(
            m.hover_handle(&sel, &disp, &cam, rotated_se),
            Some(Handle::Se)
        );
        assert_ne!(
            m.hover_handle(&sel, &disp, &cam, se_screen),
            Some(Handle::Se),
            "the un-rotated SE position must no longer read as SE"
        );
    }

    /// `TOILE-UI-407`: resizing a rotated element holds the corner opposite
    /// the dragged handle fixed in world space.
    #[test]
    fn resize_rotated_pins_the_opposite_corner_in_world_space() {
        use std::f64::consts::FRAC_PI_6;
        let base = Rect::new(0.0, 0.0, 100.0, 100.0);
        let theta = ambre_core::Radians::new(FRAC_PI_6);
        // Drag the SE handle; the NW corner (the anchor) must not move in world.
        let to = Point::new(150.0, 140.0);
        let new = resize_rect_rotated(base, Handle::Se, to, theta);
        let old_nw = rotate_about(base.center(), theta) * Point::new(base.x0, base.y0);
        let new_nw = rotate_about(new.center(), theta) * Point::new(new.x0, new.y0);
        assert!(
            (old_nw - new_nw).hypot() < 1e-9,
            "anchor drifted: {old_nw:?} -> {new_nw:?}"
        );
        // Degenerate case: no rotation matches the plain axis-aligned resize.
        assert_eq!(
            resize_rect_rotated(base, Handle::Se, to, ambre_core::Radians::ZERO),
            resize_rect(base, Handle::Se, to)
        );
    }

    /// `TOILE-UI-270`: a move, a resize and a rotate in flight each override the
    /// geometry they will commit.
    #[test]
    fn gesture_overrides_track_resize_move_and_rotate() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let sel = vec![Id::<Element>::from_text("a".into())];

        // Resize from the SE corner: the override carries the live resized rect.
        let mut tool = SelectTool::new();
        let se = Point::new(100.0, 100.0);
        tool.pointer_down(se, to_screen(&cam, se), &sel, &disp, &cam, None);
        tool.pointer_move(Point::new(150.0, 140.0));
        let ov = tool.gesture_overrides(&disp);
        assert_eq!(ov.len(), 1);
        assert_eq!(ov[0].0, Id::<Element>::from_text("a".into()));
        assert_eq!(ov[0].1, Rect::new(0.0, 0.0, 150.0, 140.0));
        assert_eq!(ov[0].2, ambre_core::Radians::ZERO);

        // Move: the override shifts every selected element by the drag delta.
        let mut tool = SelectTool::new();
        let inside = Point::new(50.0, 50.0);
        tool.pointer_down(inside, to_screen(&cam, inside), &sel, &disp, &cam, None);
        tool.pointer_move(Point::new(70.0, 60.0));
        let ov = tool.gesture_overrides(&disp);
        assert_eq!(ov[0].1, Rect::new(20.0, 10.0, 120.0, 110.0));

        // Rotate: bounds hold, rotation follows the pointer angle about the center.
        let mut tool = SelectTool::new();
        let rot_screen = to_screen(&cam, Point::new(50.0, 0.0)); // top-center world
        let rot_screen = Point::new(rot_screen.x, rot_screen.y - f64::from(ROTATE_OFFSET.get()));
        // Press the rotate handle (world point under it is the top-center edge).
        tool.pointer_down(Point::new(50.0, 0.0), rot_screen, &sel, &disp, &cam, None);
        // Drag to the right-center: a quarter turn from straight up.
        tool.pointer_move(Point::new(100.0, 50.0));
        let ov = tool.gesture_overrides(&disp);
        assert_eq!(ov[0].1, bounds);
        assert!((ov[0].2.get() - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    }

    #[test]
    fn hover_handle_only_for_a_single_select_at_rest() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![
            rect_elem("a", bounds),
            rect_elem("b", Rect::new(200.0, 0.0, 300.0, 100.0)),
        ]);
        let cam = cam();
        let mut m = ToolMachine::new();
        let sel_a = vec![Id::<Element>::from_text("a".into())];
        // On the SE corner of the single selection -> the diagonal handle.
        let se = to_screen(&cam, Point::new(100.0, 100.0));
        assert_eq!(m.hover_handle(&sel_a, &disp, &cam, se), Some(Handle::Se));
        // Interior -> no handle.
        let inside = to_screen(&cam, Point::new(50.0, 50.0));
        assert_eq!(m.hover_handle(&sel_a, &disp, &cam, inside), None);
        // Multi-selection -> handles suppressed (Phase 0 shows the box only).
        let sel_ab = vec![
            Id::<Element>::from_text("a".into()),
            Id::<Element>::from_text("b".into()),
        ];
        assert_eq!(m.hover_handle(&sel_ab, &disp, &cam, se), None);
        // A creation tool is armed -> never a resize hover.
        m.set_tool(Tool::Shape(ShapeKind::Rect));
        assert_eq!(m.hover_handle(&sel_a, &disp, &cam, se), None);
        // Mid-drag -> the hover query is silent (the drag cursor takes over).
        m.set_tool(Tool::Select);
        m.pointer_down(Point::new(100.0, 100.0), se, &sel_a, &disp, &cam, None);
        assert!(m.is_active());
        assert_eq!(m.hover_handle(&sel_a, &disp, &cam, se), None);
    }

    /// `TOILE-UI-392`: a press on the edge or a corner of a selected group's
    /// box starts no resize, and the hover offers no resize cursor there. The
    /// rotate handle still turns the group.
    #[test]
    fn a_press_on_a_selected_groups_edge_starts_no_resize() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![
            member_elem("m", bounds, "g"),
            group_node("g", bounds, None),
        ]);
        let cam = cam();
        let sel = vec![Id::<Element>::from_text("g".into())];
        for world in [Point::new(100.0, 100.0), Point::new(100.0, 50.0)] {
            let screen = to_screen(&cam, world);
            let mut m = ToolMachine::new();
            assert_eq!(m.hover_handle(&sel, &disp, &cam, screen), None);
            m.pointer_down(world, screen, &sel, &disp, &cam, None);
            assert!(
                !matches!(m.gesture_cursor(), Some(GestureCursor::Handle(_))),
                "{world:?} started a transform: {:?}",
                m.gesture_cursor()
            );
        }

        let knob = rotate_handle_point(world_rect_to_screen(bounds, &cam));
        let mut m = ToolMachine::new();
        assert_eq!(
            m.hover_handle(&sel, &disp, &cam, knob),
            Some(Handle::Rotate)
        );
        let knob_world = cam.screen_to_world() * knob;
        m.pointer_down(knob_world, knob, &sel, &disp, &cam, None);
        assert_eq!(
            m.gesture_cursor(),
            Some(GestureCursor::Handle(Handle::Rotate))
        );
    }

    #[test]
    fn gesture_cursor_reflects_the_in_flight_drag() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let sel = vec![Id::<Element>::from_text("a".into())];

        // Resize -> the grabbed handle.
        let mut m = ToolMachine::new();
        let se = to_screen(&cam, Point::new(100.0, 100.0));
        m.pointer_down(Point::new(100.0, 100.0), se, &sel, &disp, &cam, None);
        assert_eq!(m.gesture_cursor(), Some(GestureCursor::Handle(Handle::Se)));

        // Rotate -> the rotate handle.
        let mut m = ToolMachine::new();
        let rot = to_screen(&cam, Point::new(50.0, 0.0));
        let rot = Point::new(rot.x, rot.y - f64::from(ROTATE_OFFSET.get()));
        m.pointer_down(Point::new(50.0, 0.0), rot, &sel, &disp, &cam, None);
        assert_eq!(
            m.gesture_cursor(),
            Some(GestureCursor::Handle(Handle::Rotate))
        );

        // Move -> the closed-hand marker.
        let mut m = ToolMachine::new();
        let inside = to_screen(&cam, Point::new(50.0, 50.0));
        m.pointer_down(Point::new(50.0, 50.0), inside, &sel, &disp, &cam, None);
        assert_eq!(m.gesture_cursor(), Some(GestureCursor::Move));

        // At rest -> no drag cursor.
        assert_eq!(ToolMachine::new().gesture_cursor(), None);
    }

    #[test]
    fn resize_preview_then_release_emits_exactly_one_command() {
        // A move step feeds the live preview, but the gesture still commits a
        // single ResizeElement on release (undo granularity, toile-undo).
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let sel = vec![Id::<Element>::from_text("a".into())];
        let mut tool = SelectTool::new();
        let se = to_screen(&cam, Point::new(100.0, 100.0));
        tool.pointer_down(Point::new(100.0, 100.0), se, &sel, &disp, &cam, None);
        // Intermediate move updates the preview but emits nothing.
        assert!(matches!(
            tool.pointer_move(Point::new(130.0, 120.0)),
            SelectOutcome::None
        ));
        assert!(!tool.gesture_overrides(&disp).is_empty());
        // Release commits one resize to the final pointer.
        match tool.pointer_up(Point::new(150.0, 140.0), &disp) {
            SelectOutcome::Command(ToileCommand::ResizeElement { id, new }) => {
                assert_eq!(id, Id::<Element>::from_text("a".into()));
                assert_eq!(new, Rect::new(0.0, 0.0, 150.0, 140.0));
            }
            other => panic!("expected a single ResizeElement, got {other:?}"),
        }
        // The gesture is over: no lingering override.
        assert!(tool.gesture_overrides(&disp).is_empty());
    }

    // -- ToolMachine: creation tools -----------------------------------------

    fn empty_display() -> DisplayViewport {
        viewport(Vec::new())
    }

    #[test]
    fn shape_drag_emits_create_at_drag_rect() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Shape(ShapeKind::Ellipse));
        let disp = empty_display();
        let cam = cam();
        let start = Point::new(20.0, 30.0);
        m.pointer_down(start, to_screen(&cam, start), &[], &disp, &cam, None);
        m.pointer_move(Point::new(120.0, 130.0));
        // Preview reflects the drag rect as the shape it will become.
        assert!(matches!(
            m.preview(),
            Some(Preview::Shape {
                kind: ShapeKind::Ellipse,
                ..
            })
        ));
        match m.pointer_up(Point::new(120.0, 130.0), &disp) {
            ToolOutcome::Command(ToileCommand::CreateElement { kind, at, size, .. }) => {
                assert_eq!(kind, ElementKind::Shape(ShapeKind::Ellipse));
                assert_eq!(at, Point::new(20.0, 30.0));
                assert_eq!(size, ambre_draw::Size::new(100.0, 100.0));
            }
            other => panic!("expected CreateElement, got {other:?}"),
        }
    }

    /// The comment tool drops a thread where it clicks (`TOILE-UI-042`). It
    /// lands on the element under the click, so the pin follows it
    /// (`TOILE-024`). A free world point serves when the board is empty there.
    /// It writes nothing: the view opens the composer, and the thread is born
    /// with its first message.
    #[test]
    fn the_comment_tool_drops_a_thread_where_it_clicks() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Comment);
        let cam = cam();
        let disp = viewport(vec![rect_elem("note", Rect::new(0.0, 0.0, 40.0, 40.0))]);

        // On the note.
        let on_note = Point::new(20.0, 20.0);
        m.pointer_down(on_note, to_screen(&cam, on_note), &[], &disp, &cam, None);
        match m.pointer_up(on_note, &disp) {
            ToolOutcome::Comment(CommentAnchor::Element(id)) => {
                assert_eq!(id, Id::<Element>::from_text("note".to_owned()));
            }
            other => panic!("expected an element anchor, got {other:?}"),
        }

        // On empty board.
        let empty = Point::new(300.0, 300.0);
        m.set_tool(Tool::Comment);
        m.pointer_down(empty, to_screen(&cam, empty), &[], &disp, &cam, None);
        match m.pointer_up(empty, &disp) {
            ToolOutcome::Comment(CommentAnchor::Point(at)) => {
                assert_eq!(at, empty);
            }
            other => panic!("expected a point anchor, got {other:?}"),
        }
    }

    /// `TOILE-UI-225`: a tool picks under the reach of the camera at the
    /// press, not the screen constant taken as board units. At zoom 4, two
    /// board units off a hairline are eight screen pixels: a miss.
    #[test]
    fn a_tool_picks_a_hairline_under_the_reach_of_the_zoomed_camera() {
        let mut line = ambre_draw::BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((1000.0, 0.0));
        let DisplayNode::Element(mut e) = rect_elem("line", Rect::new(0.0, 0.0, 1000.0, 0.0))
        else {
            panic!("rect_elem builds an element")
        };
        e.geom = DisplayGeom::Path(line);
        e.style.stroke_w = Px::<StoredBoard>::new(0.5);
        let disp = viewport(vec![DisplayNode::Element(e)]);
        let mut cam = cam();
        cam.zoom = ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(4.0);
        let mut m = ToolMachine::new();
        for (off, on_line) in [(0.5, true), (2.0, false)] {
            let at = Point::new(500.0, off);
            m.set_tool(Tool::Comment);
            m.pointer_down(at, to_screen(&cam, at), &[], &disp, &cam, None);
            let anchored = matches!(
                m.pointer_up(at, &disp),
                ToolOutcome::Comment(CommentAnchor::Element(_))
            );
            assert_eq!(anchored, on_line, "{off} board units off at zoom 4");
        }
    }

    /// The highlighter is a tool a user picks, and it writes nothing
    /// (`TOILE-UI-074`). The point rides the next presence sample, the release
    /// clears it, and the board hears about none of it.
    #[test]
    fn the_highlighter_streams_a_point_and_writes_nothing() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Laser);
        let disp = empty_display();
        let cam = cam();

        let press = Point::new(10.0, 10.0);
        assert!(m.laser().is_none(), "nothing streams before the press");
        assert!(matches!(
            m.pointer_down(press, to_screen(&cam, press), &[], &disp, &cam, None),
            ToolOutcome::None
        ));
        assert_eq!(m.laser(), Some(press));

        let moved = Point::new(30.0, 20.0);
        assert!(matches!(m.pointer_move(moved), ToolOutcome::None));
        assert_eq!(m.laser(), Some(moved), "the point follows the pointer");

        // The release clears the point, which is what tells the peers to drop
        // the trail; and no command was ever produced.
        assert!(matches!(m.pointer_up(moved, &disp), ToolOutcome::None));
        assert!(m.laser().is_none());
        assert!(!m.is_active());
    }

    #[test]
    fn shape_click_uses_default_size() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Shape(ShapeKind::Rect));
        let disp = empty_display();
        let cam = cam();
        let at = Point::new(5.0, 5.0);
        m.pointer_down(at, to_screen(&cam, at), &[], &disp, &cam, None);
        match m.pointer_up(at, &disp) {
            ToolOutcome::Command(ToileCommand::CreateElement { size, .. }) => {
                assert_eq!(size, CLICK_SHAPE_SIZE);
            }
            other => panic!("expected CreateElement, got {other:?}"),
        }
    }

    #[test]
    fn sticky_click_creates_sticky_with_empty_text_and_picked_fill() {
        let mut m = ToolMachine::new();
        // The tool carries the picked card colour; a click must place the sticky
        // in that colour (folded into CreateElement.fill,).
        let picked = Rgba::from_bits(0xFF00_FFFF);
        m.set_tool(Tool::Sticky(picked));
        let disp = empty_display();
        let cam = cam();
        let at = Point::new(40.0, 60.0);
        m.pointer_down(at, to_screen(&cam, at), &[], &disp, &cam, None);
        match m.pointer_up(at, &disp) {
            ToolOutcome::Command(ToileCommand::CreateElement {
                kind,
                at: a,
                size,
                content,
                fill,
                ..
            }) => {
                assert_eq!(kind, ElementKind::Sticky);
                assert_eq!(a, at);
                assert_eq!(size, STICKY_SIZE);
                assert!(matches!(content, InitContent::Text(t) if t.is_empty()));
                assert_eq!(fill, Some(picked));
            }
            other => panic!("expected CreateElement, got {other:?}"),
        }
    }

    #[test]
    fn draw_stream_emits_commit_stroke() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Draw);
        let disp = empty_display();
        let cam = cam();
        let p0 = Point::new(0.0, 0.0);
        m.pointer_down(p0, to_screen(&cam, p0), &[], &disp, &cam, None);
        m.pointer_move(Point::new(10.0, 5.0));
        m.pointer_move(Point::new(20.0, 0.0));
        assert!(matches!(m.preview(), Some(Preview::Stroke(_))));
        match m.pointer_up(Point::new(30.0, 5.0), &disp) {
            ToolOutcome::Command(ToileCommand::CommitStroke { points, style }) => {
                assert!(points.len() >= 4);
                assert!(style.fill.is_none());
            }
            other => panic!("expected CommitStroke, got {other:?}"),
        }
    }

    /// `TOILE-106`, the sending half: the machine offers the samples of a
    /// stroke being drawn, so the per-frame presence sampler has a pen to
    /// broadcast. An idle machine offers none, so a peer's trail ends when the
    /// gesture does.
    #[test]
    fn a_stroke_being_drawn_is_offered_to_the_presence_sampler() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Draw);
        let disp = empty_display();
        let cam = cam();
        assert!(m.live_stroke().is_none(), "no stroke, nothing to send");
        let p0 = Point::new(0.0, 0.0);
        m.pointer_down(p0, to_screen(&cam, p0), &[], &disp, &cam, None);
        m.pointer_move(Point::new(10.0, 5.0));
        let mid = m.live_stroke().expect("a stroke in flight").to_vec();
        assert!(mid.len() >= 2, "the samples so far: {mid:?}");
        m.pointer_move(Point::new(20.0, 0.0));
        assert!(
            m.live_stroke().expect("still in flight").len() > mid.len(),
            "a new sample did not reach the sampler"
        );
        let _ = m.pointer_up(Point::new(30.0, 5.0), &disp);
        assert!(
            m.live_stroke().is_none(),
            "the trail outlived the gesture that drew it"
        );
    }

    #[test]
    fn connector_anchor_to_anchor_emits_create_connector() {
        let a = rect_elem("a", Rect::new(0.0, 0.0, 100.0, 100.0));
        let b = rect_elem("b", Rect::new(300.0, 0.0, 400.0, 100.0));
        let disp = viewport(vec![a, b]);
        let cam = cam();
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Connector);
        // Press near the right edge of a (nearest anchor = Right), release inside b.
        let down = Point::new(95.0, 50.0);
        m.pointer_down(down, to_screen(&cam, down), &[], &disp, &cam, None);
        assert!(matches!(m.preview(), Some(Preview::Connector { .. })));
        match m.pointer_up(Point::new(350.0, 50.0), &disp) {
            ToolOutcome::Command(ToileCommand::CreateConnector {
                from, to, arrows, ..
            }) => {
                assert_eq!(from, Id::<Element>::from_text("a".into()));
                assert_eq!(to, Id::<Element>::from_text("b".into()));
                assert_eq!(
                    (arrows.start, arrows.end),
                    (crate::message::Cap::Plain, crate::message::Cap::Arrow)
                );
            }
            other => panic!("expected CreateConnector, got {other:?}"),
        }
    }

    #[test]
    fn connector_release_on_empty_emits_nothing() {
        let a = rect_elem("a", Rect::new(0.0, 0.0, 100.0, 100.0));
        let disp = viewport(vec![a]);
        let cam = cam();
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Connector);
        let down = Point::new(50.0, 50.0);
        m.pointer_down(down, to_screen(&cam, down), &[], &disp, &cam, None);
        assert!(matches!(
            m.pointer_up(Point::new(999.0, 999.0), &disp),
            ToolOutcome::None
        ));
    }

    #[test]
    fn pen_clicks_then_double_click_emits_path() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Pen);
        let disp = empty_display();
        let cam = cam();
        for p in [
            Point::new(0.0, 0.0),
            Point::new(50.0, 0.0),
            Point::new(50.0, 50.0),
        ] {
            m.pointer_down(p, to_screen(&cam, p), &[], &disp, &cam, None);
            m.pointer_up(p, &disp);
        }
        assert!(m.is_active());
        match m.double_click(Point::new(50.0, 50.0)) {
            ToolOutcome::Command(ToileCommand::CreateElement { kind, content, .. }) => {
                assert_eq!(kind, ElementKind::Shape(ShapeKind::Path));
                assert!(matches!(content, InitContent::Path(_)));
            }
            other => panic!("expected CreateElement(Path), got {other:?}"),
        }
        assert!(!m.is_active(), "pen state cleared after finishing");
    }

    #[test]
    fn escape_reverts_an_armed_creation_tool_to_select() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Sticky(Rgba::from_bits(0xFEF0_8AFF)));
        let disp = empty_display();
        let cam = cam();
        // Arm a placement mid-gesture (pointer down, not yet released).
        let at = Point::new(10.0, 10.0);
        m.pointer_down(at, to_screen(&cam, at), &[], &disp, &cam, None);
        assert!(m.is_active());
        // Escape cancels the in-flight draft AND reverts the tool; it reports the
        // change so the view republishes the shared signal.
        assert!(m.escape(), "escape must report the tool changed");
        assert_eq!(m.tool(), Tool::Select);
        assert!(!m.is_active(), "the in-flight placement is cancelled");
        // A stray click no longer places anything: on the Select tool it is at
        // most a (here empty) selection change, never a CreateElement.
        m.pointer_down(at, to_screen(&cam, at), &[], &disp, &cam, None);
        assert!(!matches!(
            m.pointer_up(at, &disp),
            ToolOutcome::Command(ToileCommand::CreateElement { .. })
        ));
    }

    #[test]
    fn escape_on_the_select_tool_reports_no_tool_change() {
        let mut m = ToolMachine::new();
        // Already Select: Escape still cancels any gesture but the tool is
        // unchanged, so it returns false (nothing to republish).
        assert!(!m.escape());
        assert_eq!(m.tool(), Tool::Select);
    }

    #[test]
    fn switching_tools_cancels_in_flight_gesture() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Shape(ShapeKind::Rect));
        let disp = empty_display();
        let cam = cam();
        let p = Point::new(10.0, 10.0);
        m.pointer_down(p, to_screen(&cam, p), &[], &disp, &cam, None);
        assert!(m.is_active());
        m.set_tool(Tool::Select);
        assert!(!m.is_active());
        assert!(m.preview().is_none());
    }

    /// One pen node: a press at `at`, an optional drag to `handle`, a release.
    /// Answers what the press, the drag and the release returned.
    fn pen_node(m: &mut ToolMachine, at: Point, handle: Option<Point>) -> Vec<ToolOutcome> {
        let disp = empty_display();
        let cam = cam();
        let mut outs = vec![m.pointer_down(at, to_screen(&cam, at), &[], &disp, &cam, None)];
        if let Some(h) = handle {
            outs.push(m.pointer_move(h));
        }
        outs.push(m.pointer_up(handle.unwrap_or(at), &disp));
        outs
    }

    /// The path a pen finish creates, or a panic naming what came back.
    fn pen_path(out: ToolOutcome) -> BezPath {
        match out {
            ToolOutcome::Command(ToileCommand::CreateElement {
                content: InitContent::Path(path),
                ..
            }) => path,
            other => panic!("expected a path create, got {other:?}"),
        }
    }

    /// `TOILE-UI-058`: a node dragged out makes the span after it a curve.
    #[test]
    fn a_dragged_pen_node_makes_a_curved_segment() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Pen);
        pen_node(&mut m, Point::new(0.0, 0.0), Some(Point::new(20.0, -30.0)));
        pen_node(&mut m, Point::new(100.0, 0.0), None);
        let path = pen_path(m.double_click(Point::new(100.0, 0.0)));
        let segs: Vec<_> = path.segments().collect();
        assert_eq!(segs.len(), 1);
        assert!(
            matches!(segs[0], kurbo::PathSeg::Cubic(_)),
            "a dragged node curves its span: {segs:?}"
        );
    }

    /// `TOILE-UI-102`: nodes clicked without a drag make straight spans only.
    #[test]
    fn clicked_pen_nodes_make_straight_segments() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Pen);
        for p in [(0.0, 0.0), (50.0, 0.0), (50.0, 50.0)] {
            pen_node(&mut m, Point::new(p.0, p.1), None);
        }
        let path = pen_path(m.double_click(Point::new(50.0, 50.0)));
        let segs: Vec<_> = path.segments().collect();
        assert_eq!(segs.len(), 2);
        assert!(
            segs.iter().all(|s| matches!(s, kurbo::PathSeg::Line(_))),
            "a clicked node keeps its spans straight: {segs:?}"
        );
    }

    /// `TOILE-UI-103`: placing a node, clicked or dragged, writes nothing.
    /// Only the finish answers a command.
    #[test]
    fn placing_a_pen_node_writes_nothing() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Pen);
        let mut outs = pen_node(&mut m, Point::new(0.0, 0.0), None);
        outs.extend(pen_node(
            &mut m,
            Point::new(80.0, 0.0),
            Some(Point::new(90.0, 40.0)),
        ));
        outs.extend(pen_node(&mut m, Point::new(80.0, 80.0), None));
        assert!(
            outs.iter().all(|o| matches!(o, ToolOutcome::None)),
            "a node placement sent something: {outs:?}"
        );
    }

    /// `TOILE-UI-104`, `TOILE-UI-254`: a finish on one node emits nothing. The finishing
    /// double-click lands a second press on that node, and the duplicate is
    /// dropped before the count.
    #[test]
    fn a_pen_finish_on_one_node_emits_nothing() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Pen);
        let at = Point::new(10.0, 10.0);
        pen_node(&mut m, at, None);
        pen_node(&mut m, at + Vec2::new(1.0, 0.0), None);
        assert!(matches!(m.double_click(at), ToolOutcome::None));
        assert!(!m.is_active(), "the pen is idle again");
    }

    /// `TOILE-UI-105`: a path abandoned by `Escape` or by a tool change emits
    /// nothing, and the next finish has no node left to build on.
    #[test]
    fn an_abandoned_pen_path_emits_nothing() {
        let leave: [fn(&mut ToolMachine); 2] = [
            |m| {
                m.escape();
            },
            |m| m.set_tool(Tool::Select),
        ];
        for abandon in leave {
            let mut m = ToolMachine::new();
            m.set_tool(Tool::Pen);
            pen_node(&mut m, Point::new(0.0, 0.0), None);
            pen_node(&mut m, Point::new(60.0, 0.0), None);
            abandon(&mut m);
            assert!(!m.is_active(), "the path is gone");
            m.set_tool(Tool::Pen);
            assert!(matches!(
                m.double_click(Point::new(60.0, 0.0)),
                ToolOutcome::None
            ));
        }
    }

    /// `TOILE-105`: a stroke in flight writes nothing. The press and every
    /// move answer no command, and only the release commits.
    #[test]
    fn a_stroke_in_flight_writes_nothing() {
        let mut m = ToolMachine::new();
        m.set_tool(Tool::Draw);
        let disp = empty_display();
        let cam = cam();
        let p0 = Point::new(0.0, 0.0);
        let mut outs = vec![m.pointer_down(p0, to_screen(&cam, p0), &[], &disp, &cam, None)];
        for i in 1..20 {
            outs.push(m.pointer_move(Point::new(i as f64 * 3.0, (i % 4) as f64)));
        }
        assert!(
            outs.iter().all(|o| matches!(o, ToolOutcome::None)),
            "a sample was sent before the release: {outs:?}"
        );
        assert!(matches!(
            m.pointer_up(Point::new(60.0, 0.0), &disp),
            ToolOutcome::Command(ToileCommand::CommitStroke { .. })
        ));
    }

    /// `TOILE-108`: a stroke abandoned by `Escape` or by a tool change commits
    /// nothing, even when the pointer is released afterwards.
    #[test]
    fn an_abandoned_stroke_commits_nothing() {
        let leave: [fn(&mut ToolMachine); 2] = [
            |m| {
                m.escape();
            },
            |m| m.set_tool(Tool::Select),
        ];
        for abandon in leave {
            let mut m = ToolMachine::new();
            m.set_tool(Tool::Draw);
            let disp = empty_display();
            let cam = cam();
            let p0 = Point::new(0.0, 0.0);
            m.pointer_down(p0, to_screen(&cam, p0), &[], &disp, &cam, None);
            m.pointer_move(Point::new(10.0, 5.0));
            m.pointer_move(Point::new(20.0, 0.0));
            abandon(&mut m);
            assert!(m.live_stroke().is_none(), "the samples are gone");
            assert!(!matches!(
                m.pointer_up(Point::new(30.0, 5.0), &disp),
                ToolOutcome::Command(ToileCommand::CommitStroke { .. })
            ));
        }
    }

    #[test]
    fn nearest_anchor_picks_closest_edge() {
        let b = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert_eq!(nearest_anchor(b, Point::new(50.0, 2.0)), Anchor::Top);
        assert_eq!(nearest_anchor(b, Point::new(98.0, 50.0)), Anchor::Right);
        assert_eq!(nearest_anchor(b, Point::new(2.0, 50.0)), Anchor::Left);
        assert_eq!(nearest_anchor(b, Point::new(50.0, 98.0)), Anchor::Bottom);
    }

    /// `TOILE-UI-252`: a press on a resize handle that never moved commits
    /// nothing. The geometry is the one the document already holds, and a
    /// command for it costs a delta and an undo item for no change.
    #[test]
    fn press_on_a_resize_handle_without_movement_emits_no_command() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let mut tool = SelectTool::new();
        let sel = vec![Id::<Element>::from_text("a".into())];
        let se_world = Point::new(100.0, 100.0);
        let out = tool.pointer_down(se_world, to_screen(&cam, se_world), &sel, &disp, &cam, None);
        assert!(matches!(out, SelectOutcome::None));
        assert!(tool.is_dragging());
        assert!(matches!(
            tool.pointer_up(se_world, &disp),
            SelectOutcome::None
        ));
    }

    /// `TOILE-UI-252`, the rotate handle: the same press, the same silence.
    #[test]
    fn press_on_the_rotate_handle_without_movement_emits_no_command() {
        let bounds = Rect::new(-50.0, -50.0, 50.0, 50.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let mut tool = SelectTool::new();
        let sel = vec![Id::<Element>::from_text("a".into())];
        let screen_b = world_rect_to_screen(bounds, &cam);
        let rot_screen = rotate_handle_point(screen_b);
        let rot_world = cam.screen_to_world() * rot_screen;
        let out = tool.pointer_down(rot_world, rot_screen, &sel, &disp, &cam, None);
        assert!(matches!(out, SelectOutcome::None));
        assert!(tool.is_dragging());
        assert!(matches!(
            tool.pointer_up(rot_world, &disp),
            SelectOutcome::None
        ));
    }

    #[test]
    fn drag_rotate_handle_emits_rotate() {
        let bounds = Rect::new(-50.0, -50.0, 50.0, 50.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let mut tool = SelectTool::new();
        let sel = vec![Id::<Element>::from_text("a".into())];
        // Rotate handle sits above the top-center in screen space.
        let screen_b = world_rect_to_screen(bounds, &cam);
        let rot_screen = rotate_handle_point(screen_b);
        // The world point under that screen position (start angle reference).
        let rot_world = cam.screen_to_world() * rot_screen;
        let out = tool.pointer_down(rot_world, rot_screen, &sel, &disp, &cam, None);
        assert!(matches!(out, SelectOutcome::None));
        // Move the pointer to the element's right (world +x from center) -> the
        // vector angle changes from "straight up" to "straight right": +90 deg.
        let up = tool.pointer_up(Point::new(60.0, 0.0), &disp);
        match up {
            SelectOutcome::Command(ToileCommand::RotateElement { id, radians }) => {
                assert_eq!(id, Id::<Element>::from_text("a".into()));
                assert!((radians.get() - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
            }
            other => panic!("expected RotateElement, got {other:?}"),
        }
    }

    /// `TOILE-UI-231`: a locked element never contributes to the selection box,
    /// even when an id list puts it there (select-all).
    #[test]
    fn selection_bounds_skips_a_locked_element() {
        let mut disp = viewport(vec![
            rect_elem("a", Rect::new(0.0, 0.0, 10.0, 10.0)),
            rect_elem("b", Rect::new(40.0, 0.0, 50.0, 10.0)),
        ]);
        if let DisplayNode::Element(e) = &mut disp.nodes[0] {
            e.locked = crate::message::LockState::Locked;
        }

        // A locked-only selection wears no box.
        assert_eq!(
            selection_bounds(&[Id::<Element>::from_text("a".into())], &disp),
            None
        );
        // A mixed selection's box covers the unlocked element alone.
        let both = selection_bounds(
            &[
                Id::<Element>::from_text("a".into()),
                Id::<Element>::from_text("b".into()),
            ],
            &disp,
        )
        .expect("box");
        assert_eq!(both, Rect::new(40.0, 0.0, 50.0, 10.0));
    }
}
