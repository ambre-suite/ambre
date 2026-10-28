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
// End-to-end multi-user waits on SYNC P4, the `pub_key` -> `UserId` resolution
// that keys inbound samples (`toile-presence`). Until it lands, remote samples
// never reach this thread, though the render pass itself is complete and
// synthetic `PeerPresence` events exercise it today.
// ============================================================================

use std::collections::HashMap;
use std::collections::VecDeque;

use ambre_bus::UserId;
use ambre_bus::app::toile::{
    DisplayNode, DisplayViewport, ElementId, PresencePayload, PresenceUpdate,
};
use ambre_canvas::TextCache;
use ambre_core::DisplayName;
use ambre_draw::{
    Affine, Alignment, BezPath, Brush, Color, DrawSurface, FontContext, Point,
    PositionedLayoutItem, Rect, Stroke, peer_color,
};

use super::viewport::ViewportTransform;

/// Maximum laser-trail samples kept per peer. The painter strokes the ring
/// buffer as a fading polyline, and older samples decay to transparent. The
/// trail "melts" behind the moving highlighter without any wall-clock timer.
const LASER_TRAIL_MAX: usize = 24;

/// Screen size of a remote cursor arrow, in logical pixels.
const CURSOR_SIZE: ambre_core::ViewPx = ambre_core::ViewPx::new(14.0);

/// Name-label font size, in logical pixels.
const LABEL_FONT_SIZE: ambre_core::ViewPx = ambre_core::ViewPx::new(12.0);

/// How long a laser sample stays visible. Long enough to read a
/// gesture across the board, short enough that a highlighter held still leaves
/// nothing behind (`TOILE-UI-077`).
const LASER_FADE: ambre_core::PageMillis = ambre_core::PageMillis::new(1_200.0);

/// How often the local peer sends a sample while nothing changes. The heartbeat keeps this client in the others' live set
/// (`PRESENCE-009`), and it advances the Logic thread's only clock. It MUST
/// stay well under `PRESENCE_TTL`, since the network may drop several
/// heartbeats before a peer ages out.
pub const PRESENCE_HEARTBEAT: ambre_core::PageMillis = ambre_core::PageMillis::new(2_000.0);

/// The broadcast schedule for this peer's own presence (`TOILE-UI-073`). Pure:
/// it decides whether the sample built this frame is worth sending, and holds
/// nothing else.
///
/// A sample goes out when it differs from the last one, or when the heartbeat
/// falls due. A moved pointer, a changed selection, a stroke sample and a laser
/// point all differ. The gesture path carries the latency, and the heartbeat
/// only says "still here".
#[derive(Default)]
pub struct PresenceSender {
    last: Option<PresencePayload>,
    last_sent: ambre_core::PageTime,
}

impl PresenceSender {
    /// The sample to broadcast this frame, or `None` when nothing changed and
    /// the heartbeat is not due yet.
    pub fn due(
        &mut self,
        sample: PresencePayload,
        now: ambre_core::PageTime,
    ) -> Option<PresencePayload> {
        let changed = self.last.as_ref() != Some(&sample);
        let heartbeat = now - self.last_sent >= PRESENCE_HEARTBEAT;
        if !changed && !heartbeat {
            return None;
        }
        self.last = Some(sample.clone());
        self.last_sent = now;
        Some(sample)
    }

    /// Forget the last sample, so the next frame sends afresh. The seam for a
    /// document swap: the board these samples described is not the one the view
    /// is about to paint.
    pub fn reset(&mut self) {
        self.last = None;
        self.last_sent = ambre_bus::PageTime(0.0);
    }
}

/// The render-thread mirror of every peer's latest presence. Fed by
/// `ToileEvent::PeerPresence`, drained each overlay frame by [`Self::paint`].
#[derive(Default)]
pub struct PeerCursors {
    peers: HashMap<UserId, PeerState>,
}

