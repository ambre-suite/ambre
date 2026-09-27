//! A finger drag scrolls the column, and a flick keeps it moving
//! (`features/ambre-spec-prose-viewport.md section 3`).
//!
//! A trackpad's momentum reaches the view as wheel events the platform
//! already slows down, so only a touch pointer starts a flick here.

use crate::ui::ProseUiState;

/// How far a finger may move, in screen pixels, and still be a tap.
const TAP_SLOP_PX: f32 = 8.0;
/// The window of samples the release speed is read over, in milliseconds.
const SAMPLE_MS: f64 = 100.0;
/// The time constant of the slowdown, in milliseconds: the speed falls to
/// about a third of itself over this time.
const DECAY_MS: f32 = 325.0;
/// Below this speed, in screen pixels per millisecond, the motion stops.
const STOP_SPEED: f32 = 0.02;
/// The longest step one tick takes, so a stalled frame does not jump.
const MAX_STEP_MS: f64 = 50.0;

/// A finger on the canvas.
#[derive(Clone, Debug, Default)]
pub struct Touch {
    /// Where it went down, to tell a tap from a pan.
    start: (f32, f32),
    last_y: f32,
    panned: bool,
    /// Recent positions and their times, for the release speed.
    samples: Vec<(f64, f32)>,
}

/// The motion a flick left: a speed and the time of the last tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fling {
    /// Screen pixels per millisecond, down for a positive one.
    speed: f32,
    last_ms: Option<f64>,
}

impl ProseUiState {
    /// A finger went down. It stops any motion left (`PROSE-UI-218`).
    pub fn touch_start(&mut self, x: f32, y: f32, now_ms: f64) {
        self.stop_fling();
        self.touch = Some(Touch {
            start: (x, y),
            last_y: y,
            panned: false,
            samples: vec![(now_ms, y)],
        });
    }

    /// A finger moved: past the tap slop the column follows it. Answers
    /// whether the view moved.
    pub fn touch_move(&mut self, x: f32, y: f32, now_ms: f64) -> bool {
        let Some(touch) = &mut self.touch else {
            return false;
        };
        if !touch.panned {
            let (dx, dy) = (x - touch.start.0, y - touch.start.1);
            if dx.hypot(dy) < TAP_SLOP_PX {
                return false;
            }
            touch.panned = true;
        }
        let dy = touch.last_y - y;
        touch.last_y = y;
        touch.samples.push((now_ms, y));
        touch.samples.retain(|(at, _)| now_ms - at <= SAMPLE_MS);
        self.scroll_by(dy);
        true
    }

    /// A finger lifted. A pan released with speed leaves a flick
    /// (`PROSE-UI-217`). A finger that never left the slop is a tap: its
    /// point comes back for the host to press there.
    pub fn touch_end(&mut self, now_ms: f64) -> Option<(f32, f32)> {
        let touch = self.touch.take()?;
        if !touch.panned {
            return Some(touch.start);
        }
        let recent: Vec<_> = touch
            .samples
            .iter()
            .filter(|(at, _)| now_ms - at <= SAMPLE_MS)
            .collect();
        if let (Some(first), Some(last)) = (recent.first(), recent.last()) {
            let span = (last.0 - first.0) as f32;
            if span > 0.0 {
                // The finger moves the text: down the screen scrolls up.
                let speed = (first.1 - last.1) / span;
                if speed.abs() >= STOP_SPEED {
                    self.fling = Some(Fling {
                        speed,
                        last_ms: None,
                    });
                }
            }
        }
        None
    }

    /// Stop the motion a flick left (`PROSE-UI-218`).
    pub fn stop_fling(&mut self) {
        self.fling = None;
    }

    /// One tick of a flick, once a frame (`PROSE-UI-217`). The speed falls
    /// each frame, and the motion stops once slow or at an end of the
    /// column. Nothing is sent. Answers whether the view moved.
    pub fn tick_fling(&mut self, now_ms: f64) -> bool {
        let Some(fling) = &mut self.fling else {
            return false;
        };
        let Some(last) = fling.last_ms.replace(now_ms) else {
            return false;
        };
        let elapsed = (now_ms - last).clamp(0.0, MAX_STEP_MS) as f32;
        let step = fling.speed * elapsed;
        fling.speed *= (-elapsed / DECAY_MS).exp();
        let slow = fling.speed.abs() < STOP_SPEED;
        let before = self.viewport.scroll_y;
        self.scroll_by(step);
        let moved = self.viewport.scroll_y != before;
        if slow || !moved {
            self.fling = None;
        }
        moved
    }

    /// The speed of the flick in motion, if any.
    pub fn fling_speed(&self) -> Option<f32> {
        self.fling.map(|fling| fling.speed)
    }
}
