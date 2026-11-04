// ambre-toile/src/ui/embed.rs
// ============================================================================
// Nested Ambre documents in the view layer (`toile-embeds`).
//
// A `doc_embed` element is a rectangle on the board whose content is another
// Ambre document with its own `Id<Document>`, `DocumentKey` and `LoroDoc`. This
// module holds the two things `ToileView` needs and nothing else:
//
//   - `visible_embeds`: the culled `doc_embed`s projected to screen space, which
//     the view PUBLISHES for `FrameHost`. `ambre-toile` cannot paint a child,
//     nor route input into one. Both need `AppView`, the closed enum that lives
//     in the launcher, downstream of every app crate (architecture / ARCH P2).
//     So the composition inverts -- the view describes its embeds, the launcher
//     acts on them.
//   - `to_child_space`: the pointer transform. Screen -> world -> the element's
//     unrotated local frame -> child pixels.
//
// Focus itself lives on `ToileView` (view-local state, never CRDT, never derived
// -- `derive.rs` always emits `focused: false`; a peer's focus is not ours).
//
// Pure geometry -- host-tested, no `cfg(wasm32)`, no GPU.
// ============================================================================

use std::collections::HashMap;

use ambre_bus::app::toile::{
    DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, ElementKind, InitContent,
    InputFocus, ToileCommand, WebAddress,
};
use ambre_bus::{DocType, LogicCommand};
use ambre_core::id::{Document, Element, Id};
use ambre_draw::{Affine, Point, Rect, Size};

use ambre_canvas::InputEvent;
use ambre_commands::Key;

use super::viewport::ViewportTransform;
use ambre_core::{FracMs, Instant, PageLoad, TimeSpan, Validated};

/// How long a `doc_embed` must stay out of the culled set before its child
/// document's sync session closes ("Opening the child").
///
/// Visibility comes from the Logic R-tree cull set, which flips on every pan.
/// Binding the session lifecycle straight to it would thrash the WebSocket. A
/// scroll oscillation would storm open and close, and each re-open costs a
/// members fetch, a key unwrap and a snapshot hydration. So an embed opens the
/// instant it enters the cull set, and only closes after staying out for this
/// long. Re-entering cancels the pending close.
pub const EMBED_CLOSE_DEBOUNCE: TimeSpan<FracMs> = TimeSpan::<FracMs>::new(5_000.0);

/// One `doc_embed` in the current culled frame, projected to screen space and
/// published for `FrameHost` (`toile-embeds` "Render").
#[derive(Clone, Debug, PartialEq)]
pub struct VisibleEmbed {
    /// The board element carrying the reference (the focus / hit-test key).
    pub element: Id<Element>,
    /// The embedded document itself -- the key into `FrameHost`'s child views
    /// and the id the child's own commands carry.
    pub document_id: Id<Document>,
    pub doc_type: DocType,
    /// Where to paint the child, in screen pixels. The axis-aligned
    /// bounding box of the element's (possibly rotated) quad. `push_clip` is
    /// rect-only, so a rotated embed clips to its AABB -- accepted, see
    /// "Clip limitation".
    pub screen_rect: Rect,
    /// The child's own canvas size in logical pixels. A child pixel is a
    /// screen pixel by construction, since the camera's zoom scales both from
    /// world units. So this is the element's world size at the current zoom.
    pub child_size: Size,
    /// Child pixels -> screen pixels: what a painter pushes before
    /// drawing the child, so a rotated embed paints its child rotated
    /// (`TOILE-UI-096`). The exact inverse of [`to_child_space`], which is what
    /// makes the paint and the pointer agree. Translating to `screen_rect`'s
    /// corner instead puts the child straight, in the wrong place, while the
    /// pointer arrives as though the element had turned it.
    pub to_screen: Affine,
    /// View-local focus ("Input focus"): while `Entered`, `FrameHost` maps canvas
    /// input into this child and hands it to the child's own `handle_input`.
    pub focused: InputFocus,
}

/// Map a screen point into a `doc_embed`'s child pixel space.
///
/// * `screen -> world` undoes the camera.
/// * `world -> local` undoes the element's rotation. An element stores geometry
///   in the unrotated frame bounded by `bounds`, and `transform` is the
///   rotation about that AABB's center (see `hit.rs`).
/// * The child's origin is then `bounds.origin`, and its scale is the camera's
///   zoom.
///
/// Deltas need no equivalent: a child pixel *is* a screen pixel, since the same
/// `zoom` scales both. A wheel delta forwards verbatim.
pub fn to_child_space(elem: &DisplayElement, cam: &ViewportTransform, screen: Point) -> Point {
    let world = cam.screen_to_world() * screen;
    let local = elem.transform.inverse() * world;
    Point::new(
        (local.x - elem.bounds.x0) * cam.zoom.get(),
        (local.y - elem.bounds.y0) * cam.zoom.get(),
    )
}

/// Map the child's own pixel space onto the screen: the inverse of
/// [`to_child_space`] (`TOILE-UI-096`).
///
/// Read right to left:
///
/// * The zoom divides a child pixel into a world offset.
/// * The translation moves it to the element's unrotated origin.
/// * The element's own rotation turns it.
/// * The camera carries it to the screen.
pub fn to_screen(elem: &DisplayElement, cam: &ViewportTransform) -> Affine {
    cam.world_to_screen()
        * elem.transform
        * Affine::translate((elem.bounds.x0, elem.bounds.y0))
        * Affine::scale(1.0 / cam.zoom.get())
}

/// The child's canvas size in logical pixels at the current zoom.
pub fn child_size(elem: &DisplayElement, cam: &ViewportTransform) -> Size {
    Size::new(
        elem.bounds.width() * cam.zoom.get(),
        elem.bounds.height() * cam.zoom.get(),
    )
}

/// The screen-space AABB of an element's (possibly rotated) quad. This maps all
/// four corners, so a rotated embed yields the box that contains it -- unlike a
/// two-corner projection, which collapses under rotation.
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
    focused: Option<&Id<Element>>,
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
                to_screen: to_screen(e, cam),
                focused: InputFocus::from_bool(focused == Some(&e.id)),
            })
        })
        .collect()
}

// -- Placing an embed ---------------------------------------------------------
//
// The gesture that puts an embed on the board (`TOILE-086`, `TOILE-087`). The
// shell owns both dialogs: the user types a web address, and the document list
// is the launcher's. So what crosses into the view is the answer, not the
// gesture, exactly as a dropped file does (`ui::interop::DroppedFile`).

/// The default world size of a freshly placed embed. A web page and a nested
/// document alike land at a readable width, rather than at a point the user
/// then has to drag open. The ratio is a browser window's, not a slide's.
pub const DEFAULT_EMBED_SIZE: Size = Size::new(640.0, 400.0);

/// What a placement dialog answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbedTarget {
    /// A web page, by an address already through [`WebAddress`].
    Web(Validated<WebAddress>),
    /// Another Ambre document, by reference (`TOILE-021`): the board stores the
    /// id and never the content.
    Document {
        document_id: Id<Document>,
        doc_type: DocType,
    },
}

