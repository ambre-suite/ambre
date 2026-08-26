// ambre-toile/src/ui/embed.rs
// ============================================================================
// Nested Ambre documents in the view layer (`toile-embeds`).
//
// A `doc_embed` element is a rectangle on the board whose content is another
// Ambre document with its own `DocumentId`, `DocumentKey` and `LoroDoc`. This
// module holds the two things `ToileView` needs and nothing else:
//
//   - `visible_embeds`: the culled `doc_embed`s projected to screen space, which
//     the view PUBLISHES for `FrameHost`. `ambre-toile` cannot paint a child, nor
//     route input into one: both need `AppView`, the closed enum that lives in
// the launcher, downstream of every app crate (architecture / ARCH P2). So
//     the composition inverts -- the view describes its embeds, the launcher acts
//     on them.
//   - `to_child_space`: the pointer transform. Screen -> world -> the element's
//     unrotated local frame -> child pixels.
//
// Focus itself lives on `ToileView` (view-local state, never CRDT, never derived
// -- `derive.rs` always emits `focused: false`; a peer's focus is not ours).
//
// Pure geometry -- host-tested, no `cfg(wasm32)`, no GPU.
// ============================================================================

use std::collections::HashMap;

use ambre_bus::app::toile::{DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, ElementId};
use ambre_bus::{DocType, DocumentId, LogicCommand};
use ambre_draw::{Point, Rect, Size};

use super::viewport::ViewportTransform;

/// How long a `doc_embed` must stay out of the culled set before its child
/// document's sync session is closed ("Opening the child").
///
/// Visibility comes from the Logic R-tree cull set, which flips on every
/// pan. Binding the session lifecycle straight to it would thrash the WebSocket:
/// a scroll oscillation would storm open/close, and each re-open costs a members
/// fetch, a key unwrap and a snapshot hydration. So an embed opens the instant it
/// enters the cull set but only closes after staying out for this long; it
/// re-entering cancels the pending close.
pub const EMBED_CLOSE_DEBOUNCE_MS: f64 = 5_000.0;

/// One `doc_embed` in the current culled frame, projected to screen space and
/// published for `FrameHost` (`toile-embeds` "Render").
#[derive(Clone, Debug, PartialEq)]
pub struct VisibleEmbed {
    /// The board element carrying the reference (the focus / hit-test key).
    pub element: ElementId,
    /// The embedded document itself -- the key into `FrameHost`'s child views and
    /// the id the child's own commands are addressed to.
    pub document_id: DocumentId,
    pub doc_type: DocType,
    /// Where to paint the child, in physical screen pixels. The axis-aligned
    /// bounding box of the element's (possibly rotated) quad: `push_clip` is
    /// rect-only, so a rotated embed clips to its AABB -- accepted, see
    /// "Clip limitation".
    pub screen_rect: Rect,
    /// The child's own canvas size in physical pixels. A child pixel is a screen
    /// pixel by construction (both are world units scaled by the camera's zoom),
    /// so this is just the element's world size at the current zoom.
    pub child_size: Size,
    /// View-local focus ("Input focus"): while true, `FrameHost` maps canvas
    /// input into this child and hands it to the child's own `handle_input`.
    pub focused: bool,
}

/// Map a screen point into a `doc_embed`'s child pixel space.
///
/// `screen -> world` undoes the camera; `world -> local` undoes the element's
/// rotation (elements store geometry in the unrotated frame bounded by `bounds`,
/// with `transform` the rotation about that AABB's center -- see `hit.rs`); then
/// the child's origin is `bounds.origin` and its scale is the camera's zoom.
///
/// Deltas need no equivalent: a child pixel *is* a screen pixel, since the same
/// `zoom` scales both. A wheel delta forwards verbatim.
pub fn to_child_space(elem: &DisplayElement, cam: &ViewportTransform, screen: Point) -> Point {
    let world = cam.screen_to_world() * screen;
    let local = elem.transform.inverse() * world;
    Point::new(
        (local.x - elem.bounds.x0) * cam.zoom,
        (local.y - elem.bounds.y0) * cam.zoom,
    )
}

/// The child's canvas size in physical pixels at the current zoom.
pub fn child_size(elem: &DisplayElement, cam: &ViewportTransform) -> Size {
    Size::new(
        elem.bounds.width() * cam.zoom,
        elem.bounds.height() * cam.zoom,
    )
}

