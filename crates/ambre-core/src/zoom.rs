//! The one zoom engine of the suite (`ambre-spec-zoom.md`, `UI-THREAD-012`).
//!
//! Three editors zoom three spaces: a grid, a board, a column of pages. Each
//! one holds a [`Zoom`] over its own space and names its own [`ZoomRange`].
//! The ladder, the clamp, the step and the fixed point are the same code in
//! all three. The mapping from the factor to the pixels stays each editor's.
//!
//! A zoom is view state. Neither type here implements `serde`, so no
//! document, wire frame or presence sample can carry one (`ZOOM-001`,
//! `ZOOM-008`).

use crate::scale::Scale;
use crate::space::{Css, Rescale, ScalesTo, Space, View};
use crate::unit::{Px, Vector};

/// A log-zoom rate per CSS pixel of wheel: a step of `dy` multiplies the
/// factor by `exp(-dy * rate)`. A step down zooms out, a step up zooms in.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WheelRate(f64);

impl WheelRate {
    /// The ratio a wheel step of `dy` applies to the factor.
    fn ratio(self, dy: Px<Css>) -> f64 {
        (-f64::from(dy.get()) * self.0).exp()
    }
}

/// The rate of a wheel notch and of a trackpad pinch alike, in every editor
/// (`ZOOM-004`). A browser reports a pinch as a wheel with the control key.
const WHEEL_RATE: WheelRate = WheelRate(0.0018);

/// The number of stops on the ladder.
const STOPS: usize = 13;

/// The one ladder of presets, low to high (`ZOOM-006`). Every editor and the
/// shared chip read it, each clamped to its own [`ZoomRange`]. It spans the
/// widest range an editor supports, the board's, so no editor needs a stop
/// the ladder does not hold.
#[must_use]
pub const fn ladder<S: ScalesTo<View>>() -> [Scale<S, View>; STOPS] {
    [
        Scale::positive(0.1),
        Scale::positive(0.25),
        Scale::positive(0.5),
        Scale::positive(0.75),
        Scale::ONE,
        Scale::positive(1.25),
        Scale::positive(1.5),
        Scale::positive(2.0),
        Scale::positive(3.0),
        Scale::positive(4.0),
        Scale::positive(6.0),
        Scale::positive(8.0),
        Scale::positive(10.0),
    ]
}

/// The factors an editor supports: a floor and a ceiling, the floor never
/// above the ceiling (`ZOOM-002`). Each editor names its own, because the
/// range is a property of the space it zooms.
pub struct ZoomRange<S: ScalesTo<View>> {
    floor: Scale<S, View>,
    ceiling: Scale<S, View>,
}

impl<S: ScalesTo<View>> ZoomRange<S> {
    /// The range between two factors, in either order.
    #[must_use]
    pub const fn new(a: Scale<S, View>, b: Scale<S, View>) -> Self {
        if a.get() <= b.get() {
            Self {
                floor: a,
                ceiling: b,
            }
        } else {
            Self {
                floor: b,
                ceiling: a,
            }
        }
    }

    /// The smallest factor of the range.
    #[must_use]
    pub const fn floor(self) -> Scale<S, View> {
        self.floor
    }

    /// The largest factor of the range.
    #[must_use]
    pub const fn ceiling(self) -> Scale<S, View> {
        self.ceiling
    }

    /// `factor` inside the range.
    #[must_use]
    pub const fn clamp(self, factor: Scale<S, View>) -> Scale<S, View> {
        factor.clamped(self.floor, self.ceiling)
    }

    /// The stops of the ladder inside the range, low to high: the presets
    /// the chip lists for this editor (`ZOOM-006`).
    pub fn presets(self) -> impl DoubleEndedIterator<Item = Scale<S, View>> {
        ladder::<S>()
            .into_iter()
            .filter(move |stop| *stop >= self.floor && *stop <= self.ceiling)
    }
}

/// Where a zoom command moves the factor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomStep {
    /// To the next stop of the ladder above the factor.
    In,
    /// To the next stop of the ladder below the factor.
    Out,
    /// Back to a hundred per cent (`ZOOM-005`).
    Reset,
}

