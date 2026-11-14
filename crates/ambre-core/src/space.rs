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
//! | [`Board`] | `f64` | a frame's padding, a hit reach on a board |
//! | [`StoredBoard`] | `f32` | a freehand sample, a stroke width |
//! | [`CanvasCss`] | `u32` | the CSS box of a canvas element |
//! | [`Device`] | `i32` | a shift of the drawing buffer |
//! | [`Texel`] | `u32` | an edge of the drawing buffer, a glyph in the atlas |
//! | [`Image`] | `u32` | an edge of a decoded image |
//!
//! Every marker is an enum with no variant: it names a space and holds no
//! value. `Space` is open, so a crate may declare a space of its own.
//! `ambre-canvas` declares the spaces of a parley layout and of a glyph
//! raster. `Scalar` is sealed here, so the list holds each number those
//! spaces need.

use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use serde::Serialize;
use serde::de::DeserializeOwned;

mod sealed {
    /// Closes [`super::Scalar`] to the numbers this module lists. It also
    /// closes the source number of [`super::Rescale`], though not its target.
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

impl Scalar for f32 {
    const ZERO: Self = 0.0;

    fn lesser(self, other: Self) -> Self {
        self.min(other)
    }

    fn greater(self, other: Self) -> Self {
        self.max(other)
    }
}

impl Float for f32 {
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

impl Scalar for f64 {
    const ZERO: Self = 0.0;

    fn lesser(self, other: Self) -> Self {
        self.min(other)
    }

    fn greater(self, other: Self) -> Self {
        self.max(other)
    }
}

impl Float for f64 {
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

impl Scalar for u32 {
    const ZERO: Self = 0;

    fn lesser(self, other: Self) -> Self {
        Ord::min(self, other)
    }

    fn greater(self, other: Self) -> Self {
        Ord::max(self, other)
    }
}

impl Scalar for i32 {
    const ZERO: Self = 0;

    fn lesser(self, other: Self) -> Self {
        Ord::min(self, other)
    }

    fn greater(self, other: Self) -> Self {
        Ord::max(self, other)
    }
}

/// No space of this crate holds a `u16`. The paragraph indent of
/// `ambre-canvas` does, and the seal stops that crate from adding the impl.
impl Scalar for u16 {
    const ZERO: Self = 0;

    fn lesser(self, other: Self) -> Self {
        Ord::min(self, other)
    }

    fn greater(self, other: Self) -> Self {
        Ord::max(self, other)
    }
}

// The zoom is an `f64`, and an `f32` length scaled by it stays an `f32`
// product, as it was before the generic type.
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

// A board length is an `f64`, and its view length the `f32` a view holds.
// The product rounds once, after the `f64` multiplication, as the camera
// computed it before the generic type.
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
    const NAME: crate::StaticKey<crate::DebugNames>;
}

/// A pair of pixel spaces that a [`Scale`](crate::scale::Scale) joins: a
/// length of `Self` times the scale is a length of `B`.
///
/// The pair is directed and named one by one, never derived from the two
/// ends. Three pairs exist:
///
/// | Pair | Scale |
/// | :--- | :--- |
/// | [`Doc`] to [`View`] | the zoom of a document |
/// | [`Css`] to [`Device`] | the display density (`CANVAS-031`) |
/// | [`Board`] to [`View`] | the zoom of a board's camera |
///
/// A space that joins some scale still joins no other. A view length read as
/// a document length, or a board length read as a document length, does not
/// compile:
///
/// ```compile_fail,E0277
/// use ambre_core::{Doc, Px, Scale, View};
/// let _ = Px::<View>::new(4.0) * Scale::<View, Doc>::ONE;
/// ```
///
/// ```compile_fail,E0277
/// use ambre_core::{Board, Px, Scale, View};
/// let _: Px<Board> = Px::<View>::new(4.0) * Scale::<View, Board>::ONE;
/// ```
///
/// ```compile_fail,E0277
/// use ambre_core::{Px, Scale, StoredBoard, View};
/// let _: Px<View> = Px::<StoredBoard>::new(4.0) * Scale::<StoredBoard, View>::ONE;
/// ```
///
/// ```compile_fail,E0277
/// use ambre_core::{Css, Device, Scale};
/// let _ = Scale::<Device, Css>::ONE;
/// ```
///
/// ```compile_fail,E0308
/// use ambre_core::{Board, Doc, Px, Scale, View};
/// let _: Px<View> = Px::<Doc>::new(4.0).zoomed(Scale::<Board, View>::ONE);
/// ```
///
/// Each declared pair builds its scale:
///
/// ```
/// use ambre_core::{Board, Css, Device, Doc, Px, Scale, View};
/// let zoom = Scale::<Doc, View>::positive(2.0);
/// assert_eq!(Px::<Doc>::new(3.0) * zoom, Px::<View>::new(6.0));
/// let camera = Scale::<Board, View>::positive(2.0);
/// assert_eq!(Px::<Board>::new(3.0) * camera, Px::<View>::new(6.0));
/// assert_eq!(Px::<View>::new(6.0) / camera, Px::<Board>::new(3.0));
/// assert_eq!(Scale::<Css, Device>::new(2.0).map(Scale::get), Some(2.0));
/// ```
///
/// The trait is sealed: a pair is a decision of this crate. A crate that
/// declares a space of its own could otherwise join it to [`View`], and the
/// slip would compile again.
pub trait ScalesTo<B: Space>: Space + pair::Sealed<B> {}

mod pair {
    /// Closes [`super::ScalesTo`] to the pairs this module lists.
    pub trait Sealed<B> {}

