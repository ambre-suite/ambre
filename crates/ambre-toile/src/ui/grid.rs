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
use ambre_core::{Board, Px};
use ambre_draw::{DrawSurface, Point};

use super::viewport::ViewportTransform;

/// The grid unit in world units of a board whose register holds no usable one.
/// A power-of-two multiple of the unit is the smallest step the adaptive grid
/// shows.
pub const BASE_GRID: Px<Board> = Px::<Board>::new(20.0);

/// The unit a grid steps by: a finite, positive board length.
///
/// A peer can write any number in the `board` register. A zero, a negative or
/// a NaN unit would stall the step search of [`grid_step`], so no such unit
/// is representable.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct GridUnit(Px<Board>);

impl GridUnit {
    /// The unit of a board whose register holds no usable one.
    pub const BASE: Self = Self(BASE_GRID);

    /// The unit `length`, or `None` when it is not finite and positive.
    #[must_use]
    pub fn new(length: Px<Board>) -> Option<Self> {
        (length.is_finite() && length > Px::<Board>::ZERO).then_some(Self(length))
    }

    /// The unit a board paints its grid on: its `board` register when that
    /// holds a usable unit, else [`GridUnit::BASE`] (`TOILE-UI-279`).
    #[must_use]
    pub fn of_register(register: Px<Board>) -> Self {
        Self::new(register).unwrap_or(Self::BASE)
    }

    /// The unit as a board length.
    #[must_use]
    pub fn get(self) -> Px<Board> {
        self.0
    }
}

/// Comfortable on-screen spacing band (logical pixels): the adaptive step keeps
/// the line spacing at or above `MIN_SPACING`, doubling the world step until
/// it does.
const MIN_SPACING: ambre_core::Px<ambre_core::View> = ambre_core::Px::<ambre_core::View>::new(24.0);
const MAX_SPACING: ambre_core::Px<ambre_core::View> = ambre_core::Px::<ambre_core::View>::new(96.0);

/// Choose the grid step (world units) for `zoom`: the smallest power-of-two
/// multiple of `base` whose screen spacing (`step * zoom`) is at least
/// `MIN_SPACING`. That step is then halved back down while it stays above
/// `MAX_SPACING`, never below `base`, so the grid stays magnetic to the board
/// unit.
pub fn grid_step(
    zoom: ambre_core::Scale<ambre_core::Board, ambre_core::View>,
    base: GridUnit,
) -> Px<Board> {
    // The band, in the f64 of the board's screen geometry.
    let (min, max) = (MIN_SPACING.surface_length(), MAX_SPACING.surface_length());
    let base = base.get();
    let mut step = base;
    // Zoomed out: coarsen until the spacing is legible.
    while step.zoomed_on_surface(zoom) < min {
        step = step * 2.0;
    }
    // Zoomed in: refine toward the band, but never finer than `base`.
    while step > base && step.zoomed_on_surface(zoom) > max {
        step = step / 2.0;
    }
    step
}

/// Quantize a world point to the nearest multiple of `step` on each axis.
pub fn snap_world(p: Point, step: Px<Board>) -> Point {
    let step = step.get();
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
    unit: Px<Board>,
) {
    // Board background fills the whole canvas.
    paint_background(surface, cam, palette);

    let step = grid_step(cam.zoom, GridUnit::of_register(unit)).get();
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
            ambre_core::Px::<ambre_core::Surface>::new(1.0),
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
            ambre_core::Px::<ambre_core::Surface>::new(1.0),
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
        let mut z = min / (BASE_GRID.get() * 64.0); // very zoomed out
        while z <= MAX_ZOOM_SWEEP {
            let step = grid_step(
                ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(z),
                GridUnit::BASE,
            );
            let spacing = step.get() * z;
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
            let step = grid_step(
                ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(z),
                GridUnit::BASE,
            );
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
        paint_grid(&mut surface, cam, &palette(), Px::<Board>::new(unit));
        surface.ops().to_vec()
    }

    /// The screen x of each vertical grid line.
    fn columns(ops: &[DrawOp]) -> Vec<f64> {
        ops.iter()
            .filter_map(|o| match o {
                DrawOp::StrokeLine { line, .. } if line.p0.x == line.p1.x => Some(line.p0.x),
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
            let typed = GridUnit::of_register(Px::<Board>::new(unit));
            assert_eq!(grid_step(cam.zoom, typed), Px::<Board>::new(step));
            let xs = columns(&grid_ops(&cam, unit));
            assert!(xs.len() > 2, "unit {unit}: {xs:?}");
            for pair in xs.windows(2) {
                assert!((pair[1] - pair[0] - step).abs() < 1e-9, "unit {unit}");
            }
            // World x 0 sits under the screen centre, and it is a line.
            assert!(xs.iter().any(|x| (x - 500.0).abs() < 1e-9));
        }
        for unusable in [0.0, -8.0, f64::NAN, f64::INFINITY] {
            let length = Px::<Board>::new(unusable);
            assert_eq!(GridUnit::new(length), None, "{unusable}");
            assert_eq!(GridUnit::of_register(length), GridUnit::BASE, "{unusable}");
            assert_eq!(
                columns(&grid_ops(&cam, unusable)),
                columns(&grid_ops(&cam, BASE_GRID.get()))
            );
        }
    }

    /// A grid unit from a peer register is finite and positive, or refused.
    /// The step search then always ends, from the smallest positive unit too.
    #[test]
    fn a_grid_unit_is_finite_and_positive() {
        for refused in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(GridUnit::new(Px::<Board>::new(refused)), None, "{refused}");
        }
        let tiny = GridUnit::new(Px::<Board>::new(f64::MIN_POSITIVE)).map(GridUnit::get);
        assert_eq!(tiny, Some(Px::<Board>::new(f64::MIN_POSITIVE)));
        let zoom = ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(1.0);
        let step = grid_step(
            zoom,
            GridUnit::of_register(Px::<Board>::new(f64::MIN_POSITIVE)),
        );
        assert!(step.get() >= f64::from(MIN_SPACING.get()));
    }

    /// `TOILE-UI-280`: a grid line is one pixel wide at every zoom, and the
    /// paint pushes no transform that could scale it.
    #[test]
    fn a_grid_line_is_one_pixel_at_every_zoom() {
        for zoom in [0.1, 0.5, 1.0, 3.0, 10.0] {
            let mut cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
            cam.zoom = ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(zoom);
            let ops = grid_ops(&cam, BASE_GRID.get());
            let mut lines = 0;
            for op in &ops {
                match op {
                    DrawOp::StrokeLine { width, .. } => {
                        assert_eq!(width.get(), 1.0, "zoom {zoom}");
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
        for ops in [grid_ops(&cam, BASE_GRID.get()), bare.ops().to_vec()] {
            match ops.first() {
                Some(DrawOp::FillRect { rect, color }) => {
                    assert_eq!(*rect, ambre_draw::Rect::new(0.0, 0.0, 640.0, 480.0));
                    assert_eq!(*color, palette().content_bg);
                }
                other => panic!("the background comes first: {other:?}"),
            }
        }
    }

    /// `TOILE-UI-421`: snapping moves a world point to the nearest multiple of
    /// the grid step on each axis.
    #[test]
    fn snap_quantizes_to_step() {
        let s = snap_world(Point::new(23.0, -31.0), Px::<Board>::new(20.0));
        assert_eq!(s, Point::new(20.0, -40.0));
        // Past the half step, the point goes up: rounding down would answer 20.
        let up = snap_world(Point::new(31.0, 9.0), Px::<Board>::new(20.0));
        assert_eq!(up, Point::new(40.0, 0.0));
    }
}