/// The zoom of one view: a factor, always inside its range.
pub struct Zoom<S: ScalesTo<View>> {
    factor: Scale<S, View>,
    range: ZoomRange<S>,
}

/// A zoom about a point: the new zoom, and the content offset that keeps
/// the point fixed (`ZOOM-003`).
pub struct ZoomedAbout<S: ScalesTo<View>> {
    /// The zoom after the gesture.
    pub zoom: Zoom<S>,
    /// The content coordinate at the view's origin after the gesture.
    pub origin: Px<S>,
}

impl<S: ScalesTo<View>> Zoom<S> {
    /// The zoom at `factor`, clamped to `range` whatever the input
    /// (`ZOOM-002`).
    #[must_use]
    pub const fn at(range: ZoomRange<S>, factor: Scale<S, View>) -> Self {
        Self {
            factor: range.clamp(factor),
            range,
        }
    }

    /// The factor: view pixels per content pixel.
    #[must_use]
    pub const fn factor(self) -> Scale<S, View> {
        self.factor
    }

    /// The range the factor stays inside.
    #[must_use]
    pub const fn range(self) -> ZoomRange<S> {
        self.range
    }

    /// The zoom at `factor`, in the same range (`ZOOM-002`).
    #[must_use]
    pub const fn to(self, factor: Scale<S, View>) -> Self {
        Self::at(self.range, factor)
    }

    /// The zoom one step along the ladder, or back to one (`ZOOM-005`). A
    /// step past either end of the range keeps the factor.
    #[must_use]
    pub fn step(self, step: ZoomStep) -> Self {
        let now = self.factor;
        let next = match step {
            ZoomStep::In => self.range.presets().find(|stop| *stop > now),
            ZoomStep::Out => self.range.presets().rev().find(|stop| *stop < now),
            ZoomStep::Reset => Some(Scale::ONE),
        };
        next.map_or(self, |factor| self.to(factor))
    }

    /// The factor a wheel step of `dy` asks for. A step up zooms in and a
    /// step down zooms out, at one rate in every editor (`ZOOM-004`). The caller
    /// zooms to it about the pointer, which clamps it (`ZOOM-002`).
    #[must_use]
    pub fn wheeled(self, dy: Px<Css>) -> Scale<S, View> {
        Scale::positive(self.factor.get() * WHEEL_RATE.ratio(dy))
    }

    /// The zoom that shows a box of `content` whole inside `view`, clamped
    /// (`ZOOM-007`). The tighter axis governs. A box with an empty or
    /// non-finite side frames nothing, so the answer is `None` and the
    /// caller keeps its zoom (`ZOOM-009`).
    #[must_use]
    pub fn framing(self, content: Vector<Px<S>>, view: Vector<Px<View>>) -> Option<Self>
    where
        <View as Space>::Repr: Rescale<S::Repr>,
        S::Repr: Into<f64>,
    {
        // The room as it reads at one, so the two sides share the content's
        // space and its precision.
        let fit = |side: Px<S>, room: Px<View>| -> Option<f64> {
            let (side, room): (f64, f64) = (side.get().into(), (room / Scale::<S, View>::ONE).get().into());
            let ratio = room / side;
            let sides = side.is_finite() && side > 0.0 && room > 0.0;
            (sides && ratio.is_finite()).then_some(ratio)
        };
        let factor = fit(content.dx, view.dx)?.min(fit(content.dy, view.dy)?);
        Some(self.to(Scale::positive(factor)))
    }

    /// The zoom to `factor` about a point of the view, on one axis.
    /// `origin` is the content coordinate at the view's origin, and `at` is
    /// the distance of the point from that origin, in view pixels. The
    /// content under the point stays under it at the clamped factor
    /// (`ZOOM-003`).
    #[must_use]
    pub fn about(self, factor: Scale<S, View>, origin: Px<S>, at: Px<View>) -> ZoomedAbout<S>
    where
        <View as Space>::Repr: Rescale<S::Repr>,
    {
        let under = origin + at / self.factor;
        let zoom = self.to(factor);
        ZoomedAbout {
            zoom,
            origin: under - at / zoom.factor,
        }
    }
}

// By hand, as for `Scale`: a derive would ask the marker for each trait.
impl<S: ScalesTo<View>> Clone for ZoomRange<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: ScalesTo<View>> Copy for ZoomRange<S> {}