    impl Sealed<super::View> for super::Doc {}
    impl Sealed<super::Device> for super::Css {}
    impl Sealed<super::View> for super::Board {}
}

impl ScalesTo<View> for Doc {}
impl ScalesTo<Device> for Css {}
impl ScalesTo<View> for Board {}

/// The pixels of the document, before the zoom: what a layout measures a
/// page, a column or a run in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Doc {}

impl Space for Doc {
    type Repr = f32;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Doc");
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
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("View");
}

/// The CSS pixels of the page, outside any canvas: a wheel delta, or the
/// place of an element fixed on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Css {}

impl Space for Css {
    type Repr = f32;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Css");
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
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Surface");
}

/// The whole CSS pixels of the box of a canvas element, as the browser
/// reports it. A view lays out against that box (`CANVAS-044`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanvasCss {}

impl Space for CanvasCss {
    type Repr = u32;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("CanvasCss");
}

/// Whole device pixels, signed: a shift of the drawing buffer.
///
/// Nothing turns a CSS or a view pixel into one but the density, which
/// `ambre-canvas` reads and keeps (`CANVAS-031`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Device {}

impl Space for Device {
    type Repr = i32;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Device");
}

/// The texels of a GPU texture: an edge of the drawing buffer, or the place
/// of a glyph in the atlas. The `u32` is what wgpu takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Texel {}

impl Space for Texel {
    type Repr = u32;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Texel");
}

/// The pixels of a decoded image: samples of its bitmap, never a length on a
/// screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Image {}

impl Space for Image {
    type Repr = u32;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Image");
}

/// The world units of a board, before the zoom: what a board's camera scales
/// into view pixels.
///
/// A view length and a board length are both floats. A reach of four view
/// pixels taken as four board units picks wrong at every zoom but 1. The
/// camera's scale, a `Scale<Board, View>`, stands between the two
/// ([`ScalesTo`]). The `f64` is the one kurbo measures a board in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Board {}

impl Space for Board {
    type Repr = f64;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Board");
}

/// The world units of a board, held in the `f32` that an element stores.
///
/// A freehand sample keeps its coordinates in it, so a sample stays twelve
/// bytes. A style keeps its stroke width and its corner radius in it. The
/// camera scales it with the shape, and a screen length meets it only through
/// the camera: the space joins no scale ([`ScalesTo`]). `Px::widened` reads it
/// as the [`Board`] length the board computes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoredBoard {}

impl Space for StoredBoard {
    type Repr = f32;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("StoredBoard");
}
