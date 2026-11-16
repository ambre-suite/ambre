//! Font attributes every application shares.
//!
//! A font size travels between the threads in points, and a renderer lays
//! text out in CSS pixels. A bare number lets an application hand points to
//! the layout engine as pixels. The same picker value then draws at two
//! heights in two applications. [`FontSize`] holds points only, and
//! [`FontSize::to_px`] is the one conversion.

use serde::{Deserialize, Serialize};

use crate::bounded::{Bounded, Bounds, Defaulted, Refusal};
use crate::refused::{Refused, Refuses};
use crate::{Doc, Px};

/// CSS pixels per typographic point: a point is 1/72 inch, a CSS pixel is
/// 1/96 inch.
pub(crate) const PX_PER_PT: f32 = 96.0 / 72.0;

/// A font size in typographic points.
///
/// The value is finite, never negative, and at most [`FontSize::MAX`]. Zero
/// is a valid size, and an application that means "automatic" says so in a
/// type of its own rather than in a zero. The type serializes as the bare
/// `f32` it wraps. A decoded value that is not finite or is negative is
/// refused, and one past [`FontSize::MAX`] reads as it (`CANVAS-088`).
///
/// Each application caps its sizes lower, as its office counterpart does,
/// with [`FontSize::at_most`] and a cap of its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct FontSize(f32);

// Every constructor refuses NaN, so the equality is total.
impl Eq for FontSize {}

impl FontSize {
    /// The largest size of any application, 4000 points: the cap of the
    /// presentation formats, the highest office cap. A peer writes any number
    /// into a style, and the rasterizer allocates a glyph mask of about the
    /// size squared. The cap keeps that mask bounded (`CANVAS-088`).
    pub const MAX: Self = Self(4000.0);

    /// The size of `points` whole points, or [`FontSize::MAX`] past it.
    #[must_use]
    pub const fn from_whole_points(points: u16) -> Self {
        Self((points as f32).min(Self::MAX.0))
    }

    /// The size of `points` points, [`FontSize::MAX`] past it, or `None` when
    /// the value is not finite or is negative.
    ///
    /// A stored value past the cap reads as the cap rather than as an error,
    /// so a style an earlier build stored keeps decoding.
    #[must_use]
    pub fn new(points: f32) -> Option<Self> {
        (points.is_finite() && points >= 0.0).then_some(Self(points.min(Self::MAX.0)))
    }

    /// The size, or `cap` past it: how an application reads a stored,
    /// received or pasted size past its own cap (`CANVAS-088`).
    #[must_use]
    pub const fn at_most(self, cap: Self) -> Self {
        Self(self.0.min(cap.0))
    }

    /// The size in points.
    #[must_use]
    pub const fn points(self) -> f32 {
        self.0
    }

    /// The size in document pixels, the unit the layout engine reads.
    #[must_use]
    pub fn to_px(self) -> Px<Doc> {
        Px::<Doc>::new(self.0 * PX_PER_PT)
    }

    /// The size drawn at `px` document pixels, the inverse of
    /// [`FontSize::to_px`]. `None` as [`FontSize::new`] answers it.
    #[must_use]
    pub fn from_px(px: Px<Doc>) -> Option<Self> {
        Self::new(px.get() / PX_PER_PT)
    }

    /// The size rounded to whole points, as a picker of whole points lists it.
    #[must_use]
    pub fn whole_points(self) -> u16 {
        self.0.round() as u16
    }
}

/// The points of a font size refused for not being finite or for being
/// negative: the marker of its [`Refused<FontPoints>`], which echoes them.
///
/// The error is no [`OutOfRange`](crate::OutOfRange): [`FontSize`] reads a
/// value past its cap as the cap, so no upper bound refuses one.
pub enum FontPoints {}

impl Refuses for FontPoints {
    type Raw = f32;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("InvalidFontSize");

    fn message(raw: &Self::Raw, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a font size must be a finite, non-negative number of points, not {raw}"
        )
    }
}

impl std::fmt::Display for FontSize {
    /// The number of points, without a unit: `12` for twelve points.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<f32> for FontSize {
    type Error = Refused<FontPoints>;

    fn try_from(points: f32) -> Result<Self, Self::Error> {
        Self::new(points).ok_or(Refused::new(points))
    }
}

impl From<FontSize> for f32 {
    fn from(size: FontSize) -> Self {
        size.0
    }
}

/// A font weight on the CSS scale, from 1 to 1000: 400 is regular, 700 is
/// bold.
///
/// The type serializes as the bare `u16` it wraps, and a decoded weight off
/// the scale is refused.
pub enum FontWeight {}

