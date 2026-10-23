// ambre-trame/src/ui/view/scroll.rs
// The grid's scroll motion: wheel steps, glides, finger pans and the edge
// scroll, folded into whole-cell pans of the viewport.

use ambre_canvas::WheelDelta;
use ambre_draw::kinetic::{EdgeScroll, EdgeSpeed, Kinetic, Release, ScrollStep};

use super::ScrollOutcome;
use crate::ui::grid::{COL_WIDTH, ROW_HEIGHT, TrameUiState};
use crate::{ColDelta, RowDelta};

/// The scroll motion of the grid, apart from the browser.
///
/// Every entry takes the time as an argument, so a native test drives it with
/// the clock it chooses, as Prose's view state is driven. The view owns one and
/// reads `performance.now()` for it.
#[derive(Default)]
pub(crate) struct GridScroll {
    /// Sub-cell residue (logical px) carried between events, so slow touchpad
    /// scrolls still accumulate into whole rows and columns.
    accum_x: f64,
    accum_y: f64,
    /// The shared kinetic scroll (`1-Foundations/ambre-spec-kinetic-scroll.md`).
    kinetic: Kinetic,
    /// The edge scroll of a held drag (`KINETIC-007`).
    edge: EdgeScroll,
}

impl GridScroll {
    /// A wheel event. Pixel deltas accumulate sub-cell residue; line and page
    /// deltas are read as pixels first (`KINETIC-015`).
    pub(crate) fn wheel(
        &mut self,
        grid: &mut TrameUiState,
        delta: WheelDelta,
        now_ms: ambre_core::PageTime,
    ) -> ScrollOutcome {
        // A line step is one zoomed cell, so it scales with the grid zoom.
        let z = grid.zoom.get();
        let line = (
            f64::from(COL_WIDTH.get()) * z,
            f64::from(ROW_HEIGHT.get()) * z,
        );
        let page = (grid.canvas_width, grid.canvas_height);
        let (dx, dy) = delta.to_px(line, page);
        // The shared model applies the step one for one, and lets a trackpad
        // glide coast once it pauses; a notch never coasts (`KINETIC-002`,
        // `KINETIC-009`).
        let step = self
            .kinetic
            .wheel(dx as f32, dy as f32, delta.is_step(), now_ms);
        self.accumulate(step);
        let outcome = self.apply(grid);
        if matches!(outcome, ScrollOutcome::Edge) {
            self.kinetic.stop();
        }
        outcome
    }

    /// Add a step of the model to the sub-cell residue.
    fn accumulate(&mut self, step: ScrollStep) {
        self.accum_x += f64::from(step.x());
        self.accum_y += f64::from(step.y());
    }

    /// Peel whole rows and columns out of the accumulator and pan the grid,
    /// keeping the sub-cell remainder.
    fn apply(&mut self, grid: &mut TrameUiState) -> ScrollOutcome {
        let step_x = f64::from(COL_WIDTH.get()) * grid.zoom.get();
        let step_y = f64::from(ROW_HEIGHT.get()) * grid.zoom.get();
        let dc = (self.accum_x / step_x) as i32;
        let dr = (self.accum_y / step_y) as i32;
        if dr == 0 && dc == 0 {
            return ScrollOutcome::Idle;
        }
        self.accum_x -= f64::from(dc) * step_x;
        self.accum_y -= f64::from(dr) * step_y;
        if grid.scroll_by(RowDelta::new(dr.into()), ColDelta::new(dc.into())) {
            ScrollOutcome::Moved
        } else {
            // Pinned against a grid border: the motion had nowhere to go.
            ScrollOutcome::Edge
        }
    }

    /// Advance the kinetic motion by one frame. It stops once it dies down or
    /// the grid meets an edge.
    pub(crate) fn tick(
        &mut self,
        grid: &mut TrameUiState,
        now_ms: ambre_core::PageTime,
    ) -> ScrollOutcome {
        let Some(step) = self.kinetic.tick(now_ms) else {
            return ScrollOutcome::Idle;
        };
        self.accumulate(step);
        let outcome = self.apply(grid);
        if matches!(outcome, ScrollOutcome::Edge) {
            self.stop();
        }
        outcome
    }

