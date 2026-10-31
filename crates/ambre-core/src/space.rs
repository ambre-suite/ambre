//! The pixel spaces, one marker type each, and the number each one holds.
//!
//! A [`Px`](crate::unit::Px) is a length in one space, and the space is its
//! type parameter. Two lengths of two spaces never mix, and the compiler
//! refuses the slip. Each space fixes one representation, so a space never
//! holds two kinds of number:
//!
//! | Space | Number | Example |
//! | :--- | :--- | :--- |
//! | [`Doc`] | `f32` | a column width, a run offset |
//! | [`View`] | `f32` | a pointer, a caret |
//! | [`Css`] | `f32` | a wheel delta, a fixed element |
//! | [`Surface`] | `f64` | an absolute sheet position |
//! | [`WholeCss`] | `u32` | the CSS box of a canvas, a token length |
//! | [`Device`] | `i32` | a shift of the drawing buffer |
//! | [`Texel`] | `u32` | an edge of the drawing buffer, a glyph in the atlas |
//! | [`Image`] | `u32` | an edge of a decoded image |
//! | [`Parley`] | `f32` | a font size or a wrap width of a parley layout |
//! | [`WholeParley`] | `u16` | a paragraph indent |
//! | [`GlyphRaster`] | `f32` | the size a glyph is rasterized at |
//!
//! Every marker is an enum with no variant: it names a space and holds no
//! value.

use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use serde::Serialize;
use serde::de::DeserializeOwned;

mod sealed {
    /// Closes [`super::Scalar`] and [`super::Rescale`] to the numbers this
    /// module lists.
    pub trait Sealed {}

    impl Sealed for f32 {}
    impl Sealed for f64 {}
    impl Sealed for u32 {}
    impl Sealed for i32 {}
    impl Sealed for u16 {}
}

/// The number a pixel space holds: a float or an integer of the list in
/// this module, and nothing else.
///
/// The trait is sealed. A space picks its number from the list, so a length
/// never holds a type the operators of `Px` do not know.
pub trait Scalar:
    sealed::Sealed
    + Copy
    + Default
    + PartialEq
    + PartialOrd
    + fmt::Debug
    + fmt::Display
    + Serialize
    + DeserializeOwned
    + Add<Output = Self>
    + Sub<Output = Self>
    + AddAssign
    + SubAssign
    + Sum
    + 'static
{
    /// The number zero.
    const ZERO: Self;

    /// The smaller of the two numbers. For a float, a NaN loses to a number.
    fn lesser(self, other: Self) -> Self;

    /// The larger of the two numbers. For a float, a NaN loses to a number.
    fn greater(self, other: Self) -> Self;
}

/// A float scalar: what a length that rounds, takes an absolute value or
/// divides into a ratio holds.
pub trait Float: Scalar + Neg<Output = Self> + Mul<Output = Self> + Div<Output = Self> {
    /// The number without its sign.
    fn magnitude(self) -> Self;

    /// The number rounded to the nearest whole value, half away from zero.
    fn nearest(self) -> Self;

    /// The number rounded toward negative infinity.
    fn rounded_down(self) -> Self;

    /// Whether the number is neither infinite nor NaN.
    fn finite(self) -> bool;
}

/// A conversion from one float scalar to another through a factor: what
/// a `Scale` applies to a length.
///
/// The product is computed in the precision a length of the two spaces had
/// before the generic type, so a scaled length keeps its bits.
pub trait Rescale<To>: sealed::Sealed {
    /// The number times `factor`, in the target scalar.
    fn scaled(self, factor: f64) -> To;

    /// The number divided by `factor`, in the target scalar.
    fn unscaled(self, factor: f64) -> To;
}

macro_rules! scalar_float {
    ($t:ty) => {
        impl Scalar for $t {
            const ZERO: Self = 0.0;

            fn lesser(self, other: Self) -> Self {
                self.min(other)
            }

            fn greater(self, other: Self) -> Self {
                self.max(other)
            }
        }

        impl Float for $t {
            fn magnitude(self) -> Self {
                self.abs()
            }

            fn nearest(self) -> Self {
                self.round()
            }

            fn rounded_down(self) -> Self {
                self.floor()
            }

            fn finite(self) -> bool {
                self.is_finite()
            }
        }
    };
}

macro_rules! scalar_integer {
    ($t:ty) => {
        impl Scalar for $t {
            const ZERO: Self = 0;

            fn lesser(self, other: Self) -> Self {
                Ord::min(self, other)
            }

            fn greater(self, other: Self) -> Self {
                Ord::max(self, other)
            }
        }
    };
}