/// One peer's latest presence plus the derived laser trail. The payload carries
/// a single laser point, and the render thread accumulates the trail.
struct PeerState {
    presence: PresencePayload,
    /// Laser samples with the frame clock each arrived at. The trail therefore
    /// fades by age rather than by rank (`TOILE-UI-077`): a highlighter held
    /// still melts away instead of freezing its tail.
    laser_trail: VecDeque<(Point, ambre_core::PageTime)>,
}

impl PeerCursors {
    /// Adopt the whole live peer set (`PRESENCE-009`). Every entry replaces that
    /// peer's presence, and this drops a peer absent from the set, trail
    /// included. That is how the overlay paints a departure without a departure
    /// signal (`TOILE-UI-075`, `TOILE-185`).
    ///
    /// A laser point extends the peer's trail, a ring buffer capped at
    /// [`LASER_TRAIL_MAX`] whose samples each carry `now`. `laser = None`
    /// clears it, so the trail vanishes as soon as the peer lifts the
    /// highlighter.
    pub fn adopt(&mut self, peers: Vec<PresenceUpdate>, now: ambre_core::PageTime) {
        self.peers
            .retain(|id, _| peers.iter().any(|p| &p.user_id == id));
        for PresenceUpdate { user_id, presence } in peers {
            let state = self.peers.entry(user_id).or_insert_with(|| PeerState {
                presence: PresencePayload::default(),
                laser_trail: VecDeque::new(),
            });
            match presence.laser {
                Some(p) => {
                    state.laser_trail.push_back((p, now));
                    while state.laser_trail.len() > LASER_TRAIL_MAX {
                        state.laser_trail.pop_front();
                    }
                }
                None => state.laser_trail.clear(),
            }
            state.presence = presence;
        }
    }

    /// Forget every peer (document swap).
    pub fn clear(&mut self) {
        self.peers.clear();
    }

