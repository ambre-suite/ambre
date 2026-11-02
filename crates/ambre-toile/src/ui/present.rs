// ambre-toile/src/ui/present.rs
// ============================================================================
// Presentation transition core (toile-presentation Smart Animate + Focus
// Mode), pure and host-testable. Given the endpoints of a slide change and a
// wall-clock timestamp, it produces the camera transform: an eased glide toward
// the fitted target frame. It also produces the per-element content overrides
// for the current frame, the Smart Animate tween / fade set. It owns no GPU
// state and reads no board data.
//
// The enriched `SlideChanged` event carries the two `FrameSnapshot`s and the
// camera fit target into the view. The view builds
// a `Transition` and advances it each `plan_repaint` until it is `done`.
// ============================================================================

use ambre_bus::app::toile::FrameSnapshot;
use ambre_draw::Point;

use crate::logic::animate::{AnimatedElement, Phase, ease_in_out_cubic, smart_animate};

use super::viewport::ViewportTransform;
use ambre_core::{FracMs, Instant, PageLoad, TimeSpan};

/// One in-flight slide transition. Built by the view on a `SlideChanged` event
/// and advanced each frame. The camera glides from `cam_from` to `(cam_to_center,
/// cam_to_zoom)`; the content tweens from `from` to `to`. `started_at` is
/// stamped lazily on the first `advance` so the transition is measured from the
/// first frame it is actually shown, not from event dispatch.
pub struct Transition {
    started_at: Option<Instant<PageLoad>>,
    dur: std::time::Duration,
    cam_from: ViewportTransform,
    cam_to_center: Point,
    cam_to_zoom: ambre_core::Scale<ambre_core::Doc, ambre_core::View>,
    from: FrameSnapshot,
    to: FrameSnapshot,
}

/// The camera + content state for one transition frame, returned by
/// [`Transition::advance`]. `overrides` are the Smart Animate elements to paint
/// this frame; `phase` is `Done` once the raw [`Phase`] reaches its end.
pub struct TransitionFrame {
    pub cam: ViewportTransform,
    pub overrides: Vec<AnimatedElement>,
    pub phase: TransitionPhase,
}

ambre_core::define_flag! {
    /// Whether a transition still runs or has reached its end. How far it has
    /// come is a [`Phase`], a factor in `0..=1`; this flag only reads whether
    /// that factor is at its end.
    pub enum TransitionPhase {
        Running,
        Done,
    }
}

impl Transition {
    /// Build a transition. `cam_to_center` / `cam_to_zoom` are the fit target for
    /// the new slide. The view resolves them from the frame rect, falling back to
    /// the current camera when the slide has no resolvable frame. A zero `dur`
    /// makes the transition resolve in a single `advance` (an instant snap).
    pub fn new(
        cam_from: ViewportTransform,
        cam_to_center: Point,
        cam_to_zoom: ambre_core::Scale<ambre_core::Doc, ambre_core::View>,
        from: FrameSnapshot,
        to: FrameSnapshot,
        dur: std::time::Duration,
    ) -> Self {
        Self {
            started_at: None,
            dur,
            cam_from,
            cam_to_center,
            cam_to_zoom,
            from,
            to,
        }
    }

    /// The culling viewport that covers both ends of this transition
    /// (`TOILE-UI-085`). `cam` is the camera the frame is drawn at, and only
    /// its size is read.
    pub fn covering_viewport(&self, cam: &ViewportTransform) -> ambre_bus::app::toile::Viewport {
        let from = ViewportTransform {
            size: cam.size,
            ..self.cam_from
        };
        from.covering_viewport(self.cam_to_center, self.cam_to_zoom)
    }

