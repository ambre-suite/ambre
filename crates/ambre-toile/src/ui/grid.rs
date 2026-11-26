// ambre-toile/src/ui/grid.rs
// ============================================================================
// Adaptive magnetic grid (toile-viewport). The grid step is chosen so the
// on-screen spacing stays in a comfortable pixel band as `zoom` changes (powers
// of the board `grid_size`). `snap_world` quantizes a world point to the current
// step for drag / resize snapping. Step selection is pure and host-tested; the
// paint helper emits one screen-space dot per grid crossing through the
// abstract DrawSurface.
// ============================================================================

use ambre_canvas::Palette;
use ambre_core::{Board, Px};
use ambre_draw::{DrawSurface, Point, Rect, Vec2};

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

/// Whether this user shows the board's grid. View state alone: the status bar
/// toggles it and no peer sees it (`CHROME-149`). A hidden grid also holds
/// back the snap, so no gesture lands on a grid the user cannot see.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GridShown {
    /// The grid paints, and the snap of the board applies.
    #[default]
    Shown,
    /// The ground paints bare, and every gesture commits where it ends.
    Hidden,
}

impl GridShown {
    /// The other state, which the toggle of the status bar picks.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Shown => Self::Hidden,
            Self::Hidden => Self::Shown,
        }
    }
}

/// The snap of the board and the grid of this user, which the panel and the
/// status bar read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BoardLook {
    /// Whether the board asks its gestures to land on the grid.
    pub snap: crate::message::GridSnap,
    /// Whether this user shows the grid.
    pub grid: GridShown,
    /// What the board lies on.
    pub ground: crate::message::BoardGround,
    /// A write of the board section the view has not sent yet. One signal
    /// carries the look and the write, so Main holds one signal type.
    pub asked: Option<crate::message::BoardEdit>,
}

/// The light tints the board section offers after the ground of the theme
/// (`TOILE-UI-430`). Each one is light, so dark content reads on it in both
/// themes.
pub const TINTS: [(ambre_commands::TitleWords, ambre_core::Rgba); 6] = [
    (
        ambre_commands::words!("Ivory", "Ivoire"),
        ambre_core::Rgba::from_bits(0xFFFD_F5FF),
    ),
    (
        ambre_commands::words!("Lemon", "Citron"),
        ambre_core::Rgba::from_bits(0xFDF8_DCFF),
    ),
    (
        ambre_commands::words!("Mint", "Menthe"),
        ambre_core::Rgba::from_bits(0xE6F4_EAFF),
    ),
    (
        ambre_commands::words!("Sky", "Ciel"),
        ambre_core::Rgba::from_bits(0xE8F1_FBFF),
    ),
    (
        ambre_commands::words!("Lavender", "Lavande"),
        ambre_core::Rgba::from_bits(0xEFEA_F8FF),
    ),
    (
        ambre_commands::words!("Blush", "Rose poudr\u{e9}"),
        ambre_core::Rgba::from_bits(0xFBEA_EAFF),
    ),
];

/// The step a drag, a resize and a nudge land on, `None` when they land
/// free. It is the step the grid draws at `zoom`, so the dots a user sees
/// are the points a gesture lands on (`TOILE-UI-025`).
#[must_use]
pub fn snap_step(
    snap: crate::message::GridSnap,
    shown: GridShown,
    zoom: ambre_core::Scale<Board, ambre_core::View>,
    unit: Px<Board>,
) -> Option<Px<Board>> {
    (snap == crate::message::GridSnap::Snapped && shown == GridShown::Shown)
        .then(|| grid_step(zoom, GridUnit::of_register(unit)))
}

/// The delta that lands the corner `anchor` of a moved box on the grid,
/// from the raw delta of the pointer. The boxes of the selection keep their
/// offsets to each other.
#[must_use]
pub fn snap_delta(anchor: Point, raw: Vec2, step: Px<Board>) -> Vec2 {
    snap_world(anchor + raw, step) - anchor
}

/// `r` with each edge on the grid, never narrower or lower than one step.
#[must_use]
pub fn snap_rect(r: Rect, step: Px<Board>) -> Rect {
    let p0 = snap_world(Point::new(r.x0, r.y0), step);
    let p1 = snap_world(Point::new(r.x1, r.y1), step);
    let s = step.get();
    // An edge that collapses onto its opposite keeps one step from it, on
    // the side the pointer dragged it to.
    let far = |lo: f64, hi: f64, raw: f64| {
        if (hi - lo).abs() >= s {
            hi
        } else if raw >= 0.0 {
            lo + s
        } else {
            lo - s
        }
    };
    Rect::new(
        p0.x,
        p0.y,
        far(p0.x, p1.x, r.x1 - r.x0),
        far(p0.y, p1.y, r.y1 - r.y0),
    )
}

