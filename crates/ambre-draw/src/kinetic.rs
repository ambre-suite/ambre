//! The one kinetic scroll every application shares
//! (`1-Foundations/ambre-spec-kinetic-scroll.md`).
//!
//! Pure arithmetic over timed pointer positions: no DOM, no GPU, no
//! application state. The model hands back distances in screen pixels. Each
//! application moves its own view by them: a column of pages, a grid of
//! cells, a board under a zoom. A trackpad glide that stops keeps its
//! momentum; a wheel notch never does (`KINETIC-002`, `KINETIC-009`).

use ambre_core::{CssPx, CssVector, PageMillis, ViewPoint, ViewPx, ViewSpeed, ViewSpeedVector};

use crate::wheel::WheelDeltaMode;

/// How far a pointer may move, in screen pixels, and still be a tap.
pub const TAP_SLOP: ViewPx = ViewPx::new(8.0);
/// How long a press held inside the slop lasts before it is a long press
/// (`KINETIC-014`).
pub const LONG_PRESS: PageMillis = PageMillis::new(500.0);
/// The window of samples the release speed is read over.
pub const SAMPLE_WINDOW: PageMillis = PageMillis::new(100.0);
/// The time constant of the slowdown: the speed falls to about a third of
/// itself over this time.
pub const DECAY: PageMillis = PageMillis::new(325.0);
/// Below this speed, in screen pixels per millisecond, a release leaves no
/// motion and a motion stops.
pub const STOP_SPEED: ViewSpeed = ViewSpeed::new(0.02);
/// The longest step one tick takes, so a stalled frame does not jump. It is
/// private: a step is timed here alone (`KINETIC-013`).
const MAX_STEP: PageMillis = PageMillis::new(50.0);

/// A pixel wheel step at or past this, in screen pixels, is a notch.
/// Trackpad glides stay well under it; a notch sits well over it.
pub const WHEEL_NOTCH: CssPx = CssPx::new(40.0);
/// A pause between wheel events this long ends a glide.
pub const WHEEL_GAP: PageMillis = PageMillis::new(60.0);
/// A glide slower than this at its end, in screen pixels per millisecond,
/// leaves no motion.
pub const WHEEL_START_SPEED: ViewSpeed = ViewSpeed::new(0.05);
/// The weight of the newest wheel sample in the glide speed.
pub const WHEEL_BLEND: f32 = 0.8;

/// How far from an edge a drag starts to scroll, in screen pixels.
pub const EDGE_BAND: ViewPx = ViewPx::new(32.0);
/// How deep past the band the edge speed still grows, in screen pixels.
pub const EDGE_DEPTH: ViewPx = ViewPx::new(160.0);
/// The span the edge speed covers its depth in: 1/80 px/ms for each pixel
/// of depth, so 2 px/ms at most.
pub const EDGE_SPAN: PageMillis = PageMillis::new(80.0);

/// How far the view moves, in screen pixels on each axis (`KINETIC-013`).
///
/// Only this model builds one, so an application reads a step and never
/// computes its own. A view's scroll by a pan, a flick, a glide or an edge
/// takes one. A notch is a step the model hands back one for one:
///
/// ```
/// use ambre_draw::kinetic::Kinetic;
/// use ambre_draw::wheel::WheelDeltaMode;
/// use ambre_core::{CssPx, CssVector};
/// let delta = CssVector::new(CssPx::ZERO, CssPx::new(30.0));
/// let step = Kinetic::default().wheel(delta, WheelDeltaMode::Line, ambre_core::PageTime(0.0));
/// assert_eq!(step.y(), ambre_core::ViewPx::new(30.0));
/// ```
///
/// Each block below fails on the step it builds outside the model alone:
///
/// ```compile_fail
/// let (x, y) = (ambre_core::ViewPx::ZERO, ambre_core::ViewPx::new(30.0));
/// let step = ambre_draw::kinetic::ScrollStep { x, y };
/// ```
///
/// ```compile_fail
/// let step = ambre_draw::kinetic::ScrollStep::ZERO;
/// ```
///
/// ```compile_fail
/// let clamp = ambre_draw::kinetic::MAX_STEP;
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollStep {
    x: ViewPx,
    y: ViewPx,
}