impl Bounds for FontWeight {
    type Repr = u16;
    const MIN: u16 = 1;
    const MAX: u16 = 1000;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("FontWeight");
    const REFUSAL: Refusal =
        Refusal::new("a font weight lies on the CSS scale of 1 to 1000, not ", "");
}

impl Defaulted for FontWeight {
    const DEFAULT: Bounded<Self> = Bounded::<Self>::REGULAR;
}

impl Bounded<FontWeight> {
    /// The regular weight, 400.
    pub const REGULAR: Self = Self::at(400);
    /// The bold weight, 700.
    pub const BOLD: Self = Self::at(700);

    /// The weight on the CSS scale a face of `weight` sets: what a bold toggle
    /// writes.
    #[must_use]
    pub const fn of(weight: Weight) -> Self {
        match weight {
            Weight::Regular => Self::REGULAR,
            Weight::Bold => Self::BOLD,
        }
    }

    /// Whether the weight draws as bold: 700 or heavier.
    #[must_use]
    pub const fn is_bold(self) -> bool {
        self.get() >= Self::BOLD.get()
    }
}

crate::define_flag! {
    /// The weight of a text's face: the family's regular face or its real
    /// bold. A record that stores a bold toggle keeps the byte of its `bool`.
    /// A record that stores a weight on the CSS scale holds a [`Bounded<FontWeight>`].
    pub enum Weight {
        /// The family's regular face, 400 on the CSS scale.
        Regular,
        /// The family's real bold face, 700 on the CSS scale.
        Bold,
    }

    /// The slant of a text's face: upright, or the family's real italic.
    pub enum Slant {
        /// The family's upright face.
        Upright,
        /// The family's real italic face, never a sheared upright one.
        Italic,
    }

    /// Where a line may break: between words only, or within a word too, so
    /// a long unbreakable token wraps instead of overflowing.
    pub enum WordBreak {
        /// A line breaks between words only, and a long word overflows.
        Whole,
        /// A line may break inside a word as well, where it would overflow.
        Anywhere,
    }

    /// Whether a run of text is underlined.
    pub enum Underline {
        Plain,
        /// A line just below the baseline.
        Underlined,
    }

    /// Whether a run of text is struck through.
    pub enum Strikethrough {
        Plain,
        /// A line through the middle of the glyphs.
        Struck,
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

/// The family of a text body, with one spelling per bundled family.
///
/// A family the picker offers reads as its variant, whatever spelling wrote
/// it, so two elements on one face compare equal. A name the picker does not
/// offer, such as one an imported file carried, is kept as `Other`. The type
/// travels and is stored as its label.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum FontFamily {
    /// The default face, the bundled sans.
    #[default]
    Sans,
    Serif,
    Mono,
    /// A family no bundled face is named for, kept as written.
    Other(String),
}

impl FontFamily {
    /// The family `label` names. The picker's labels, the CSS generics and
    /// the bundled face names all read as their family; a blank label reads
    /// as the default.
    #[must_use]
    pub fn from_label(label: &str) -> Self {
        match label {
            "" | "Default" | "Sans" | "sans-serif" | "Noto Sans" => Self::Sans,
            "Serif" | "serif" | "DejaVu Serif" => Self::Serif,
            "Monospace" | "monospace" | "DejaVu Sans Mono" => Self::Mono,
            other => Self::Other(other.to_owned()),
        }
    }

    /// The label the family is stored under, and the one the renderer
    /// resolves to a face. The default keeps the CSS generic it was always
    /// stored as, so a board written before reads the same bytes.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Sans => "sans-serif",
            Self::Other(name) => name,
            Self::Serif => "Serif",
            Self::Mono => "Monospace",
        }
    }

    /// The label the font picker shows for the family, or `None` for a
    /// family it does not offer. A family named Sans shows as "Sans": the
    /// "Default" entry is for a style that names no family, which only the
    /// caller can tell.
    #[must_use]
    pub fn picker_label(&self) -> Option<&'static str> {
        match self {
            Self::Sans => Some("Sans"),
            Self::Serif => Some("Serif"),
            Self::Mono => Some("Monospace"),
            Self::Other(_) => None,
        }
    }
}

impl From<String> for FontFamily {
    fn from(label: String) -> Self {
        Self::from_label(&label)
    }
}