/// The delta of a nudge on the grid. The corner `anchor` moves to the next
/// grid line along `dir`, on each axis that `dir` names by its sign.
#[must_use]
pub fn nudge_on_grid(anchor: Point, dir: Vec2, step: Px<Board>) -> Vec2 {
    let s = step.get();
    let axis = |at: f64, sign: f64| {
        if sign == 0.0 {
            return 0.0;
        }
        let q = at / s;
        let k = q.round();
        // A corner on a line moves one whole step. A corner between two
        // lines moves to the next one in the direction of the nudge.
        let on = (q - k).abs() < 1e-6;
        let to = match (on, sign > 0.0) {
            (true, true) => k + 1.0,
            (true, false) => k - 1.0,
            (false, true) => q.ceil(),
            (false, false) => q.floor(),
        };
        to * s - at
    };
    Vec2::new(axis(anchor.x, dir.x), axis(anchor.y, dir.y))
}

/// The side of a grid dot, in screen pixels, at every zoom (`TOILE-UI-280`).
const DOT: ambre_core::Px<ambre_core::Surface> = ambre_core::Px::<ambre_core::Surface>::new(1.5);

/// The ground a board lies on and the colour of its grid dots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundColours {
    /// The fill under the whole canvas.
    pub ground: ambre_draw::Color,
    /// The colour of one grid dot.
    pub dot: ambre_draw::Color,
}

impl GroundColours {
    /// The colours of `ground` in the theme of `palette` (`TOILE-UI-430`).
    /// A tint draws as it is in both themes. Its dots take a shade off it,
    /// darker on a light tint and lighter on a dark one.
    #[must_use]
    pub fn of(palette: &Palette, ground: crate::message::BoardGround) -> Self {
        match ground {
            crate::message::BoardGround::Theme => Self {
                ground: palette.ground,
                dot: palette.grid_dot,
            },
            crate::message::BoardGround::Tint(tint) => {
                let [r, g, b, _] = tint.channels();
                let luma = 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
                // The light theme puts its dots at about 0.86 of its ground.
                let shade = |c: u8| {
                    let c = f64::from(c);
                    let to = if luma >= 128.0 {
                        c * 0.84
                    } else {
                        c + (255.0 - c) * 0.2
                    };
                    // A channel in 0..=255 stays in it, so the cast keeps it.
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "the shade of a byte channel stays within 0..=255"
                    )]
                    let byte = to.round().clamp(0.0, 255.0) as u8;
                    byte
                };
                Self {
                    ground: ambre_draw::Color::from_rgba8(r, g, b, 255),
                    dot: ambre_draw::Color::from_rgba8(shade(r), shade(g), shade(b), 255),
                }
            }
        }
    }
}

/// Fill the board background only, without the grid. Used by presentation
/// focus mode (`toile-presentation`), which hides the grid so the audience sees
/// a clean slide.
pub fn paint_background<S: DrawSurface>(
    surface: &mut S,
    cam: &ViewportTransform,
    colours: GroundColours,
) {
    surface.fill_rect(
        ambre_draw::Rect::new(0.0, 0.0, cam.size.width, cam.size.height),
        colours.ground,
    );
}