impl ScrollStep {
    const ZERO: Self = Self {
        x: ViewPx::ZERO,
        y: ViewPx::ZERO,
    };

    /// The step along the horizontal axis.
    pub fn x(self) -> ViewPx {
        self.x
    }

    /// The step along the vertical axis.
    pub fn y(self) -> ViewPx {
        self.y
    }

    /// The step of a pointer dragged from `from` to `to`, one for one, the
    /// content going with the pointer. It has no slop and leaves no motion,
    /// which is what a mouse held on the board asks for (`KINETIC-001`).
    pub fn dragged(from: ViewPoint, to: ViewPoint) -> Self {
        let by = from - to;
        Self { x: by.dx, y: by.dy }
    }
}

/// The speed a drag held near an edge asks for, in screen pixels per
/// millisecond, towards the far end for a positive one (`KINETIC-007`).
/// Only [`EdgeSpeed::at`] builds a speed other than none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeSpeed(ViewSpeed);

impl EdgeSpeed {
    /// No edge scroll on this axis.
    pub const NONE: Self = Self(ViewSpeed::ZERO);

    /// The speed of a pointer at `at` along one axis, in a view of `extent`.
    /// Zero away from both edges.
    pub fn at(at: ViewPx, extent: ViewPx) -> Self {
        Self(edge_speed(at, extent))
    }

    /// The speed, for a reader.
    pub fn get(self) -> ViewSpeed {
        self.0
    }
}

/// The edge scroll of a held drag: its clock, and the step each frame takes
/// (`KINETIC-007`).
#[derive(Clone, Debug, Default)]
pub struct EdgeScroll {
    last_at: Option<ambre_core::PageTime>,
}

impl EdgeScroll {
    /// One frame of the edge scroll. No speed, or none on either axis, ends
    /// it. The first frame only starts the clock.
    pub fn tick(
        &mut self,
        speed: Option<(EdgeSpeed, EdgeSpeed)>,
        now: ambre_core::PageTime,
    ) -> Option<ScrollStep> {
        let still = (EdgeSpeed::NONE, EdgeSpeed::NONE);
        let Some((sx, sy)) = speed.filter(|&s| s != still) else {
            self.last_at = None;
            return None;
        };
        let last = self.last_at.replace(now)?;
        let elapsed = step_span(now - last);
        Some(ScrollStep {
            x: sx.0.over(elapsed),
            y: sy.0.over(elapsed),
        })
    }

    /// End the edge scroll: the drag ended.
    pub fn stop(&mut self) {
        self.last_at = None;
    }
}

/// What a release leaves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Release {
    /// The pointer never left the slop: a tap at its press point
    /// (`KINETIC-003`).
    Tap { at: ViewPoint },
    /// A pan released with speed: the view keeps moving (`KINETIC-004`).
    Fling,
    /// A pan released at rest, a long press, or nothing pressed.
    Rest,
}

/// One pointer's pan, then the motion its release left.
#[derive(Clone, Debug, Default)]
pub struct Kinetic {
    pressed: Option<Pan>,
    motion: Option<Motion>,
    glide: Option<Glide>,
}

/// A trackpad glide under way: its speed so far and its last event.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Glide {
    speed: ViewSpeedVector,
    last_at: ambre_core::PageTime,
}

