//! A finger drag scrolls the column, and a flick keeps it moving
//! (`features/ambre-spec-prose-viewport.md section 3`).
//!
//! The gesture is the shared kinetic model
//! (`1-Foundations/ambre-spec-kinetic-scroll.md`): the column takes its
//! vertical distance, and meeting an end of the column stops it.

use ambre_draw::kinetic::Release;

use crate::ui::ProseUiState;

impl ProseUiState {
    /// A finger went down. It stops any motion left (`PROSE-UI-218`).
    pub fn touch_start(&mut self, x: f32, y: f32, now_ms: f64) {
        self.kinetic.press(x, y, now_ms);
    }

    /// A finger moved: past the tap slop the column follows it. Answers
    /// whether the view moved.
    pub fn touch_move(&mut self, x: f32, y: f32, now_ms: f64) -> bool {
        let Some((_, dy)) = self.kinetic.sample(x, y, now_ms) else {
            return false;
        };
        self.scroll_by(dy);
        true
    }

    /// A finger lifted. A pan released with speed leaves a flick
    /// (`PROSE-UI-217`). A finger that never left the slop is a tap: its
    /// point comes back for the host to press there.
    pub fn touch_end(&mut self, now_ms: f64) -> Option<(f32, f32)> {
        match self.kinetic.release(now_ms) {
            Release::Tap { x, y } => Some((x, y)),
            Release::Fling | Release::Rest => None,
        }
    }

    /// The browser cancelled the finger: no tap and no flick (`KINETIC-010`).
    pub fn touch_cancel(&mut self) {
        self.kinetic.cancel();
    }

    /// A wheel event, in screen pixels: the column takes its step now. A
    /// trackpad glide that pauses keeps its momentum; a notch never does
    /// (`KINETIC-002`). `stepped` marks a line or page delta mode.
    pub fn wheel(&mut self, dy: f32, stepped: bool, now_ms: f64) {
        let (_, dy) = self.kinetic.wheel(0.0, dy, stepped, now_ms);
        self.scroll_by(dy);
    }

    /// Stop the motion a flick left (`PROSE-UI-218`).
    pub fn stop_fling(&mut self) {
        self.kinetic.stop();
    }

    /// One tick of a flick, once a frame (`PROSE-UI-217`). The speed falls
    /// each frame, and the motion stops once slow or at an end of the
    /// column. Nothing is sent. Answers whether the view moved.
    pub fn tick_fling(&mut self, now_ms: f64) -> bool {
        let Some((_, dy)) = self.kinetic.tick(now_ms) else {
            return false;
        };
        let before = self.viewport.scroll_y;
        self.scroll_by(dy);
        let moved = self.viewport.scroll_y != before;
        if !moved && dy != 0.0 {
            self.kinetic.stop();
        }
        moved
    }

    /// The vertical speed of the flick in motion, if any.
    pub fn fling_speed(&self) -> Option<f32> {
        self.kinetic.speed().map(|(_, dy)| dy)
    }
}