/// An embed the shell resolved and the view has yet to place (`TOILE-086`,
/// `TOILE-087`). `at` is the drop point in logical pixels, or `None` when a
/// menu entry started the placement and there is no point to speak of. The
/// view then centres it on what the user is looking at, like a picker import.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedEmbed {
    pub target: EmbedTarget,
    pub at: Option<Point>,
}

/// The one write that puts `placed` on the board seen through `cam`
/// (`TOILE-086`, `TOILE-087`). The point the user aimed at is the embed's
/// centre, not its corner. With no drop point, a menu entry started it. It
/// then lands in the middle of what the viewer sees, like a picker import.
pub fn placement(placed: PlacedEmbed, cam: &ViewportTransform) -> ToileCommand {
    let screen = placed
        .at
        .unwrap_or(Point::new(cam.size.width / 2.0, cam.size.height / 2.0));
    let size = DEFAULT_EMBED_SIZE;
    let centre = cam.screen_to_world() * screen;
    let at = Point::new(centre.x - size.width / 2.0, centre.y - size.height / 2.0);
    let (kind, content) = match placed.target {
        EmbedTarget::Web(url) => (ElementKind::Embed, InitContent::Embed(url)),
        EmbedTarget::Document {
            document_id,
            doc_type,
        } => (
            ElementKind::DocEmbed,
            InitContent::DocEmbed {
                document_id,
                doc_type,
            },
        ),
    };
    ToileCommand::CreateElement {
        kind,
        at,
        size,
        content,
        fill: None,
        correlate: None,
    }
}

/// Whether an input event belongs to the focused embed's child rather than to
/// the board ("Input focus").
///
/// `inside` answers whether a screen point lands in the focused embed's quad,
/// and `last_pointer` is where the cursor was last seen. Splitting this out of
/// the view keeps the routing rule host-tested, since `ToileView` is not
/// natively constructible. Two rules matter here:
///
/// * `Escape` is never captured (`TOILE-UI-331`): the board needs it to blur,
///   and a child that swallowed it would trap the user inside the embed;
/// * a wheel is bound by the pointer, not by focus alone (`TOILE-UI-329`). It
///   carries no position of its own, so capturing it unconditionally made the
///   board impossible to pan or zoom while any embed held focus.
pub fn routes_to_child<F>(ev: &InputEvent, last_pointer: Point, inside: F) -> bool
where
    F: Fn(Point) -> bool,
{
    match ev {
        InputEvent::Key { key, .. } => *key != Key::Escape,
        // A release follows its press: the child that took the key takes it.
        InputEvent::KeyUp { .. } => true,
        // Typing has no position at all: it follows focus, as a caret does.
        InputEvent::Text { .. }
        | InputEvent::CompositionUpdate { .. }
        | InputEvent::CompositionEnd { .. } => true,
        InputEvent::Wheel { .. } => inside(last_pointer),
        InputEvent::PointerDown { at, .. }
        | InputEvent::PointerUp { at, .. }
        | InputEvent::PointerMove { at }
        | InputEvent::DoubleClick { at } => {
            inside(Point::new(f64::from(at.x.get()), f64::from(at.y.get())))
        }
    }
}

/// One web `embed` in the current culled frame, projected to screen space for
/// the shell to place its frame over (`TOILE-088`). The board itself paints only
/// the placeholder box: a web page is DOM, and the canvas has no way to hold one.
#[derive(Clone, Debug, PartialEq)]
pub struct VisibleWebEmbed {
    /// The board element carrying the address: the shell's key for its frame.
    pub element: Id<Element>,
    /// The address, as stored. A peer may have written it, so the board read
    /// it back through [`WebAddress`] (`TOILE-267`).
    pub url: Validated<WebAddress>,
    /// Where the frame goes, in screen pixels: the AABB of the
    /// element's quad, like a nested document's. What the board clips to.
    pub screen_rect: Rect,
    /// The frame's own layout size, in world units -- the element's size, and
    /// **not** a function of the zoom.
    ///
    /// The browser lays a page out at this size once. The transform below then
    /// scales it, so zooming the board moves a frame rather than reflowing the
    /// page inside it. Sizing the frame in screen pixels instead would re-run the
    /// page's layout on every wheel notch, at a cost the board does not control.
    pub layout_size: Size,
    /// Layout units -> screen pixels: where the frame sits, turned by
    /// the element's own rotation. Screen pixels are CSS pixels (`CANVAS-032`).
    pub to_screen: Affine,
    /// View-local focus, on the same arbitration as a nested document's: a click
    /// enters, a drag moves. Until the user enters it, the frame must let the
    /// pointer through to the board, or no one could move or resize the embed.
    pub focused: InputFocus,
}

/// The web `embed`s of `display`, projected for the shell. Order follows the
/// display walk, which is z-order, so frames stacked in sequence reproduce the
/// board's own stacking.
pub fn visible_web_embeds(
    display: &DisplayViewport,
    cam: &ViewportTransform,
    focused: Option<&Id<Element>>,
) -> Vec<VisibleWebEmbed> {
    display
        .nodes
        .iter()
        .filter_map(|node| {
            let DisplayNode::Element(e) = node else {
                return None;
            };
            // An address the check refused derives as `None`: the placeholder
            // paints, and no frame loads anything.
            let DisplayGeom::Embed(Some(url)) = &e.geom else {
                return None;
            };
            Some(VisibleWebEmbed {
                element: e.id.clone(),
                url: url.clone(),
                screen_rect: screen_aabb(e, cam),
                layout_size: Size::new(e.bounds.width(), e.bounds.height()),
                to_screen: cam.world_to_screen()
                    * e.transform
                    * Affine::translate((e.bounds.x0, e.bounds.y0)),
                focused: InputFocus::from_bool(focused == Some(&e.id)),
            })
        })
        .collect()
}

/// How far the pointer may travel between press and release and still count
/// as a click. A click focuses a `doc_embed`, and a longer travel is a drag
/// that moves it ("Input focus"). The view reads the same slop for
/// click-to-edit (`toile-text-editing`).
pub const CLICK_SLOP: ambre_core::Px<ambre_core::View> =
    ambre_core::Px::<ambre_core::View>::new(4.0);

/// Which `doc_embed`, if any, holds input focus, and the press-to-release
/// arbitration that decides it ("Input focus"). View-local state, never
/// CRDT: a peer's focus is not ours, and `derive.rs` always emits
/// `focused: false`.
///
/// Pure: the view feeds it the press target, the release point and whatever
/// `Escape` and culling do. It answers only "did focus change", so the view can
/// mark itself dirty. Split out of `ToileView` because that type is not natively
/// constructible, and the click-vs-drag rule is the behavioural core of 3.c.
#[derive(Default)]
pub struct EmbedFocus {
    /// The focused embed's board element, if any.
    focused: Option<Id<Element>>,
    /// The embed under the last press (`None` = empty canvas / non-embed) and
    /// where it landed. The release settles focus, and only if the pointer
    /// barely moved. A drag that starts on an embed must still move it on the
    /// board, so entering a document is a distinct gesture from repositioning.
    ///
    /// The press records the hit even when it names the already-focused embed.
    /// A release there does nothing rather than blurring: a repeat click, for
    /// example the second of a double-click, must not kick focus out.
    press: Option<(Option<Id<Element>>, Point)>,
}

