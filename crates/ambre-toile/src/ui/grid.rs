// ambre-toile/src/ui/grid.rs
// ============================================================================
// Adaptive magnetic grid (toile-viewport). The grid step is chosen so the
// on-screen spacing stays in a comfortable pixel band as `zoom` changes (powers
// of the board `grid_size`). `snap_world` quantizes a world point to the current
// step for drag / resize snapping. Step selection is pure and host-tested; the
// paint helper emits screen-space `stroke_line` calls through the abstract
// DrawSurface.
// ============================================================================

use ambre_canvas::Palette;
use ambre_draw::{DrawSurface, Point};

use super::viewport::ViewportTransform;

/// The grid unit in world units of a board whose register holds no usable one.
/// A power-of-two multiple of the unit is the smallest step the adaptive grid
/// shows.
pub const BASE_GRID: f64 = 20.0;

/// The unit a board paints its grid on: its `board` register when that holds
/// a finite, positive length, else [`BASE_GRID`] (`TOILE-UI-279`). A peer can
/// write any number there, and a zero or a NaN would stall the step search.
pub fn board_unit(register: f64) -> f64 {
    if register.is_finite() && register > 0.0 {
        register
    } else {
        BASE_GRID
    }
}

/// Comfortable on-screen spacing band (logical pixels): the adaptive step keeps
/// the line spacing at or above `MIN_SPACING`, doubling the world step until
/// it does.
const MIN_SPACING: ambre_core::ViewPx = ambre_core::ViewPx::new(24.0);
const MAX_SPACING: ambre_core::ViewPx = ambre_core::ViewPx::new(96.0);

/// Choose the grid step (world units) for `zoom`: the smallest power-of-two
/// multiple of `base` whose screen spacing (`step * zoom`) is at least
/// `MIN_SPACING`. That step is then halved back down while it stays above
/// `MAX_SPACING`, never below `base`, so the grid stays magnetic to the board
/// unit.
pub fn grid_step(zoom: ambre_core::ZoomFactor, base: f64) -> f64 {
    let zoom = zoom.get();
    // The band, in the f64 of the board's screen geometry.
    let (min, max) = (f64::from(MIN_SPACING.get()), f64::from(MAX_SPACING.get()));
    debug_assert!(base > 0.0);
    let mut step = base;
    // Zoomed out: coarsen until the spacing is legible.
    while step * zoom < min {
        step *= 2.0;
    }
    // Zoomed in: refine toward the band, but never finer than `base`.
    while step > base && step * zoom > max {
        step /= 2.0;
    }
    step
}

/// Quantize a world point to the nearest multiple of `step` on each axis.
pub fn snap_world(p: Point, step: f64) -> Point {
    Point::new((p.x / step).round() * step, (p.y / step).round() * step)
}

/// Fill the board background only, without grid lines. Used by presentation
/// focus mode (`toile-presentation`), which hides the grid so the audience sees
/// a clean slide.
pub fn paint_background<S: DrawSurface>(
    surface: &mut S,
    cam: &ViewportTransform,
    palette: &Palette,
) {
    surface.fill_rect(
        ambre_draw::Rect::new(0.0, 0.0, cam.size.width, cam.size.height),
        palette.content_bg,
    );
}

