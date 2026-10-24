//! Font attributes every application shares.
//!
//! A font size travels between the threads in points, and a renderer lays
//! text out in CSS pixels. A bare number lets an application hand points to
//! the layout engine as pixels. The same picker value then draws at two
//! heights in two applications. [`FontSize`] holds points only, and
//! [`FontSize::to_px`] is the one conversion.

use serde::{Deserialize, Serialize};

/// CSS pixels per typographic point: a point is 1/72 inch, a CSS pixel is
/// 1/96 inch.
pub(crate) const PX_PER_PT: f32 = 96.0 / 72.0;

/// A font size in typographic points.
///
/// The value is finite and never negative. Zero is a valid size, and an
/// application that means "automatic" says so in a type of its own rather
/// than in a zero. The type serializes as the bare `f32` it wraps, and a
/// decoded value outside that range is refused.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct FontSize(f32);

// Every constructor refuses NaN, so the equality is total.
impl Eq for FontSize {}

impl FontSize {
    /// The size of `points` whole points.
    #[must_use]
    pub const fn from_whole_points(points: u16) -> Self {
        Self(points as f32)
    }

    /// The size of `points` points, or `None` when the value is not finite or
    /// is negative.
    #[must_use]
    pub fn new(points: f32) -> Option<Self> {
        (points.is_finite() && points >= 0.0).then_some(Self(points))
    }

    /// The size in points.
    #[must_use]
    pub const fn points(self) -> f32 {
        self.0
    }

    /// The size in CSS pixels, the unit the layout engine reads.
    #[must_use]
    pub fn to_px(self) -> f32 {
        self.0 * PX_PER_PT
    }

    /// The size rounded to whole points, as a picker of whole points lists it.
    /// The cast saturates, so a size past `u16::MAX` reads as `u16::MAX`.
    #[must_use]
    pub fn whole_points(self) -> u16 {
        self.0.round() as u16
    }
}

/// The error of a decoded font size that is not finite or is negative.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("a font size must be a finite, non-negative number of points, not {0}")]
pub struct InvalidFontSize(pub f32);

impl std::fmt::Display for FontSize {
    /// The number of points, without a unit: `12` for twelve points.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<f32> for FontSize {
    type Error = InvalidFontSize;

    fn try_from(points: f32) -> Result<Self, Self::Error> {
        Self::new(points).ok_or(InvalidFontSize(points))
    }
}

impl From<FontSize> for f32 {
    fn from(size: FontSize) -> Self {
        size.0
    }
}

/// A serde adapter for an `Option<FontSize>` stored as whole points, a
/// `u16`, for a record whose layout predates [`FontSize`].
///
/// Use it with `#[serde(with = "ambre_core::font::whole_points")]`. A size
/// with a fraction is stored rounded, as [`FontSize::whole_points`] rounds it.
pub mod whole_points {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::FontSize;

    /// Write the size as its whole points.
    ///
    /// # Errors
    /// The serializer's own error.
    pub fn serialize<S: Serializer>(size: &Option<FontSize>, out: S) -> Result<S::Ok, S::Error> {
        size.map(FontSize::whole_points).serialize(out)
    }

    /// Read a size stored as whole points.
    ///
    /// # Errors
    /// The deserializer's own error.
    pub fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<Option<FontSize>, D::Error> {
        Ok(Option::<u16>::deserialize(input)?.map(FontSize::from_whole_points))
    }
}

#[cfg(test)]
mod tests {
    use super::FontSize;

    #[test]
    fn twelve_points_are_sixteen_pixels() {
        assert_eq!(FontSize::from_whole_points(12).to_px(), 16.0);
        assert_eq!(FontSize::from_whole_points(9).to_px(), 12.0);
    }

    #[test]
    fn a_size_outside_the_range_is_refused() {
        assert_eq!(FontSize::new(f32::NAN), None);
        assert_eq!(FontSize::new(f32::INFINITY), None);
        assert_eq!(FontSize::new(-1.0), None);
        assert_eq!(FontSize::new(0.0), Some(FontSize::from_whole_points(0)));
    }

    #[test]
    fn a_size_serializes_as_its_points() {
        let size = FontSize::from_whole_points(14);
        let bytes = postcard::to_allocvec(&size).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&14.0_f32).unwrap());
        assert_eq!(postcard::from_bytes::<FontSize>(&bytes).unwrap(), size);
        let nan = postcard::to_allocvec(&f32::NAN).unwrap();
        assert!(postcard::from_bytes::<FontSize>(&nan).is_err());
    }

    #[test]
    fn a_size_in_whole_points_keeps_the_bytes_of_a_u16() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Stored(#[serde(with = "super::whole_points")] Option<FontSize>);
        let stored = Stored(Some(FontSize::from_whole_points(14)));
        let bytes = postcard::to_allocvec(&stored).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&Some(14_u16)).unwrap());
        assert_eq!(postcard::from_bytes::<Stored>(&bytes).unwrap(), stored);
    }
}