impl<S: ScalesTo<View>> PartialEq for ZoomRange<S> {
    fn eq(&self, other: &Self) -> bool {
        self.floor == other.floor && self.ceiling == other.ceiling
    }
}

impl<S: ScalesTo<View>> std::fmt::Debug for ZoomRange<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZoomRange")
            .field("floor", &self.floor)
            .field("ceiling", &self.ceiling)
            .finish()
    }
}

impl<S: ScalesTo<View>> Clone for Zoom<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: ScalesTo<View>> Copy for Zoom<S> {}

impl<S: ScalesTo<View>> PartialEq for Zoom<S> {
    fn eq(&self, other: &Self) -> bool {
        self.factor == other.factor && self.range == other.range
    }
}

impl<S: ScalesTo<View>> std::fmt::Debug for Zoom<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Zoom")
            .field("factor", &self.factor)
            .field("range", &self.range)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{Zoom, ZoomRange, ZoomStep, ladder};
    use crate::scale::Scale;
    use crate::space::{Board, Doc, View};
    use crate::space::Css;
    use crate::unit::{Px, Vector};
    use proptest::prelude::*;

    const PAGE: ZoomRange<Doc> = ZoomRange::new(Scale::positive(0.25), Scale::positive(4.0));
    const BOARD: ZoomRange<Board> = ZoomRange::new(Scale::positive(0.1), Scale::positive(10.0));

    /// Any number a gesture or a menu could hand in, the hostile ones too.
    fn any_factor() -> impl Strategy<Value = f64> {
        prop_oneof![
            -1.0e9f64..1.0e9,
            0.0f64..20.0,
            Just(f64::NAN),
            Just(f64::INFINITY),
            Just(f64::NEG_INFINITY),
            Just(0.0),
            Just(-0.0),
            Just(f64::MIN_POSITIVE),
        ]
    }

    proptest! {
        /// `ZOOM-002`: whatever the input, the factor lands inside the range,
        /// away from zero.
        #[test]
        fn a_zoom_clamps_to_its_range_whatever_the_input(asked in any_factor()) {
            let zoom = Zoom::at(PAGE, Scale::positive(1.0)).to(Scale::positive(asked));
            prop_assert!(zoom.factor() >= PAGE.floor() && zoom.factor() <= PAGE.ceiling());
            prop_assert!(zoom.factor().get() > 0.0 && zoom.factor().get().is_finite());
        }

        /// A step in never lowers the factor, a step out never raises it, and
        /// both stay inside the range. A step lands on a stop of the ladder.
        #[test]
        fn a_step_moves_along_the_ladder_inside_the_range(asked in any_factor()) {
            let zoom = Zoom::at(BOARD, Scale::positive(asked));
            let up = zoom.step(ZoomStep::In);
            let down = zoom.step(ZoomStep::Out);
            prop_assert!(up.factor() >= zoom.factor());
            prop_assert!(down.factor() <= zoom.factor());
            for moved in [up, down] {
                prop_assert!(moved.factor() >= BOARD.floor() && moved.factor() <= BOARD.ceiling());
                prop_assert!(
                    moved == zoom || BOARD.presets().any(|stop| stop == moved.factor()),
                    "{moved:?} is no stop"
                );
            }
        }

        /// `ZOOM-005`: a reset lands on one from any factor, in any range
        /// that holds one.
        #[test]
        fn a_reset_returns_to_one(asked in any_factor()) {
            let zoom = Zoom::at(PAGE, Scale::positive(asked)).step(ZoomStep::Reset);
            prop_assert_eq!(zoom.factor(), Scale::ONE);
        }

        /// `ZOOM-003`: the content under the point stays under it. Two paths
        /// agree: the content read before the zoom, and the content read at
        /// the same view point after it.
        #[test]
        fn a_zoom_about_a_point_keeps_the_content_under_it(
            from in 0.1f64..10.0,
            to in any_factor(),
            origin in -1.0e6f64..1.0e6,
            at in 0.0f32..4000.0,
        ) {
            let zoom = Zoom::at(BOARD, Scale::positive(from));
            let at = Px::<View>::new(at);
            let origin = Px::<Board>::new(origin);
            let before = origin + at / zoom.factor();
            let after = zoom.about(Scale::positive(to), origin, at);
            let under = after.origin + at / after.zoom.factor();
            let drift = (under - before).get().abs();
            prop_assert!(drift <= 1.0e-6 * before.get().abs().max(1.0), "{drift}");
        }
    }

    proptest! {
        /// `ZOOM-004`: a wheel step up zooms in, a step down zooms out, and
        /// no step keeps the factor, at any factor.
        #[test]
        fn a_wheel_step_zooms_the_way_it_turns(from in 0.1f64..10.0, dy in 1.0f32..2000.0) {
            let zoom = Zoom::at(BOARD, Scale::positive(from));
            prop_assert!(zoom.wheeled(Px::<Css>::new(-dy)) > zoom.factor());
            prop_assert!(zoom.wheeled(Px::<Css>::new(dy)) < zoom.factor());
            prop_assert_eq!(zoom.wheeled(Px::<Css>::ZERO), zoom.factor());
        }

        /// `ZOOM-007`: a framed box shows whole inside the view, and fills it
        /// on one axis unless the range stops it. `ZOOM-009`: a box with an
        /// empty side frames nothing.
        #[test]
        fn a_box_is_framed_whole_and_an_empty_one_is_ignored(
            w in prop_oneof![0.0f64..1.0e5, Just(0.0), Just(f64::NAN), Just(f64::INFINITY)],
            h in prop_oneof![0.0f64..1.0e5, Just(0.0), Just(-5.0)],
            vw in 1.0f32..4000.0,
            vh in 1.0f32..4000.0,
        ) {
            let zoom = Zoom::at(BOARD, Scale::ONE);
            let content = Vector::new(Px::<crate::space::Board>::new(w), Px::<crate::space::Board>::new(h));
            let view = Vector::new(Px::<View>::new(vw), Px::<View>::new(vh));
            let framed = zoom.framing(content, view);
            if !(w.is_finite() && w > 0.0 && h > 0.0) {
                prop_assert!(framed.is_none());
            } else if let Some(framed) = framed {
                let f = framed.factor();
                prop_assert!(f >= BOARD.floor() && f <= BOARD.ceiling());
                let (shown_w, shown_h) = (w * f.get(), h * f.get());
                let fits = shown_w <= f64::from(vw) * (1.0 + 1e-5) && shown_h <= f64::from(vh) * (1.0 + 1e-5);
                prop_assert!(fits || f == BOARD.floor(), "{shown_w}x{shown_h} in {vw}x{vh}");
                let fills = (shown_w - f64::from(vw)).abs() <= f64::from(vw) * 1e-5
                    || (shown_h - f64::from(vh)).abs() <= f64::from(vh) * 1e-5;
                prop_assert!(fills || f == BOARD.floor() || f == BOARD.ceiling());
            } else {
                prop_assert!(false, "a box with two sides frames");
            }
        }
    }

    /// `ZOOM-006`: the ladder runs low to high, holds one, and each editor's
    /// presets are the stops of the one ladder inside its range.
    #[test]
    fn every_editor_reads_its_presets_off_the_one_ladder() {
        let stops = ladder::<Doc>();
        assert!(stops.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(stops.contains(&Scale::ONE));
        let page: Vec<_> = PAGE.presets().map(Scale::get).collect();
        assert_eq!(page, [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0]);
        assert_eq!(BOARD.presets().count(), stops.len());
    }

    /// A step past either end of the range keeps the factor.
    #[test]
    fn a_step_past_an_end_keeps_the_factor() {
        let top = Zoom::at(PAGE, Scale::positive(4.0));
        assert_eq!(top.step(ZoomStep::In), top);
        let bottom = Zoom::at(PAGE, Scale::positive(0.25));
        assert_eq!(bottom.step(ZoomStep::Out), bottom);
        // A factor between two stops steps to the nearer stop on its side.
        let between = Zoom::at(PAGE, Scale::positive(1.1));
        assert_eq!(between.step(ZoomStep::Out).factor(), Scale::ONE);
        assert_eq!(between.step(ZoomStep::In).factor().get(), 1.25);
    }
}