    /// True when no peer presence has arrived, so the view may skip the overlay
    /// pass.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Paint every known peer's presence into the top overlay pass, in screen
    /// space above all content. `now` is the frame clock, which ages the
    /// laser trails (`TOILE-UI-077`). Pure over `DrawSurface`: host-testable.
    #[allow(clippy::too_many_arguments)]
    pub fn paint<S: DrawSurface>(
        &self,
        surface: &mut S,
        vp: &DisplayViewport,
        cam: &ViewportTransform,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        now: ambre_core::PageTime,
    ) {
        let w2s = cam.world_to_screen();
        // Deterministic peer order so the overlay draw log is stable for tests
        // and the paint order does not flicker frame to frame.
        let mut peers: Vec<(&UserId, &PeerState)> = self.peers.iter().collect();
        peers.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        for (user_id, state) in peers {
            let color = peer_color(user_id);
            paint_peer(
                surface, user_id, state, &color, vp, &w2s, font_cx, text_cache, now,
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
    now: ambre_core::PageTime,
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
        let path = screen_polyline(
            pts.iter()
                .map(|s| Point::new(s.x.widened().get(), s.y.widened().get())),
            w2s,
        );
        surface.stroke_path(&path, &Brush::Solid(*color), &Stroke::new(2.0));
    }

    // 3. Laser trail: a fading highlighter trail, oldest -> newest, each segment
    // a touch more opaque so the trail melts behind the moving point.
    paint_laser(surface, &state.laser_trail, color, w2s, now);

    // 4. Cursor + name chip at the peer's pointer, if the pointer is on-board.
    if let Some(cursor) = p.cursor {
        let screen = *w2s * cursor;
        let label = peer_label(user_id, p.name.as_ref());
        paint_cursor(surface, screen, label.as_str(), color, font_cx, text_cache);
    }
}

/// Stroke the laser ring buffer as an age-faded polyline. A segment's alpha
/// falls with how long ago it arrived, and this skips one older than
/// [`LASER_FADE`]. Fading by age rather than by rank is what makes a
/// highlighter held still fade out instead of freezing its tail
/// (`TOILE-UI-077`).
fn paint_laser<S: DrawSurface>(
    surface: &mut S,
    trail: &VecDeque<(Point, ambre_core::PageTime)>,
    color: &Color,
    w2s: &Affine,
    now: ambre_core::PageTime,
) {
    if trail.len() < 2 {
        return;
    }
    for i in 1..trail.len() {
        let (pa, _) = trail[i - 1];
        let (pb, tb) = trail[i];
        let alpha = laser_alpha(tb, now);
        if alpha <= 0.0 {
            continue;
        }
        let a = *w2s * pa;
        let b = *w2s * pb;
        surface.stroke_line(
            ambre_draw::Line::new((a.x, a.y), (b.x, b.y)),
            color.with_alpha(alpha),
            3.0,
        );
    }
}

/// The alpha of a laser segment that arrived at `sampled_at`, read at `now`:
/// opaque when fresh, zero once [`LASER_FADE`] has passed. A sample from the
/// future (a clock that stepped back) reads as fresh rather than as invisible.
pub fn laser_alpha(sampled_at: ambre_core::PageTime, now: ambre_core::PageTime) -> f32 {
    let age = (now - sampled_at).max(ambre_core::PageMillis::ZERO);
    if age >= LASER_FADE {
        return 0.0;
    }
    (1.0 - age / LASER_FADE) as f32
}

/// What a peer's chip reads: the display name their sample carried, or a short
/// handle derived from the id when it carried none (`TOILE-UI-076`). A blank
/// name is no name: the sample's decode drops it, as it would paint an empty
/// chip.
pub fn peer_label(user_id: &UserId, name: Option<&DisplayName>) -> DisplayName {
    name.cloned()
        .unwrap_or_else(|| DisplayName::handle_of(user_id))
}

/// Draw the pointer arrow and a name chip next to it, both in the peer colour.
fn paint_cursor<S: DrawSurface>(
    surface: &mut S,
    at: Point,
    label: &str,
    color: &Color,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
) {
    // Arrowhead: a filled triangle whose tip sits on the pointer, opening down-
    // right (a conventional cursor silhouette), drawn in screen space.
    let s = f64::from(CURSOR_SIZE.get());
    let mut arrow = BezPath::new();
    arrow.move_to((at.x, at.y));
    arrow.line_to((at.x, at.y + s));
    arrow.line_to((at.x + s * 0.30, at.y + s * 0.72));
    arrow.line_to((at.x + s * 0.72, at.y + s * 0.72));
    arrow.close_path();
    surface.fill_path(&arrow, &Brush::Solid(*color));

    // Name chip: the peer's label in their colour, right of the pointer. What
    // the label reads is `peer_label`; it decides nothing (`PRESENCE-004`).
    let layout = text_cache.get_or_build(
        font_cx,
        label,
        LABEL_FONT_SIZE,
        ambre_core::ViewPx::new(200.0),
        Alignment::Start,
        ambre_canvas::Weight::Bold,
        ambre_canvas::Slant::Upright,
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

/// The world-space AABB of a displayed element, or `None` when the id is not in
/// the culled set. An off-screen selection ghost has no box this frame.
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
            locked: ambre_bus::app::toile::LockState::Unlocked,
            style: ElementStyle::default(),
            geom: DisplayGeom::Rect,
            link: None,
            group: None,
        })
    }