impl EmbedFocus {
    /// The focused element, for publishing and for the child input route.
    pub fn current(&self) -> Option<&Id<Element>> {
        self.focused.as_ref()
    }

    /// A press landed at `screen` over the embed `hit` (`None` = empty canvas or a
    /// non-embed element). Records it for the release to settle; consumes
    /// nothing, so the board's own gesture proceeds regardless.
    pub fn on_press(&mut self, hit: Option<&Id<Element>>, screen: Point) {
        self.press = Some((hit.cloned(), screen));
    }

    /// A release landed at `screen`. If the pointer stayed within [`CLICK_SLOP`]
    /// of the press it is a click. On an embed that click focuses the embed,
    /// and on empty canvas it returns focus to the board. Past the slop it is a
    /// drag (a move / resize / marquee) and focus stays put. Returns whether
    /// focus changed.
    pub fn on_release(&mut self, screen: Point) -> bool {
        let Some((hit, from)) = self.press.take() else {
            return false;
        };
        // A drag moved something on the board; the focus rides along with none of
        // it. Only a click settles focus either way.
        if (screen - from).hypot() > f64::from(CLICK_SLOP.get()) {
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
    /// focus sat on an embed.
    pub fn blur(&mut self) -> bool {
        self.press = None;
        self.focused.take().is_some()
    }

    /// Drop focus if `still_present` reports the focused element gone from the
    /// culled frame. A peer deleted it, or it scrolled far enough for the cull
    /// to close it, and nothing remains to route input into. Returns whether
    /// focus changed.
    pub fn retain_if<F>(&mut self, still_present: F) -> bool
    where
        F: Fn(&Id<Element>) -> bool,
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
/// Pure: the view tells it what is visible and what time it is, and it answers
/// with the session commands to send. `ToileView` owns one and forwards its
/// output.
#[derive(Default)]
pub struct EmbedSessions {
    /// Open children. `Some(deadline)` = out of the cull set, closing at that
    /// frame-clock reading; `None` = visible, nothing pending.
    open: HashMap<Id<Document>, Option<Instant<PageLoad>>>,
}

impl EmbedSessions {
    /// Reconcile against the culled frame at `now`, handing each session
    /// command to `send` and recording it only once `send` reports it left
    /// (`TOILE-UI-338`, `TOILE-UI-339`).
    ///
    /// The order is the whole point. Writing the open set first and sending
    /// afterwards loses a command for good on a full queue. The set already
    /// marks the work done, so no later frame retries it.
    ///
    /// * A dropped open leaves a blank placeholder until the user leaves the
    ///   board.
    /// * A dropped close leaks a child session nothing names any more.
    ///
    /// Recording on the answer instead makes both self-healing. The embed is
    /// still in the cull set next frame, so that frame offers the open again.
    /// The child is still in the open set, so the frame after its deadline
    /// offers the close again.
    ///
    /// `send` returns whether the command left. A caller that cannot send (a
    /// full queue, a disconnected one) returns `false` and the state stays as
    /// it was.
    pub fn sync<F>(&mut self, display: &DisplayViewport, now: Instant<PageLoad>, mut send: F)
    where
        F: FnMut(LogicCommand) -> bool,
    {
        for (document_id, doc_type) in embedded_docs(display).collect::<Vec<_>>() {
            match self.open.get_mut(&document_id) {
                // Back in view: cancel any pending close. Local, nothing to send.
                Some(pending) => *pending = None,
                None => {
                    if send(LogicCommand::OpenDocument {
                        document_id,
                        doc_type,
                    }) {
                        self.open.insert(document_id, None);
                    }
                }
            }
        }

        let gone: Vec<Id<Document>> = self
            .open
            .keys()
            .filter(|id| !references(display, id))
            .copied()
            .collect();
        for document_id in gone {
            // `gone` was read from `open`'s own keys, so the lookup always hits.
            let Some(&countdown) = self.open.get(&document_id) else {
                continue;
            };
            match countdown {
                // Deadline reached: close for real, if the queue takes it.
                Some(deadline) if now >= deadline => {
                    if send(LogicCommand::CloseDocument { document_id }) {
                        self.open.remove(&document_id);
                    }
                }
                // Counting down.
                Some(_) => {}
                // Just left the cull set: start the countdown.
                None => {
                    self.open
                        .insert(document_id, Some(now.after(EMBED_CLOSE_DEBOUNCE)));
                }
            }
        }
    }

    /// Close every child at once, with no debounce: the board itself is going
    /// away (a route change), so its embeds go with it. A child whose close
    /// `send` refuses stays in the set, so a caller that retries can still
    /// reclaim it (`TOILE-UI-339`).
    pub fn close_all<F>(&mut self, mut send: F)
    where
        F: FnMut(LogicCommand) -> bool,
    {
        self.open.retain(|document_id, _| {
            !send(LogicCommand::CloseDocument {
                document_id: *document_id,
            })
        });
    }

    /// Whether `document_id` currently has a session (open or closing).
    pub fn is_open(&self, document_id: &Id<Document>) -> bool {
        self.open.contains_key(document_id)
    }
}

/// Every nested document referenced by a `doc_embed` in the culled frame.
fn embedded_docs(display: &DisplayViewport) -> impl Iterator<Item = (Id<Document>, DocType)> + '_ {
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
fn references(display: &DisplayViewport, document_id: &Id<Document>) -> bool {
    embedded_docs(display).any(|(id, _)| &id == document_id)
}

/// The embed under a world point, if any: the topmost one whose bounds contain
/// it. Used to resolve a click to a focus target ("Input focus"). Mirrors
/// `hit::exact_hit`'s rule for this geometry (bounds in the local frame),
/// including the `locked` skip -- a locked embed is not interactive.
///
/// Both kinds, because the gesture is the same for both: a click enters, a drag
/// moves. What entering means differs: a nested document takes canvas events
/// through the board, while a web page takes DOM events directly from its
/// frame. Code downstream of this settles that difference.
pub fn embed_at(display: &DisplayViewport, world: Point) -> Option<&DisplayElement> {
    display
        .nodes
        .iter()
        .rev()
        .filter_map(|node| match node {
            DisplayNode::Element(e)
                if matches!(e.geom, DisplayGeom::DocEmbed { .. } | DisplayGeom::Embed(_)) =>
            {
                Some(e)
            }
            _ => None,
        })
        .find(|e| {
            e.locked == ambre_bus::app::toile::LockState::Unlocked
                && e.bounds.contains(e.transform.inverse() * world)
        })
}

// -- Tests (host tier) -------------------------------------------------------
// Pure geometry + the session debounce. No GPU, no DOM: `ToileView` itself is
// not natively constructible, since it wants a `TextInput` and leptos signals.
// That is exactly why the decidable parts live here rather than inside it.
#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{ElementKind, ElementStyle, Viewport};
    use ambre_core::{Board, Px};
    use ambre_core::{Instant, PageLoad};
    use ambre_draw::{Affine, Vec2};

    fn approx(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6
    }

    pub(super) fn doc_embed(
        id: &str,
        document_id: Id<Document>,
        bounds: Rect,
        rot: f64,
    ) -> DisplayElement {
        DisplayElement {
            id: Id::<Element>::from_text(id.to_owned()),
            kind: ElementKind::DocEmbed,
            // Rotation about the bounds' center, matching `derive.rs`.
            transform: Affine::translate(bounds.center().to_vec2())
                * Affine::rotate(rot)
                * Affine::translate(-bounds.center().to_vec2()),
            bounds,
            locked: ambre_bus::app::toile::LockState::Unlocked,
            style: ElementStyle::default(),
            geom: DisplayGeom::DocEmbed {
                document_id,
                doc_type: DocType::Trame,
                // Never true off the CRDT: focus is view-local.
                focused: ambre_bus::app::toile::InputFocus::Board,
            },
            link: None,
            group: None,
        }
    }

    pub(super) fn viewport(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            version: ambre_core::Seq::ZERO,
            grid_unit: Px::<Board>::new(20.0),
            pins: Vec::new(),
            viewport: Viewport::default(),
            nodes,
            reactions: Default::default(),
        }
    }

    // -- The pointer transform -------------------------------------------------

    #[test]
    fn child_origin_is_the_embeds_top_left() {
        let cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
        let e = doc_embed(
            "e1",
            Id::<Document>::mint(),
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
        let mut cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
        cam.zoom = ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(2.5);
        let e = doc_embed(
            "e1",
            Id::<Document>::mint(),
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
        let cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
        let bounds = Rect::new(0.0, 0.0, 100.0, 40.0);
        let e = doc_embed(
            "e1",
            Id::<Document>::mint(),
            bounds,
            std::f64::consts::FRAC_PI_2,
        );
        // The unrotated top-left, carried through the element's own rotation, is
        // still the child's origin. Naively subtracting `bounds.origin` in world
        // space would land somewhere else entirely.
        let world_tl = e.transform * Point::new(0.0, 0.0);
        let screen_tl = cam.world_to_screen() * world_tl;
        assert!(approx(to_child_space(&e, &cam, screen_tl), Point::ZERO));
    }

    #[test]
    fn child_size_scales_with_zoom() {
        let mut cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
        cam.zoom = ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(0.5);
        let e = doc_embed(
            "e1",
            Id::<Document>::mint(),
            Rect::new(0.0, 0.0, 200.0, 80.0),
            0.0,
        );
        assert_eq!(child_size(&e, &cam), Size::new(100.0, 40.0));
    }

    #[test]
    fn screen_aabb_contains_a_rotated_quad() {
        let cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
        // A 100x20 bar turned a quarter turn: its AABB must become 20x100. A
        // two-corner projection would report it as 100x20 still, and the clip
        // would take the whole child away.
        let e = doc_embed(
            "e1",
            Id::<Document>::mint(),
            Rect::new(0.0, 0.0, 100.0, 20.0),
            std::f64::consts::FRAC_PI_2,
        );
        let r = screen_aabb(&e, &cam);
        assert!((r.width() - 20.0).abs() < 1e-6, "width {}", r.width());
        assert!((r.height() - 100.0).abs() < 1e-6, "height {}", r.height());
    }

    // -- Publishing for FrameHost ---------------------------------------------

    /// `TOILE-UI-096`: what the painter pushes is the exact inverse of what the
    /// pointer undoes. Proving one half of the mapping while the other half
    /// contradicts it is the old bug. It put a rotated embed's child straight,
    /// in the wrong corner, while its pointer arrived turned.
    #[test]
    fn the_paint_transform_is_the_inverse_of_the_pointer_transform() {
        let bounds = Rect::new(40.0, 30.0, 240.0, 130.0);
        for rot in [0.0, 0.4, std::f64::consts::FRAC_PI_2, -1.1] {
            for zoom in [0.5, 1.0, 2.5] {
                let e = doc_embed("e", Id::<Document>::mint(), bounds, rot);
                let cam = ViewportTransform {
                    zoom: ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(zoom),
                    size: Size::new(800.0, 600.0),
                    ..Default::default()
                };
                let paint = to_screen(&e, &cam);
                // Round-trip a handful of child pixels: paint them onto the
                // screen, then map them back the way a pointer event travels.
                for child in [
                    Point::new(0.0, 0.0),
                    Point::new(10.0, 4.0),
                    Point::new(199.0, 99.0),
                ] {
                    let back = to_child_space(&e, &cam, paint * child);
                    assert!(
                        approx(back, child),
                        "rot={rot} zoom={zoom}: {child:?} came back as {back:?}"
                    );
                }
            }
        }
    }

    /// The child's origin lands where the element's own top-left is, not where
    /// its bounding box begins: the two differ the moment the embed turns.
    #[test]
    fn the_paint_origin_is_the_elements_corner_not_its_bounding_box() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        let e = doc_embed("e", Id::<Document>::mint(), bounds, 0.6);
        let cam = ViewportTransform {
            size: Size::new(800.0, 600.0),
            ..Default::default()
        };
        let origin = to_screen(&e, &cam) * Point::new(0.0, 0.0);
        let aabb = screen_aabb(&e, &cam);
        assert!(
            (origin.x - aabb.x0).abs() > 1.0 || (origin.y - aabb.y0).abs() > 1.0,
            "a turned embed's corner must not be its bounding box's corner"
        );
        // And it is the element's own top-left, taken through the camera.
        let expect = cam.world_to_screen() * e.transform * Point::new(bounds.x0, bounds.y0);
        assert!(approx(origin, expect));
    }

    #[test]
    fn only_the_focused_embed_comes_back_focused() {
        let cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
        let (a, b) = (Id::<Document>::mint(), Id::<Document>::mint());
        let dv = viewport(vec![
            DisplayNode::Element(doc_embed("e1", a, Rect::new(0.0, 0.0, 10.0, 10.0), 0.0)),
            DisplayNode::Element(doc_embed("e2", b, Rect::new(20.0, 0.0, 30.0, 10.0), 0.0)),
        ]);
        let out = visible_embeds(&dv, &cam, Some(&Id::<Element>::from_text("e2".into())));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].focused, ambre_bus::app::toile::InputFocus::Board);
        assert_eq!(out[1].focused, ambre_bus::app::toile::InputFocus::Entered);
        // The published id is the CHILD's, not the board element's: it is the key
        // into FrameHost's child views and the id the child's commands carry.
        assert_eq!(out[1].document_id, b);
    }