impl From<FontFamily> for String {
    fn from(family: FontFamily) -> Self {
        match family {
            FontFamily::Other(name) => name,
            known => known.label().to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FontFamily, FontSize, FontWeight, Weight};
    use crate::{Bounded, Doc, Px};

    /// A weight lies on the CSS scale, and bold starts at 700.
    #[test]
    fn a_font_weight_lies_on_the_css_scale() {
        assert_eq!(Bounded::<FontWeight>::new(0), None);
        assert_eq!(Bounded::<FontWeight>::new(1001), None);
        assert_eq!(
            Bounded::<FontWeight>::new(700),
            Some(Bounded::<FontWeight>::BOLD)
        );
        assert!(Bounded::<FontWeight>::new(800).is_some_and(Bounded::<FontWeight>::is_bold));
        assert!(!Bounded::<FontWeight>::new(600).is_some_and(Bounded::<FontWeight>::is_bold));
        assert_eq!(
            Bounded::<FontWeight>::of(Weight::Regular),
            Bounded::<FontWeight>::REGULAR
        );
        assert_eq!(
            Bounded::<FontWeight>::of(Weight::Bold),
            Bounded::<FontWeight>::BOLD
        );
        assert!(Bounded::<FontWeight>::try_new(0).is_err());
    }

    #[test]
    fn twelve_points_are_sixteen_pixels() {
        assert_eq!(
            FontSize::from_whole_points(12).to_px(),
            Px::<Doc>::new(16.0)
        );
        assert_eq!(FontSize::from_whole_points(9).to_px(), Px::<Doc>::new(12.0));
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
        let refused = FontSize::try_from(-2.0).expect_err("negative");
        assert_eq!(refused.get(), -2.0);
        assert_eq!(
            refused.to_string(),
            "a font size must be a finite, non-negative number of points, not -2"
        );
        assert_eq!(format!("{refused:?}"), "InvalidFontSize(-2.0)");
    }

    /// A stored size past the cap reads as the cap (`CANVAS-088`). A peer
    /// cannot ask for a glyph a million points high, and an earlier style
    /// still decodes.
    #[test]
    fn a_size_past_the_cap_reads_as_the_cap() {
        let huge = postcard::to_allocvec(&1.0e6_f32).unwrap();
        assert_eq!(postcard::from_bytes::<FontSize>(&huge), Ok(FontSize::MAX));
        assert_eq!(FontSize::MAX.points(), 4000.0);
        assert_eq!(FontSize::new(1.0e6), Some(FontSize::MAX));
        assert_eq!(FontSize::from_whole_points(u16::MAX), FontSize::MAX);
        let at_cap = postcard::to_allocvec(&FontSize::MAX).unwrap();
        assert_eq!(at_cap, postcard::to_allocvec(&4000.0_f32).unwrap());
        let cap = FontSize::from_whole_points(409);
        assert_eq!(FontSize::from_whole_points(410).at_most(cap), cap);
        let under = FontSize::from_whole_points(408);
        assert_eq!(under.at_most(cap), under);
        let in_range = postcard::to_allocvec(&7.5_f32).unwrap();
        let size = postcard::from_bytes::<FontSize>(&in_range).unwrap();
        assert_eq!(size.points(), 7.5);
        assert_eq!(postcard::to_allocvec(&size).unwrap(), in_range);
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

    #[test]
    fn every_spelling_of_a_family_reads_as_it_and_stores_one() {
        for label in ["", "Default", "Sans", "sans-serif", "Noto Sans"] {
            assert_eq!(FontFamily::from_label(label), FontFamily::Sans, "{label:?}");
        }
        assert_eq!(FontFamily::from_label("serif"), FontFamily::Serif);
        assert_eq!(FontFamily::from_label("Monospace"), FontFamily::Mono);
        assert_eq!(FontFamily::Sans.picker_label(), Some("Sans"));
        assert_eq!(FontFamily::Serif.label(), "Serif");
        assert_eq!(FontFamily::Mono.label(), "Monospace");
        let other = FontFamily::from_label("Comic Sans MS");
        assert_eq!(other.picker_label(), None);
        assert_eq!(other.label(), "Comic Sans MS");
        let bytes = postcard::to_allocvec(&FontFamily::Sans).unwrap();
        assert_eq!(bytes, postcard::to_allocvec("sans-serif").unwrap());
        for family in [FontFamily::Sans, FontFamily::Serif, FontFamily::Mono, other] {
            let bytes = postcard::to_allocvec(&family).unwrap();
            assert_eq!(postcard::from_bytes::<FontFamily>(&bytes).unwrap(), family);
        }
    }
}