#[derive(Clone, Debug)]
struct Pan {
    start: ViewPoint,
    last: ViewPoint,
    panned: bool,
    /// When the pointer went down.
    pressed_at: ambre_core::PageTime,
    /// The press became a long press: it pans nothing and taps nothing.
    held: bool,
    /// Recent positions and their times, for the release speed.
    samples: Vec<(ambre_core::PageTime, ViewPoint)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Motion {
    /// Screen pixels per millisecond, on each axis.
    speed: ViewSpeedVector,
    last_at: Option<ambre_core::PageTime>,
}

impl Kinetic {
    /// A pointer went down. It stops any motion left (`KINETIC-006`).
    pub fn press(&mut self, at: ViewPoint, now: ambre_core::PageTime) {
        self.motion = None;
        self.glide = None;
        self.pressed = Some(Pan {
            start: at,
            last: at,
            panned: false,
            pressed_at: now,
            held: false,
            samples: vec![(now, at)],
        });
    }

    /// Whether the press became a long press by `now`: held inside the
    /// slop past the delay. The answer is its press point, once. The press
    /// then pans nothing, and its release is no tap (`KINETIC-014`).
    pub fn held(&mut self, now: ambre_core::PageTime) -> Option<ViewPoint> {
        let pan = self.pressed.as_mut()?;
        if pan.panned || pan.held || now - pan.pressed_at < LONG_PRESS {
            return None;
        }
        pan.held = true;
        Some(pan.start)
    }

    /// The pointer moved. Inside the slop it moves nothing (`KINETIC-003`).
    /// Past it, the answer is how far the view follows, in screen pixels.
    /// The content moves with the pointer, so it is the pointer's step
    /// reversed.
    pub fn sample(&mut self, at: ViewPoint, now: ambre_core::PageTime) -> Option<ScrollStep> {
        let pan = self.pressed.as_mut()?;
        if pan.held {
            return None;
        }
        if !pan.panned {
            let moved = at - pan.start;
            if moved.length() < TAP_SLOP {
                return None;
            }
            pan.panned = true;
        }
        let step = ScrollStep::dragged(pan.last, at);
        pan.last = at;
        pan.samples.push((now, at));
        pan.samples
            .retain(|(sampled, _)| now - *sampled <= SAMPLE_WINDOW);
        Some(step)
    }

    /// The pointer lifted. A pan released with speed leaves a motion, read
    /// over the last window of samples (`KINETIC-004`).
    pub fn release(&mut self, now: ambre_core::PageTime) -> Release {
        let Some(pan) = self.pressed.take() else {
            return Release::Rest;
        };
        if pan.held {
            return Release::Rest;
        }
        if !pan.panned {
            return Release::Tap { at: pan.start };
        }
        let mut recent = pan
            .samples
            .iter()
            .filter(|(sampled, _)| now - *sampled <= SAMPLE_WINDOW);
        let (Some(first), Some(last)) = (recent.next(), recent.next_back()) else {
            return Release::Rest;
        };
        let span = last.0 - first.0;
        if span <= PageMillis::ZERO {
            return Release::Rest;
        }
        let speed = (first.1 - last.1).per(span);
        if speed.magnitude() < STOP_SPEED {
            return Release::Rest;
        }
        self.motion = Some(Motion {
            speed,
            last_at: None,
        });
        Release::Fling
    }

    /// The browser cancelled the pointer: it took the gesture over. The pan
    /// ends with no tap and leaves no motion (`KINETIC-010`).
    pub fn cancel(&mut self) {
        self.pressed = None;
        self.motion = None;
        self.glide = None;
    }