/// The screen-space AABB of an element's (possibly rotated) quad. All four
/// corners are mapped, so a rotated embed yields the box that contains it --
/// unlike a two-corner projection, which collapses under rotation.
pub fn screen_aabb(elem: &DisplayElement, cam: &ViewportTransform) -> Rect {
    let t = cam.world_to_screen() * elem.transform;
    let b = elem.bounds;
    let nw = t * Point::new(b.x0, b.y0);
    [
        t * Point::new(b.x1, b.y0),
        t * Point::new(b.x1, b.y1),
        t * Point::new(b.x0, b.y1),
    ]
    .into_iter()
    .fold(Rect::from_points(nw, nw), |r, p| r.union_pt(p))
}

/// The `doc_embed`s in `display`, projected for `FrameHost`. `focused` is the
/// view's own focused element, if any; at most one entry comes back focused.
///
/// Order follows the display walk, which is z-order (`derive.rs`), so a caller
/// painting these in sequence reproduces the board's stacking.
pub fn visible_embeds(
    display: &DisplayViewport,
    cam: &ViewportTransform,
    focused: Option<&ElementId>,
) -> Vec<VisibleEmbed> {
    display
        .nodes
        .iter()
        .filter_map(|node| {
            let DisplayNode::Element(e) = node else {
                return None;
            };
            let DisplayGeom::DocEmbed {
                document_id,
                doc_type,
                ..
            } = &e.geom
            else {
                return None;
            };
            Some(VisibleEmbed {
                element: e.id.clone(),
                document_id: *document_id,
                doc_type: *doc_type,
                screen_rect: screen_aabb(e, cam),
                child_size: child_size(e, cam),
                focused: focused == Some(&e.id),
            })
        })
        .collect()
}

/// How far (physical pixels) the pointer may travel between press and release
/// and still count as a click that focuses a `doc_embed` rather than a drag that
/// moves it ("Input focus").
pub const CLICK_SLOP: f64 = 4.0;

/// Which `doc_embed`, if any, holds input focus, and the press-to-release
/// arbitration that decides it ("Input focus"). View-local state, never
/// CRDT: a peer's focus is not ours, and `derive.rs` always emits
/// `focused: false`.
///
/// Pure: it is fed the press target, the release point and whatever `Escape` /
/// culling do, and answers only "did focus change" so the view can mark itself
/// dirty. Split out of `ToileView` because that type is not natively
/// constructible, and the click-vs-drag rule is the behavioural core of 3.c.
#[derive(Default)]
pub struct EmbedFocus {
    /// The focused embed's board element, if any.
    focused: Option<ElementId>,
    /// The embed under the last press (`None` = empty canvas / non-embed) and
    /// where it landed. Focus is settled on RELEASE, and only if the pointer
    /// barely moved: a drag that starts on an embed must still move it on the
    /// board, so entering a document is a distinct gesture from repositioning it.
    /// The press records the hit even when it is the already-focused embed, so a
    /// release there is a no-op rather than a blur (a repeat click, e.g. the
    /// second of a double-click, must not kick focus out).
    press: Option<(Option<ElementId>, Point)>,
}

impl EmbedFocus {
    /// The focused element, for publishing and for the child input route.
    pub fn current(&self) -> Option<&ElementId> {
        self.focused.as_ref()
    }

    /// A press landed at `screen` over the embed `hit` (`None` = empty canvas or a
    /// non-embed element). Records it for the release to settle; consumes
    /// nothing, so the board's own gesture proceeds regardless.
    pub fn on_press(&mut self, hit: Option<&ElementId>, screen: Point) {
        self.press = Some((hit.cloned(), screen));
    }

    /// A release landed at `screen`. If the pointer stayed within [`CLICK_SLOP`]
    /// of the press it is a click: on an embed it focuses that embed, on empty
    /// canvas it returns focus to the board. Past the slop it is a drag (a move /
    /// resize / marquee) and focus is left untouched. Returns whether focus
    /// changed.
    pub fn on_release(&mut self, screen: Point) -> bool {
        let Some((hit, from)) = self.press.take() else {
            return false;
        };
        // A drag moved something on the board; the focus rides along with none of
        // it. Only a click settles focus either way.
        if (screen - from).hypot() > CLICK_SLOP {
            return false;
        }
        match hit {
            // A click on an embed focuses it. On the already-focused one this is
            // a no-op -- crucially NOT a blur, or a repeat click would drop focus.
            Some(id) => {
                let changed = self.focused.as_ref() != Some(&id);
                self.focused = Some(id);
                changed
            }
            // A click on empty canvas / a non-embed element leaves the child.
            None => self.blur(),
        }
    }