    #[test]
    fn a_board_with_no_embeds_publishes_nothing() {
        let cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
        assert!(visible_embeds(&viewport(Vec::new()), &cam, None).is_empty());
    }

    // -- Focus hit-test --------------------------------------------------------

    #[test]
    fn embed_at_picks_the_topmost_and_skips_locked() {
        let (a, b) = (Id::<Document>::mint(), Id::<Document>::mint());
        let over = Rect::new(0.0, 0.0, 50.0, 50.0);
        let mut locked = doc_embed("e3", Id::<Document>::mint(), over, 0.0);
        locked.locked = ambre_bus::app::toile::LockState::Locked;

        // Two overlapping embeds: the later node is on top (`derive.rs` walks
        // z-order back to front).
        let dv = viewport(vec![
            DisplayNode::Element(doc_embed("e1", a, over, 0.0)),
            DisplayNode::Element(doc_embed("e2", b, over, 0.0)),
        ]);
        assert_eq!(
            embed_at(&dv, Point::new(25.0, 25.0)).map(|e| e.id.clone()),
            Some(Id::<Element>::from_text("e2".into()))
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
            Some(Id::<Element>::from_text("e1".into()))
        );
    }

    // -- Focus: the click-vs-drag arbitration ---------------------------------

    fn eid(s: &str) -> Id<Element> {
        Id::<Element>::from_text(s.to_owned())
    }