    /// Advance to the page clock's `now`, stamping the start on the first call. The
    /// raw phase `t` is `(now - start) / dur` clamped to `0..=1`. It is eased
    /// before both the camera glide and the content solve, so the whole
    /// transition shares one easing curve. `phase` uses the raw (un-eased) phase
    /// so the end is reached exactly at `dur`.
    pub fn advance(&mut self, now: Instant<PageLoad>) -> TransitionFrame {
        let start = *self.started_at.get_or_insert(now);
        let raw = if self.dur.is_zero() {
            Phase::END
        } else {
            Phase::clamped((now - start) / TimeSpan::<FracMs>::from_duration(self.dur))
        };
        let eased = ease_in_out_cubic(raw);
        TransitionFrame {
            cam: ViewportTransform::glide(
                self.cam_from,
                self.cam_to_center,
                self.cam_to_zoom,
                eased,
            ),
            overrides: smart_animate(&self.from, &self.to, eased),
            phase: TransitionPhase::from_bool(raw == Phase::END),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::SnapElement;
    use ambre_bus::{Instant, PageLoad};
    use ambre_core::Rgba;
    use ambre_core::id::{Element, Id};
    use ambre_draw::{Rect, Size};

    fn snap(id: &str, bounds: Rect) -> SnapElement {
        SnapElement {
            id: Id::<Element>::from_text(id.to_owned()),
            bounds,
            rotation: ambre_core::Radians::ZERO,
            opacity: ambre_core::Bounded::<ambre_core::Opacity>::new(1.0).unwrap(),
            fill: Some(Rgba::from_bits(0x0000_00FF)),
        }
    }

    fn cam(cx: f64, cy: f64, zoom: f64) -> ViewportTransform {
        ViewportTransform {
            center: Point::new(cx, cy),
            zoom: ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(zoom),
            size: Size::new(1000.0, 800.0),
        }
    }

    fn transition() -> Transition {
        let from = FrameSnapshot {
            elements: vec![snap("shared", Rect::new(0.0, 0.0, 10.0, 10.0))],
        };
        let to = FrameSnapshot {
            elements: vec![
                snap("shared", Rect::new(100.0, 100.0, 110.0, 110.0)),
                snap("new", Rect::new(200.0, 200.0, 210.0, 210.0)),
            ],
        };
        Transition::new(
            cam(0.0, 0.0, 1.0),
            Point::new(105.0, 105.0),
            ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(2.0),
            from,
            to,
            std::time::Duration::from_millis(300),
        )
    }

    #[test]
    fn start_frame_holds_source_camera_and_content() {
        let mut t = transition();
        let f = t.advance(Instant::<PageLoad>::new(1000.0));
        // t = 0: camera is exactly the source, no progress yet.
        assert_eq!(f.cam.center, Point::new(0.0, 0.0));
        assert_eq!(f.cam.zoom.get(), 1.0);
        assert_eq!(f.phase, TransitionPhase::Running);
        // The shared element sits at its source bounds; the new element is fully
        // transparent (fade-in at t=0).
        let shared = f
            .overrides
            .iter()
            .find(|a| a.id == Id::<Element>::from_text("shared".into()))
            .unwrap();
        assert_eq!(shared.bounds, Rect::new(0.0, 0.0, 10.0, 10.0));
        let new = f
            .overrides
            .iter()
            .find(|a| a.id == Id::<Element>::from_text("new".into()))
            .unwrap();
        assert_eq!(new.opacity.get(), 0.0);
    }

    #[test]
    fn end_frame_lands_on_target_camera_and_content() {
        let mut t = transition();
        t.advance(Instant::<PageLoad>::new(1000.0)); // stamp start
        let f = t.advance(Instant::<PageLoad>::new(1000.0 + 300.0));
        assert_eq!(
            f.phase,
            TransitionPhase::Done,
            "the transition is done at its full duration"
        );
        // Camera reached the fit target exactly.
        assert_eq!(f.cam.center, Point::new(105.0, 105.0));
        assert!((f.cam.zoom.get() - 2.0).abs() < 1e-9);
        // The shared element reached its target bounds; the new element is fully
        // opaque.
        let shared = f
            .overrides
            .iter()
            .find(|a| a.id == Id::<Element>::from_text("shared".into()))
            .unwrap();
        assert_eq!(shared.bounds, Rect::new(100.0, 100.0, 110.0, 110.0));
        let new = f
            .overrides
            .iter()
            .find(|a| a.id == Id::<Element>::from_text("new".into()))
            .unwrap();
        assert!((new.opacity.get() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn midpoint_tweens_between_endpoints() {
        let mut t = transition();
        t.advance(Instant::<PageLoad>::new(0.0)); // start at 0
        let f = t.advance(Instant::<PageLoad>::new(150.0)); // half of 300 ms
        assert_eq!(f.phase, TransitionPhase::Running);
        // Eased 0.5 == 0.5, so the shared element sits at the geometric midpoint
        // and the camera is halfway to the target.
        let shared = f
            .overrides
            .iter()
            .find(|a| a.id == Id::<Element>::from_text("shared".into()))
            .unwrap();
        assert_eq!(shared.bounds, Rect::new(50.0, 50.0, 60.0, 60.0));
        assert_eq!(f.cam.center, Point::new(52.5, 52.5));
        assert!((f.cam.zoom.get() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn zero_duration_snaps_immediately() {
        let from = FrameSnapshot::default();
        let to = FrameSnapshot {
            elements: vec![snap("x", Rect::new(0.0, 0.0, 5.0, 5.0))],
        };
        let mut t = Transition::new(
            cam(0.0, 0.0, 1.0),
            Point::new(10.0, 10.0),
            ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(3.0),
            from,
            to,
            std::time::Duration::ZERO,
        );
        let f = t.advance(Instant::<PageLoad>::new(42.0));
        assert_eq!(
            f.phase,
            TransitionPhase::Done,
            "a zero-duration transition is done on first advance"
        );
        assert_eq!(f.cam.center, Point::new(10.0, 10.0));
    }

    /// `TOILE-UI-085`: the cull a transition asks for covers both ends of the
    /// glide. An element only the previous slide holds therefore stays in the
    /// visible set. Its fade-out runs to the end, instead of stopping the frame
    /// the camera leaves it.
    #[test]
    fn the_covering_cull_holds_both_ends_of_the_glide() {
        let from = cam(0.0, 0.0, 1.0);
        let t = Transition::new(
            from,
            Point::new(4000.0, 0.0),
            ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(1.0),
            FrameSnapshot::default(),
            FrameSnapshot::default(),
            std::time::Duration::from_millis(300),
        );
        let covering = t.covering_viewport(&from);
        let half_w = covering.size.width / (2.0 * covering.zoom.get());
        let (left, right) = (covering.center.x - half_w, covering.center.x + half_w);
        let start_left = from.center.x - from.size.width / (2.0 * from.zoom.get());
        let end_right = 4000.0 + from.size.width / 2.0;
        assert!(
            left <= start_left && right >= end_right,
            "the cull dropped one end: {left}..{right}"
        );
    }
}