scalar_float!(f32);
scalar_float!(f64);
scalar_integer!(u32);
scalar_integer!(i32);
scalar_integer!(u16);

// The zoom is an `f64`, and an `f32` length scaled by it stays an `f32`
// product, as it was before the generic type.
#[allow(clippy::cast_possible_truncation)]
impl Rescale<f32> for f32 {
    fn scaled(self, factor: f64) -> f32 {
        self * factor as f32
    }

    fn unscaled(self, factor: f64) -> f32 {
        self / factor as f32
    }
}

impl Rescale<f64> for f32 {
    fn scaled(self, factor: f64) -> f64 {
        f64::from(self) * factor
    }

    fn unscaled(self, factor: f64) -> f64 {
        f64::from(self) / factor
    }
}

impl Rescale<f64> for f64 {
    fn scaled(self, factor: f64) -> f64 {
        self * factor
    }

    fn unscaled(self, factor: f64) -> f64 {
        self / factor
    }
}

#[allow(clippy::cast_possible_truncation)]
impl Rescale<f32> for f64 {
    fn scaled(self, factor: f64) -> f32 {
        (self * factor) as f32
    }

    fn unscaled(self, factor: f64) -> f32 {
        (self / factor) as f32
    }
}

/// A pixel space: the marker type a `Px` names, and the one number its
/// lengths hold.
pub trait Space: 'static {
    /// The number a length of this space holds.
    type Repr: Scalar;

    /// The name of the space, as `Debug` prints a length of it.
    const NAME: &'static str;
}

/// The pixels of the document, before the zoom: what a layout measures a
/// page, a column or a run in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Doc {}

impl Space for Doc {
    type Repr = f32;
    const NAME: &'static str = "Doc";
}

/// The pixels of the zoomed view, measured from the top-left corner of the
/// canvas.
///
/// A view pixel is a CSS pixel in size (`CANVAS-032`). Only the origin
/// differs from [`Css`]: the corner of the canvas, not the corner of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {}

impl Space for View {
    type Repr = f32;
    const NAME: &'static str = "View";
}

/// The CSS pixels of the page, outside any canvas: a wheel delta, or the
/// place of an element fixed on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Css {}

impl Space for Css {
    type Repr = f32;
    const NAME: &'static str = "Css";
}

/// The pixels of a drawing surface, in the `f64` the surface takes.
///
/// A sheet walks its cells at absolute positions, at zoom 1: a hundred
/// thousand rows of twenty pixels reach two million pixels. An `f32` steps
/// by a quarter pixel there, so the `f32` spaces cannot hold such a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Surface {}

impl Space for Surface {
    type Repr = f64;
    const NAME: &'static str = "Surface";
}

/// Whole CSS pixels: the CSS box of a canvas element as the browser reports
/// it, or a token length as the token source writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WholeCss {}

impl Space for WholeCss {
    type Repr = u32;
    const NAME: &'static str = "WholeCss";
}

/// Whole device pixels, signed: a shift of the drawing buffer.
///
/// Nothing turns a CSS or a view pixel into one but the density, which
/// `ambre-canvas` reads and keeps (`CANVAS-031`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Device {}

impl Space for Device {
    type Repr = i32;
    const NAME: &'static str = "Device";
}

/// The texels of a GPU texture: an edge of the drawing buffer, or the place
/// of a glyph in the atlas. The `u32` is what wgpu takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Texel {}

impl Space for Texel {
    type Repr = u32;
    const NAME: &'static str = "Texel";
}

/// The pixels of a decoded image: samples of its bitmap, never a length on a
/// screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Image {}

impl Space for Image {
    type Repr = u32;
    const NAME: &'static str = "Image";
}

/// The units of a parley layout, which are the units of its caller: view
/// pixels for a screen text, document pixels for a board text.
///
/// A drawn glyph run no longer knows which, so the units get a space of
/// their own. A font size, a wrap width and an indent of one layout share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Parley {}

impl Space for Parley {
    type Repr = f32;
    const NAME: &'static str = "Parley";
}

/// Whole units of the width a layout breaks to: a paragraph indent
/// (`CANVAS-084`). Whole, so a span keeps its `Eq` and its `Hash` in the
/// cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WholeParley {}

impl Space for WholeParley {
    type Repr = u16;
    const NAME: &'static str = "WholeParley";
}

/// The device pixels a glyph is rasterized at, as the float Swash takes. It
/// is the layout size, times the scale of the transform, times the density.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphRaster {}

impl Space for GlyphRaster {
    type Repr = f32;
    const NAME: &'static str = "GlyphRaster";
}