    /// A wheel event, already in screen pixels. The answer is the step the
    /// view takes now, one for one. A notch stops any motion and never starts
    /// one; a glide feeds a speed the motion starts from once the glide
    /// pauses (`KINETIC-002`, `KINETIC-009`). `mode` is the unit the event came
    /// in: a line or a page is a notch.
    pub fn wheel(
        &mut self,
        delta: CssVector,
        mode: WheelDeltaMode,
        now: ambre_core::PageTime,
    ) -> ScrollStep {
        self.motion = None;
        // The wheel delta is already in screen pixels, a length and no
        // coordinate, so the step takes it one for one.
        let moved = ambre_core::ViewVector::new(delta.dx.view_length(), delta.dy.view_length());
        let step = ScrollStep {
            x: moved.dx,
            y: moved.dy,
        };
        let notch = delta.dx.abs() >= WHEEL_NOTCH || delta.dy.abs() >= WHEEL_NOTCH;
        if mode.is_step() || notch {
            self.glide = None;
            return step;
        }
        let speed = match self.glide {
            Some(glide) if now - glide.last_at <= WHEEL_GAP => {
                let dt = (now - glide.last_at).clamp(PageMillis::new(1.0), PageMillis::new(100.0));
                let b = WHEEL_BLEND;
                moved.per(dt).scaled(b) + glide.speed.scaled(1.0 - b)
            }
            _ => moved.per(PageMillis::new(16.0)),
        };
        self.glide = Some(Glide {
            speed,
            last_at: now,
        });
        step
    }

    /// One tick of the motion, once a frame. The answer is how far the view
    /// moves, each tick less than the one before; the first tick only starts
    /// the clock (`KINETIC-005`). The motion stops below the stop speed.
    /// A glide that paused past the gap launches its motion here.
    pub fn tick(&mut self, now: ambre_core::PageTime) -> Option<ScrollStep> {
        if self.motion.is_none() {
            let glide = self.glide?;
            if now - glide.last_at < WHEEL_GAP {
                return Some(ScrollStep::ZERO);
            }
            self.glide = None;
            if glide.speed.magnitude() < WHEEL_START_SPEED {
                return None;
            }
            self.motion = Some(Motion {
                speed: glide.speed,
                last_at: Some(now),
            });
            return Some(ScrollStep::ZERO);
        }
        let motion = self.motion.as_mut()?;
        let Some(last) = motion.last_at.replace(now) else {
            return Some(ScrollStep::ZERO);
        };
        let elapsed = step_span(now - last);
        let step = ScrollStep {
            x: motion.speed.dx.over(elapsed),
            y: motion.speed.dy.over(elapsed),
        };
        let decay = (-(elapsed / DECAY)).exp();
        #[allow(clippy::cast_possible_truncation)]
        let decay = decay as f32;
        motion.speed = motion.speed.scaled(decay);
        if motion.speed.magnitude() < STOP_SPEED {
            self.motion = None;
        }
        Some(step)
    }

    /// Stop the motion: the application met an edge, or took an input that
    /// stops it (`KINETIC-006`).
    pub fn stop(&mut self) {
        self.motion = None;
        self.glide = None;
    }

    /// Whether a motion is under way, or a glide may still launch one: the
    /// application keeps ticking while this holds.
    pub fn moving(&self) -> bool {
        self.motion.is_some() || self.glide.is_some()
    }

    /// The speed of the motion under way, in screen pixels per millisecond.
    pub fn speed(&self) -> Option<ViewSpeedVector> {
        self.motion.map(|motion| motion.speed)
    }