    /// Return focus to the board (`Escape`, a document swap). Returns whether
    /// anything was focused.
    pub fn blur(&mut self) -> bool {
        self.press = None;
        self.focused.take().is_some()
    }

    /// Drop focus if `still_present` reports the focused element is gone from the
    /// culled frame -- a peer deleted it, or it scrolled far enough to be closed,
    /// leaving nothing to route input into. Returns whether focus changed.
    pub fn retain_if<F>(&mut self, still_present: F) -> bool
    where
        F: Fn(&ElementId) -> bool,
    {
        if self.focused.as_ref().is_some_and(|id| !still_present(id)) {
            self.focused = None;
            return true;
        }
        false
    }
}

/// The child sync sessions this board has open, and the debounce that keeps a
/// pan from thrashing them ("Opening the child").
///
/// Pure: it is told what is visible and what time it is, and answers with the
/// session commands to send. `ToileView` owns one and forwards its output.
#[derive(Default)]
pub struct EmbedSessions {
    /// Open children. `Some(deadline)` = out of the cull set, closing at that
    /// frame-clock reading; `None` = visible, nothing pending.
    open: HashMap<DocumentId, Option<f64>>,
}

impl EmbedSessions {
    /// Reconcile against the culled frame at `now_ms` and return the session
    /// commands to send: an `OpenDocument` for each child that just appeared, a
    /// `CloseDocument` for each that has been gone past the debounce. A child
    /// that reappears before its deadline has its close cancelled silently.
    pub fn sync(&mut self, display: &DisplayViewport, now_ms: f64) -> Vec<LogicCommand> {
        let mut cmds = Vec::new();

        for (document_id, doc_type) in embedded_docs(display) {
            match self.open.get_mut(&document_id) {
                // Back in view: cancel any pending close.
                Some(pending) => *pending = None,
                None => {
                    self.open.insert(document_id, None);
                    cmds.push(LogicCommand::OpenDocument {
                        document_id,
                        doc_type,
                    });
                }
            }
        }

        let gone: Vec<DocumentId> = self
            .open
            .keys()
            .filter(|id| !references(display, id))
            .copied()
            .collect();
        for document_id in gone {
            match self.open[&document_id] {
                // Deadline reached: close for real.
                Some(deadline) if now_ms >= deadline => {
                    self.open.remove(&document_id);
                    cmds.push(LogicCommand::CloseDocument { document_id });
                }
                // Counting down.
                Some(_) => {}
                // Just left the cull set: start the countdown.
                None => {
                    self.open
                        .insert(document_id, Some(now_ms + EMBED_CLOSE_DEBOUNCE_MS));
                }
            }
        }
        cmds
    }

    /// Close every child at once, with no debounce: the board itself is going
    /// away (a route change), so its embeds go with it.
    pub fn close_all(&mut self) -> Vec<LogicCommand> {
        std::mem::take(&mut self.open)
            .into_keys()
            .map(|document_id| LogicCommand::CloseDocument { document_id })
            .collect()
    }

    /// Whether `document_id` currently has a session (open or closing).
    pub fn is_open(&self, document_id: &DocumentId) -> bool {
        self.open.contains_key(document_id)
    }
}

/// Every nested document referenced by a `doc_embed` in the culled frame.
fn embedded_docs(display: &DisplayViewport) -> impl Iterator<Item = (DocumentId, DocType)> + '_ {
    display.nodes.iter().filter_map(|node| match node {
        DisplayNode::Element(e) => match &e.geom {
            DisplayGeom::DocEmbed {
                document_id,
                doc_type,
                ..
            } => Some((*document_id, *doc_type)),
            _ => None,
        },
        _ => None,
    })
}

/// Whether the culled frame still references `document_id`.
fn references(display: &DisplayViewport, document_id: &DocumentId) -> bool {
    embedded_docs(display).any(|(id, _)| &id == document_id)
}