    #[test]
    fn a_click_on_an_embed_focuses_it() {
        let mut f = EmbedFocus::default();
        let at = Point::new(100.0, 100.0);
        f.on_press(Some(&eid("e1")), at);
        // Released within the slop -> a click -> focus, and focus changed.
        assert!(f.on_release(at + Vec2::new(f64::from(CLICK_SLOP.get()) - 0.5, 0.0)));
        assert_eq!(f.current(), Some(&eid("e1")));
    }

    #[test]
    fn a_drag_from_an_embed_does_not_focus_it() {
        let mut f = EmbedFocus::default();
        let at = Point::new(100.0, 100.0);
        f.on_press(Some(&eid("e1")), at);
        // Past the slop -> a drag that moved the embed on the board -> no focus.
        assert!(!f.on_release(at + Vec2::new(f64::from(CLICK_SLOP.get()) + 0.5, 0.0)));
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
        // Clicking the focused embed again must not blur it. The press arms
        // nothing, since focus already sits there, so the release does nothing.
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

    /// Reconcile with a queue that takes everything, returning what left.
    fn sync(
        s: &mut EmbedSessions,
        dv: &DisplayViewport,
        now: Instant<PageLoad>,
    ) -> Vec<LogicCommand> {
        let mut sent = Vec::new();
        s.sync(dv, now, |cmd| {
            sent.push(cmd);
            true
        });
        sent
    }

    /// Reconcile with a queue that refuses everything, returning what the
    /// registry offered it. The registry MUST record nothing from it.
    fn sync_refused(
        s: &mut EmbedSessions,
        dv: &DisplayViewport,
        now: Instant<PageLoad>,
    ) -> Vec<LogicCommand> {
        let mut offered = Vec::new();
        s.sync(dv, now, |cmd| {
            offered.push(cmd);
            false
        });
        offered
    }

    /// Close every child through a queue that takes everything.
    fn close_all(s: &mut EmbedSessions) -> Vec<LogicCommand> {
        let mut sent = Vec::new();
        s.close_all(|cmd| {
            sent.push(cmd);
            true
        });
        sent
    }

    fn opened(cmds: &[LogicCommand]) -> Vec<Id<Document>> {
        cmds.iter()
            .filter_map(|c| match c {
                LogicCommand::OpenDocument { document_id, .. } => Some(*document_id),
                _ => None,
            })
            .collect()
    }

    fn closed(cmds: &[LogicCommand]) -> Vec<Id<Document>> {
        cmds.iter()
            .filter_map(|c| match c {
                LogicCommand::CloseDocument { document_id } => Some(*document_id),
                _ => None,
            })
            .collect()
    }

    fn board_with(child: Id<Document>) -> DisplayViewport {
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
        let child = Id::<Document>::mint();
        let dv = board_with(child);

        assert_eq!(
            opened(&sync(&mut s, &dv, Instant::<PageLoad>::new(0.0))),
            vec![child]
        );
        assert!(s.is_open(&child));
        // Still visible on later frames: no second open. A re-open costs a
        // members fetch, a key unwrap and a snapshot hydration.
        assert!(sync(&mut s, &dv, Instant::<PageLoad>::new(16.0)).is_empty());
        assert!(sync(&mut s, &dv, Instant::<PageLoad>::new(32.0)).is_empty());
    }

    #[test]
    fn close_waits_the_full_debounce() {
        let mut s = EmbedSessions::default();
        let child = Id::<Document>::mint();
        let empty = viewport(Vec::new());
        sync(&mut s, &board_with(child), Instant::<PageLoad>::new(0.0));

        // Leaves the cull set: the countdown starts, and nothing leaves yet.
        assert!(sync(&mut s, &empty, Instant::<PageLoad>::new(1_000.0)).is_empty());
        // One millisecond short of the deadline: still nothing.
        let almost = 1_000.0 + EMBED_CLOSE_DEBOUNCE.get() - 1.0;
        assert!(sync(&mut s, &empty, Instant::<PageLoad>::new(almost)).is_empty());
        assert!(s.is_open(&child), "closed before the debounce elapsed");
        // Deadline reached.
        assert_eq!(
            closed(&sync(
                &mut s,
                &empty,
                Instant::<PageLoad>::new(1_000.0 + EMBED_CLOSE_DEBOUNCE.get())
            )),
            vec![child]
        );
        assert!(!s.is_open(&child));
    }

    #[test]
    fn a_scroll_oscillation_never_thrashes_the_session() {
        let mut s = EmbedSessions::default();
        let child = Id::<Document>::mint();
        let dv = board_with(child);
        let empty = viewport(Vec::new());
        assert_eq!(
            opened(&sync(&mut s, &dv, Instant::<PageLoad>::new(0.0))),
            vec![child]
        );

        // Pan the embed in and out well within the debounce, many times over --
        // the failure mode this exists for. Binding the session straight to the
        // cull set would storm the WebSocket here.
        for i in 1..50 {
            let t = i as f64 * 100.0;
            assert!(
                sync(&mut s, &empty, Instant::<PageLoad>::new(t)).is_empty(),
                "close storm at {t}"
            );
            assert!(
                sync(&mut s, &dv, Instant::<PageLoad>::new(t + 50.0)).is_empty(),
                "open storm at {t}"
            );
        }
        assert!(s.is_open(&child));
    }

    #[test]
    fn returning_before_the_deadline_cancels_the_close() {
        let mut s = EmbedSessions::default();
        let child = Id::<Document>::mint();
        let dv = board_with(child);
        let empty = viewport(Vec::new());
        sync(&mut s, &dv, Instant::<PageLoad>::new(0.0));

        sync(&mut s, &empty, Instant::<PageLoad>::new(0.0)); // countdown armed for t = 5000
        sync(&mut s, &dv, Instant::<PageLoad>::new(4_000.0)); // back in view: cancelled
        // Past the ORIGINAL deadline with the embed visible: nothing closes.
        assert!(sync(&mut s, &dv, Instant::<PageLoad>::new(6_000.0)).is_empty());
        assert!(s.is_open(&child));
        // And the next departure gets a fresh full window, not the old deadline.
        assert!(sync(&mut s, &empty, Instant::<PageLoad>::new(6_000.0)).is_empty());
        assert!(sync(&mut s, &empty, Instant::<PageLoad>::new(10_000.0)).is_empty());
        assert_eq!(
            closed(&sync(
                &mut s,
                &empty,
                Instant::<PageLoad>::new(6_000.0 + EMBED_CLOSE_DEBOUNCE.get())
            )),
            vec![child]
        );
    }

    #[test]
    fn close_all_takes_every_child_immediately() {
        let mut s = EmbedSessions::default();
        let (a, b) = (Id::<Document>::mint(), Id::<Document>::mint());
        let dv = viewport(vec![
            DisplayNode::Element(doc_embed("e1", a, Rect::new(0.0, 0.0, 10.0, 10.0), 0.0)),
            DisplayNode::Element(doc_embed("e2", b, Rect::new(20.0, 0.0, 30.0, 10.0), 0.0)),
        ]);
        sync(&mut s, &dv, Instant::<PageLoad>::new(0.0));

        // The board is going away: no debounce, and the sweep empties the
        // registry so a later frame cannot close them twice.
        let got: std::collections::HashSet<Id<Document>> =
            closed(&close_all(&mut s)).into_iter().collect();
        assert_eq!(got, std::collections::HashSet::from([a, b]));
        assert!(close_all(&mut s).is_empty());
        assert!(!s.is_open(&a) && !s.is_open(&b));
    }

    /// `TOILE-UI-338`: when the queue refuses an open, the next frame offers
    /// it again. Recording it before it left used to lose it for good: the set
    /// marked the work done, so no frame retried. The embed then stayed a blank
    /// placeholder until the user left the board.
    #[test]
    fn an_open_the_channel_refuses_comes_back_next_frame() {
        let mut s = EmbedSessions::default();
        let child = Id::<Document>::mint();
        let dv = board_with(child);

        assert_eq!(
            opened(&sync_refused(&mut s, &dv, Instant::<PageLoad>::new(0.0))),
            vec![child]
        );
        assert!(!s.is_open(&child), "a refused open records nothing");
        // The embed is still in the cull set, so the next frame offers it again.
        assert_eq!(
            opened(&sync(&mut s, &dv, Instant::<PageLoad>::new(16.0))),
            vec![child]
        );
        assert!(s.is_open(&child));
        // And once it is through, it is not offered a third time.
        assert!(sync(&mut s, &dv, Instant::<PageLoad>::new(32.0)).is_empty());
    }

    /// `TOILE-UI-339`: when the queue refuses a close, the next frame offers
    /// it again, so a full queue defers the session rather than leaking it.
    /// Forgetting the child before the command left used to leave a session
    /// open with nothing that named it, which `close_all` could not reclaim
    /// either.
    #[test]
    fn a_close_the_channel_refuses_comes_back_next_frame() {
        let mut s = EmbedSessions::default();
        let child = Id::<Document>::mint();
        let dv = board_with(child);
        let empty = viewport(Vec::new());
        sync(&mut s, &dv, Instant::<PageLoad>::new(0.0));

        sync(&mut s, &empty, Instant::<PageLoad>::new(0.0)); // countdown armed
        let past = EMBED_CLOSE_DEBOUNCE.get() + 1.0;
        assert_eq!(
            closed(&sync_refused(
                &mut s,
                &empty,
                Instant::<PageLoad>::new(past)
            )),
            vec![child]
        );
        assert!(s.is_open(&child), "a refused close forgets nothing");
        // Still past the deadline: offered again, and taken this time.
        assert_eq!(
            closed(&sync(&mut s, &empty, Instant::<PageLoad>::new(past + 16.0))),
            vec![child]
        );
        assert!(!s.is_open(&child));
    }

    /// `TOILE-UI-339` on the way out: a child whose close the queue refuses
    /// stays in the registry. A caller that tries again still reclaims it.
    #[test]
    fn close_all_keeps_what_the_channel_refused() {
        let mut s = EmbedSessions::default();
        let (a, b) = (Id::<Document>::mint(), Id::<Document>::mint());
        let dv = viewport(vec![
            DisplayNode::Element(doc_embed("e1", a, Rect::new(0.0, 0.0, 10.0, 10.0), 0.0)),
            DisplayNode::Element(doc_embed("e2", b, Rect::new(20.0, 0.0, 30.0, 10.0), 0.0)),
        ]);
        sync(&mut s, &dv, Instant::<PageLoad>::new(0.0));

        // Only `a` gets through; the queue refuses `b`, which must survive
        // the sweep.
        s.close_all(
            |cmd| matches!(cmd, LogicCommand::CloseDocument { document_id } if document_id == a),
        );
        assert!(!s.is_open(&a), "the close that left is done");
        assert!(s.is_open(&b), "the close that did not leave is still owed");
        assert_eq!(closed(&close_all(&mut s)), vec![b]);
    }

    #[test]
    fn two_embeds_of_the_same_document_share_one_session() {
        let mut s = EmbedSessions::default();
        let child = Id::<Document>::mint();
        // The same document placed twice on one board: the registry keys a
        // session by document, not by element, so this must open exactly one.
        let dv = viewport(vec![
            DisplayNode::Element(doc_embed("e1", child, Rect::new(0.0, 0.0, 10.0, 10.0), 0.0)),
            DisplayNode::Element(doc_embed(
                "e2",
                child,
                Rect::new(20.0, 0.0, 30.0, 10.0),
                0.0,
            )),
        ]);
        assert_eq!(
            opened(&sync(&mut s, &dv, Instant::<PageLoad>::new(0.0))),
            vec![child]
        );
        assert!(sync(&mut s, &dv, Instant::<PageLoad>::new(16.0)).is_empty());
    }
}

#[cfg(test)]
mod address_tests {
    use super::{Validated, WebAddress};
    use ambre_bus::app::toile::AddressError;

