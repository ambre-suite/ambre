//! The zoom factor every canvas application scales by.

/// A zoom factor, `1.0` being 100%: finite and strictly positive, so a reader
/// may divide by it. Each application builds it inside its own range.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ZoomFactor(f64);

impl ZoomFactor {
    /// 100%.
    pub const ONE: Self = Self(1.0);

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
        Self(z.clamp(lo, hi))
    }

    /// `z` as a factor with no range of its own beyond the invariant: finite
    /// and strictly positive. A NaN reads as 1.
    #[must_use]
    pub const fn positive(z: f64) -> Self {
        Self::within(z, f64::MIN_POSITIVE, f64::MAX)
    }

    /// This factor clamped into `lo ..= hi`.
    #[must_use]
    pub const fn clamped(self, lo: Self, hi: Self) -> Self {
        Self::within(self.0, lo.0, hi.0)
    }

    /// The factor as a number, `1.0` being 100%.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Default for ZoomFactor {
    fn default() -> Self {
        Self::ONE
    }
}

#[cfg(test)]
mod tests {
    use super::ZoomFactor;

    /// A factor built with no range of its own still holds the invariant, and
    /// a factor clamps into a range named by two factors.
    #[test]
    fn a_positive_zoom_factor_stays_finite_and_clamps_into_a_range() {
        assert!(ZoomFactor::positive(0.0).get() > 0.0);
        assert!(ZoomFactor::positive(f64::INFINITY).get().is_finite());
        assert_eq!(ZoomFactor::positive(f64::NAN), ZoomFactor::ONE);
        let lo = ZoomFactor::positive(0.1);
        let hi = ZoomFactor::positive(10.0);
        assert_eq!(ZoomFactor::positive(40.0).clamped(lo, hi), hi);
    }
}
