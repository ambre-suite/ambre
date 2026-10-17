//! The one kinetic scroll every application shares
//! (`1-Foundations/ambre-spec-kinetic-scroll.md`).
//!
//! Pure arithmetic over timed pointer positions: no DOM, no GPU, no
//! application state. The model hands back distances in screen pixels. Each
//! application moves its own view by them: a column of pages, a grid of
//! cells, a board under a zoom. A trackpad glide that stops keeps its
//! momentum; a wheel notch never does (`KINETIC-002`, `KINETIC-009`).

/// How far a pointer may move, in screen pixels, and still be a tap.
pub const TAP_SLOP_PX: f32 = 8.0;
/// The window of samples the release speed is read over, in milliseconds.
pub const SAMPLE_MS: f64 = 100.0;
/// The time constant of the slowdown, in milliseconds: the speed falls to
/// about a third of itself over this time.
pub const DECAY_MS: f32 = 325.0;
/// Below this speed, in screen pixels per millisecond, a release leaves no
/// motion and a motion stops.
pub const STOP_SPEED: f32 = 0.02;
/// The longest step one tick takes, so a stalled frame does not jump. It is
/// private: a step is timed here alone (`KINETIC-013`).
const MAX_STEP_MS: f64 = 50.0;

/// A pixel wheel step at or past this, in screen pixels, is a notch.
/// Trackpad glides stay well under it; a notch sits well over it.
pub const WHEEL_NOTCH_PX: f32 = 40.0;
/// A pause between wheel events this long, in milliseconds, ends a glide.
pub const WHEEL_GAP_MS: f64 = 60.0;
/// A glide slower than this at its end, in screen pixels per millisecond,
/// leaves no motion.
pub const WHEEL_START_SPEED: f32 = 0.05;
/// The weight of the newest wheel sample in the glide speed.
pub const WHEEL_BLEND: f32 = 0.8;

/// How far from an edge a drag starts to scroll, in screen pixels.
pub const EDGE_BAND_PX: f32 = 32.0;
/// How deep past the band the edge speed still grows, in screen pixels.
pub const EDGE_DEPTH_PX: f32 = 160.0;
/// Screen pixels per millisecond for each pixel of depth: 2 px/ms at most.
pub const EDGE_GAIN: f32 = 1.0 / 80.0;

/// How far the view moves, in screen pixels on each axis (`KINETIC-013`).
///
/// Only this model builds one, so an application reads a step and never
/// computes its own. A view's scroll by a pan, a flick, a glide or an edge
/// takes one. A notch is a step the model hands back one for one:
///
/// ```
/// use ambre_draw::kinetic::Kinetic;
/// let step = Kinetic::default().wheel(0.0, 30.0, true, 0.0);
/// assert_eq!(step.y(), 30.0);
/// ```
///
/// Each block below fails on the step it builds outside the model alone:
///
/// ```compile_fail
/// let step = ambre_draw::kinetic::ScrollStep { x: 0.0, y: 30.0 };
/// ```
///
/// ```compile_fail
/// let step = ambre_draw::kinetic::ScrollStep::ZERO;
/// ```
///
/// ```compile_fail
/// let clamp = ambre_draw::kinetic::MAX_STEP_MS;
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollStep {
    x: f32,
    y: f32,
}

impl ScrollStep {
    const ZERO: Self = Self { x: 0.0, y: 0.0 };

    /// The step along the horizontal axis.
    pub fn x(self) -> f32 {
        self.x
    }

    /// The step along the vertical axis.
    pub fn y(self) -> f32 {
        self.y
    }

    /// The step of a pointer dragged from `from` to `to`, one for one, the
    /// content going with the pointer. It has no slop and leaves no motion,
    /// which is what a mouse held on the board asks for (`KINETIC-001`).
    pub fn dragged(from: (f32, f32), to: (f32, f32)) -> Self {
        Self {
            x: from.0 - to.0,
            y: from.1 - to.1,
        }
    }
}