    fn vp_with(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            version: ambre_bus::ViewportVersion(0),
            grid_unit: ambre_core::BoardLength::new(20.0),
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes,
        }
    }

    fn cam() -> ViewportTransform {
        ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0))
    }

    fn paint(cursors: &PeerCursors, vp: &DisplayViewport) -> RecordingSurface {
        paint_at(cursors, vp, ambre_bus::PageTime(0.0))
    }

    /// Paint at a frame clock, for the trail's age fade.
    fn paint_at(
        cursors: &PeerCursors,
        vp: &DisplayViewport,
        now: ambre_core::PageTime,
    ) -> RecordingSurface {
        let mut surface = RecordingSurface::new();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new();
        cursors.paint(&mut surface, vp, &cam(), &mut font_cx, &mut tc, now);
        surface
    }

    /// Adopt a one-peer live set at clock zero, the shape every test below used
    /// before the set replaced the per-peer update (`PRESENCE-009`).
    fn adopt_one(cursors: &mut PeerCursors, user_id: UserId, presence: PresencePayload) {
        cursors.adopt(
            vec![PresenceUpdate { user_id, presence }],
            ambre_bus::PageTime(0.0),
        );
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
        adopt_one(
            &mut cursors,
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
        adopt_one(
            &mut cursors,
            user(5),
            PresencePayload {
                // "a" is on-board, the cull dropped "gone" -> only one box.
                selection: vec![ElementId("a".into()), ElementId("gone".into())],
                ..Default::default()
            },
        );
        let vp = vp_with(vec![elem("a", Rect::new(0.0, 0.0, 50.0, 50.0))]);
        let s = paint(&cursors, &vp);
        assert_eq!(s.count(|o| matches!(o, DrawOp::StrokeRect { .. })), 1);
    }

    /// `PRESENCE-010`: a remote pointer that jumps is painted at its new
    /// sample at once, exactly where a peer seen there first is painted.
    #[test]
    fn a_remote_pointer_that_jumps_snaps_to_its_new_sample() {
        let at = |x: f64| PresencePayload {
            cursor: Some(Point::new(x, x)),
            ..Default::default()
        };
        // The arrow's path is not recorded, so the name chip's placement
        // stands for the pointer's.
        let painted = |cursors: &PeerCursors| paint(cursors, &vp_with(vec![])).ops().to_vec();
        let mut jumped = PeerCursors::default();
        adopt_one(&mut jumped, user(4), at(0.0));
        let start = painted(&jumped);
        adopt_one(&mut jumped, user(4), at(900.0));
        let mut fresh = PeerCursors::default();
        adopt_one(&mut fresh, user(4), at(900.0));

        assert_eq!(painted(&jumped), painted(&fresh));
        assert_ne!(painted(&jumped), start, "the pointer moved");
    }

    /// `PRESENCE-007`: a selection anchor names an element, never a position
    /// in the board. After a structural edit deletes one element, inserts
    /// another before it and moves it, the ghost still outlines that element.
    #[test]
    fn a_selection_anchor_survives_a_structural_edit() {
        let mut cursors = PeerCursors::default();
        adopt_one(
            &mut cursors,
            user(5),
            PresencePayload {
                selection: vec![ElementId("b".into())],
                ..Default::default()
            },
        );
        let moved = Rect::new(200.0, 200.0, 260.0, 240.0);
        let before = vp_with(vec![
            elem("a", Rect::new(0.0, 0.0, 50.0, 50.0)),
            elem("b", Rect::new(100.0, 0.0, 150.0, 50.0)),
        ]);
        let after = vp_with(vec![
            elem("c", Rect::new(0.0, 0.0, 30.0, 30.0)),
            elem("b", moved),
        ]);
        let ghosts = |vp: &DisplayViewport| -> Vec<DrawOp> {
            paint(&cursors, vp)
                .ops()
                .iter()
                .filter(|o| matches!(o, DrawOp::StrokeRect { .. }))
                .cloned()
                .collect()
        };

        assert_eq!(ghosts(&before).len(), 1);
        assert_eq!(ghosts(&after), ghosts(&vp_with(vec![elem("b", moved)])));
        assert_ne!(ghosts(&after), ghosts(&before), "the ghost followed it");
    }

    #[test]
    fn live_stroke_draws_a_polyline() {
        let mut cursors = PeerCursors::default();
        let sp = |x: f32, y: f32| {
            StrokePoint::full_pressure(
                ambre_core::StrokeCoord::new(x),
                ambre_core::StrokeCoord::new(y),
            )
        };
        adopt_one(
            &mut cursors,
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
            adopt_one(
                &mut cursors,
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
        adopt_one(&mut cursors, user(11), PresencePayload::default());
        let s = paint(&cursors, &vp_with(vec![]));
        assert_eq!(s.count(|o| matches!(o, DrawOp::StrokeLine { .. })), 0);
    }

    /// TOILE-UI-075 / TOILE-185: the overlay drops a peer absent from the live
    /// set, trail included. No departure signal exists anywhere, so the set is
    /// the whole answer (`PRESENCE-009`).
    #[test]
    fn a_peer_absent_from_the_set_stops_being_painted() {
        let mut cursors = PeerCursors::default();
        let (a, b) = (user(1), user(2));
        let at = |x: f64| PresencePayload {
            cursor: Some(Point::new(x, 0.0)),
            laser: Some(Point::new(x, 1.0)),
            ..Default::default()
        };
        cursors.adopt(
            vec![
                PresenceUpdate {
                    user_id: a,
                    presence: at(0.0),
                },
                PresenceUpdate {
                    user_id: b,
                    presence: at(10.0),
                },
            ],
            ambre_bus::PageTime(0.0),
        );
        assert_eq!(
            paint(&cursors, &vp_with(vec![])).count(|o| matches!(o, DrawOp::FillPath { .. })),
            2,
            "one arrow per live peer"
        );

        // Bob left: the next set carries Alice alone.
        cursors.adopt(
            vec![PresenceUpdate {
                user_id: a,
                presence: at(0.0),
            }],
            ambre_bus::PageTime(0.0),
        );
        let s = paint(&cursors, &vp_with(vec![]));
        assert_eq!(
            s.count(|o| matches!(o, DrawOp::FillPath { .. })),
            1,
            "the departed peer is gone"
        );

        // And an empty set paints nothing at all.
        cursors.adopt(Vec::new(), ambre_bus::PageTime(0.0));
        assert!(cursors.is_empty());
        assert!(paint(&cursors, &vp_with(vec![])).ops().is_empty());
    }

    /// TOILE-UI-077: the trail fades by age. A highlighter held still keeps
    /// sending the same point, and the trail must still melt away.
    #[test]
    fn a_still_highlighter_fades_out() {
        let mut cursors = PeerCursors::default();
        for _ in 0..3 {
            cursors.adopt(
                vec![PresenceUpdate {
                    user_id: user(7),
                    presence: PresencePayload {
                        laser: Some(Point::new(5.0, 5.0)),
                        ..Default::default()
                    },
                }],
                ambre_bus::PageTime(0.0),
            );
        }
        // Read at the sample clock: the segments are there.
        assert_eq!(
            paint_at(&cursors, &vp_with(vec![]), ambre_bus::PageTime(0.0))
                .count(|o| matches!(o, DrawOp::StrokeLine { .. })),
            2
        );
        // Read a fade later, with no new sample: the painter draws nothing.
        assert_eq!(
            paint_at(
                &cursors,
                &vp_with(vec![]),
                ambre_bus::PageTime(LASER_FADE.get() + 1.0)
            )
            .count(|o| matches!(o, DrawOp::StrokeLine { .. })),
            0,
            "an aged trail is not painted"
        );
        assert_eq!(
            laser_alpha(ambre_bus::PageTime(0.0), ambre_bus::PageTime(0.0)),
            1.0
        );
        assert_eq!(
            laser_alpha(
                ambre_bus::PageTime(0.0),
                ambre_bus::PageTime(LASER_FADE.get())
            ),
            0.0
        );
        // A sample from the future reads as fresh, never as invisible.
        assert_eq!(
            laser_alpha(ambre_bus::PageTime(100.0), ambre_bus::PageTime(0.0)),
            1.0
        );
    }

    /// TOILE-UI-076: the chip reads the name the sample carried, and falls back
    /// to the short handle when it carried none -- or a blank one.
    #[test]
    fn the_label_is_the_name_the_sample_carried() {
        let id = user(42);
        let ada = DisplayName::new("Ada").unwrap();
        assert_eq!(peer_label(&id, Some(&ada)), "Ada");
        assert_eq!(peer_label(&id, None), DisplayName::handle_of(&id));
        assert_eq!(DisplayName::handle_of(&id).as_str().len(), 8);
    }

    /// TOILE-UI-073: a sample goes out when it changed, and otherwise only on
    /// the heartbeat. The heartbeat keeps this peer in the others' live set.
    #[test]
    fn a_sample_is_sent_on_a_change_or_on_the_heartbeat() {
        let mut sender = PresenceSender::default();
        let at = |x: f64| PresencePayload {
            cursor: Some(Point::new(x, 0.0)),
            ..Default::default()
        };
        assert!(
            sender.due(at(0.0), ambre_bus::PageTime(0.0)).is_some(),
            "the first sample sends"
        );
        assert!(
            sender.due(at(0.0), ambre_bus::PageTime(1.0)).is_none(),
            "an unchanged sample waits for the heartbeat"
        );
        assert!(
            sender.due(at(1.0), ambre_bus::PageTime(2.0)).is_some(),
            "a moved pointer sends at once"
        );
        assert!(
            sender
                .due(at(1.0), ambre_bus::PageTime(2.0 + PRESENCE_HEARTBEAT.get()))
                .is_some(),
            "an idle peer still says it is there"
        );
        // A document swap forgets the last sample, so the new board sends
        // afresh.
        sender.reset();
        assert!(
            sender
                .due(at(1.0), ambre_bus::PageTime(2.0 + PRESENCE_HEARTBEAT.get()))
                .is_some()
        );
    }

    /// `PRESENCE-004`: a name decides nothing. Two peers under one name are
    /// two cursors in their own colours, and a peer that renames itself keeps
    /// its colour.
    #[test]
    fn a_peer_name_decides_neither_identity_nor_colour() {
        let sample = |seed: u8, name: &str| PresenceUpdate {
            user_id: user(seed),
            presence: PresencePayload {
                name: ambre_core::DisplayName::new(name).ok(),
                cursor: Some(Point::new(f64::from(seed), 0.0)),
                ..Default::default()
            },
        };
        let painted = |peers: Vec<PresenceUpdate>| {
            let mut cursors = PeerCursors::default();
            cursors.adopt(peers, ambre_bus::PageTime(0.0));
            paint(&cursors, &vp_with(vec![]))
        };

        let twins = painted(vec![sample(1, "Ada"), sample(2, "Ada")]);
        assert_eq!(
            twins.count(|o| matches!(o, DrawOp::FillPath { .. })),
            2,
            "one arrow per peer"
        );
        let colours = twins.fill_colors();
        assert!(colours.contains(&peer_color(&user(1))), "{colours:?}");
        assert!(colours.contains(&peer_color(&user(2))), "{colours:?}");
        assert_eq!(
            painted(vec![sample(1, "Ada")]).fill_colors(),
            painted(vec![sample(1, "Grace")]).fill_colors()
        );
    }

    #[test]
    fn two_peers_get_distinct_colours() {
        let mut cursors = PeerCursors::default();
        cursors.adopt(
            vec![
                PresenceUpdate {
                    user_id: user(1),
                    presence: PresencePayload {
                        cursor: Some(Point::new(0.0, 0.0)),
                        ..Default::default()
                    },
                },
                PresenceUpdate {
                    user_id: user(2),
                    presence: PresencePayload {
                        cursor: Some(Point::new(5.0, 5.0)),
                        ..Default::default()
                    },
                },
            ],
            ambre_bus::PageTime(0.0),
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
