// ambre-toile/src/ui/tool.rs
// ============================================================================
// Tool state machine (toile-ui §5). `SelectTool` implements the Select tool
// fully (move / resize / rotate / marquee); `ToolMachine` wraps it and the
// creation tools (Shape / Connector / Draw / Pen / Sticky / Text), dispatching
// each pointer event to the active tool and emitting exactly one `ToileCommand`
// on gesture end (or a local selection change for a marquee).
//
// The machine is pure: it maps (pointer event, current selection, DisplayViewport,
// ViewportTransform) to an outcome, holding only in-flight draft state. Selection
// itself is local UI state owned by the view (§7). Host-tested with synthetic
// pointer sequences.
// ============================================================================

use ambre_bus::app::toile::{
    Anchor, DisplayGeom, DisplayNode, DisplayViewport, ElementId, ElementKind, EndCaps,
    InitContent, PathType, ShapeKind, StrokePoint, ToileCommand,
};
use ambre_draw::{BezPath, Point, Rect, Size};
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
    /// Sticky note. Carries the card colour (ARGB) picked from the palette so a
    /// click places the note in that colour atomically (folded into
    /// `CreateElement.fill`, toile-ui §5quinquies.A). `aria-pressed` compares by
    /// discriminant, so any `Sticky(_)` reads active regardless of colour.
    Sticky(u32),
    Shape(ShapeKind),
    Connector,
    Draw,
    Text,
    Hand,
}

/// Screen-space pick radius (physical px) around a corner / rotate handle.
const HANDLE_PICK_PX: f64 = 9.0;
/// Screen-space grab band (physical px) around a selection edge: the whole side
/// resizes (with a directional cursor), even though only the corners draw a
/// visual square (toile-ui §7).
const EDGE_GRAB_PX: f64 = 6.0;
/// Rotate handle offset above the selection's top edge (physical px).
pub const ROTATE_OFFSET_PX: f64 = 26.0;
/// Drawn size of a square resize handle (physical px), used by the overlay.
pub const HANDLE_SIZE_PX: f64 = 8.0;

