// ambre-toile/src/ui/presence.rs
// ============================================================================
// Multi-user presence overlay (toile-presence). Remote presence arrives as
// `ToileEvent::PeerPresence` (ephemeral, `toile-presence`), keyed by the
// sender's canonical `UserId`. This module owns the render-thread mirror of
// every peer's latest presence and paints it in the top overlay pass, above
// all content and never culled:
//
//   * selection ghosts -- the peer's selected elements outlined in their colour;
//   * live stroke       -- an in-flight freehand stroke, streamed before commit;
//   * laser trail       -- a short, fading highlighter trail (a ring buffer of
//                          the last laser samples, alpha-decayed by recency);
//   * cursor + label    -- the peer's pointer with a name chip in their colour.
//
// Every peer's colour comes from the shared cross-app `ambre_draw::peer_color`
// hashed over the canonical `UserId`, so a collaborator keeps one colour across
// Prose, Trame and Toile. The paint entry point is a pure function over
// `DrawSurface`, so it is host-testable against `RecordingSurface` with no GPU.
//
// End-to-end multi-user is gated on SYNC P4 (the `pub_key` -> `UserId`
// resolution that keys inbound frames, `toile-presence`): until it lands, remote
// frames never reach this thread. The render pass itself is complete and
// exercised now against synthetic `PeerPresence` events.
// ============================================================================

use std::collections::HashMap;
use std::collections::VecDeque;

use ambre_bus::UserId;
use ambre_bus::app::toile::{DisplayNode, DisplayViewport, ElementId, PresencePayload};
use ambre_canvas::TextCache;
use ambre_draw::{
    Affine, Alignment, BezPath, Brush, Color, DrawSurface, FontContext, Point,
    PositionedLayoutItem, Rect, Stroke, peer_color,
};

use super::viewport::ViewportTransform;

/// Maximum laser-trail samples kept per peer. The trail is a short ring buffer
/// stroked as a fading polyline; older samples decay to transparent so the trail
/// "melts" behind the moving highlighter without any wall-clock timer.
const LASER_TRAIL_MAX: usize = 24;

/// Screen size of a remote cursor arrow, in logical pixels.
const CURSOR_SIZE_PX: f64 = 14.0;

/// Name-label font size, in logical pixels.
const LABEL_FONT_PX: f32 = 12.0;

/// The render-thread mirror of every peer's latest presence. Fed by
/// `ToileEvent::PeerPresence`, drained each overlay frame by [`Self::paint`].
#[derive(Default)]
pub struct PeerCursors {
    peers: HashMap<UserId, PeerState>,
}

/// One peer's latest presence plus the derived laser trail (the payload carries
/// a single laser point; the trail is accumulated render-side).
struct PeerState {
    presence: PresencePayload,
    laser_trail: VecDeque<Point>,
}

impl PeerCursors {
    /// Fold one `PeerPresence` update in, replacing the peer's presence and
    /// extending (or clearing) its laser trail. A laser point pushes onto the
    /// ring buffer (capped at [`LASER_TRAIL_MAX`]); `laser = None` clears it, so
    /// the trail vanishes as soon as the peer lifts the highlighter.
    pub fn update(&mut self, user_id: UserId, presence: PresencePayload) {
        let state = self.peers.entry(user_id).or_insert_with(|| PeerState {
            presence: PresencePayload::default(),
            laser_trail: VecDeque::new(),
        });
        match presence.laser {
            Some(p) => {
                state.laser_trail.push_back(p);
                while state.laser_trail.len() > LASER_TRAIL_MAX {
                    state.laser_trail.pop_front();
                }
            }
            None => state.laser_trail.clear(),
        }
        state.presence = presence;
    }

    /// Drop a peer that has left (a future Sync-level departure signal); also the
    /// reset seam on a document swap.
    pub fn remove(&mut self, user_id: &UserId) {
        self.peers.remove(user_id);
    }

    /// Forget every peer (document swap).
    pub fn clear(&mut self) {
        self.peers.clear();
    }

    /// True when no peer presence is known -- the overlay pass can be skipped.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Paint every known peer's presence into the top overlay pass, in screen
    /// space above all content. Pure over `DrawSurface`: host-testable.
    pub fn paint<S: DrawSurface>(
        &self,
        surface: &mut S,
        vp: &DisplayViewport,
        cam: &ViewportTransform,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
    ) {
        let w2s = cam.world_to_screen();
        // Deterministic peer order so the overlay draw log is stable for tests
        // and the paint order does not flicker frame to frame.
        let mut peers: Vec<(&UserId, &PeerState)> = self.peers.iter().collect();
        peers.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        for (user_id, state) in peers {
            let color = peer_color(user_id);
            paint_peer(
                surface, user_id, state, &color, vp, &w2s, font_cx, text_cache,
            );
        }
    }
}