    /// A host typed on its own is the common case, and it is https.
    #[test]
    fn a_bare_host_becomes_an_https_address() {
        assert_eq!(
            Validated::<WebAddress>::parse("example.com")
                .as_ref()
                .map(Validated::as_str),
            Ok("https://example.com")
        );
        assert_eq!(
            Validated::<WebAddress>::parse("  example.com/a/b?q=1#top  ")
                .as_ref()
                .map(Validated::as_str),
            Ok("https://example.com/a/b?q=1#top")
        );
    }

    /// A port is not a scheme, however much its colon looks like one.
    #[test]
    fn a_host_with_a_port_keeps_its_port() {
        assert_eq!(
            Validated::<WebAddress>::parse("example.com:8443/x")
                .as_ref()
                .map(Validated::as_str),
            Ok("https://example.com:8443/x")
        );
    }

    #[test]
    fn an_https_address_survives_verbatim() {
        assert_eq!(
            Validated::<WebAddress>::parse("https://example.com/a")
                .as_ref()
                .map(Validated::as_str),
            Ok("https://example.com/a")
        );
        // The parser compares the scheme without case, then normalizes it.
        assert_eq!(
            Validated::<WebAddress>::parse("HTTPS://Example.COM/A")
                .as_ref()
                .map(Validated::as_str),
            Ok("https://Example.COM/A")
        );
    }