/// Paint the board background and the adaptive grid lines for `cam`, on the
/// board's `unit`. Lines are emitted in screen space. The surface transform
/// stack is left untouched here, so the hairline width does not scale with zoom.
pub fn paint_grid<S: DrawSurface>(
    surface: &mut S,
    cam: &ViewportTransform,
    palette: &Palette,
    unit: f64,
) {
    // Board background fills the whole canvas.
    paint_background(surface, cam, palette);

    let step = grid_step(cam.zoom, board_unit(unit));
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
        surface.stroke_line(
            ambre_draw::Line::new((sx, 0.0), (sx, cam.size.height)),
            color,
            ambre_core::SurfacePx::new(1.0),
        );
        x += step;
    }
    // Horizontal lines at each world-y multiple of `step`.
    let mut y = (min_y / step).ceil() * step;
    while y <= max_y {
        let sy = (w2s * Point::new(0.0, y)).y;
        surface.stroke_line(
            ambre_draw::Line::new((0.0, sy), (cam.size.width, sy)),
            color,
            ambre_core::SurfacePx::new(1.0),
        );
        y += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_draw::testing::{DrawOp, RecordingSurface};

    #[test]
    fn spacing_never_illegible_across_zoom_sweep() {
        // For any zoom, the chosen step keeps the on-screen spacing at or above
        // the minimum legible band.
        let min = f64::from(MIN_SPACING.get());
        let mut z = min / (BASE_GRID * 64.0); // very zoomed out
        while z <= MAX_ZOOM_SWEEP {
            let step = grid_step(ambre_core::ZoomFactor::positive(z), BASE_GRID);
            let spacing = step * z;
            assert!(
                spacing >= min - 1e-9,
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
            let step = grid_step(ambre_core::ZoomFactor::positive(z), BASE_GRID);
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

    fn palette() -> Palette {
        ambre_canvas::theme::LIGHT
    }

    fn grid_ops(cam: &ViewportTransform, unit: f64) -> Vec<DrawOp> {
        let mut surface = RecordingSurface::new();
        paint_grid(&mut surface, cam, &palette(), unit);
        surface.ops().to_vec()
    }

    /// The screen x of each vertical grid line.
    fn columns(ops: &[DrawOp]) -> Vec<f64> {
        ops.iter()
            .filter_map(|o| match o {
                DrawOp::StrokeLine { x0, x1, .. } if x0 == x1 => Some(*x0),
                _ => None,
            })
            .collect()
    }

    /// `TOILE-UI-279`: the painted lines sit on the board's own unit, a board
    /// on 32 apart from one on 20. A unit no step search can use falls back.
    #[test]
    fn a_board_paints_its_grid_on_its_own_unit() {
        let cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
        // At 1x a unit of 20 doubles to stay legible; 32 and 50 already are.
        for (unit, step) in [(20.0, 40.0), (32.0, 32.0), (50.0, 50.0)] {
            assert_eq!(grid_step(cam.zoom, unit), step);
            let xs = columns(&grid_ops(&cam, unit));
            assert!(xs.len() > 2, "unit {unit}: {xs:?}");
            for pair in xs.windows(2) {
                assert!((pair[1] - pair[0] - step).abs() < 1e-9, "unit {unit}");
            }
            // World x 0 sits under the screen centre, and it is a line.
            assert!(xs.iter().any(|x| (x - 500.0).abs() < 1e-9));
        }
        for unusable in [0.0, -8.0, f64::NAN, f64::INFINITY] {
            assert_eq!(board_unit(unusable), BASE_GRID, "{unusable}");
            assert_eq!(
                columns(&grid_ops(&cam, unusable)),
                columns(&grid_ops(&cam, BASE_GRID))
            );
        }
    }

    /// `TOILE-UI-280`: a grid line is one pixel wide at every zoom, and the
    /// paint pushes no transform that could scale it.
    #[test]
    fn a_grid_line_is_one_pixel_at_every_zoom() {
        for zoom in [0.1, 0.5, 1.0, 3.0, 10.0] {
            let mut cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
            cam.zoom = ambre_core::ZoomFactor::positive(zoom);
            let ops = grid_ops(&cam, BASE_GRID);
            let mut lines = 0;
            for op in &ops {
                match op {
                    DrawOp::StrokeLine { width, .. } => {
                        assert_eq!(*width, 1.0, "zoom {zoom}");
                        lines += 1;
                    }
                    DrawOp::PushTransform { .. } => panic!("zoom {zoom}: a scaled line"),
                    _ => {}
                }
            }
            assert!(lines > 0, "zoom {zoom} paints lines");
        }
    }

    /// `TOILE-UI-045`: the background is the first thing painted, and it fills
    /// the whole canvas, with or without the grid.
    #[test]
    fn the_background_fills_the_canvas_first() {
        let cam = ViewportTransform::new(ambre_draw::Size::new(640.0, 480.0));
        let mut bare = RecordingSurface::new();
        paint_background(&mut bare, &cam, &palette());
        for ops in [grid_ops(&cam, BASE_GRID), bare.ops().to_vec()] {
            match ops.first() {
                Some(DrawOp::FillRect { rect, color }) => {
                    assert_eq!(*rect, ambre_draw::Rect::new(0.0, 0.0, 640.0, 480.0));
                    assert_eq!(*color, palette().content_bg);
                }
                other => panic!("the background comes first: {other:?}"),
            }
        }
    }

    #[test]
    fn snap_quantizes_to_step() {
        let s = snap_world(Point::new(23.0, -31.0), 20.0);
        assert_eq!(s, Point::new(20.0, -40.0));
    }
}