/// The cursor an in-flight Select gesture calls for (§5quinquies.A): a directional
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
        /// Live pointer (world), updated each move so the view can preview the
        /// resize without a Logic round-trip.
        last: Point,
    },
    Rotate {
        id: ElementId,
        center: Point,
        start_angle: f64,
        base_rot: f64,
        /// Live pointer angle (world radians about `center`), updated each move
        /// for the live preview.
        last_angle: f64,
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
        scope: Option<&ElementId>,
    ) -> SelectOutcome {
        // A single-element selection exposes transform handles; test them first so
        // a handle press starts a resize/rotate rather than a move.
        if selection.len() == 1
            && let Some(sb) = selection_bounds(selection, display)
        {
            let screen_b = world_rect_to_screen(sb, cam);
            if let Some(h) = edge_handle_at(screen_b, screen) {
                let id = selection[0].clone();
                match h {
                    Handle::Rotate => {
                        let center = sb.center();
                        let start_angle = (world - center).atan2();
                        self.drag = Some(Drag::Rotate {
                            id: id.clone(),
                            center,
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
                            last: world,
                        });
                    }
                }
                return SelectOutcome::None;
            }
        }

        // No handle: hit-test the board, then resolve the raw hit up its group
        // chain to the selection target (the top-most enclosing group, unless a
        // scope has been entered), so a grouped element selects as its group.
        match hit_topmost(&display.nodes, world) {
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
                *last_angle = (world - *center).atan2();
                SelectOutcome::None
            }
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
            Drag::Resize {
                id, base, handle, ..
            } => {
                let new = resize_rect(base, handle, world);
                SelectOutcome::Command(ToileCommand::ResizeElement { id, new })
            }
            Drag::Rotate {
                id,
                center,
                start_angle,
                base_rot,
                ..
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

    /// The live marquee rectangle (world space) if a rubber-band drag is in
    /// flight, so the machine can surface it as an overlay preview without an
    /// outcome round-trip.
    pub fn marquee(&self) -> Option<Rect> {
        match &self.drag {
            Some(Drag::Marquee { start, current }) => Some(Rect::from_points(*start, *current)),
            _ => None,
        }
    }

    /// The handle a resting hover targets, for the directional cursor (§5quinquies.A).
    /// Same hit-test as `pointer_down`, but pure (no drag started): returns `None`
    /// unless the selection is exactly one element and the pointer is on its edge /
    /// corner / rotate handle. Never called while a drag is in flight.
    pub fn hover_handle(
        &self,
        selection: &[ElementId],
        display: &DisplayViewport,
        cam: &ViewportTransform,
        screen: Point,
    ) -> Option<Handle> {
        if selection.len() != 1 {
            return None;
        }
        let sb = selection_bounds(selection, display)?;
        edge_handle_at(world_rect_to_screen(sb, cam), screen)
    }

    /// The cursor the in-flight drag calls for (§5quinquies.A), so it stays stable
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

    /// Per-element live transform for the in-flight gesture (§5): `(id, bounds,
    /// rotation)` in world space, so the view can paint the real object being
    /// resized / rotated / moved before the command commits on release. Empty when
    /// no gesture (or a marquee) is in flight.
    pub fn gesture_overrides(&self, display: &DisplayViewport) -> Vec<(ElementId, Rect, f64)> {
        match &self.drag {
            Some(Drag::Resize {
                id,
                base,
                handle,
                last,
            }) => vec![(
                id.clone(),
                resize_rect(*base, *handle, *last),
                element_rotation(display, id),
            )],
            Some(Drag::Rotate {
                id,
                start_angle,
                base_rot,
                last_angle,
                ..
            }) => {
                let bounds = element_bounds(display, id).unwrap_or_default();
                vec![(id.clone(), bounds, base_rot + (last_angle - start_angle))]
            }
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
// Tool machine (§5): the canvas-side consumer of the active tool. Owns the
// Select gesture machine plus the in-flight state of the creation tools (Shape /
// Connector / Draw / Pen / Sticky / Text) and maps a pointer sequence to exactly
// one `ToileCommand` on gesture end. Pure: it holds only in-flight draft state,
// never document data. Host-tested with synthetic pointer sequences.
// =============================================================================

/// Default sizes for click-placed (not drag-sized) elements, world units.
// A sticky note is square (Miro-style): same height as before, narrower width.
const STICKY_SIZE: Size = Size::new(120.0, 120.0);
const TEXT_SIZE: Size = Size::new(200.0, 40.0);
const CLICK_SHAPE_SIZE: Size = Size::new(120.0, 80.0);
/// Below this drag extent (world units) a shape gesture counts as a click.
const MIN_DRAG: f64 = 3.0;
/// Trailing pen nodes closer than this (world units) are the closing
/// double-click's duplicate press; dropped before the path commits.
const PEN_DEDUP: f64 = 2.0;

/// What a pointer / keyboard event produced across all tools. Overlay previews
/// (marquee, rubber-band, pen path, live stroke) are queried separately via
/// [`ToolMachine::preview`], so this enum carries only observable results.
#[derive(Debug)]
pub enum ToolOutcome {
    /// Nothing to act on (idle move, or a draft step the view just repaints).
    None,
    /// Local selection changed to this set (never a command, never synced, §7).
    SelectionChanged(Vec<ElementId>),
    /// A committed document mutation to forward to Logic.
    Command(ToileCommand),
}

/// A local, unsynced overlay to paint while a gesture is in flight (§5).
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
        from: ElementId,
        from_anch: Anchor,
        origin: Point,
        current: Point,
    },
    /// Freehand stroke samples accumulated between press and release.
    Draw { points: Vec<StrokePoint> },
    /// A click-placed sticky / text element, committed on release. `fill` carries
    /// the sticky's picked card colour (`None` for a text element, which keeps its
    /// per-kind default, §5quinquies.A).
    Place {
        kind: ElementKind,
        at: Point,
        fill: Option<u32>,
    },
}

/// The active-tool state machine. Replaces the bare `SelectTool` the view drove
/// in Phase 0: it dispatches each pointer event to the active tool and exposes a
/// single `ToolOutcome` plus a queryable `preview`.
pub struct ToolMachine {
    tool: Tool,
    select: SelectTool,
    /// Single-commit creation gesture in flight (Shape / Connector / Draw / Place).
    draft: Option<Draft>,
    /// Vector pen nodes committed so far (built across clicks; §5bis).
    pen: Vec<PenNode>,
    /// True while a pen node's control handle is being dragged.
    pen_dragging: bool,
    /// Last world cursor, for the pen's live segment preview.
    cursor: Point,
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
            cursor: Point::ZERO,
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
        self.select.is_dragging() || self.draft.is_some() || !self.pen.is_empty()
    }

    /// Abort any in-flight gesture (Escape / focus loss / tool switch).
    pub fn cancel(&mut self) {
        self.select.cancel();
        self.draft = None;
        self.pen.clear();
        self.pen_dragging = false;
    }

    /// Handle Escape at the tool layer (§5): cancel any in-flight gesture and, if
    /// a creation tool is armed, revert to Select so it stops silently placing
    /// elements on the next click. Returns `true` when the active tool changed, so
    /// the caller can republish the shared active-tool signal (§5quinquies.A).
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
        selection: &[ElementId],
        display: &DisplayViewport,
        cam: &ViewportTransform,
        scope: Option<&ElementId>,
    ) -> ToolOutcome {
        self.cursor = world;
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
            Tool::Connector => {
                if let Some(id) = hit_topmost(&display.nodes, world) {
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
            Some(Draft::Place { .. }) => ToolOutcome::None,
            None => {
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
        if let Some(draft) = self.draft.take() {
            return match draft {
                Draft::Shape {
                    kind,
                    start,
                    current,
                } => {
                    let mut rect = Rect::from_points(start, current).abs();
                    if rect.width() < MIN_DRAG || rect.height() < MIN_DRAG {
                        rect = Rect::from_origin_size(start, CLICK_SHAPE_SIZE);
                    }
                    ToolOutcome::Command(ToileCommand::CreateElement {
                        kind: ElementKind::Shape(kind),
                        at: Point::new(rect.x0, rect.y0),
                        size: rect.size(),
                        content: InitContent::None,
                        fill: None,
                    })
                }
                Draft::Connector {
                    from, from_anch, ..
                } => match hit_topmost(&display.nodes, world) {
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
                                start: false,
                                end: true,
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

    /// Double-click. Finishes the vector pen path (§5bis); a no-op otherwise.
    pub fn double_click(&mut self, _world: Point) -> ToolOutcome {
        if self.tool == Tool::Pen {
            self.finish_pen()
        } else {
            ToolOutcome::None
        }
    }

    /// Build the pen path and emit it as a single `CreateElement` (§5bis).
    fn finish_pen(&mut self) -> ToolOutcome {
        self.pen_dragging = false;
        let mut nodes = std::mem::take(&mut self.pen);
        // The finishing double-click lands a duplicate press on the last node;
        // drop trailing near-duplicates so the path is not degenerate.
        while nodes.len() >= 2
            && nodes[nodes.len() - 1]
                .pt
                .distance(nodes[nodes.len() - 2].pt)
                < PEN_DEDUP
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
        })
    }

    /// The current overlay preview, derived from in-flight state (§5).
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
                        .map(|p| Point::new(p.x as f64, p.y as f64))
                        .collect(),
                )),
                Draft::Place { .. } => None,
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

    /// The handle a resting hover targets, for the directional resize cursor
    /// (§5quinquies.A). `None` unless the Select tool is armed, no gesture is in
    /// flight, and the pointer is on a single selection's edge / corner.
    pub fn hover_handle(
        &self,
        selection: &[ElementId],
        display: &DisplayViewport,
        cam: &ViewportTransform,
        screen: Point,
    ) -> Option<Handle> {
        if self.tool != Tool::Select || self.is_active() {
            return None;
        }
        self.select.hover_handle(selection, display, cam, screen)
    }

    /// Per-element live transform for the in-flight Select gesture (§5), forwarded
    /// so the view can preview the real object during resize / rotate / move.
    pub fn gesture_overrides(&self, display: &DisplayViewport) -> Vec<(ElementId, Rect, f64)> {
        self.select.gesture_overrides(display)
    }

    /// The cursor the in-flight Select drag calls for (§5quinquies.A); `None`
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
    StrokePoint {
        x: world.x as f32,
        y: world.y as f32,
        pressure: 1.0,
    }
}

/// Default freehand style: a solid stroke, no fill (§4).
fn draw_style() -> ambre_bus::app::toile::ElementStyle {
    ambre_bus::app::toile::ElementStyle {
        fill: None,
        stroke: Some(0xFF33_3333),
        stroke_w: 2.0,
        ..Default::default()
    }
}

/// World AABB of a single element (`None` if not in the current viewport).
fn element_bounds(display: &DisplayViewport, id: &ElementId) -> Option<Rect> {
    selection_bounds(std::slice::from_ref(id), display)
}

/// The immediate enclosing group of `id` in the current display, if any (the
/// `DisplayElement.group` edge, §2.F bis / §7). `None` for a top-level element or
/// an id absent from the culled frame.
fn enclosing_group_of(id: &ElementId, display: &DisplayViewport) -> Option<ElementId> {
    display.nodes.iter().find_map(|n| match n {
        DisplayNode::Element(e) if &e.id == id => e.group.clone(),
        _ => None,
    })
}

/// Whether `id` names a group node in the current display (its geometry is the
/// derived member union; it paints nothing, §2.F bis).
pub(crate) fn is_group(id: &ElementId, display: &DisplayViewport) -> bool {
    display.nodes.iter().any(|n| {
        matches!(n, DisplayNode::Element(e) if &e.id == id
            && matches!(e.geom, DisplayGeom::Group))
    })
}

/// Resolve a raw hit element to the selection target under the current group
/// `scope` (§2.F bis / §7). Walk the enclosing-group chain from the hit upward:
/// - no scope -> the top-most enclosing group (the whole group selects/transforms
///   as one), or the element itself if it is ungrouped.
/// - inside a scope `s` -> the chain member one level below `s` (so a scoped click
///   selects a member or an immediate sub-group); a click outside the scope falls
///   back to the top-most group.
pub(crate) fn resolve_group_target(
    hit: ElementId,
    display: &DisplayViewport,
    scope: Option<&ElementId>,
) -> ElementId {
    let mut chain = vec![hit.clone()];
    let mut cur = hit;
    while let Some(g) = enclosing_group_of(&cur, display) {
        chain.push(g.clone());
        cur = g;
    }
    match scope {
        Some(s) => match chain.iter().position(|c| c == s) {
            Some(0) => chain[0].clone(),
            Some(pos) => chain[pos - 1].clone(),
            // Clicked outside the entered scope: select the whole top-most group.
            None => chain.last().cloned().unwrap_or_else(|| chain[0].clone()),
        },
        None => chain.last().cloned().unwrap_or_else(|| chain[0].clone()),
    }
}

/// The world point of an anchor on an element's AABB (§5).
fn anchor_point(b: Rect, a: Anchor) -> Point {
    let c = b.center();
    match a {
        Anchor::Top => Point::new(c.x, b.y0),
        Anchor::Bottom => Point::new(c.x, b.y1),
        Anchor::Left => Point::new(b.x0, c.y),
        Anchor::Right => Point::new(b.x1, c.y),
        Anchor::Center => c,
        Anchor::Free { u, v } => {
            Point::new(b.x0 + u as f64 * b.width(), b.y0 + v as f64 * b.height())
        }
    }
}

/// The cardinal anchor of `b` closest to world point `p`.
fn nearest_anchor(b: Rect, p: Point) -> Anchor {
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
/// control handle, a straight line otherwise (§5bis).
fn pen_bezpath(nodes: &[PenNode]) -> BezPath {
    let mut p = BezPath::new();
    let Some(first) = nodes.first() else {
        return p;
    };
    p.move_to(first.pt);
    for w in nodes.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if a.out.is_none() && b.out.is_none() {
            p.line_to(b.pt);
        } else {
            p.curve_to(a.out.unwrap_or(a.pt), b.in_handle(), b.pt);
        }
    }
    p
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

/// The 4 corner resize-handle centers (screen space) for a selection's screen
/// bounds -- the only handles drawn as a **visual square**. The sides are still
/// grabbable / resizable (see `edge_handle_at`), they just show no square, which
/// keeps the box light (toile-ui §7).
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
    Point::new(b.center().x, b.y0 - ROTATE_OFFSET_PX)
}

/// Which handle (if any) a screen point targets on a selection's screen bounds.
/// The whole edge is grabbable (Miro): the rotate knob and the 4 corners test by
/// proximity to their point (`HANDLE_PICK_PX`), while the 4 sides test by a grab
/// band (`EDGE_GRAB_PX`) along the full length of the side -- so a side resizes
/// (with a directional cursor) even though only the corners draw a visual square
/// (toile-ui §7). Corners win over sides (tested first), so a corner press always
/// resizes diagonally. Used by both `pointer_down` (start a resize/rotate) and
/// `hover_handle` (pick the cursor), so press and hover always agree.
fn edge_handle_at(screen_b: Rect, screen: Point) -> Option<Handle> {
    if rotate_handle_point(screen_b).distance(screen) <= HANDLE_PICK_PX {
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
        if p.distance(screen) <= HANDLE_PICK_PX {
            return Some(h);
        }
    }
    // Sides: within the grab band of the edge line AND projecting inside the
    // side's own extent (so the band does not stick out past the corners).
    let in_x = screen.x >= screen_b.x0 - EDGE_GRAB_PX && screen.x <= screen_b.x1 + EDGE_GRAB_PX;
    let in_y = screen.y >= screen_b.y0 - EDGE_GRAB_PX && screen.y <= screen_b.y1 + EDGE_GRAB_PX;
    if in_x && (screen.y - screen_b.y0).abs() <= EDGE_GRAB_PX {
        return Some(Handle::N);
    }
    if in_x && (screen.y - screen_b.y1).abs() <= EDGE_GRAB_PX {
        return Some(Handle::S);
    }
    if in_y && (screen.x - screen_b.x1).abs() <= EDGE_GRAB_PX {
        return Some(Handle::E);
    }
    if in_y && (screen.x - screen_b.x0).abs() <= EDGE_GRAB_PX {
        return Some(Handle::W);
    }
    None
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
            group: None,
        })
    }

    /// A rectangle that is a member of group `group`.
    fn member_elem(id: &str, bounds: Rect, group: &str) -> DisplayNode {
        let DisplayNode::Element(mut e) = rect_elem(id, bounds) else {
            unreachable!()
        };
        e.group = Some(ElementId(group.to_owned()));
        DisplayNode::Element(e)
    }

    /// A group node (paints nothing): `parent` is its own enclosing group, if any.
    fn group_node(id: &str, bounds: Rect, parent: Option<&str>) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: ElementId(id.to_owned()),
            kind: ElementKind::Group,
            transform: Affine::IDENTITY,
            bounds,
            locked: false,
            style: ElementStyle::default(),
            geom: DisplayGeom::Group,
            group: parent.map(|p| ElementId(p.to_owned())),
        })
    }

    fn viewport(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
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
        let target = resolve_group_target(ElementId("m".into()), &disp, None);
        assert_eq!(target, ElementId("outer".into()));
    }

    #[test]
    fn scoped_click_selects_the_member_inside_the_scope() {
        let disp = viewport(vec![
            member_elem("m", Rect::new(0.0, 0.0, 100.0, 100.0), "g"),
            group_node("g", Rect::new(0.0, 0.0, 100.0, 100.0), Some("outer")),
            group_node("outer", Rect::new(0.0, 0.0, 100.0, 100.0), None),
        ]);
        // Scope = outer: the click peels one level and selects the sub-group `g`.
        let scope = ElementId("outer".into());
        let target = resolve_group_target(ElementId("m".into()), &disp, Some(&scope));
        assert_eq!(target, ElementId("g".into()));
        // Scope = g: the click selects the member itself.
        let scope = ElementId("g".into());
        let target = resolve_group_target(ElementId("m".into()), &disp, Some(&scope));
        assert_eq!(target, ElementId("m".into()));
    }

    #[test]
    fn ungrouped_element_resolves_to_itself() {
        let disp = viewport(vec![rect_elem("a", Rect::new(0.0, 0.0, 100.0, 100.0))]);
        let target = resolve_group_target(ElementId("a".into()), &disp, None);
        assert_eq!(target, ElementId("a".into()));
        assert!(!is_group(&ElementId("a".into()), &disp));
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
                assert_eq!(ids, vec![ElementId("g".into())]);
            }
            other => panic!("expected the group to be selected, got {other:?}"),
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
        let out = tool.pointer_down(w0, to_screen(&cam, w0), &[], &disp, &cam, None);
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
        tool.pointer_down(w, to_screen(&cam, w), &[], &disp, &cam, None);
        assert!(matches!(tool.pointer_up(w, &disp), SelectOutcome::None));
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
        let out = tool.pointer_down(se_world, se_screen, &sel, &disp, &cam, None);
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

    #[test]
    fn gesture_overrides_track_resize_move_and_rotate() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let sel = vec![ElementId("a".into())];

        // Resize from the SE corner: the override carries the live resized rect.
        let mut tool = SelectTool::new();
        let se = Point::new(100.0, 100.0);
        tool.pointer_down(se, to_screen(&cam, se), &sel, &disp, &cam, None);
        tool.pointer_move(Point::new(150.0, 140.0));
        let ov = tool.gesture_overrides(&disp);
        assert_eq!(ov.len(), 1);
        assert_eq!(ov[0].0, ElementId("a".into()));
        assert_eq!(ov[0].1, Rect::new(0.0, 0.0, 150.0, 140.0));
        assert_eq!(ov[0].2, 0.0);

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
        let rot_screen = Point::new(rot_screen.x, rot_screen.y - ROTATE_OFFSET_PX);
        // Press the rotate handle (world point under it is the top-center edge).
        tool.pointer_down(Point::new(50.0, 0.0), rot_screen, &sel, &disp, &cam, None);
        // Drag to the right-center: a quarter turn from straight up.
        tool.pointer_move(Point::new(100.0, 50.0));
        let ov = tool.gesture_overrides(&disp);
        assert_eq!(ov[0].1, bounds);
        assert!((ov[0].2 - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
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
        let sel_a = vec![ElementId("a".into())];
        // On the SE corner of the single selection -> the diagonal handle.
        let se = to_screen(&cam, Point::new(100.0, 100.0));
        assert_eq!(m.hover_handle(&sel_a, &disp, &cam, se), Some(Handle::Se));
        // Interior -> no handle.
        let inside = to_screen(&cam, Point::new(50.0, 50.0));
        assert_eq!(m.hover_handle(&sel_a, &disp, &cam, inside), None);
        // Multi-selection -> handles suppressed (Phase 0 shows the box only).
        let sel_ab = vec![ElementId("a".into()), ElementId("b".into())];
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

    #[test]
    fn gesture_cursor_reflects_the_in_flight_drag() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let sel = vec![ElementId("a".into())];

        // Resize -> the grabbed handle.
        let mut m = ToolMachine::new();
        let se = to_screen(&cam, Point::new(100.0, 100.0));
        m.pointer_down(Point::new(100.0, 100.0), se, &sel, &disp, &cam, None);
        assert_eq!(m.gesture_cursor(), Some(GestureCursor::Handle(Handle::Se)));

        // Rotate -> the rotate handle.
        let mut m = ToolMachine::new();
        let rot = to_screen(&cam, Point::new(50.0, 0.0));
        let rot = Point::new(rot.x, rot.y - ROTATE_OFFSET_PX);
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
        // single ResizeElement on release (undo granularity, toile-logic §949).
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let disp = viewport(vec![rect_elem("a", bounds)]);
        let cam = cam();
        let sel = vec![ElementId("a".into())];
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
                assert_eq!(id, ElementId("a".into()));
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
        // in that colour (folded into CreateElement.fill, §5quinquies.A).
        let picked = 0xFFFF_00FF;
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
                assert_eq!(from, ElementId("a".into()));
                assert_eq!(to, ElementId("b".into()));
                assert!(arrows.end && !arrows.start);
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
        m.set_tool(Tool::Sticky(0xFFFE_F08A));
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

    #[test]
    fn nearest_anchor_picks_closest_edge() {
        let b = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert_eq!(nearest_anchor(b, Point::new(50.0, 2.0)), Anchor::Top);
        assert_eq!(nearest_anchor(b, Point::new(98.0, 50.0)), Anchor::Right);
        assert_eq!(nearest_anchor(b, Point::new(2.0, 50.0)), Anchor::Left);
        assert_eq!(nearest_anchor(b, Point::new(50.0, 98.0)), Anchor::Bottom);
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
        let out = tool.pointer_down(rot_world, rot_screen, &sel, &disp, &cam, None);
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