/// The speed a drag held near an edge asks for, in screen pixels per
/// millisecond, towards the far end for a positive one (`KINETIC-007`).
/// Only [`EdgeSpeed::at`] builds a speed other than none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeSpeed(f32);

impl EdgeSpeed {
    /// No edge scroll on this axis.
    pub const NONE: Self = Self(0.0);

    /// The speed of a pointer at `at` along one axis, in a view of `extent`.
    /// Zero away from both edges.
    pub fn at(at: f32, extent: f32) -> Self {
        Self(edge_speed(at, extent))
    }

    /// The speed, for a reader.
    pub fn get(self) -> f32 {
        self.0
    }
}

/// The edge scroll of a held drag: its clock, and the step each frame takes
/// (`KINETIC-007`).
#[derive(Clone, Debug, Default)]
pub struct EdgeScroll {
    last_ms: Option<f64>,
}

impl EdgeScroll {
    /// One frame of the edge scroll. No speed, or none on either axis, ends
    /// it. The first frame only starts the clock.
    pub fn tick(
        &mut self,
        speed: Option<(EdgeSpeed, EdgeSpeed)>,
        now_ms: f64,
    ) -> Option<ScrollStep> {
        let still = (EdgeSpeed::NONE, EdgeSpeed::NONE);
        let Some((sx, sy)) = speed.filter(|&s| s != still) else {
            self.last_ms = None;
            return None;
        };
        let last = self.last_ms.replace(now_ms)?;
        let elapsed = (now_ms - last).clamp(0.0, MAX_STEP_MS) as f32;
        Some(ScrollStep {
            x: sx.0 * elapsed,
            y: sy.0 * elapsed,
        })
    }

    /// End the edge scroll: the drag ended.
    pub fn stop(&mut self) {
        self.last_ms = None;
    }
}

/// What a release leaves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Release {
    /// The pointer never left the slop: a tap at its press point
    /// (`KINETIC-003`).
    Tap { x: f32, y: f32 },
    /// A pan released with speed: the view keeps moving (`KINETIC-004`).
    Fling,
    /// A pan released at rest, or nothing pressed.
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
    speed: (f32, f32),
    last_ms: f64,
}