    /// `TOILE-088`: the schemes that would run in the board's stead never reach
    /// a frame. This is the security-bearing assertion of the whole feature.
    #[test]
    fn a_scheme_that_could_run_in_the_boards_stead_is_refused() {
        for typed in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "blob:https://example.com/abc",
            "vbscript:msgbox",
            "file:///etc/passwd",
            "about:blank",
        ] {
            assert_eq!(
                Validated::<WebAddress>::parse(typed),
                Err(AddressError::NotHttps),
                "{typed} must not reach a frame"
            );
        }
    }

    /// A scheme split by a control character is the classic way past a prefix
    /// test. The parser therefore rejects the control character first.
    #[test]
    fn a_scheme_hidden_behind_a_control_character_is_refused() {
        for typed in ["java\nscript:alert(1)", "java\tscript:alert(1)"] {
            assert!(
                Validated::<WebAddress>::parse(typed).is_err(),
                "{typed:?} must not be accepted"
            );
        }
        // The parser strips leading control characters, then reads the scheme
        // beneath them for what it is.
        assert_eq!(
            Validated::<WebAddress>::parse("\u{0}\njavascript:alert(1)"),
            Err(AddressError::NotHttps)
        );
    }

    /// The parser refuses plain http rather than silently upgrading it. The
    /// browser blocks an http frame inside a page served over TLS as mixed
    /// content. Accepting one would put a box on the board that never paints.
    #[test]
    fn plain_http_is_refused_and_not_upgraded() {
        assert_eq!(
            Validated::<WebAddress>::parse("http://example.com"),
            Err(AddressError::NotHttps)
        );
    }

    /// `TOILE-087`: an address that names no site is refused before it is placed.
    #[test]
    fn an_address_with_no_site_in_it_is_refused() {
        assert_eq!(Validated::<WebAddress>::parse(""), Err(AddressError::Empty));
        assert_eq!(
            Validated::<WebAddress>::parse("   "),
            Err(AddressError::Empty)
        );
        assert_eq!(
            Validated::<WebAddress>::parse("https://"),
            Err(AddressError::NoHost)
        );
        assert_eq!(
            Validated::<WebAddress>::parse("https:///a"),
            Err(AddressError::NoHost)
        );
        assert_eq!(
            Validated::<WebAddress>::parse("https://@/a"),
            Err(AddressError::NoHost)
        );
        assert_eq!(
            Validated::<WebAddress>::parse("https://:8080/a"),
            Err(AddressError::NoHost)
        );
        assert_eq!(
            Validated::<WebAddress>::parse("example .com"),
            Err(AddressError::NoHost)
        );
    }

    /// `TOILE-087`: every refusal has a sentence, and none of them is empty.
    #[test]
    fn every_refusal_says_what_to_do() {
        for e in [
            AddressError::Empty,
            AddressError::NotHttps,
            AddressError::NoHost,
        ] {
            assert!(!e.message().is_empty(), "{e:?}");
        }
    }
}

#[cfg(test)]
mod web_embed_tests {
    use ambre_bus::app::toile::{DisplayGeom, DisplayNode};
    use ambre_core::id::{Document, Id};
    use ambre_draw::{Rect, Size};

    use super::tests::{doc_embed, viewport};
    use super::{DisplayElement, ViewportTransform, visible_web_embeds};

    /// A web `embed` element: the doc-embed helper with its geometry swapped, so
    /// one helper covers both kinds.
    fn web(id: &str, bounds: Rect, url: &str) -> DisplayElement {
        let mut e = doc_embed(id, Id::<Document>::mint(), bounds, 0.0);
        e.geom = DisplayGeom::Embed(ambre_core::Validated::<super::WebAddress>::parse(url).ok());
        e
    }

    fn cam() -> ViewportTransform {
        ViewportTransform {
            size: Size::new(800.0, 600.0),
            ..Default::default()
        }
    }

    /// `TOILE-UI-337`: the view hands the shell the web embeds and nothing
    /// else. A nested document is a child view, not a frame, and the two lists
    /// must not cross.
    #[test]
    fn only_web_embeds_are_published_for_the_shell() {
        let page = web("w", Rect::new(0.0, 0.0, 100.0, 50.0), "https://example.com");
        let nested = doc_embed(
            "s",
            Id::<Document>::mint(),
            Rect::new(0.0, 0.0, 10.0, 10.0),
            0.0,
        );
        let dv = viewport(vec![
            DisplayNode::Element(page),
            DisplayNode::Element(nested),
        ]);

        let got = visible_web_embeds(&dv, &cam(), None);
        assert_eq!(got.len(), 1, "one frame, for the one web embed");
        assert_eq!(got[0].url.as_str(), "https://example.com");
        assert_eq!(got[0].element.as_str(), "w");
    }

    /// `TOILE-UI-337`: a board with no web embed asks the shell for no frame at
    /// all.
    #[test]
    fn a_board_with_no_web_embed_publishes_nothing() {
        let dv = viewport(vec![DisplayNode::Element(doc_embed(
            "s",
            Id::<Document>::mint(),
            Rect::new(0.0, 0.0, 10.0, 10.0),
            0.0,
        ))]);
        assert!(visible_web_embeds(&dv, &cam(), None).is_empty());
    }

    /// `TOILE-UI-413`: the browser lays the page out once, at the element's
    /// world size, and the transform moves it from there. Zooming the board
    /// must not reflow the page inside the frame.
    #[test]
    fn the_frames_layout_size_does_not_move_with_the_zoom() {
        let dv = viewport(vec![DisplayNode::Element(web(
            "w",
            Rect::new(0.0, 0.0, 100.0, 50.0),
            "https://example.com",
        ))]);
        let mut zoomed = cam();
        zoomed.zoom = ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(3.0);
        let one = visible_web_embeds(&dv, &cam(), None)[0].layout_size;
        let two = visible_web_embeds(&dv, &zoomed, None)[0].layout_size;
        assert_eq!(
            one, two,
            "the page's layout size is the element's, not the camera's"
        );
        assert_eq!(one, ambre_draw::Size::new(100.0, 50.0));
        // The zoom lives in the transform instead.
        let at = visible_web_embeds(&dv, &zoomed, None)[0].to_screen;
        let far = at * ambre_draw::Point::new(100.0, 0.0);
        let near = at * ambre_draw::Point::new(0.0, 0.0);
        assert!(((far - near).hypot() - 300.0).abs() < 1e-9);
    }