/// The `doc_embed` under a world point, if any: the topmost one whose bounds
/// contain it. Used to resolve a click to a focus target ("Input focus").
/// Mirrors `hit::exact_hit`'s rule for this geometry (bounds in the local frame),
/// including the `locked` skip -- a locked embed is not interactive.
pub fn embed_at(display: &DisplayViewport, world: Point) -> Option<&DisplayElement> {
    display
        .nodes
        .iter()
        .rev()
        .filter_map(|node| match node {
            DisplayNode::Element(e) if matches!(e.geom, DisplayGeom::DocEmbed { .. }) => Some(e),
            _ => None,
        })
        .find(|e| !e.locked && e.bounds.contains(e.transform.inverse() * world))
}

// ── Tests (host tier) ───────────────────────────────────────────────────────
// Pure geometry + the session debounce. No GPU, no DOM: `ToileView` itself is
// not natively constructible (it wants a `TextInput` and leptos signals), which
// is exactly why the decidable parts live here rather than inside it.
#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{ElementKind, ElementStyle, Viewport};
    use ambre_draw::{Affine, Vec2};

    fn approx(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6
    }

    fn doc_embed(id: &str, document_id: DocumentId, bounds: Rect, rot: f64) -> DisplayElement {
        DisplayElement {
            id: ElementId(id.to_owned()),
            kind: ElementKind::DocEmbed,
            // Rotation about the bounds' center, matching `derive.rs`.
            transform: Affine::translate(bounds.center().to_vec2())
                * Affine::rotate(rot)
                * Affine::translate(-bounds.center().to_vec2()),
            bounds,
            locked: false,
            style: ElementStyle::default(),
            geom: DisplayGeom::DocEmbed {
                document_id,
                doc_type: DocType::Trame,
                // Never true off the CRDT: focus is view-local.
                focused: false,
            },
            group: None,
        }
    }

    fn viewport(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            viewport: Viewport::default(),
            nodes,
            reactions: Default::default(),
        }
    }

    // -- The pointer transform -------------------------------------------------

    #[test]
    fn child_origin_is_the_embeds_top_left() {
        let cam = ViewportTransform::new(1000.0, 800.0);
        let e = doc_embed(
            "e1",
            DocumentId::new(),
            Rect::new(20.0, 10.0, 120.0, 90.0),
            0.0,
        );
        // The element's own top-left corner is the child's (0, 0), whatever the
        // camera: that is what makes a child pixel meaningful.
        let screen_tl = cam.world_to_screen() * Point::new(20.0, 10.0);
        assert!(approx(to_child_space(&e, &cam, screen_tl), Point::ZERO));
    }

    #[test]
    fn a_child_pixel_is_a_screen_pixel() {
        let mut cam = ViewportTransform::new(1000.0, 800.0);
        cam.zoom = 2.5;
        let e = doc_embed(
            "e1",
            DocumentId::new(),
            Rect::new(0.0, 0.0, 100.0, 100.0),
            0.0,
        );
        let a = cam.world_to_screen() * Point::new(0.0, 0.0);
        let b = Point::new(a.x + 37.0, a.y + 11.0);
        let d = to_child_space(&e, &cam, b) - to_child_space(&e, &cam, a);
        // The same zoom scales the screen and the child, so a screen-space delta
        // survives the mapping unscaled. This is why a wheel forwards verbatim.
        assert!((d.x - 37.0).abs() < 1e-6 && (d.y - 11.0).abs() < 1e-6);
    }

    #[test]
    fn transform_survives_a_rotated_embed() {
        let cam = ViewportTransform::new(1000.0, 800.0);
        let bounds = Rect::new(0.0, 0.0, 100.0, 40.0);
        let e = doc_embed("e1", DocumentId::new(), bounds, std::f64::consts::FRAC_PI_2);
        // The unrotated top-left, carried through the element's own rotation, is
        // still the child's origin. Naively subtracting `bounds.origin` in world
        // space would land somewhere else entirely.
        let world_tl = e.transform * Point::new(0.0, 0.0);
        let screen_tl = cam.world_to_screen() * world_tl;
        assert!(approx(to_child_space(&e, &cam, screen_tl), Point::ZERO));
    }

    #[test]
    fn child_size_scales_with_zoom() {
        let mut cam = ViewportTransform::new(1000.0, 800.0);
        cam.zoom = 0.5;
        let e = doc_embed(
            "e1",
            DocumentId::new(),
            Rect::new(0.0, 0.0, 200.0, 80.0),
            0.0,
        );
        assert_eq!(child_size(&e, &cam), Size::new(100.0, 40.0));
    }

    #[test]
    fn screen_aabb_contains_a_rotated_quad() {
        let cam = ViewportTransform::new(1000.0, 800.0);
        // A 100x20 bar turned a quarter turn: its AABB must become 20x100. A
        // two-corner projection would report it as 100x20 still, and the child
        // would be clipped away to nothing.
        let e = doc_embed(
            "e1",
            DocumentId::new(),
            Rect::new(0.0, 0.0, 100.0, 20.0),
            std::f64::consts::FRAC_PI_2,
        );
        let r = screen_aabb(&e, &cam);
        assert!((r.width() - 20.0).abs() < 1e-6, "width {}", r.width());
        assert!((r.height() - 100.0).abs() < 1e-6, "height {}", r.height());
    }

    // -- Publishing for FrameHost ---------------------------------------------

    #[test]
    fn only_the_focused_embed_comes_back_focused() {
        let cam = ViewportTransform::new(1000.0, 800.0);
        let (a, b) = (DocumentId::new(), DocumentId::new());
        let dv = viewport(vec![
            DisplayNode::Element(doc_embed("e1", a, Rect::new(0.0, 0.0, 10.0, 10.0), 0.0)),
            DisplayNode::Element(doc_embed("e2", b, Rect::new(20.0, 0.0, 30.0, 10.0), 0.0)),
        ]);
        let out = visible_embeds(&dv, &cam, Some(&ElementId("e2".into())));
        assert_eq!(out.len(), 2);
        assert!(!out[0].focused);
        assert!(out[1].focused);
        // The published id is the CHILD's, not the board element's: it is the key
        // into FrameHost's child views and the id the child's commands carry.
        assert_eq!(out[1].document_id, b);
    }

    #[test]
    fn a_board_with_no_embeds_publishes_nothing() {
        let cam = ViewportTransform::new(1000.0, 800.0);
        assert!(visible_embeds(&viewport(Vec::new()), &cam, None).is_empty());
    }

    // -- Focus hit-test --------------------------------------------------------

    #[test]
    fn embed_at_picks_the_topmost_and_skips_locked() {
        let (a, b) = (DocumentId::new(), DocumentId::new());
        let over = Rect::new(0.0, 0.0, 50.0, 50.0);
        let mut locked = doc_embed("e3", DocumentId::new(), over, 0.0);
        locked.locked = true;

        // Two overlapping embeds: the later node is on top (`derive.rs` walks
        // z-order back to front).
        let dv = viewport(vec![
            DisplayNode::Element(doc_embed("e1", a, over, 0.0)),
            DisplayNode::Element(doc_embed("e2", b, over, 0.0)),
        ]);
        assert_eq!(
            embed_at(&dv, Point::new(25.0, 25.0)).map(|e| e.id.clone()),
            Some(ElementId("e2".into()))
        );
        assert!(embed_at(&dv, Point::new(500.0, 500.0)).is_none());

        // A locked embed is not interactive, so it is not a focus target either
        // -- and it must not shadow the live one underneath it.
        let dv = viewport(vec![
            DisplayNode::Element(doc_embed("e1", a, over, 0.0)),
            DisplayNode::Element(locked),
        ]);
        assert_eq!(
            embed_at(&dv, Point::new(25.0, 25.0)).map(|e| e.id.clone()),
            Some(ElementId("e1".into()))
        );
    }

    // -- Focus: the click-vs-drag arbitration ---------------------------------

    fn eid(s: &str) -> ElementId {
        ElementId(s.to_owned())
    }

    #[test]
    fn a_click_on_an_embed_focuses_it() {
        let mut f = EmbedFocus::default();
        let at = Point::new(100.0, 100.0);
        f.on_press(Some(&eid("e1")), at);
        // Released within the slop -> a click -> focus, and focus changed.
        assert!(f.on_release(at + Vec2::new(CLICK_SLOP - 0.5, 0.0)));
        assert_eq!(f.current(), Some(&eid("e1")));
    }

    #[test]
    fn a_drag_from_an_embed_does_not_focus_it() {
        let mut f = EmbedFocus::default();
        let at = Point::new(100.0, 100.0);
        f.on_press(Some(&eid("e1")), at);
        // Past the slop -> a drag that moved the embed on the board -> no focus.
        assert!(!f.on_release(at + Vec2::new(CLICK_SLOP + 0.5, 0.0)));
        assert_eq!(f.current(), None);
    }

    #[test]
    fn a_click_outside_returns_focus_to_the_board() {
        let mut f = EmbedFocus::default();
        f.on_press(Some(&eid("e1")), Point::ZERO);
        f.on_release(Point::ZERO);
        assert_eq!(f.current(), Some(&eid("e1")));

        // A press on empty canvas arms nothing; the release blurs.
        f.on_press(None, Point::new(500.0, 500.0));
        assert!(
            f.on_release(Point::new(500.0, 500.0)),
            "blur should change focus"
        );
        assert_eq!(f.current(), None);
    }

    #[test]
    fn a_click_on_the_already_focused_embed_is_not_rearmed() {
        let mut f = EmbedFocus::default();
        f.on_press(Some(&eid("e1")), Point::ZERO);
        f.on_release(Point::ZERO);
        // Clicking the focused embed again must not blur it: the press is not
        // armed (it is already focused), so the release is a no-op, not a blur.
        // Otherwise the second click of a double-click would kick focus out.
        f.on_press(Some(&eid("e1")), Point::ZERO);
        assert!(
            !f.on_release(Point::ZERO),
            "focus flickered on a repeat click"
        );
        assert_eq!(f.current(), Some(&eid("e1")));
    }

    #[test]
    fn a_drag_leaves_an_existing_focus_untouched() {
        let mut f = EmbedFocus::default();
        f.on_press(Some(&eid("e1")), Point::ZERO);
        f.on_release(Point::ZERO); // e1 focused
        // A drag starting on a DIFFERENT embed moves it; focus stays on e1
        // rather than following the gesture.
        let at = Point::new(300.0, 300.0);
        f.on_press(Some(&eid("e2")), at);
        assert!(!f.on_release(at + Vec2::new(50.0, 0.0)));
        assert_eq!(f.current(), Some(&eid("e1")));
    }

    #[test]
    fn escape_blurs_only_when_something_is_focused() {
        let mut f = EmbedFocus::default();
        assert!(!f.blur(), "blur on nothing must report no change");
        f.on_press(Some(&eid("e1")), Point::ZERO);
        f.on_release(Point::ZERO);
        assert!(f.blur());
        assert_eq!(f.current(), None);
    }

    #[test]
    fn focus_is_dropped_when_its_element_leaves_the_frame() {
        let mut f = EmbedFocus::default();
        f.on_press(Some(&eid("e1")), Point::ZERO);
        f.on_release(Point::ZERO);
        // Still present: retained.
        assert!(!f.retain_if(|id| id == &eid("e1")));
        assert_eq!(f.current(), Some(&eid("e1")));
        // Gone (deleted by a peer, or scrolled out and closed): dropped.
        assert!(f.retain_if(|id| id != &eid("e1")));
        assert_eq!(f.current(), None);
        // Idempotent once empty.
        assert!(!f.retain_if(|_| false));
    }

    // -- Session lifecycle + debounce -----------------------------------------

    fn opened(cmds: &[LogicCommand]) -> Vec<DocumentId> {
        cmds.iter()
            .filter_map(|c| match c {
                LogicCommand::OpenDocument { document_id, .. } => Some(*document_id),
                _ => None,
            })
            .collect()
    }

    fn closed(cmds: &[LogicCommand]) -> Vec<DocumentId> {
        cmds.iter()
            .filter_map(|c| match c {
                LogicCommand::CloseDocument { document_id } => Some(*document_id),
                _ => None,
            })
            .collect()
    }

    fn board_with(child: DocumentId) -> DisplayViewport {
        viewport(vec![DisplayNode::Element(doc_embed(
            "e1",
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            0.0,
        ))])
    }

    #[test]
    fn a_visible_embed_opens_once_and_only_once() {
        let mut s = EmbedSessions::default();
        let child = DocumentId::new();
        let dv = board_with(child);

        assert_eq!(opened(&s.sync(&dv, 0.0)), vec![child]);
        assert!(s.is_open(&child));
        // Still visible on later frames: no second open. A re-open costs a
        // members fetch, a key unwrap and a snapshot hydration.
        assert!(s.sync(&dv, 16.0).is_empty());
        assert!(s.sync(&dv, 32.0).is_empty());
    }

    #[test]
    fn close_waits_the_full_debounce() {
        let mut s = EmbedSessions::default();
        let child = DocumentId::new();
        let empty = viewport(Vec::new());
        s.sync(&board_with(child), 0.0);

        // Leaves the cull set: the countdown starts, nothing is sent yet.
        assert!(s.sync(&empty, 1_000.0).is_empty());
        // One millisecond short of the deadline: still nothing.
        let almost = 1_000.0 + EMBED_CLOSE_DEBOUNCE_MS - 1.0;
        assert!(s.sync(&empty, almost).is_empty());
        assert!(s.is_open(&child), "closed before the debounce elapsed");
        // Deadline reached.
        assert_eq!(
            closed(&s.sync(&empty, 1_000.0 + EMBED_CLOSE_DEBOUNCE_MS)),
            vec![child]
        );
        assert!(!s.is_open(&child));
    }

    #[test]
    fn a_scroll_oscillation_never_thrashes_the_session() {
        let mut s = EmbedSessions::default();
        let child = DocumentId::new();
        let dv = board_with(child);
        let empty = viewport(Vec::new());
        assert_eq!(opened(&s.sync(&dv, 0.0)), vec![child]);

        // Pan the embed in and out well within the debounce, many times over --
        // the failure mode this exists for. Binding the session straight to the
        // cull set would storm the WebSocket here.
        for i in 1..50 {
            let t = i as f64 * 100.0;
            assert!(s.sync(&empty, t).is_empty(), "close storm at {t}");
            assert!(s.sync(&dv, t + 50.0).is_empty(), "open storm at {t}");
        }
        assert!(s.is_open(&child));
    }

    #[test]
    fn returning_before_the_deadline_cancels_the_close() {
        let mut s = EmbedSessions::default();
        let child = DocumentId::new();
        let dv = board_with(child);
        let empty = viewport(Vec::new());
        s.sync(&dv, 0.0);

        s.sync(&empty, 0.0); // countdown armed for t = 5000
        s.sync(&dv, 4_000.0); // back in view: cancelled
        // Past the ORIGINAL deadline with the embed visible: nothing closes.
        assert!(s.sync(&dv, 6_000.0).is_empty());
        assert!(s.is_open(&child));
        // And the next departure gets a fresh full window, not the old deadline.
        assert!(s.sync(&empty, 6_000.0).is_empty());
        assert!(s.sync(&empty, 10_000.0).is_empty());
        assert_eq!(
            closed(&s.sync(&empty, 6_000.0 + EMBED_CLOSE_DEBOUNCE_MS)),
            vec![child]
        );
    }

    #[test]
    fn close_all_takes_every_child_immediately() {
        let mut s = EmbedSessions::default();
        let (a, b) = (DocumentId::new(), DocumentId::new());
        let dv = viewport(vec![
            DisplayNode::Element(doc_embed("e1", a, Rect::new(0.0, 0.0, 10.0, 10.0), 0.0)),
            DisplayNode::Element(doc_embed("e2", b, Rect::new(20.0, 0.0, 30.0, 10.0), 0.0)),
        ]);
        s.sync(&dv, 0.0);

        // The board is going away: no debounce, and the registry is emptied so a
        // later frame cannot close them twice.
        let got: std::collections::HashSet<DocumentId> =
            closed(&s.close_all()).into_iter().collect();
        assert_eq!(got, std::collections::HashSet::from([a, b]));
        assert!(s.close_all().is_empty());
        assert!(!s.is_open(&a) && !s.is_open(&b));
    }

    #[test]
    fn two_embeds_of_the_same_document_share_one_session() {
        let mut s = EmbedSessions::default();
        let child = DocumentId::new();
        // The same document placed twice on one board: sessions are keyed by
        // document, not by element, so this must open exactly one.
        let dv = viewport(vec![
            DisplayNode::Element(doc_embed("e1", child, Rect::new(0.0, 0.0, 10.0, 10.0), 0.0)),
            DisplayNode::Element(doc_embed(
                "e2",
                child,
                Rect::new(20.0, 0.0, 30.0, 10.0),
                0.0,
            )),
        ]);
        assert_eq!(opened(&s.sync(&dv, 0.0)), vec![child]);
        assert!(s.sync(&dv, 16.0).is_empty());
    }
}