    /// Scroll a held drag at `speed`, once a frame (`KINETIC-007`). `None`
    /// ends it; the first tick only starts the clock.
    pub(crate) fn edge(
        &mut self,
        grid: &mut TrameUiState,
        speed: Option<(EdgeSpeed, EdgeSpeed)>,
        now_ms: ambre_core::PageTime,
    ) -> ScrollOutcome {
        let Some(step) = self.edge.tick(speed, now_ms) else {
            return ScrollOutcome::Idle;
        };
        self.accumulate(step);
        self.apply(grid)
    }

    /// A finger went down: it stops any motion left (`KINETIC-006`).
    pub(crate) fn press(&mut self, x: f32, y: f32, now_ms: ambre_core::PageTime) {
        self.stop();
        self.kinetic.press(x, y, now_ms);
    }

    /// A finger moved: past the tap slop the grid follows it, cell by cell.
    pub(crate) fn pan(
        &mut self,
        grid: &mut TrameUiState,
        x: f32,
        y: f32,
        now_ms: ambre_core::PageTime,
    ) -> ScrollOutcome {
        let Some(step) = self.kinetic.sample(x, y, now_ms) else {
            return ScrollOutcome::Idle;
        };
        self.accumulate(step);
        self.apply(grid)
    }

    /// A finger lifted: a pan released with speed keeps moving.
    pub(crate) fn release(&mut self, now_ms: ambre_core::PageTime) -> Release {
        self.kinetic.release(now_ms)
    }

    /// The browser cancelled the finger: no click and no flick (`KINETIC-010`).
    pub(crate) fn cancel(&mut self) {
        self.kinetic.cancel();
    }

    /// Halt any kinetic motion and clear its residue.
    pub(crate) fn stop(&mut self) {
        self.kinetic.stop();
        self.accum_x = 0.0;
        self.accum_y = 0.0;
    }

    /// The speed of the motion in flight, px per ms, or `None` at rest.
    #[cfg(test)]
    pub(crate) fn speed(&self) -> Option<(f32, f32)> {
        self.kinetic.speed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> TrameUiState {
        TrameUiState::new(1200.0, 800.0)
    }

    /// `KINETIC-002`, `KINETIC-009`: a trackpad glide that pauses keeps the
    /// grid moving at the glide's speed, and a notch of the same size never
    /// does.
    #[test]
    fn a_paused_glide_keeps_the_grid_moving_and_a_notch_never_does() {
        let (mut g, mut s) = (grid(), GridScroll::default());
        for step in 0..5 {
            s.wheel(
                &mut g,
                WheelDelta::pixels(0.0, 16.0),
                ambre_bus::PageTime(16.0 * f64::from(step)),
            );
        }
        let glided = g.top_row;
        s.tick(&mut g, ambre_bus::PageTime(64.0));
        assert_eq!(s.speed(), None, "no motion while the glide lasts");
        // The frame past the pause launches the motion, and later ones move.
        s.tick(&mut g, ambre_bus::PageTime(64.0 + 70.0));
        let (_, vy) = s.speed().expect("the paused glide carries on");
        assert!((vy - 1.0).abs() < 0.05, "the glide's speed, {vy}");
        let mut now = 64.0 + 70.0;
        while g.top_row == glided {
            now += 16.0;
            s.tick(&mut g, ambre_bus::PageTime(now));
            assert!(now < 2_000.0, "the coast reaches a whole row");
        }

        let (mut g, mut s) = (grid(), GridScroll::default());
        for step in 0..5 {
            s.wheel(
                &mut g,
                WheelDelta::new(0.0, 16.0, ambre_canvas::WheelDeltaMode::Line),
                ambre_bus::PageTime(16.0 * f64::from(step)),
            );
        }
        let stepped = g.top_row;
        assert!(
            matches!(
                s.tick(&mut g, ambre_bus::PageTime(64.0 + 70.0)),
                ScrollOutcome::Idle
            ),
            "a notch never coasts"
        );
        assert_eq!(g.top_row, stepped);
        assert_eq!(s.speed(), None);
    }
}
