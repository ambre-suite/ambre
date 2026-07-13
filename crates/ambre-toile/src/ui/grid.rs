// ambre-toile/src/ui/grid.rs
// ============================================================================
// Adaptive magnetic grid (toile-ui §3). The grid step is chosen so the on-screen
// spacing stays in a comfortable pixel band as `zoom` changes (powers of the
// board `grid_size`); `snap_world` quantizes a world point to the current step
// for drag / resize snapping. Step selection is pure and host-tested; the paint
// helper emits screen-space `stroke_line` calls through the abstract DrawSurface.
// ============================================================================

use ambre_canvas::Palette;
use ambre_draw::{DrawSurface, Point};

use super::viewport::ViewportTransform;

/// The default board grid unit in world units. A power-of-two multiple of this
/// is the smallest step the adaptive grid shows.
pub const BASE_GRID: f64 = 20.0;

/// Comfortable on-screen spacing band (physical pixels): the adaptive step keeps
/// the line spacing at or above `MIN_SPACING_PX`, doubling the world step until
/// it does.
const MIN_SPACING_PX: f64 = 24.0;
const MAX_SPACING_PX: f64 = 96.0;

/// Choose the grid step (world units) for `zoom`: the smallest power-of-two
/// multiple of `base` whose screen spacing (`step * zoom`) is at least
/// `MIN_SPACING_PX`, then halved back down while it stays above `MAX_SPACING_PX`
/// (never below `base`, so the grid stays magnetic to the board unit).
pub fn grid_step(zoom: f64, base: f64) -> f64 {
    debug_assert!(base > 0.0 && zoom > 0.0);
    let mut step = base;
    // Zoomed out: coarsen until the spacing is legible.
    while step * zoom < MIN_SPACING_PX {
        step *= 2.0;
    }
    // Zoomed in: refine toward the band, but never finer than `base`.
    while step > base && step * zoom > MAX_SPACING_PX {
        step /= 2.0;
    }
    step
}

/// Quantize a world point to the nearest multiple of `step` on each axis.
pub fn snap_world(p: Point, step: f64) -> Point {
    Point::new((p.x / step).round() * step, (p.y / step).round() * step)
}

/// Paint the board background and the adaptive grid lines for `cam`. Lines are
/// emitted in screen space (the surface transform stack is left untouched here so
/// the hairline width does not scale with zoom).
pub fn paint_grid<S: DrawSurface>(surface: &mut S, cam: &ViewportTransform, palette: &Palette) {
    // Board background fills the whole canvas.
    surface.fill_rect(
        ambre_draw::Rect::new(0.0, 0.0, cam.size.width, cam.size.height),
        palette.content_bg,
    );

    let step = grid_step(cam.zoom, BASE_GRID);
    let s2w = cam.screen_to_world();
    let w2s = cam.world_to_screen();
    // Visible world rect: the transform is translate+scale (axis-aligned), so the
    // top-left and bottom-right screen corners bound it.
    let tl = s2w * Point::new(0.0, 0.0);
    let br = s2w * Point::new(cam.size.width, cam.size.height);
    let (min_x, max_x) = (tl.x.min(br.x), tl.x.max(br.x));
    let (min_y, max_y) = (tl.y.min(br.y), tl.y.max(br.y));

    let color = palette.grid_line;
    // Vertical lines at each world-x multiple of `step`.
    let mut x = (min_x / step).ceil() * step;
    while x <= max_x {
        let sx = (w2s * Point::new(x, 0.0)).x;
        surface.stroke_line(sx, 0.0, sx, cam.size.height, color, 1.0);
        x += step;
    }
    // Horizontal lines at each world-y multiple of `step`.
    let mut y = (min_y / step).ceil() * step;
    while y <= max_y {
        let sy = (w2s * Point::new(0.0, y)).y;
        surface.stroke_line(0.0, sy, cam.size.width, sy, color, 1.0);
        y += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_never_illegible_across_zoom_sweep() {
        // For any zoom, the chosen step keeps the on-screen spacing at or above
        // the minimum legible band.
        let mut z = MIN_SPACING_PX / (BASE_GRID * 64.0); // very zoomed out
        while z <= MAX_ZOOM_SWEEP {
            let step = grid_step(z, BASE_GRID);
            let spacing = step * z;
            assert!(
                spacing >= MIN_SPACING_PX - 1e-9,
                "zoom {z}: spacing {spacing} below min"
            );
            z *= 1.3;
        }
    }

    // A zoom range wide enough to exercise several doublings without exceeding
    // the camera clamp.
    const MAX_ZOOM_SWEEP: f64 = 10.0;

    #[test]
    fn step_is_power_of_two_multiple_of_base() {
        for z in [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0] {
            let step = grid_step(z, BASE_GRID);
            let ratio = step / BASE_GRID;
            assert!(ratio >= 1.0 - 1e-9, "step below base at zoom {z}");
            // ratio must be a power of two.
            let log2 = ratio.log2();
            assert!(
                (log2 - log2.round()).abs() < 1e-9,
                "step ratio {ratio} not a power of two at zoom {z}"
            );
        }
    }

    #[test]
    fn snap_quantizes_to_step() {
        let s = snap_world(Point::new(23.0, -31.0), 20.0);
        assert_eq!(s, Point::new(20.0, -40.0));
    }
}