/// Paint one peer's overlay: selection ghosts, live stroke, laser trail, cursor.
#[allow(clippy::too_many_arguments)]
fn paint_peer<S: DrawSurface>(
    surface: &mut S,
    user_id: &UserId,
    state: &PeerState,
    color: &Color,
    vp: &DisplayViewport,
    w2s: &Affine,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
) {
    let p = &state.presence;

    // 1. Selection ghosts: outline each of the peer's selected elements that is
    // in the culled set, in the peer's colour.
    for id in &p.selection {
        if let Some(wb) = element_bounds(vp, id) {
            surface.stroke_rect(rect_to_screen(wb, w2s), *color, 1.5);
        }
    }

    // 2. Live stroke: the in-flight freehand polyline, before it commits.
    if let Some(pts) = &p.live_stroke
        && pts.len() >= 2
    {
        let path = screen_polyline(pts.iter().map(|s| Point::new(s.x as f64, s.y as f64)), w2s);
        surface.stroke_path(&path, &Brush::Solid(*color), &Stroke::new(2.0));
    }

    // 3. Laser trail: a fading highlighter trail, oldest -> newest, each segment
    // a touch more opaque so the trail melts behind the moving point.
    paint_laser(surface, &state.laser_trail, color, w2s);

    // 4. Cursor + name chip at the peer's pointer, if the pointer is on-board.
    if let Some(cursor) = p.cursor {
        let screen = *w2s * cursor;
        paint_cursor(surface, screen, user_id, color, font_cx, text_cache);
    }
}

/// Stroke the laser ring buffer as a recency-faded polyline. Each segment's
/// alpha rises toward the head so the trail decays smoothly without a timer.
fn paint_laser<S: DrawSurface>(
    surface: &mut S,
    trail: &VecDeque<Point>,
    color: &Color,
    w2s: &Affine,
) {
    if trail.len() < 2 {
        return;
    }
    let n = trail.len();
    for i in 1..n {
        let a = *w2s * trail[i - 1];
        let b = *w2s * trail[i];
        // Newest segment fully opaque, tail fades to ~10%.
        let t = i as f32 / n as f32;
        let faded = color.with_alpha(0.1 + 0.9 * t);
        surface.stroke_line(a.x, a.y, b.x, b.y, faded, 3.0);
    }
}

/// Draw the pointer arrow and a name chip next to it, both in the peer colour.
fn paint_cursor<S: DrawSurface>(
    surface: &mut S,
    at: Point,
    user_id: &UserId,
    color: &Color,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
) {
    // Arrowhead: a filled triangle whose tip sits on the pointer, opening down-
    // right (a conventional cursor silhouette), drawn in screen space.
    let s = CURSOR_SIZE_PX;
    let mut arrow = BezPath::new();
    arrow.move_to((at.x, at.y));
    arrow.line_to((at.x, at.y + s));
    arrow.line_to((at.x + s * 0.30, at.y + s * 0.72));
    arrow.line_to((at.x + s * 0.72, at.y + s * 0.72));
    arrow.close_path();
    surface.fill_path(&arrow, &Brush::Solid(*color));

    // Name chip: a short label in the peer colour, right of the pointer. Until
    // the `UserId` -> display-name directory lands (SYNC P4), the label is the
    // id's short hex prefix -- a stable, human-distinguishable handle.
    let label = short_handle(user_id);
    let layout = text_cache.get_or_build(
        font_cx,
        &label,
        LABEL_FONT_PX,
        200.0,
        Alignment::Start,
        true,
        false,
    );
    let pad = 4.0;
    let chip_x = at.x + s * 0.8;
    let chip_y = at.y + s * 0.9;
    let text_w = layout.width() as f64;
    let text_h = layout.height() as f64;
    surface.fill_rect(
        Rect::new(
            chip_x,
            chip_y,
            chip_x + text_w + 2.0 * pad,
            chip_y + text_h + 2.0 * pad,
        ),
        *color,
    );
    let brush = Brush::Solid(Color::from_rgba8(255, 255, 255, 255));
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = chip_x + pad + run.offset() as f64;
            let ty = chip_y + pad + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
        }
    }
}

/// A short, human-distinguishable handle for a peer: the first 8 hex chars of
/// the canonical `UserId`. Placeholder until the display-name directory lands.
fn short_handle(user_id: &UserId) -> String {
    let bytes = user_id.as_bytes();
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3]
    )
}

/// The world-space AABB of a displayed element, or `None` when the id is not in
/// the culled set (an off-screen selection ghost simply has no box this frame).
fn element_bounds(vp: &DisplayViewport, id: &ElementId) -> Option<Rect> {
    vp.nodes.iter().find_map(|n| match n {
        DisplayNode::Element(e) if &e.id == id => Some(e.bounds),
        _ => None,
    })
}

/// Map a world-space rectangle to a screen-space rectangle through `w2s`.
fn rect_to_screen(b: Rect, w2s: &Affine) -> Rect {
    let tl = *w2s * Point::new(b.x0, b.y0);
    let br = *w2s * Point::new(b.x1, b.y1);
    Rect::new(tl.x, tl.y, br.x, br.y)
}