    /// `TOILE-UI-414`: the frame goes where the element is, through the
    /// camera: a rotated embed yields the box containing its quad, like a
    /// nested document's.
    #[test]
    fn the_frame_follows_the_element_through_the_camera() {
        let dv = viewport(vec![DisplayNode::Element(web(
            "w",
            Rect::new(0.0, 0.0, 100.0, 50.0),
            "https://example.com",
        ))]);

        let mut zoomed = cam();
        zoomed.zoom = ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(2.0);
        let one = visible_web_embeds(&dv, &cam(), None)[0].screen_rect;
        let two = visible_web_embeds(&dv, &zoomed, None)[0].screen_rect;
        assert!(
            (two.width() - 2.0 * one.width()).abs() < 1e-9,
            "the frame scales with the camera"
        );
    }
}

#[cfg(test)]
mod routing_tests {
    use ambre_canvas::InputEvent;
    use ambre_commands::{Key, ModifierKeys};
    use ambre_draw::{Point, Rect};

    use super::routes_to_child;

    /// The focused embed's quad: a 100x50 box at the origin, in screen space.
    fn quad() -> impl Fn(Point) -> bool {
        |p: Point| Rect::new(0.0, 0.0, 100.0, 50.0).contains(p)
    }

    fn wheel() -> InputEvent {
        InputEvent::Wheel {
            delta: ambre_canvas::WheelDelta::pixels(ambre_canvas::WheelVector::new(
                ambre_canvas::WheelAmount::new(0.0),
                ambre_canvas::WheelAmount::new(10.0),
            )),
            mods: ModifierKeys::NONE,
        }
    }

    /// `TOILE-UI-329`: a wheel with the pointer outside the embed is the board's,
    /// so a focused embed no longer freezes the board's own pan and zoom.
    #[test]
    fn a_wheel_outside_the_embed_stays_with_the_board() {
        let outside = Point::new(400.0, 400.0);
        assert!(!routes_to_child(&wheel(), outside, quad()));
    }

    /// And inside it is the child's: scrolling an embedded document works
    /// exactly where the user is pointing.
    #[test]
    fn a_wheel_inside_the_embed_is_the_childs() {
        let inside = Point::new(50.0, 25.0);
        assert!(routes_to_child(&wheel(), inside, quad()));
    }

    /// `TOILE-UI-331`: `Escape` stays the board's, whatever the pointer is
    /// doing. It is the only way out of an embed, so a child that swallowed it
    /// would trap the user inside.
    #[test]
    fn escape_never_reaches_the_child() {
        let ev = InputEvent::Key {
            key: Key::Escape,
            mods: ModifierKeys::NONE,
        };
        for at in [Point::new(50.0, 25.0), Point::new(400.0, 400.0)] {
            assert!(!routes_to_child(&ev, at, quad()));
        }
    }

    /// Typing follows focus and not the pointer: a caret has a position, a
    /// keystroke does not.
    #[test]
    fn typing_follows_focus_not_the_pointer() {
        let outside = Point::new(400.0, 400.0);
        for ev in [
            InputEvent::Text {
                text: "a".to_owned(),
            },
            InputEvent::Key {
                key: Key::Enter,
                mods: ModifierKeys::NONE,
            },
        ] {
            assert!(routes_to_child(&ev, outside, quad()));
        }
    }

    /// A pointer event is bound by its own position, which is what lets a click
    /// outside return focus to the board (`TOILE-UI-330`).
    #[test]
    fn a_pointer_event_is_bound_by_its_own_position() {
        let far = Point::new(400.0, 400.0);
        let down = |x, y| InputEvent::PointerDown {
            at: ambre_core::Point::new(
                ambre_core::Px::<ambre_core::View>::new(x),
                ambre_core::Px::<ambre_core::View>::new(y),
            ),
            button: ambre_canvas::PointerButton::Primary,
            mods: ModifierKeys::NONE,
        };
        // The pointer's own coordinates decide, not `last_pointer`.
        assert!(routes_to_child(&down(50.0, 25.0), far, quad()));
        assert!(!routes_to_child(
            &down(400.0, 400.0),
            Point::new(50.0, 25.0),
            quad()
        ));
    }
}

#[cfg(test)]
mod placement_tests {
    use super::{DEFAULT_EMBED_SIZE, EmbedTarget, PlacedEmbed, placement};
    use crate::ui::viewport::ViewportTransform;
    use ambre_bus::DocType;
    use ambre_bus::app::toile::{ElementKind, InitContent, ToileCommand};
    use ambre_core::id::{Document, Id};
    use ambre_draw::{Point, Size};

    fn created(cmd: ToileCommand) -> (ElementKind, Point, Size, InitContent) {
        match cmd {
            ToileCommand::CreateElement {
                kind,
                at,
                size,
                content,
                ..
            } => (kind, at, size, content),
            other => panic!("expected one element written, got {other:?}"),
        }
    }

    /// `TOILE-086`: a picked document lands as a reference to it, centred on
    /// what the viewer is looking at. The board stores the id, never the
    /// content.
    #[test]
    fn a_picked_document_lands_as_a_reference_in_the_middle_of_the_view() {
        let cam = ViewportTransform::new(ambre_draw::Size::new(800.0, 600.0));
        let document_id = Id::<Document>::mint();
        let target = EmbedTarget::Document {
            document_id,
            doc_type: DocType::Trame,
        };
        let (kind, at, size, content) = created(placement(PlacedEmbed { target, at: None }, &cam));
        assert_eq!(kind, ElementKind::DocEmbed);
        assert_eq!(size, DEFAULT_EMBED_SIZE);
        let centre = cam.screen_to_world() * Point::new(400.0, 300.0);
        assert_eq!(at, centre - (size.to_vec2() / 2.0));
        assert!(matches!(
            content,
            InitContent::DocEmbed { document_id: d, doc_type: DocType::Trame } if d == document_id
        ));
    }

    /// `TOILE-087`: an address lands as a web embed, centred under the point
    /// it was dropped on.
    #[test]
    fn a_web_address_lands_centred_under_its_drop_point() {
        let cam = ViewportTransform::new(ambre_draw::Size::new(800.0, 600.0));
        let url = ambre_core::Validated::<super::WebAddress>::parse("https://example.com")
            .expect("a safe address");
        let placed = PlacedEmbed {
            target: EmbedTarget::Web(url.clone()),
            at: Some(Point::new(100.0, 50.0)),
        };
        let (kind, at, size, content) = created(placement(placed, &cam));
        assert_eq!(kind, ElementKind::Embed);
        let centre = cam.screen_to_world() * Point::new(100.0, 50.0);
        assert_eq!(at, centre - (size.to_vec2() / 2.0));
        assert!(matches!(content, InitContent::Embed(u) if u == url));
    }
}