    /// Whether a pointer is down.
    pub fn pressed(&self) -> bool {
        self.pressed.is_some()
    }
}

/// The span one tick moves by: never negative, and never past
/// [`MAX_STEP`].
fn step_span(elapsed: PageMillis) -> PageMillis {
    elapsed.clamp(PageMillis::ZERO, MAX_STEP)
}

/// The speed a drag held near an edge scrolls at, in screen pixels per
/// millisecond, towards the far end for a positive one (`KINETIC-007`).
///
/// `at` is the pointer along one axis, and `extent` the length of the view
/// on it. The speed is zero away from both edges. It grows with how deep
/// the pointer sits in the band or past it, and stops growing further out.
fn edge_speed(at: ViewPx, extent: ViewPx) -> ViewSpeed {
    let far = extent - EDGE_BAND;
    let depth = if at < EDGE_BAND {
        at - EDGE_BAND
    } else if at > far {
        at - far
    } else {
        ViewPx::ZERO
    };
    depth.clamp(-EDGE_DEPTH, EDGE_DEPTH).per(EDGE_SPAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canvas point at `(x, y)`.
    fn pt(x: f32, y: f32) -> ViewPoint {
        ViewPoint::new(ViewPx::new(x), ViewPx::new(y))
    }

    /// A wheel delta of `(dx, dy)` CSS pixels.
    fn px(dx: f32, dy: f32) -> CssVector {
        CssVector::new(ambre_core::CssPx::new(dx), ambre_core::CssPx::new(dy))
    }

    /// Pan up at 2 px/ms on y, 1 px/ms on x, and lift.
    fn flicked() -> Kinetic {
        let mut kinetic = Kinetic::default();
        kinetic.press(pt(100.0, 600.0), ambre_core::PageTime(0.0));
        for step in 1..=5 {
            let t = 16.0 * f64::from(step);
            kinetic.sample(
                pt(100.0 - 16.0 * step as f32, 600.0 - 32.0 * step as f32),
                ambre_core::PageTime(t),
            );
        }
        assert_eq!(kinetic.release(ambre_core::PageTime(80.0)), Release::Fling);
        kinetic
    }

    /// `KINETIC-003`: a pointer that stays inside the slop is a tap and
    /// moves nothing.
    #[test]
    fn a_pointer_inside_the_slop_is_a_tap_and_moves_nothing() {
        let mut kinetic = Kinetic::default();
        kinetic.press(pt(10.0, 20.0), ambre_core::PageTime(0.0));
        assert_eq!(
            kinetic.sample(pt(13.0, 24.0), ambre_core::PageTime(10.0)),
            None
        );
        assert_eq!(
            kinetic.release(ambre_core::PageTime(20.0)),
            Release::Tap { at: pt(10.0, 20.0) }
        );
        assert!(!kinetic.moving());
        assert_eq!(kinetic.tick(ambre_core::PageTime(36.0)), None);
    }

    /// `KINETIC-014`: a press held inside the slop past the delay is a long
    /// press, said once. It then pans nothing and taps nothing, and a pan
    /// never becomes one.
    #[test]
    fn a_press_held_inside_the_slop_is_a_long_press_and_no_tap() {
        let mut kinetic = Kinetic::default();
        kinetic.press(pt(10.0, 20.0), ambre_core::PageTime(0.0));
        assert_eq!(
            kinetic.sample(pt(13.0, 24.0), ambre_core::PageTime(100.0)),
            None
        );
        assert_eq!(
            kinetic.held(ambre_core::PageTime(LONG_PRESS.get() - 1.0)),
            None
        );
        assert_eq!(
            kinetic.held(ambre_core::PageTime(LONG_PRESS.get())),
            Some(pt(10.0, 20.0))
        );
        assert_eq!(
            kinetic.held(ambre_core::PageTime(LONG_PRESS.get() + 100.0)),
            None
        );
        assert_eq!(
            kinetic.sample(
                pt(60.0, 90.0),
                ambre_core::PageTime(LONG_PRESS.get() + 120.0)
            ),
            None
        );
        assert_eq!(
            kinetic.release(ambre_core::PageTime(LONG_PRESS.get() + 200.0)),
            Release::Rest
        );
        assert!(!kinetic.moving());

        kinetic.press(pt(10.0, 20.0), ambre_core::PageTime(0.0));
        assert!(
            kinetic
                .sample(pt(10.0, 60.0), ambre_core::PageTime(16.0))
                .is_some()
        );
        assert_eq!(kinetic.held(ambre_core::PageTime(LONG_PRESS.get())), None);
        assert_eq!(
            Kinetic::default().held(ambre_core::PageTime(LONG_PRESS.get())),
            None
        );
    }

    /// `KINETIC-001`: a drag moves the view as a pan past the slop does, the
    /// content going with the pointer.
    #[test]
    fn a_drag_moves_the_view_as_a_pan_past_the_slop_does() {
        let mut kinetic = Kinetic::default();
        kinetic.press(pt(0.0, 0.0), ambre_core::PageTime(0.0));
        let panned = kinetic.sample(pt(-12.0, 30.0), ambre_core::PageTime(16.0));
        let dragged = ScrollStep::dragged(pt(0.0, 0.0), pt(-12.0, 30.0));
        assert_eq!(panned, Some(dragged));
        assert_eq!((dragged.x().get(), dragged.y().get()), (12.0, -30.0));
    }

    /// `KINETIC-004`: a pan released carries on at the speed of its last
    /// window, on both axes; one released at rest leaves nothing.
    #[test]
    fn a_pan_released_carries_on_at_the_speed_of_its_last_window() {
        let kinetic = flicked();
        let speed = kinetic.speed().expect("a motion");
        let (sx, sy) = (speed.dx.get(), speed.dy.get());
        assert!(
            (sx - 1.0).abs() < 1e-3 && (sy - 2.0).abs() < 1e-3,
            "{sx} {sy}"
        );
        // The view follows the pointer while it pans.
        let mut kinetic = Kinetic::default();
        kinetic.press(pt(0.0, 0.0), ambre_core::PageTime(0.0));
        let step = ScrollStep {
            x: ViewPx::new(0.0),
            y: ViewPx::new(20.0),
        };
        assert_eq!(
            kinetic.sample(pt(0.0, -20.0), ambre_core::PageTime(16.0)),
            Some(step)
        );
        // Held still for longer than the window, then lifted: no motion.
        assert_eq!(kinetic.release(ambre_core::PageTime(400.0)), Release::Rest);
        assert!(!kinetic.moving());
    }

    /// `KINETIC-005`: each tick moves less than the one before, and the
    /// motion stops below the stop speed.
    #[test]
    fn each_tick_moves_less_and_the_motion_stops() {
        let mut kinetic = flicked();
        assert_eq!(
            kinetic.tick(ambre_core::PageTime(80.0)),
            Some(ScrollStep::ZERO)
        );
        let (mut now, mut last) = (80.0, f32::INFINITY);
        while let Some(step) = kinetic.tick(ambre_core::PageTime(now + 16.0)) {
            let dy = step.y().get();
            now += 16.0;
            assert!(dy > 0.0 && dy < last, "{dy} after {last}");
            last = dy;
            assert!(now < 10_000.0, "the motion stops");
        }
        assert!(!kinetic.moving());
        // A stalled frame takes one bounded step.
        let mut kinetic = flicked();
        kinetic.tick(ambre_core::PageTime(80.0));
        let dy = kinetic
            .tick(ambre_core::PageTime(5_080.0))
            .expect("a step")
            .y()
            .get();
        assert!(dy <= ViewSpeed::new(2.0).over(MAX_STEP).get() + 1e-3);
    }

    /// `KINETIC-006`: a press or an edge the application met stops the
    /// motion, and no tick moves after.
    #[test]
    fn a_press_or_an_edge_stops_the_motion() {
        let mut kinetic = flicked();
        kinetic.tick(ambre_core::PageTime(80.0));
        kinetic.press(pt(0.0, 0.0), ambre_core::PageTime(90.0));
        assert!(!kinetic.moving());
        assert_eq!(kinetic.tick(ambre_core::PageTime(106.0)), None);
        let mut kinetic = flicked();
        kinetic.stop();
        assert_eq!(kinetic.tick(ambre_core::PageTime(96.0)), None);
    }

    /// `KINETIC-002`, `KINETIC-009`: a glide that pauses carries on; a notch never starts a
    /// motion and stops one under way.
    #[test]
    fn a_glide_that_pauses_carries_on_and_a_notch_never_does() {
        let mut kinetic = Kinetic::default();
        for step in 0..6 {
            let t = 8.0 * f64::from(step);
            let step = ScrollStep {
                x: ViewPx::new(0.0),
                y: ViewPx::new(12.0),
            };
            assert_eq!(
                kinetic.wheel(
                    px(0.0, 12.0),
                    WheelDeltaMode::Pixel,
                    ambre_core::PageTime(t)
                ),
                step
            );
        }
        // Inside the gap the glide waits; past it the motion starts.
        assert_eq!(
            kinetic.tick(ambre_core::PageTime(60.0)),
            Some(ScrollStep::ZERO)
        );
        assert_eq!(
            kinetic.tick(ambre_core::PageTime(110.0)),
            Some(ScrollStep::ZERO)
        );
        let dy = kinetic
            .tick(ambre_core::PageTime(126.0))
            .expect("a motion")
            .y()
            .get();
        assert!(dy > 0.0, "{dy}");
        // A notch stops it, moves one for one, and leaves nothing behind.
        let notch = ScrollStep {
            x: ViewPx::new(0.0),
            y: ViewPx::new(100.0),
        };
        assert_eq!(
            kinetic.wheel(
                px(0.0, 100.0),
                WheelDeltaMode::Pixel,
                ambre_core::PageTime(130.0)
            ),
            notch
        );
        assert!(!kinetic.moving());
        assert_eq!(kinetic.tick(ambre_core::PageTime(300.0)), None);
        kinetic.wheel(
            px(0.0, 3.0),
            WheelDeltaMode::Line,
            ambre_core::PageTime(400.0),
        );
        assert_eq!(kinetic.tick(ambre_core::PageTime(600.0)), None);
        // A glide that dies out slowly leaves no motion.
        kinetic.wheel(
            px(0.0, 0.5),
            WheelDeltaMode::Pixel,
            ambre_core::PageTime(700.0),
        );
        assert_eq!(kinetic.tick(ambre_core::PageTime(800.0)), None);
    }

    /// `KINETIC-010`: a cancelled pan leaves no tap and no motion, even one
    /// that moved fast.
    #[test]
    fn a_cancelled_pan_leaves_no_tap_and_no_motion() {
        let mut kinetic = Kinetic::default();
        kinetic.press(pt(100.0, 600.0), ambre_core::PageTime(0.0));
        for step in 1..=5 {
            let t = 16.0 * f64::from(step);
            kinetic.sample(
                pt(100.0, 600.0 - 32.0 * step as f32),
                ambre_core::PageTime(t),
            );
        }
        kinetic.cancel();
        assert!(!kinetic.pressed() && !kinetic.moving());
        assert_eq!(kinetic.release(ambre_core::PageTime(90.0)), Release::Rest);
        assert_eq!(kinetic.tick(ambre_core::PageTime(96.0)), None);
        // A tap cancelled before it lifts is no tap either.
        kinetic.press(pt(10.0, 10.0), ambre_core::PageTime(200.0));
        kinetic.cancel();
        assert_eq!(kinetic.release(ambre_core::PageTime(210.0)), Release::Rest);
    }

    /// `KINETIC-007`: the edge speed is zero away from the edges, grows with
    /// depth, and is capped.
    #[test]
    fn the_edge_speed_grows_with_depth_and_is_capped() {
        let px = ViewPx::new;
        let extent = px(900.0);
        assert_eq!(edge_speed(px(400.0), extent), ViewSpeed::ZERO);
        assert_eq!(edge_speed(EDGE_BAND, extent), ViewSpeed::ZERO);
        let (near, deeper) = (edge_speed(px(880.0), extent), edge_speed(px(890.0), extent));
        assert!(ViewSpeed::ZERO < near && near < deeper);
        assert!(edge_speed(px(10.0), extent) < ViewSpeed::ZERO);
        let cap = EDGE_DEPTH.per(EDGE_SPAN);
        assert_eq!(cap, ViewSpeed::new(2.0));
        assert_eq!(edge_speed(px(5_000.0), extent), cap);
        assert_eq!(edge_speed(px(-5_000.0), extent), -cap);
    }
}