/// Build a screen-space open polyline from world-space points through `w2s`.
fn screen_polyline(pts: impl Iterator<Item = Point>, w2s: &Affine) -> BezPath {
    let mut path = BezPath::new();
    let mut started = false;
    for wp in pts {
        let sp = *w2s * wp;
        if started {
            path.line_to(sp);
        } else {
            path.move_to(sp);
            started = true;
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{
        DisplayElement, DisplayGeom, ElementKind, ElementStyle, ShapeKind, StrokePoint,
    };
    use ambre_draw::testing::{DrawOp, RecordingSurface};

    fn user(seed: u8) -> UserId {
        UserId::from_bytes([seed; 16])
    }

    fn elem(id: &str, bounds: Rect) -> DisplayNode {
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

    fn vp_with(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            reactions: Default::default(),
            viewport: Default::default(),
            nodes,
        }
    }

    fn cam() -> ViewportTransform {
        ViewportTransform::new(1000.0, 800.0)
    }

    fn paint(cursors: &PeerCursors, vp: &DisplayViewport) -> RecordingSurface {
        let mut surface = RecordingSurface::new();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new(1.0);
        cursors.paint(&mut surface, vp, &cam(), &mut font_cx, &mut tc);
        surface
    }

    #[test]
    fn empty_paints_nothing_and_is_empty() {
        let cursors = PeerCursors::default();
        assert!(cursors.is_empty());
        let s = paint(&cursors, &vp_with(vec![]));
        assert!(s.ops().is_empty());
        assert_eq!(s.transform_depth(), 0);
    }

    #[test]
    fn cursor_draws_arrow_and_label_chip() {
        let mut cursors = PeerCursors::default();
        cursors.update(
            user(3),
            PresencePayload {
                cursor: Some(Point::new(0.0, 0.0)),
                ..Default::default()
            },
        );
        assert!(!cursors.is_empty());
        let s = paint(&cursors, &vp_with(vec![]));
        // Arrow is one filled path; the chip is one filled rect; the label is a
        // glyph run.
        assert_eq!(s.count(|o| matches!(o, DrawOp::FillPath { .. })), 1);
        assert_eq!(s.count(|o| matches!(o, DrawOp::FillRect { .. })), 1);
        assert_eq!(s.count(|o| matches!(o, DrawOp::Glyphs { .. })), 1);
    }

    #[test]
    fn selection_ghosts_outline_visible_elements_only() {
        let mut cursors = PeerCursors::default();
        cursors.update(
            user(5),
            PresencePayload {
                // "a" is on-board, "gone" is culled out -> only one box.
                selection: vec![ElementId("a".into()), ElementId("gone".into())],
                ..Default::default()
            },
        );
        let vp = vp_with(vec![elem("a", Rect::new(0.0, 0.0, 50.0, 50.0))]);
        let s = paint(&cursors, &vp);
        assert_eq!(s.count(|o| matches!(o, DrawOp::StrokeRect { .. })), 1);
    }

    #[test]
    fn live_stroke_draws_a_polyline() {
        let mut cursors = PeerCursors::default();
        let sp = |x: f32, y: f32| StrokePoint {
            x,
            y,
            pressure: 1.0,
        };
        cursors.update(
            user(9),
            PresencePayload {
                live_stroke: Some(vec![sp(0.0, 0.0), sp(10.0, 10.0), sp(20.0, 5.0)]),
                ..Default::default()
            },
        );
        let s = paint(&cursors, &vp_with(vec![]));
        assert_eq!(s.count(|o| matches!(o, DrawOp::StrokePath { .. })), 1);
    }

    #[test]
    fn laser_accumulates_a_fading_trail_and_clears_on_lift() {
        let mut cursors = PeerCursors::default();
        // Three successive laser samples -> a 3-point trail -> 2 faded segments.
        for i in 0..3 {
            cursors.update(
                user(11),
                PresencePayload {
                    laser: Some(Point::new(i as f64 * 10.0, 0.0)),
                    ..Default::default()
                },
            );
        }
        let s = paint(&cursors, &vp_with(vec![]));
        assert_eq!(s.count(|o| matches!(o, DrawOp::StrokeLine { .. })), 2);

        // Lifting the highlighter (laser = None) clears the trail.
        cursors.update(user(11), PresencePayload::default());
        let s = paint(&cursors, &vp_with(vec![]));
        assert_eq!(s.count(|o| matches!(o, DrawOp::StrokeLine { .. })), 0);
    }

    #[test]
    fn two_peers_get_distinct_colours() {
        let mut cursors = PeerCursors::default();
        cursors.update(
            user(1),
            PresencePayload {
                cursor: Some(Point::new(0.0, 0.0)),
                ..Default::default()
            },
        );
        cursors.update(
            user(2),
            PresencePayload {
                cursor: Some(Point::new(5.0, 5.0)),
                ..Default::default()
            },
        );
        let s = paint(&cursors, &vp_with(vec![]));
        // Two arrows, each its own peer colour.
        let colors = s.fill_colors();
        assert_eq!(
            s.count(|o| matches!(o, DrawOp::FillPath { .. })),
            2,
            "one arrow per peer"
        );
        // The two chip fills use two different peer colours.
        assert!(colors.len() >= 2, "distinct peer colours: {colors:?}");
    }
}