/// Paint the board background and the adaptive dot grid for `cam`, on the
/// board's `unit`: one discreet dot at each crossing of the grid (`TOILE-UI-424`).
/// The dots are emitted in screen space. The surface transform stack is left
/// untouched here, so a dot keeps its size at every zoom.
pub fn paint_grid<S: DrawSurface>(
    surface: &mut S,
    cam: &ViewportTransform,
    colours: GroundColours,
    unit: Px<Board>,
) {
    // Board background fills the whole canvas.
    paint_background(surface, cam, colours);

    let step = grid_step(cam.zoom, GridUnit::of_register(unit)).get();
    let s2w = cam.screen_to_world();
    let w2s = cam.world_to_screen();
    // Visible world rect: the transform is translate+scale (axis-aligned), so the
    // top-left and bottom-right screen corners bound it.
    let tl = s2w * Point::new(0.0, 0.0);
    let br = s2w * Point::new(cam.size.width, cam.size.height);
    let (min_x, max_x) = (tl.x.min(br.x), tl.x.max(br.x));
    let (min_y, max_y) = (tl.y.min(br.y), tl.y.max(br.y));

    // The screen position of each world multiple of `step` the canvas shows.
    let crossings = |min: f64, max: f64, axis: fn(Point) -> f64, to_point: fn(f64) -> Point| {
        let mut at = (min / step).ceil() * step;
        let mut found = Vec::new();
        while at <= max {
            found.push(axis(w2s * to_point(at)));
            at += step;
        }
        found
    };
    let xs = crossings(min_x, max_x, |p| p.x, |x| Point::new(x, 0.0));
    let ys = crossings(min_y, max_y, |p| p.y, |y| Point::new(0.0, y));
    let half = DOT.get() / 2.0;
    for &sy in &ys {
        for &sx in &xs {
            surface.fill_rect(
                ambre_draw::Rect::new(sx - half, sy - half, sx + half, sy + half),
                colours.dot,
            );
        }
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

    /// The colours of a board on the ground of the light theme.
    fn theme_colours() -> GroundColours {
        GroundColours::of(&palette(), crate::message::BoardGround::Theme)
    }

    fn grid_ops(cam: &ViewportTransform, unit: f64) -> Vec<DrawOp> {
        let mut surface = RecordingSurface::new();
        paint_grid(&mut surface, cam, theme_colours(), Px::<Board>::new(unit));
        surface.ops().to_vec()
    }

    /// Each grid dot as its screen rectangle.
    fn dots(ops: &[DrawOp]) -> Vec<ambre_draw::Rect> {
        ops.iter()
            .filter_map(|o| match o {
                DrawOp::FillRect { rect, color } if *color == palette().grid_dot => Some(*rect),
                _ => None,
            })
            .collect()
    }

    /// The screen x of each column of grid dots.
    fn columns(ops: &[DrawOp]) -> Vec<f64> {
        let all = dots(ops);
        let Some(top) = all.first().map(|d| d.y0) else {
            return Vec::new();
        };
        all.iter()
            .filter(|d| d.y0 == top)
            .map(|d| d.center().x)
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

    /// `TOILE-UI-280`: a grid dot keeps its screen size at every zoom, and
    /// the paint pushes no transform that could scale it.
    #[test]
    fn a_grid_dot_keeps_its_size_at_every_zoom() {
        for zoom in [0.1, 0.5, 1.0, 3.0, 10.0] {
            let mut cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
            cam.zoom = ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(zoom);
            let ops = grid_ops(&cam, BASE_GRID.get());
            assert!(
                !ops.iter()
                    .any(|op| matches!(op, DrawOp::PushTransform { .. })),
                "zoom {zoom}: a scaled dot"
            );
            let all = dots(&ops);
            assert!(!all.is_empty(), "zoom {zoom} paints dots");
            for dot in all {
                assert_eq!(
                    (dot.width(), dot.height()),
                    (DOT.get(), DOT.get()),
                    "zoom {zoom}"
                );
            }
        }
    }

    /// `TOILE-UI-424`: the default grid is dots at the crossings, in the dot
    /// colour, on the warm ground: no line is drawn. Between two crossings the
    /// ground shows, so the grid stays discreet.
    #[test]
    fn the_default_grid_is_dots_at_the_crossings_on_the_ground() {
        let cam = ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
        let ops = grid_ops(&cam, BASE_GRID.get());
        assert!(
            !ops.iter().any(|op| matches!(op, DrawOp::StrokeLine { .. })),
            "a dot grid draws no line"
        );
        let step = grid_step(cam.zoom, GridUnit::BASE).get();
        let xs = columns(&ops);
        let rows = dots(&ops).len() / xs.len();
        assert_eq!(dots(&ops).len(), xs.len() * rows, "one dot per crossing");
        for pair in xs.windows(2) {
            assert!((pair[1] - pair[0] - step).abs() < 1e-9);
        }
        for theme in [ambre_canvas::Theme::Light, ambre_canvas::Theme::Dark] {
            let p = theme.palette();
            assert_ne!(p.grid_dot, p.ground, "{theme:?}: a dot reads on the ground");
            assert_eq!(
                p.ground,
                theme.chrome_palette().surface_2,
                "{theme:?}: the board lies on the sunken ground of the chrome"
            );
        }
    }

    /// `TOILE-UI-045`: the background is the first thing painted, and it fills
    /// the whole canvas, with or without the grid.
    #[test]
    fn the_background_fills_the_canvas_first() {
        let cam = ViewportTransform::new(ambre_draw::Size::new(640.0, 480.0));
        let mut bare = RecordingSurface::new();
        paint_background(&mut bare, &cam, theme_colours());
        for ops in [grid_ops(&cam, BASE_GRID.get()), bare.ops().to_vec()] {
            match ops.first() {
                Some(DrawOp::FillRect { rect, color }) => {
                    assert_eq!(*rect, ambre_draw::Rect::new(0.0, 0.0, 640.0, 480.0));
                    assert_eq!(*color, palette().ground);
                }
                other => panic!("the background comes first: {other:?}"),
            }
        }
    }

    /// `TOILE-UI-025`: a gesture lands on the step the grid draws, and on
    /// none while the board lands free or this user hides the grid.
    #[test]
    fn the_snap_step_is_the_drawn_step() {
        use crate::message::GridSnap;
        let unit = Px::<Board>::new(32.0);
        for z in [0.1, 0.5, 1.0, 3.0, 10.0] {
            let zoom = ambre_core::Scale::<Board, ambre_core::View>::positive(z);
            let drawn = grid_step(zoom, GridUnit::of_register(unit));
            assert_eq!(
                snap_step(GridSnap::Snapped, GridShown::Shown, zoom, unit),
                Some(drawn)
            );
            assert_eq!(
                snap_step(GridSnap::Free, GridShown::Shown, zoom, unit),
                None
            );
            assert_eq!(
                snap_step(GridSnap::Snapped, GridShown::Hidden, zoom, unit),
                None
            );
        }
    }

    /// `TOILE-UI-425`, `TOILE-UI-426`: a moved corner lands on a crossing. A
    /// resized box keeps one step, and a nudge reaches the next line.
    #[test]
    fn a_snapped_geometry_sits_on_the_step() {
        let step = Px::<Board>::new(20.0);
        let moved = snap_delta(Point::new(3.0, 7.0), Vec2::new(25.0, -18.0), step);
        assert_eq!(Point::new(3.0, 7.0) + moved, Point::new(20.0, -20.0));
        assert_eq!(
            snap_rect(Rect::new(3.0, 7.0, 52.0, 9.0), step),
            Rect::new(0.0, 0.0, 60.0, 20.0)
        );
        assert_eq!(
            snap_rect(Rect::new(3.0, 7.0, -2.0, 9.0), step),
            Rect::new(0.0, 0.0, -20.0, 20.0)
        );
        let at = |p: Point, d: Vec2| p + nudge_on_grid(p, d, step);
        assert_eq!(
            at(Point::new(20.0, 40.0), Vec2::new(1.0, 0.0)),
            Point::new(40.0, 40.0)
        );
        assert_eq!(
            at(Point::new(20.0, 40.0), Vec2::new(0.0, -1.0)),
            Point::new(20.0, 20.0)
        );
        assert_eq!(
            at(Point::new(23.0, 5.0), Vec2::new(-1.0, 0.0)),
            Point::new(20.0, 5.0)
        );
        assert_eq!(
            at(Point::new(23.0, 5.0), Vec2::new(0.0, 1.0)),
            Point::new(23.0, 20.0)
        );
    }

    /// `TOILE-UI-430`: the ground of the theme follows the theme. A tint
    /// draws as it is in both, and its dots take a shade off it by its
    /// luminance.
    #[test]
    fn a_tint_draws_alike_in_both_themes_with_dots_off_it() {
        use crate::message::BoardGround;
        use ambre_canvas::theme::{DARK, LIGHT};
        for p in [&LIGHT, &DARK] {
            let theme = GroundColours::of(p, BoardGround::Theme);
            assert_eq!((theme.ground, theme.dot), (p.ground, p.grid_dot));
        }
        let luma = |c: ambre_draw::Color| {
            let [r, g, b, _] = c.to_rgba8().to_u8_array();
            u32::from(r) + u32::from(g) + u32::from(b)
        };
        for (_, tint) in TINTS {
            let light = GroundColours::of(&LIGHT, BoardGround::Tint(tint));
            assert_eq!(light, GroundColours::of(&DARK, BoardGround::Tint(tint)));
            assert_eq!(ambre_draw::list::rgba_of(light.ground), tint);
            assert!(
                luma(light.dot) < luma(light.ground),
                "a light tint takes darker dots"
            );
        }
        let night = GroundColours::of(
            &LIGHT,
            BoardGround::Tint(ambre_core::Rgba::from_bits(0x2020_30FF)),
        );
        assert!(
            luma(night.dot) > luma(night.ground),
            "a dark tint takes lighter dots"
        );
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
