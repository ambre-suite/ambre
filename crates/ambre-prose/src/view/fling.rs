//! A finger drag scrolls the column, and a flick keeps it moving
//! (`features/ambre-spec-prose-viewport.md section 3`).
//!
//! The gesture is the shared kinetic model
//! (`1-Foundations/ambre-spec-kinetic-scroll.md`): the column takes its
//! vertical distance, and meeting an end of the column stops it.

use ambre_core::{CssPx, CssVector, ViewPoint};
use ambre_draw::kinetic::Release;
use ambre_draw::{WHEEL_LINE_PX, WheelDelta};

use crate::ui::ProseUiState;

impl ProseUiState {
    /// A finger went down. It stops any motion left (`PROSE-UI-218`).
    pub fn touch_start(&mut self, at: ViewPoint, now_ms: ambre_core::PageTime) {
        self.kinetic.press(at, now_ms);
    }

    /// A finger moved: past the tap slop the column follows it. Answers
    /// whether the view moved.
    pub fn touch_move(&mut self, at: ViewPoint, now_ms: ambre_core::PageTime) -> bool {
        let Some(step) = self.kinetic.sample(at, now_ms) else {
            return false;
        };
        self.scroll_by(step);
        true
    }

    /// A finger lifted. A pan released with speed leaves a flick
    /// (`PROSE-UI-217`). A finger that never left the slop is a tap: its
    /// point comes back for the host to press there.
    pub fn touch_end(&mut self, now_ms: ambre_core::PageTime) -> Option<ViewPoint> {
        match self.kinetic.release(now_ms) {
            Release::Tap { at } => Some(at),
            Release::Fling | Release::Rest => None,
        }
    }

    /// The browser cancelled the finger: no tap and no flick (`KINETIC-010`).
    pub fn touch_cancel(&mut self) {
        self.kinetic.cancel();
    }

    /// A wheel event: the column takes its step now. A line is
    /// [`WHEEL_LINE_PX`] and a page is the viewport's height (`KINETIC-015`).
    /// A trackpad glide that pauses keeps its momentum; a notch never does
    /// (`KINETIC-002`).
    pub fn wheel(&mut self, delta: WheelDelta, now_ms: ambre_core::PageTime) {
        let page = CssPx::new(self.viewport.height_px.get());

        let line = CssVector::new(WHEEL_LINE_PX, WHEEL_LINE_PX);
        let dy = delta.to_px(line, CssVector::new(page, page)).dy;
        let step = self
            .kinetic
            .wheel(CssVector::new(CssPx::ZERO, dy), delta.mode(), now_ms);
        self.scroll_by(step);
    }

    /// Stop the motion a flick left (`PROSE-UI-218`).
    pub fn stop_fling(&mut self) {
        self.kinetic.stop();
    }

    /// One tick of a flick, once a frame (`PROSE-UI-217`). The speed falls
    /// each frame, and the motion stops once slow or at an end of the
    /// column. Nothing is sent. Answers whether the view moved.
    pub fn tick_fling(&mut self, now_ms: ambre_core::PageTime) -> bool {
        let Some(step) = self.kinetic.tick(now_ms) else {
            return false;
        };
        let dy = step.y();
        let before = self.viewport.scroll_y;
        self.scroll_by(step);
        let moved = self.viewport.scroll_y != before;
        if !moved && dy != ambre_core::ViewPx::ZERO {
            self.kinetic.stop();
        }
        moved
    }

    /// The vertical speed of the flick in motion, if any.
    pub fn fling_speed(&self) -> Option<f32> {
        self.kinetic.speed().map(|(_, dy)| dy)
    }
}
