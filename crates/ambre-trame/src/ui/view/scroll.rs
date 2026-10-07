// ambre-trame/src/ui/view/scroll.rs
// The grid's scroll motion: wheel steps, glides, finger pans and the edge
// scroll, folded into whole-cell pans of the viewport.

use ambre_draw::kinetic::{Kinetic, MAX_STEP_MS, Release};

use super::ScrollOutcome;
use crate::ui::grid::{COL_WIDTH, ROW_HEIGHT, TrameUiState};

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
    /// When the edge scroll last ticked (`KINETIC-007`).
    edge_ms: Option<f64>,
}

impl GridScroll {
    /// A wheel event. Pixel deltas accumulate sub-cell residue; line and page
    /// deltas (`delta_mode` 1 and 2) are normalised to pixels first.
    pub(crate) fn wheel(
        &mut self,
        grid: &mut TrameUiState,
        dx: f64,
        dy: f64,
        delta_mode: u32,
        now_ms: f64,
    ) -> ScrollOutcome {
        // A line step is one zoomed cell, so it scales with the grid zoom.
        let z = grid.zoom;
        let (sx, sy) = match delta_mode {
            1 => (COL_WIDTH * z, ROW_HEIGHT * z),
            2 => (grid.canvas_width, grid.canvas_height),
            _ => (1.0, 1.0),
        };
        // The shared model applies the step one for one, and lets a trackpad
        // glide coast once it pauses; a notch never coasts (`KINETIC-002`,
        // `KINETIC-009`).
        let (px, py) =
            self.kinetic
                .wheel((dx * sx) as f32, (dy * sy) as f32, delta_mode != 0, now_ms);
        self.accum_x += f64::from(px);
        self.accum_y += f64::from(py);
        let outcome = self.apply(grid);
        if matches!(outcome, ScrollOutcome::Edge) {
            self.kinetic.stop();
        }
        outcome
    }

    /// Peel whole rows and columns out of the accumulator and pan the grid,
    /// keeping the sub-cell remainder.
    fn apply(&mut self, grid: &mut TrameUiState) -> ScrollOutcome {
        let step_x = COL_WIDTH * grid.zoom;
        let step_y = ROW_HEIGHT * grid.zoom;
        let dc = (self.accum_x / step_x) as i32;
        let dr = (self.accum_y / step_y) as i32;
        if dr == 0 && dc == 0 {
            return ScrollOutcome::Idle;
        }
        self.accum_x -= f64::from(dc) * step_x;
        self.accum_y -= f64::from(dr) * step_y;
        if grid.scroll_by(dr, dc) {
            ScrollOutcome::Moved
        } else {
            // Pinned against a grid border: the motion had nowhere to go.
            ScrollOutcome::Edge
        }
    }

    /// Advance the kinetic motion by one frame. It stops once it dies down or
    /// the grid meets an edge.
    pub(crate) fn tick(&mut self, grid: &mut TrameUiState, now_ms: f64) -> ScrollOutcome {
        let Some((dx, dy)) = self.kinetic.tick(now_ms) else {
            return ScrollOutcome::Idle;
        };
        self.accum_x += f64::from(dx);
        self.accum_y += f64::from(dy);
        let outcome = self.apply(grid);
        if matches!(outcome, ScrollOutcome::Edge) {
            self.stop();
        }
        outcome
    }

    /// Scroll a held drag at `speed` (px per ms), once a frame
    /// (`KINETIC-007`). `None` ends it; the first tick only starts the clock.
    pub(crate) fn edge(
        &mut self,
        grid: &mut TrameUiState,
        speed: Option<(f64, f64)>,
        now_ms: f64,
    ) -> ScrollOutcome {
        let Some((sx, sy)) = speed else {
            self.edge_ms = None;
            return ScrollOutcome::Idle;
        };
        let Some(last) = self.edge_ms.replace(now_ms) else {
            return ScrollOutcome::Idle;
        };
        let dt = (now_ms - last).clamp(0.0, MAX_STEP_MS);
        self.accum_x += sx * dt;
        self.accum_y += sy * dt;
        self.apply(grid)
    }

    /// A finger went down: it stops any motion left (`KINETIC-006`).
    pub(crate) fn press(&mut self, x: f32, y: f32, now_ms: f64) {
        self.stop();
        self.kinetic.press(x, y, now_ms);
    }

    /// A finger moved: past the tap slop the grid follows it, cell by cell.
    pub(crate) fn pan(
        &mut self,
        grid: &mut TrameUiState,
        x: f32,
        y: f32,
        now_ms: f64,
    ) -> ScrollOutcome {
        let Some((dx, dy)) = self.kinetic.sample(x, y, now_ms) else {
            return ScrollOutcome::Idle;
        };
        self.accum_x += f64::from(dx);
        self.accum_y += f64::from(dy);
        self.apply(grid)
    }

    /// A finger lifted: a pan released with speed keeps moving.
    pub(crate) fn release(&mut self, now_ms: f64) -> Release {
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
            s.wheel(&mut g, 0.0, 16.0, 0, 16.0 * f64::from(step));
        }
        let glided = g.top_row;
        s.tick(&mut g, 64.0);
        assert_eq!(s.speed(), None, "no motion while the glide lasts");
        // The frame past the pause launches the motion, and later ones move.
        s.tick(&mut g, 64.0 + 70.0);
        let (_, vy) = s.speed().expect("the paused glide carries on");
        assert!((vy - 1.0).abs() < 0.05, "the glide's speed, {vy}");
        let mut now = 64.0 + 70.0;
        while g.top_row == glided {
            now += 16.0;
            s.tick(&mut g, now);
            assert!(now < 2_000.0, "the coast reaches a whole row");
        }

        let (mut g, mut s) = (grid(), GridScroll::default());
        for step in 0..5 {
            s.wheel(&mut g, 0.0, 16.0, 1, 16.0 * f64::from(step));
        }
        let stepped = g.top_row;
        assert!(
            matches!(s.tick(&mut g, 64.0 + 70.0), ScrollOutcome::Idle),
            "a notch never coasts"
        );
        assert_eq!(g.top_row, stepped);
        assert_eq!(s.speed(), None);
    }
}