#[derive(Clone, Debug)]
struct Pan {
    start: (f32, f32),
    last: (f32, f32),
    panned: bool,
    /// Recent positions and their times, for the release speed.
    samples: Vec<(f64, f32, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Motion {
    /// Screen pixels per millisecond, on each axis.
    speed: (f32, f32),
    last_ms: Option<f64>,
}

impl Kinetic {
    /// A pointer went down. It stops any motion left (`KINETIC-006`).
    pub fn press(&mut self, x: f32, y: f32, now_ms: f64) {
        self.motion = None;
        self.glide = None;
        self.pressed = Some(Pan {
            start: (x, y),
            last: (x, y),
            panned: false,
            samples: vec![(now_ms, x, y)],
        });
    }

    /// The pointer moved. Inside the slop it moves nothing (`KINETIC-003`).
    /// Past it, the answer is how far the view follows, in screen pixels.
    /// The content moves with the pointer, so it is the pointer's step
    /// reversed.
    pub fn sample(&mut self, x: f32, y: f32, now_ms: f64) -> Option<ScrollStep> {
        let pan = self.pressed.as_mut()?;
        if !pan.panned {
            if (x - pan.start.0).hypot(y - pan.start.1) < TAP_SLOP_PX {
                return None;
            }
            pan.panned = true;
        }
        let step = ScrollStep {
            x: pan.last.0 - x,
            y: pan.last.1 - y,
        };
        pan.last = (x, y);
        pan.samples.push((now_ms, x, y));
        pan.samples.retain(|(at, _, _)| now_ms - at <= SAMPLE_MS);
        Some(step)
    }

    /// The pointer lifted. A pan released with speed leaves a motion, read
    /// over the last window of samples (`KINETIC-004`).
    pub fn release(&mut self, now_ms: f64) -> Release {
        let Some(pan) = self.pressed.take() else {
            return Release::Rest;
        };
        if !pan.panned {
            return Release::Tap {
                x: pan.start.0,
                y: pan.start.1,
            };
        }
        let mut recent = pan
            .samples
            .iter()
            .filter(|(at, _, _)| now_ms - at <= SAMPLE_MS);
        let (Some(first), Some(last)) = (recent.next(), recent.next_back()) else {
            return Release::Rest;
        };
        let span = (last.0 - first.0) as f32;
        if span <= 0.0 {
            return Release::Rest;
        }
        let speed = ((first.1 - last.1) / span, (first.2 - last.2) / span);
        if speed.0.hypot(speed.1) < STOP_SPEED {
            return Release::Rest;
        }
        self.motion = Some(Motion {
            speed,
            last_ms: None,
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
    /// pauses (`KINETIC-002`, `KINETIC-009`). `stepped` marks a line or page delta mode.
    pub fn wheel(&mut self, dx: f32, dy: f32, stepped: bool, now_ms: f64) -> ScrollStep {
        self.motion = None;
        let step = ScrollStep { x: dx, y: dy };
        if stepped || dx.abs() >= WHEEL_NOTCH_PX || dy.abs() >= WHEEL_NOTCH_PX {
            self.glide = None;
            return step;
        }
        let speed = match self.glide {
            Some(glide) if now_ms - glide.last_ms <= WHEEL_GAP_MS => {
                let dt = (now_ms - glide.last_ms).clamp(1.0, 100.0) as f32;
                let b = WHEEL_BLEND;
                (
                    b * dx / dt + (1.0 - b) * glide.speed.0,
                    b * dy / dt + (1.0 - b) * glide.speed.1,
                )
            }
            _ => (dx / 16.0, dy / 16.0),
        };
        self.glide = Some(Glide {
            speed,
            last_ms: now_ms,
        });
        step
    }

    /// One tick of the motion, once a frame. The answer is how far the view
    /// moves, each tick less than the one before; the first tick only starts
    /// the clock (`KINETIC-005`). The motion stops below the stop speed.
    /// A glide that paused past the gap launches its motion here.
    pub fn tick(&mut self, now_ms: f64) -> Option<ScrollStep> {
        if self.motion.is_none() {
            let glide = self.glide?;
            if now_ms - glide.last_ms < WHEEL_GAP_MS {
                return Some(ScrollStep::ZERO);
            }
            self.glide = None;
            if glide.speed.0.hypot(glide.speed.1) < WHEEL_START_SPEED {
                return None;
            }
            self.motion = Some(Motion {
                speed: glide.speed,
                last_ms: Some(now_ms),
            });
            return Some(ScrollStep::ZERO);
        }
        let motion = self.motion.as_mut()?;
        let Some(last) = motion.last_ms.replace(now_ms) else {
            return Some(ScrollStep::ZERO);
        };
        let elapsed = (now_ms - last).clamp(0.0, MAX_STEP_MS) as f32;
        let step = ScrollStep {
            x: motion.speed.0 * elapsed,
            y: motion.speed.1 * elapsed,
        };
        let decay = (-elapsed / DECAY_MS).exp();
        motion.speed = (motion.speed.0 * decay, motion.speed.1 * decay);
        if motion.speed.0.hypot(motion.speed.1) < STOP_SPEED {
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
    pub fn speed(&self) -> Option<(f32, f32)> {
        self.motion.map(|motion| motion.speed)
    }

    /// Whether a pointer is down.
    pub fn pressed(&self) -> bool {
        self.pressed.is_some()
    }
}

/// The speed a drag held near an edge scrolls at, in screen pixels per
/// millisecond, towards the far end for a positive one (`KINETIC-007`).
///
/// `at` is the pointer along one axis, and `extent` the length of the view
/// on it. The speed is zero away from both edges. It grows with how deep
/// the pointer sits in the band or past it, and stops growing further out.
fn edge_speed(at: f32, extent: f32) -> f32 {
    let far = extent - EDGE_BAND_PX;
    let depth = if at < EDGE_BAND_PX {
        at - EDGE_BAND_PX
    } else if at > far {
        at - far
    } else {
        0.0
    };
    depth.clamp(-EDGE_DEPTH_PX, EDGE_DEPTH_PX) * EDGE_GAIN
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pan up at 2 px/ms on y, 1 px/ms on x, and lift.
    fn flicked() -> Kinetic {
        let mut kinetic = Kinetic::default();
        kinetic.press(100.0, 600.0, 0.0);
        for step in 1..=5 {
            let t = 16.0 * f64::from(step);
            kinetic.sample(100.0 - 16.0 * step as f32, 600.0 - 32.0 * step as f32, t);
        }
        assert_eq!(kinetic.release(80.0), Release::Fling);
        kinetic
    }

    /// `KINETIC-003`: a pointer that stays inside the slop is a tap and
    /// moves nothing.
    #[test]
    fn a_pointer_inside_the_slop_is_a_tap_and_moves_nothing() {
        let mut kinetic = Kinetic::default();
        kinetic.press(10.0, 20.0, 0.0);
        assert_eq!(kinetic.sample(13.0, 24.0, 10.0), None);
        assert_eq!(kinetic.release(20.0), Release::Tap { x: 10.0, y: 20.0 });
        assert!(!kinetic.moving());
        assert_eq!(kinetic.tick(36.0), None);
    }

    /// `KINETIC-001`: a drag moves the view as a pan past the slop does, the
    /// content going with the pointer.
    #[test]
    fn a_drag_moves_the_view_as_a_pan_past_the_slop_does() {
        let mut kinetic = Kinetic::default();
        kinetic.press(0.0, 0.0, 0.0);
        let panned = kinetic.sample(-12.0, 30.0, 16.0);
        let dragged = ScrollStep::dragged((0.0, 0.0), (-12.0, 30.0));
        assert_eq!(panned, Some(dragged));
        assert_eq!((dragged.x(), dragged.y()), (12.0, -30.0));
    }

    /// `KINETIC-004`: a pan released carries on at the speed of its last
    /// window, on both axes; one released at rest leaves nothing.
    #[test]
    fn a_pan_released_carries_on_at_the_speed_of_its_last_window() {
        let kinetic = flicked();
        let (sx, sy) = kinetic.speed().expect("a motion");
        assert!(
            (sx - 1.0).abs() < 1e-3 && (sy - 2.0).abs() < 1e-3,
            "{sx} {sy}"
        );
        // The view follows the pointer while it pans.
        let mut kinetic = Kinetic::default();
        kinetic.press(0.0, 0.0, 0.0);
        let step = ScrollStep { x: 0.0, y: 20.0 };
        assert_eq!(kinetic.sample(0.0, -20.0, 16.0), Some(step));
        // Held still for longer than the window, then lifted: no motion.
        assert_eq!(kinetic.release(400.0), Release::Rest);
        assert!(!kinetic.moving());
    }

    /// `KINETIC-005`: each tick moves less than the one before, and the
    /// motion stops below the stop speed.
    #[test]
    fn each_tick_moves_less_and_the_motion_stops() {
        let mut kinetic = flicked();
        assert_eq!(kinetic.tick(80.0), Some(ScrollStep::ZERO));
        let (mut now, mut last) = (80.0, f32::INFINITY);
        while let Some(step) = kinetic.tick(now + 16.0) {
            let dy = step.y();
            now += 16.0;
            assert!(dy > 0.0 && dy < last, "{dy} after {last}");
            last = dy;
            assert!(now < 10_000.0, "the motion stops");
        }
        assert!(!kinetic.moving());
        // A stalled frame takes one bounded step.
        let mut kinetic = flicked();
        kinetic.tick(80.0);
        let dy = kinetic.tick(5_080.0).expect("a step").y();
        assert!(dy <= 2.0 * MAX_STEP_MS as f32 + 1e-3);
    }

    /// `KINETIC-006`: a press or an edge the application met stops the
    /// motion, and no tick moves after.
    #[test]
    fn a_press_or_an_edge_stops_the_motion() {
        let mut kinetic = flicked();
        kinetic.tick(80.0);
        kinetic.press(0.0, 0.0, 90.0);
        assert!(!kinetic.moving());
        assert_eq!(kinetic.tick(106.0), None);
        let mut kinetic = flicked();
        kinetic.stop();
        assert_eq!(kinetic.tick(96.0), None);
    }

    /// `KINETIC-002`, `KINETIC-009`: a glide that pauses carries on; a notch never starts a
    /// motion and stops one under way.
    #[test]
    fn a_glide_that_pauses_carries_on_and_a_notch_never_does() {
        let mut kinetic = Kinetic::default();
        for step in 0..6 {
            let t = 8.0 * f64::from(step);
            let step = ScrollStep { x: 0.0, y: 12.0 };
            assert_eq!(kinetic.wheel(0.0, 12.0, false, t), step);
        }
        // Inside the gap the glide waits; past it the motion starts.
        assert_eq!(kinetic.tick(60.0), Some(ScrollStep::ZERO));
        assert_eq!(kinetic.tick(110.0), Some(ScrollStep::ZERO));
        let dy = kinetic.tick(126.0).expect("a motion").y();
        assert!(dy > 0.0, "{dy}");
        // A notch stops it, moves one for one, and leaves nothing behind.
        let notch = ScrollStep { x: 0.0, y: 100.0 };
        assert_eq!(kinetic.wheel(0.0, 100.0, false, 130.0), notch);
        assert!(!kinetic.moving());
        assert_eq!(kinetic.tick(300.0), None);
        kinetic.wheel(0.0, 3.0, true, 400.0);
        assert_eq!(kinetic.tick(600.0), None);
        // A glide that dies out slowly leaves no motion.
        kinetic.wheel(0.0, 0.5, false, 700.0);
        assert_eq!(kinetic.tick(800.0), None);
    }

    /// `KINETIC-010`: a cancelled pan leaves no tap and no motion, even one
    /// that moved fast.
    #[test]
    fn a_cancelled_pan_leaves_no_tap_and_no_motion() {
        let mut kinetic = Kinetic::default();
        kinetic.press(100.0, 600.0, 0.0);
        for step in 1..=5 {
            let t = 16.0 * f64::from(step);
            kinetic.sample(100.0, 600.0 - 32.0 * step as f32, t);
        }
        kinetic.cancel();
        assert!(!kinetic.pressed() && !kinetic.moving());
        assert_eq!(kinetic.release(90.0), Release::Rest);
        assert_eq!(kinetic.tick(96.0), None);
        // A tap cancelled before it lifts is no tap either.
        kinetic.press(10.0, 10.0, 200.0);
        kinetic.cancel();
        assert_eq!(kinetic.release(210.0), Release::Rest);
    }

    /// `KINETIC-007`: the edge speed is zero away from the edges, grows with
    /// depth, and is capped.
    #[test]
    fn the_edge_speed_grows_with_depth_and_is_capped() {
        assert_eq!(edge_speed(400.0, 900.0), 0.0);
        assert_eq!(edge_speed(EDGE_BAND_PX, 900.0), 0.0);
        let (near, deeper) = (edge_speed(880.0, 900.0), edge_speed(890.0, 900.0));
        assert!(0.0 < near && near < deeper);
        assert!(edge_speed(10.0, 900.0) < 0.0);
        let cap = EDGE_DEPTH_PX * EDGE_GAIN;
        assert_eq!(edge_speed(5_000.0, 900.0), cap);
        assert_eq!(edge_speed(-5_000.0, 900.0), -cap);
    }
}
