//! A finger drag scrolls the column, and a flick keeps it moving
//! (`features/ambre-spec-prose-viewport.md section 3`).
//!
//! The gesture is the shared kinetic model
//! (`1-Foundations/ambre-spec-kinetic-scroll.md`): the column takes its
//! vertical distance, and meeting an end of the column stops it.

use ambre_core::{Css, Point, Px, Vector, View};
use ambre_draw::kinetic::Release;
use ambre_draw::{WHEEL_LINE, WheelDelta};

use crate::ui::ProseUiState;

impl ProseUiState {
    /// A finger went down. It stops any motion left (`PROSE-UI-218`).
    pub fn touch_start(
        &mut self,
        at: Point<Px<View>>,
        now: ambre_core::Instant<ambre_core::PageLoad>,
    ) {
        self.kinetic.press(at, now);
    }

    /// A finger moved: past the tap slop the column follows it. Answers
    /// whether the view moved.
    pub fn touch_move(
        &mut self,
        at: Point<Px<View>>,
        now: ambre_core::Instant<ambre_core::PageLoad>,
    ) -> bool {
        let Some(step) = self.kinetic.sample(at, now) else {
            return false;
        };
        self.scroll_by(step);
        true
    }

    /// A finger lifted. A pan released with speed leaves a flick
    /// (`PROSE-UI-217`). A finger that never left the slop is a tap: its
    /// point comes back for the host to press there.
    pub fn touch_end(
        &mut self,
        now: ambre_core::Instant<ambre_core::PageLoad>,
    ) -> Option<Point<Px<View>>> {
        match self.kinetic.release(now) {
            Release::Tap { at } => Some(at),
            Release::Fling | Release::Rest => None,
        }
    }

    /// The browser cancelled the finger: no tap and no flick (`KINETIC-010`).
    pub fn touch_cancel(&mut self) {
        self.kinetic.cancel();
    }

    /// A wheel event: the column takes its step now. A line is
    /// [`WHEEL_LINE`] and a page is the viewport's height (`KINETIC-015`).
    /// A trackpad glide that pauses keeps its momentum; a notch never does
    /// (`KINETIC-002`).
    pub fn wheel(&mut self, delta: WheelDelta, now: ambre_core::Instant<ambre_core::PageLoad>) {
        let page = self.viewport.height.css_length();

        let line = Vector::new(WHEEL_LINE, WHEEL_LINE);
        let dy = delta.to_px(line, Vector::new(page, page)).dy;
        let step = self
            .kinetic
            .wheel(Vector::new(Px::<Css>::ZERO, dy), delta.mode(), now);
        self.scroll_by(step);
    }

    /// Stop the motion a flick left (`PROSE-UI-218`).
    pub fn stop_fling(&mut self) {
        self.kinetic.stop();
    }

    /// One tick of a flick, once a frame (`PROSE-UI-217`). The speed falls
    /// each frame, and the motion stops once slow or at an end of the
    /// column. Nothing is sent. Answers whether the view moved.
    pub fn tick_fling(&mut self, now: ambre_core::Instant<ambre_core::PageLoad>) -> bool {
        let Some(step) = self.kinetic.tick(now) else {
            return false;
        };
        let dy = step.y();
        let before = self.viewport.scroll_y;
        self.scroll_by(step);
        let moved = self.viewport.scroll_y != before;
        if !moved && dy != ambre_core::Px::<ambre_core::View>::ZERO {
            self.kinetic.stop();
        }
        moved
    }

    /// The vertical speed of the flick in motion, if any.
    pub fn fling_speed(&self) -> Option<ambre_core::ViewSpeed> {
        self.kinetic.speed().map(|speed| speed.dy)
    }
}
