//! The scale between two pixel spaces: the zoom, and the display density.

use std::marker::PhantomData;

use crate::space::{Board, Doc, ScalesTo, Space, View};

/// The pixels of the space `B` per pixel of the space `A`: finite and
/// strictly positive, so a reader may divide by it.
///
/// A scale exists only for a pair that [`ScalesTo`] declares.
/// `Scale<Doc, View>` is the zoom of a document, `1.0` being 100%, and each
/// application builds it inside its own range. `Scale<Board, View>` is the
/// zoom of a board's camera. `Scale<Css, Device>` is the display density,
/// which `ambre-canvas` alone reads (`CANVAS-031`). A length of `A` times the
/// scale is a length of `B`. A length of `B` divided by the scale is a length
/// of `A` again.
///
/// The scale converts between float spaces only. A conversion into an
/// integer space names its rounding at the call.
pub struct Scale<A: ScalesTo<B>, B: Space>(f64, PhantomData<fn() -> (A, B)>);

impl<A: ScalesTo<B>, B: Space> Scale<A, B> {
    /// One pixel of `B` per pixel of `A`.
    pub const ONE: Self = Self(1.0, PhantomData);

    /// `z` clamped into `lo ..= hi`. A NaN reads as 1, clamped the same way.
    /// A bound that is not positive and finite is pulled back to one that is.
    #[must_use]
    pub const fn within(z: f64, lo: f64, hi: f64) -> Self {
        let lo = if lo.is_finite() {
            lo.max(f64::MIN_POSITIVE)
        } else {
            f64::MIN_POSITIVE
        };
        let hi = if hi.is_finite() { hi.max(lo) } else { lo };
        let z = if z.is_nan() { 1.0 } else { z };
        Self(z.clamp(lo, hi), PhantomData)
    }

    /// `z` as a scale with no range of its own beyond the invariant: finite
    /// and strictly positive. A NaN reads as 1.
    #[must_use]
    pub const fn positive(z: f64) -> Self {
        Self::within(z, f64::MIN_POSITIVE, f64::MAX)
    }

    /// The scale `z`, or `None` when it is not a positive finite number.
    #[must_use]
    pub fn new(z: f64) -> Option<Self> {
        (z.is_finite() && z > 0.0).then_some(Self(z, PhantomData))
    }

    /// This scale clamped into `lo ..= hi`.
    #[must_use]
    pub const fn clamped(self, lo: Self, hi: Self) -> Self {
        Self::within(self.0, lo.0, hi.0)
    }

    /// The scale as a number, for the arithmetic a foreign API or a
    /// rounding into an integer space takes.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Scale<Board, View> {
    /// The board camera's zoom, read as the zoom of a text a board lays out.
    ///
    /// The shared text engine measures in document pixels, and a board text
    /// lays out at one document pixel per board unit. Under that convention
    /// the camera scales the text as it scales the board. This method is the
    /// one place the two zooms meet.
    #[must_use]
    pub const fn text_zoom(self) -> Scale<Doc, View> {
        Scale(self.0, PhantomData)
    }
}

// By hand, as every impl below: a derive would ask the two markers for a
// trait they have no use for.
impl<A: ScalesTo<B>, B: Space> Clone for Scale<A, B> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A: ScalesTo<B>, B: Space> Copy for Scale<A, B> {}

impl<A: ScalesTo<B>, B: Space> PartialEq for Scale<A, B> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<A: ScalesTo<B>, B: Space> PartialOrd for Scale<A, B> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

/// The two spaces and the number, as `Scale<Doc, View>(2.0)`.
impl<A: ScalesTo<B>, B: Space> std::fmt::Debug for Scale<A, B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Scale<{}, {}>({:?})", A::NAME, B::NAME, self.0)
    }
}

impl<A: ScalesTo<B>, B: Space> Default for Scale<A, B> {
    fn default() -> Self {
        Self::ONE
    }
}

#[cfg(test)]
mod tests {
    use super::Scale;
    use crate::space::{Board, Css, Device, Doc, Surface, View};
    use crate::unit::Px;

    /// A scale built with no range of its own still holds the invariant, and
    /// a scale clamps into a range named by two scales.
    #[test]
    fn a_positive_scale_stays_finite_and_clamps_into_a_range() {
        assert!(Scale::<Doc, View>::positive(0.0).get() > 0.0);
        assert!(
            Scale::<Doc, View>::positive(f64::INFINITY)
                .get()
                .is_finite()
        );
        assert_eq!(Scale::<Doc, View>::positive(f64::NAN), Scale::ONE);
        let lo = Scale::<Doc, View>::positive(0.1);
        let hi = Scale::positive(10.0);
        assert_eq!(Scale::positive(40.0).clamped(lo, hi), hi);
    }

    /// A value the browser could give that is no density builds no scale.
    #[test]
    fn a_scale_that_is_not_positive_and_finite_is_refused() {
        for z in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(Scale::<Css, Device>::new(z), None);
        }
        assert_eq!(Scale::<Css, Device>::new(2.0).map(Scale::get), Some(2.0));
    }

    /// A length scaled there and back is the length again.
    #[test]
    fn a_length_scales_into_the_other_space_and_back() {
        let zoom = Scale::<Doc, View>::positive(2.0);
        assert_eq!(Px::<Doc>::new(12.0) * zoom, Px::<View>::new(24.0));
        assert_eq!(Px::<View>::new(24.0) / zoom, Px::<Doc>::new(12.0));
    }

    /// The board camera carries a length both ways with the bits the bare
    /// `f64` arithmetic gave before the scale was typed.
    #[test]
    fn a_board_length_scales_into_the_view_with_the_bits_of_the_bare_arithmetic() {
        for (len, z) in [(3.0_f64, 2.0_f64), (0.1, 0.3), (12_345.678, 9.99)] {
            let camera = Scale::<Board, View>::positive(z);
            assert_eq!(
                Px::<Board>::new(len) * camera,
                Px::<View>::new((len * z) as f32)
            );
            assert_eq!(
                Px::<Board>::new(len).zoomed_on_surface(camera),
                Px::<Surface>::new(len * z)
            );
            let screen = Px::<View>::new(len as f32);
            assert_eq!(
                screen / camera,
                Px::<Board>::new(f64::from(screen.get()) / z)
            );
        }
    }

    /// A board text lays out at the camera's zoom, read as a document zoom.
    #[test]
    fn the_text_zoom_of_a_board_holds_the_camera_number() {
        let camera = Scale::<Board, View>::within(4.7, 0.1, 10.0);
        assert_eq!(camera.text_zoom().get(), camera.get());
        assert_eq!(
            Px::<Doc>::new(14.0).zoomed(camera.text_zoom()),
            Px::<View>::new(14.0 * 4.7_f64 as f32)
        );
    }
}
